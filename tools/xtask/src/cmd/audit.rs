//! Operator audits of library code: a `syn` scan of every crate's module tree that finds the
//! sites of chosen operators, classifies their right operand and reads the proof comment next to
//! each site.
//!
//! Shared by `cargo xtask shifts` (R5.S09: shift amounts, `// SHIFT:` proofs), `cargo xtask
//! divisions` (R5.S10: divisors, `// DIV:` proofs) and `cargo xtask guidelines` (the register
//! scan that runs both): an audit chooses its [`Op`]s and a proof tag; file discovery, test-code
//! exclusion, operand classification and proof lookup stay here.
//!
//! **Divisors** ([`scan_divisions`]) are classified beyond literal / constant / run-time: a
//! divisor whose value is non-zero by construction is recognised without a proof comment —
//! a `NonZero*` value ([`Operand::NonZero`]: a parameter, `let` or struct field declared with a
//! `NonZero*` type, its `.get()`, `NonZero*::get(..)` or a lossless `from` of it) and a guard
//! ([`Operand::Guarded`]: `.max(n)` or `.clamp(n, ..)` with a literal `n ≥ 1`, directly or through an
//! immutable `let`). Floating-point divisions ([`Operand::Float`], a float literal, an `as f32`/`as
//! f64` cast or a value declared `f32`/`f64` on either side) cannot panic and are reported apart.
//! A cast of a guarded or `NonZero` value is *not* recognised (a narrowing cast can make it zero).
//!
//! **What is scanned.** Every crate under `crates/` that has a `src/lib.rs` (profile crates, the
//! non-profile ones and the out-of-workspace `twine-esp`), following the module tree from
//! `lib.rs` (`mod x;` → `x.rs` / `x/mod.rs`, `#[path]` honoured), so only compiled files are read.
//! Left out: `#[cfg(test)]` items and modules, `#[test]` functions, and compile-time contexts
//! (`const`/`static` initializers, associated consts, enum discriminants, array lengths, inline
//! `const { .. }` blocks), where an out-of-range operand is a compile error.
//!
//! **Macros.** A macro call whose input parses as comma-separated expressions (`format!`,
//! `vec!`, `debug_assert!`, most local macros) is scanned as code. Other macro input and the
//! bodies of `macro_rules!` definitions are scanned token by token (an operator token pair whose
//! next token is a literal counts as literal); such sites are marked `in_macro` and need the same
//! proof, which keeps the audit conservative.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Spacing, Span, TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{Attribute, BinOp, Expr, Lit, Token};

use crate::util::workspace_root;

/// An audited operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Op {
    /// `a << b`, `a <<= b`, `a.shl(b)`, `a.wrapping_shl(b)`, `a.overflowing_shl(b)`.
    Shl,
    /// `a >> b`, `a >>= b`, `a.shr(b)`, `a.wrapping_shr(b)`, `a.overflowing_shr(b)`.
    Shr,
    /// `a / b`, `a /= b`, `a.div(b)` and the panicking division methods (`div_euclid`,
    /// `div_ceil`, `wrapping_div*`, `overflowing_div*`, `saturating_div`).
    Div,
    /// `a % b`, `a %= b`, `a.rem(b)`, `rem_euclid`, `wrapping_rem*`, `overflowing_rem*`.
    Rem,
}

impl Op {
    /// The operator of a binary expression (plain or compound assignment).
    fn of_bin(op: &BinOp) -> Option<Op> {
        match op {
            BinOp::Shl(_) | BinOp::ShlAssign(_) => Some(Op::Shl),
            BinOp::Shr(_) | BinOp::ShrAssign(_) => Some(Op::Shr),
            BinOp::Div(_) | BinOp::DivAssign(_) => Some(Op::Div),
            BinOp::Rem(_) | BinOp::RemAssign(_) => Some(Op::Rem),
            _ => None,
        }
    }

    /// The operator of a method call whose argument is the audited operand. `checked_*` forms
    /// are not audited (they report an out-of-range operand or a zero divisor); `wrapping_*`
    /// and `overflowing_*` shifts mask the amount silently and every non-`checked` division
    /// method panics on a zero divisor, so they are.
    fn of_method(name: &str) -> Option<Op> {
        match name {
            "shl" | "shl_assign" | "wrapping_shl" | "overflowing_shl" => Some(Op::Shl),
            "shr" | "shr_assign" | "wrapping_shr" | "overflowing_shr" => Some(Op::Shr),
            "div"
            | "div_assign"
            | "div_euclid"
            | "div_ceil"
            | "div_floor"
            | "wrapping_div"
            | "wrapping_div_euclid"
            | "overflowing_div"
            | "overflowing_div_euclid"
            | "saturating_div" => Some(Op::Div),
            "rem"
            | "rem_assign"
            | "rem_euclid"
            | "wrapping_rem"
            | "wrapping_rem_euclid"
            | "overflowing_rem"
            | "overflowing_rem_euclid" => Some(Op::Rem),
            _ => None,
        }
    }

    /// Whether this is a division or remainder.
    #[must_use]
    pub fn is_division(self) -> bool {
        matches!(self, Op::Div | Op::Rem)
    }
}

/// How the right operand of a site is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operand {
    /// An integer literal (possibly in parentheses or cast).
    Literal,
    /// A constant expression: literals and named constants (`FIELD_SHIFT`, `u32::BITS`,
    /// `Self::K` outside generic code) combined by arithmetic and casts. Rustc's deny-by-default
    /// `arithmetic_overflow` / `unconditional_panic` lints reject such an operand at compile time
    /// when it is out of range on a reachable path, so it needs no proof comment.
    Const,
    /// Anything else (a variable, a field, a call, a generic constant): needs a proof.
    Runtime,
    /// Divisors only: a `NonZero*` value (see the module docs) — non-zero by its type.
    NonZero,
    /// Divisors only: a value guarded by `.max(n)` / `.clamp(n, ..)` with a literal `n ≥ 1`.
    Guarded,
    /// Divisors only: a floating-point division (cannot panic).
    Float,
}

/// Whether operands that are constant expressions ([`Operand::Const`]: named constants such as
/// `FIELD_SHIFT` or `FRAC_ONE`, outside generic code) are exempt from the proof comment, in both
/// the shift and the divisor audit. User decision (2026-10-20): exempt — rustc's deny-by-default
/// `arithmetic_overflow` and `unconditional_panic` lints reject an out-of-range constant shift
/// amount and a division by a constant zero at compile time on every reachable path, so a
/// comment would only repeat the compiler's check. The inventories still list these sites.
pub const CONSTANT_OPERANDS_EXEMPT: bool = true;

impl Operand {
    /// Whether a site with this operand needs a proof comment: a run-time operand always, a
    /// constant one unless [`CONSTANT_OPERANDS_EXEMPT`].
    #[must_use]
    pub fn needs_proof(self) -> bool {
        match self {
            Operand::Runtime => true,
            Operand::Const => !CONSTANT_OPERANDS_EXEMPT,
            Operand::Literal | Operand::NonZero | Operand::Guarded | Operand::Float => false,
        }
    }
}

/// An audited operator site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpSite {
    /// The crate (package name).
    pub krate: String,
    /// The file, relative to the workspace root.
    pub file: String,
    /// The 1-based line of the operator.
    pub line: u64,
    /// The operator.
    pub op: Op,
    /// The source line of the operator, trimmed.
    pub code: String,
    /// The right operand as written (whitespace collapsed).
    pub operand: String,
    /// How the operand is known.
    pub kind: Operand,
    /// The site is inside macro input or a `macro_rules!` body (token scan).
    pub in_macro: bool,
    /// The lines whose comment block may carry the proof: the operator's line, the expression's
    /// first line and the enclosing statement's first line.
    pub anchor_lines: Vec<u64>,
}

/// A library source file of a crate.
#[derive(Clone, Debug)]
pub struct SourceFile {
    /// The crate (package name).
    pub krate: String,
    /// The path relative to the workspace root (`/` separators).
    pub rel: String,
    /// The file's text.
    pub src: String,
}

/// Every crate under `crates/` with a `src/lib.rs`: `(package name, lib.rs path)`, sorted by
/// name.
pub fn library_crates() -> Result<Vec<(String, PathBuf)>, Box<dyn std::error::Error>> {
    let root = workspace_root().join("crates");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root)?.flatten() {
        let dir = entry.path();
        let lib = dir.join("src/lib.rs");
        let manifest = dir.join("Cargo.toml");
        if !lib.is_file() || !manifest.is_file() {
            continue;
        }
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(&manifest)?)?;
        let name = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{}: no package name", manifest.display()))?;
        out.push((name.to_owned(), lib));
    }
    out.sort();
    Ok(out)
}

/// The compiled source files of every library crate ([`library_crates`]), following each
/// module tree from `lib.rs` and leaving out `#[cfg(test)]` modules.
pub fn library_files() -> Result<Vec<SourceFile>, Box<dyn std::error::Error>> {
    let root = workspace_root();
    let mut files = Vec::new();
    for (krate, lib) in library_crates()? {
        let mut queue = vec![(lib, true)];
        let mut seen = BTreeSet::new();
        while let Some((path, mod_rs)) = queue.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = syn::parse_file(&src).map_err(|e| {
                let at = e.span().start();
                format!("{}:{}:{}: {e}", path.display(), at.line, at.column + 1)
            })?;
            let dir = module_dir(&path, mod_rs);
            let mut found = Vec::new();
            child_modules(&file.items, &path, &dir, &mut found);
            queue.extend(found);
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push(SourceFile {
                krate: krate.clone(),
                rel,
                src,
            });
        }
    }
    files.sort_by(|a, b| (&a.krate, &a.rel).cmp(&(&b.krate, &b.rel)));
    Ok(files)
}

/// The directory in which the `mod x;` declarations of `file` are looked up.
fn module_dir(file: &Path, mod_rs: bool) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("")).to_path_buf();
    if mod_rs {
        parent
    } else {
        parent.join(file.file_stem().unwrap_or_default())
    }
}

/// Collects the files of the out-of-line, non-test modules declared in `items`
/// (`(path, is a mod-rs file)`), descending into inline modules.
fn child_modules(items: &[syn::Item], file: &Path, dir: &Path, out: &mut Vec<(PathBuf, bool)>) {
    for item in items {
        let syn::Item::Mod(m) = item else { continue };
        if is_test_cfg(&m.attrs) {
            continue;
        }
        let name = m.ident.to_string();
        if let Some(path) = path_attr(&m.attrs) {
            // `#[path]`: relative to the directory of the current file (or of the inline module).
            let base = if m.content.is_some() {
                dir.to_path_buf()
            } else {
                file.parent().unwrap_or(Path::new("")).to_path_buf()
            };
            let target = base.join(path);
            match &m.content {
                Some((_, inner)) => child_modules(inner, file, &target, out),
                None => out.push((target, true)),
            }
            continue;
        }
        if let Some((_, inner)) = &m.content {
            child_modules(inner, file, &dir.join(&name), out);
        } else {
            let flat = dir.join(format!("{name}.rs"));
            if flat.is_file() {
                out.push((flat, false));
            } else {
                out.push((dir.join(&name).join("mod.rs"), true));
            }
        }
    }
}

/// The value of a `#[path = "…"]` attribute.
fn path_attr(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|a| {
        if !a.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(nv) = &a.meta else {
            return None;
        };
        match &nv.value {
            Expr::Lit(syn::ExprLit { lit: Lit::Str(s), .. }) => Some(s.value()),
            _ => None,
        }
    })
}

/// Whether `attrs` mark test-only code: `#[test]`, `#[cfg(test)]` or `#[cfg(all(test, ..))]`.
#[must_use]
pub fn is_test_cfg(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("test") {
            return true;
        }
        if !a.path().is_ident("cfg") {
            return false;
        }
        let Ok(list) = a.meta.require_list() else {
            return false;
        };
        let text: String = list.tokens.to_string().split_whitespace().collect();
        text == "test" || text.starts_with("all(test,")
    })
}

/// Finds the sites of `ops` in `file` (see the module docs for what is left out).
#[must_use]
pub fn scan(file: &SourceFile, ops: &[Op]) -> Vec<OpSite> {
    scan_with(file, ops, None)
}

/// Finds the divisions and remainders in `file` and classifies their divisors, recognising
/// `NonZero*` values, guards and floating-point divisions (module docs); `types` are the field
/// types of `file`'s crate ([`CrateTypes::of`]).
#[must_use]
pub fn scan_divisions(file: &SourceFile, types: &CrateTypes) -> Vec<OpSite> {
    scan_with(file, &[Op::Div, Op::Rem], Some(types))
}

fn scan_with(file: &SourceFile, ops: &[Op], types: Option<&CrateTypes>) -> Vec<OpSite> {
    let Ok(ast) = syn::parse_file(&file.src) else {
        return Vec::new();
    };
    let empty = CrateTypes::default();
    let mut v = Scanner {
        file,
        ops,
        lines: file.src.lines().collect(),
        generics: vec![BTreeSet::new()],
        self_generic: vec![false],
        stmt_line: Vec::new(),
        expr_line: Vec::new(),
        in_macro: false,
        divisors: types.is_some(),
        types: types.unwrap_or(&empty),
        locals: vec![BTreeMap::new()],
        sites: Vec::new(),
    };
    v.visit_file(&ast);
    v.sites.sort_by_key(|s| s.line);
    v.sites
}

/// What the type or the construction of a value says about it as a divisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Hint {
    /// A `NonZero*` value.
    NonZero,
    /// `.max(n)` / `.clamp(n, ..)` with a literal `n ≥ 1`.
    Guarded,
    /// A floating-point value.
    Float,
}

/// The hint a declared type gives: `NonZero*` or `f32`/`f64` (through references).
fn type_hint(ty: &syn::Type) -> Option<Hint> {
    match ty {
        syn::Type::Reference(r) => type_hint(&r.elem),
        syn::Type::Paren(p) => type_hint(&p.elem),
        syn::Type::Group(g) => type_hint(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => {
            let last = p.path.segments.last()?.ident.to_string();
            if last.starts_with("NonZero") {
                Some(Hint::NonZero)
            } else if last == "f32" || last == "f64" {
                Some(Hint::Float)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether `name` is a primitive integer type (a lossless `from` of a `NonZero*` keeps it
/// non-zero).
fn is_int_type(name: &str) -> bool {
    matches!(
        name,
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
    )
}

/// Whether `e` combines only integer literals (`100 << 16`): its value is fixed in the source
/// and rustc rejects a zero divisor of that form at compile time.
fn literal_only(e: &Expr) -> bool {
    match e {
        Expr::Lit(l) => matches!(l.lit, Lit::Int(_)),
        Expr::Paren(p) => literal_only(&p.expr),
        Expr::Group(g) => literal_only(&g.expr),
        Expr::Cast(c) => literal_only(&c.expr),
        Expr::Binary(b) => literal_only(&b.left) && literal_only(&b.right),
        _ => false,
    }
}

/// Whether `e` is `NonZero*::new(..)` (an `Option` of a non-zero value).
fn is_nonzero_new(e: &Expr) -> bool {
    let Expr::Call(c) = e else { return false };
    let Expr::Path(p) = &*c.func else { return false };
    let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
    matches!(segs.as_slice(), [.., ty, f] if ty.starts_with("NonZero") && f == "new")
}

/// Whether `e` is an integer literal `≥ 1`.
fn is_positive_int_literal(e: &Expr) -> bool {
    match e {
        Expr::Lit(syn::ExprLit { lit: Lit::Int(i), .. }) => i.base10_parse::<u128>().is_ok_and(|v| v >= 1),
        Expr::Paren(p) => is_positive_int_literal(&p.expr),
        Expr::Group(g) => is_positive_int_literal(&g.expr),
        _ => false,
    }
}

/// The named struct fields of a crate whose every declaration is `NonZero*` (or every one
/// `f32`/`f64`), and likewise the functions and methods by their return type: a field access
/// `x.rows` or a call `f(..)` / `x.f(..)` is classified by its name, so a name declared with
/// different types in the crate is not used.
#[derive(Clone, Debug, Default)]
pub struct CrateTypes {
    nonzero: BTreeSet<String>,
    float: BTreeSet<String>,
    /// Functions and methods whose every declaration returns `NonZero*` / `f32`/`f64`.
    fn_nonzero: BTreeSet<String>,
    fn_float: BTreeSet<String>,
}

impl CrateTypes {
    /// Collects the field types of `files` (the files of one crate).
    #[must_use]
    pub fn of<'f>(files: impl IntoIterator<Item = &'f SourceFile>) -> Self {
        type Decls = BTreeMap<String, BTreeSet<Option<Hint>>>;
        #[derive(Default)]
        struct Collect {
            fields: Decls,
            fns: Decls,
        }
        fn ret(sig: &syn::Signature) -> Option<Hint> {
            match &sig.output {
                syn::ReturnType::Type(_, ty) => type_hint(ty),
                syn::ReturnType::Default => None,
            }
        }
        impl<'ast> Visit<'ast> for Collect {
            fn visit_field(&mut self, f: &'ast syn::Field) {
                if let Some(name) = &f.ident {
                    self.fields
                        .entry(name.to_string())
                        .or_default()
                        .insert(type_hint(&f.ty));
                }
                visit::visit_field(self, f);
            }
            fn visit_signature(&mut self, sig: &'ast syn::Signature) {
                self.fns
                    .entry(sig.ident.to_string())
                    .or_default()
                    .insert(ret(sig));
                visit::visit_signature(self, sig);
            }
        }
        fn split(decls: Decls, nonzero: &mut BTreeSet<String>, float: &mut BTreeSet<String>) {
            for (name, hints) in decls {
                match hints.iter().collect::<Vec<_>>().as_slice() {
                    [Some(Hint::NonZero)] => {
                        nonzero.insert(name);
                    }
                    [Some(Hint::Float)] => {
                        float.insert(name);
                    }
                    _ => {}
                }
            }
        }
        let mut c = Collect::default();
        for file in files {
            if let Ok(ast) = syn::parse_file(&file.src) {
                c.visit_file(&ast);
            }
        }
        let mut out = CrateTypes::default();
        split(c.fields, &mut out.nonzero, &mut out.float);
        split(c.fns, &mut out.fn_nonzero, &mut out.fn_float);
        out
    }
}

/// The text after `// <tag>` in the comment block directly above one of `lines` (or a trailing
/// comment on that line), joined with the following comment lines of the block.
#[must_use]
pub fn proof_above(src: &str, lines: &[u64], tag: &str) -> Option<String> {
    let all: Vec<&str> = src.lines().collect();
    let marker = format!("// {tag}");
    for &line in lines {
        let Some(idx) = usize::try_from(line).ok().and_then(|l| l.checked_sub(1)) else {
            continue;
        };
        let Some(here) = all.get(idx) else { continue };
        if let Some(pos) = here.find(&marker) {
            return Some(here[pos + marker.len()..].trim().to_owned());
        }
        let mut block = Vec::new();
        for l in all[..idx].iter().rev() {
            let t = l.trim_start();
            match t.strip_prefix("//") {
                Some(_) if t.starts_with("///") || t.starts_with("//!") => break,
                Some(c) => block.push(c.trim()),
                None => break,
            }
        }
        block.reverse();
        if let Some(start) = block.iter().position(|l| l.starts_with(tag)) {
            // The proof runs until the next tagged comment (`// SHIFT:` above `// DIV:`).
            let end = block[start + 1..]
                .iter()
                .position(|l| is_tag_line(l))
                .map_or(block.len(), |i| start + 1 + i);
            let text = block[start..end].join(" ");
            return Some(text[tag.len()..].trim().to_owned());
        }
    }
    None
}

/// Whether a comment line (without its `//`, trimmed) starts a tagged comment (`SHIFT: …`,
/// `DIV: …`, `PANIC: …`, `SAFE-INDEX: …`, `SAFE-ARITH: …`): a proof runs until the next one.
pub(crate) fn is_tag_line(line: &str) -> bool {
    line.split_once(": ").is_some_and(|(tag, _)| {
        !tag.is_empty()
            && tag
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '-' || c == '_')
    })
}

/// The AST walk of [`scan`].
struct Scanner<'a> {
    file: &'a SourceFile,
    ops: &'a [Op],
    lines: Vec<&'a str>,
    /// Generic parameter names (type and const) in scope, innermost last.
    generics: Vec<BTreeSet<String>>,
    /// Whether `Self` is generic (a trait, or an impl with generic parameters), innermost last.
    self_generic: Vec<bool>,
    /// First lines of the enclosing statements.
    stmt_line: Vec<u64>,
    /// First lines of the enclosing audited expressions.
    expr_line: Vec<u64>,
    /// Inside macro input.
    in_macro: bool,
    /// Classify divisors ([`Scanner::classify_divisor`]).
    divisors: bool,
    /// The crate's field types (divisor classification).
    types: &'a CrateTypes,
    /// Local bindings in scope (parameters and `let`s) with their hint, innermost scope last;
    /// `None` shadows an outer binding.
    locals: Vec<BTreeMap<String, Option<Hint>>>,
    sites: Vec<OpSite>,
}

impl Scanner<'_> {
    fn push_generics(&mut self, g: &syn::Generics) {
        let mut set = self.generics.last().cloned().unwrap_or_default();
        for p in &g.params {
            match p {
                syn::GenericParam::Type(t) => {
                    set.insert(t.ident.to_string());
                }
                syn::GenericParam::Const(c) => {
                    set.insert(c.ident.to_string());
                }
                syn::GenericParam::Lifetime(_) => {}
            }
        }
        self.generics.push(set);
    }

    fn source_line(&self, line: u64) -> String {
        usize::try_from(line)
            .ok()
            .and_then(|l| l.checked_sub(1))
            .and_then(|i| self.lines.get(i))
            .map_or_else(String::new, |l| l.trim().to_owned())
    }

    /// The source text of `span` (lines joined, whitespace collapsed).
    fn span_text(&self, span: Span) -> String {
        let (s, e) = (span.start(), span.end());
        let mut out = String::new();
        for line in s.line..=e.line {
            let Some(text) = line.checked_sub(1).and_then(|i| self.lines.get(i)) else {
                break;
            };
            let chars: Vec<char> = text.chars().collect();
            let from = if line == s.line { s.column } else { 0 };
            let to = if line == e.line {
                e.column.min(chars.len())
            } else {
                chars.len()
            };
            if from < to {
                out.push_str(&chars[from..to].iter().collect::<String>());
                out.push(' ');
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn record(&mut self, op: Op, op_span: Span, start: Span, left: &Expr, operand: &Expr) {
        if !self.ops.contains(&op) {
            return;
        }
        let kind = if self.divisors && op.is_division() {
            self.classify_divisor(left, operand)
        } else {
            self.classify(operand)
        };
        let operand_text = self.span_text(syn::spanned::Spanned::span(operand));
        self.push_site(op, op_span, start, operand_text, kind);
    }

    fn push_site(&mut self, op: Op, op_span: Span, start: Span, operand: String, kind: Operand) {
        let line = op_span.start().line as u64;
        let mut anchors = vec![line, start.start().line as u64];
        anchors.extend(self.expr_line.iter().copied());
        anchors.extend(self.stmt_line.last().copied());
        anchors.sort_unstable();
        anchors.dedup();
        anchors.reverse();
        self.sites.push(OpSite {
            krate: self.file.krate.clone(),
            file: self.file.rel.clone(),
            line,
            op,
            code: self.source_line(line),
            operand,
            kind,
            in_macro: self.in_macro,
            anchor_lines: anchors,
        });
    }

    /// Classifies an operand ([`Operand`]).
    fn classify(&self, e: &Expr) -> Operand {
        match e {
            Expr::Lit(l) if matches!(l.lit, Lit::Int(_)) => Operand::Literal,
            Expr::Paren(p) => self.classify(&p.expr),
            Expr::Group(g) => self.classify(&g.expr),
            Expr::Cast(c) => self.classify(&c.expr),
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => self.classify(&u.expr),
            Expr::Binary(b) => {
                let assign = matches!(
                    b.op,
                    BinOp::AddAssign(_)
                        | BinOp::SubAssign(_)
                        | BinOp::MulAssign(_)
                        | BinOp::DivAssign(_)
                        | BinOp::RemAssign(_)
                        | BinOp::BitXorAssign(_)
                        | BinOp::BitAndAssign(_)
                        | BinOp::BitOrAssign(_)
                        | BinOp::ShlAssign(_)
                        | BinOp::ShrAssign(_)
                        | BinOp::And(_)
                        | BinOp::Or(_)
                        | BinOp::Eq(_)
                        | BinOp::Lt(_)
                        | BinOp::Le(_)
                        | BinOp::Ne(_)
                        | BinOp::Ge(_)
                        | BinOp::Gt(_)
                );
                if assign {
                    return Operand::Runtime;
                }
                match (self.classify(&b.left), self.classify(&b.right)) {
                    (Operand::Runtime, _) | (_, Operand::Runtime) => Operand::Runtime,
                    _ => Operand::Const,
                }
            }
            Expr::Path(p) if p.qself.is_none() => {
                let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                let Some(last) = segs.last() else {
                    return Operand::Runtime;
                };
                if !is_const_name(last) {
                    return Operand::Runtime;
                }
                let generics = self.generics.last();
                if generics.is_some_and(|g| segs.iter().any(|s| g.contains(s))) {
                    return Operand::Runtime;
                }
                if segs.first().is_some_and(|s| s == "Self") && self.self_generic.last() == Some(&true) {
                    return Operand::Runtime;
                }
                Operand::Const
            }
            _ => Operand::Runtime,
        }
    }

    /// Classifies a divisor (module docs): literal, `Float` (either side), constant, `NonZero`,
    /// guarded, else run-time.
    fn classify_divisor(&self, left: &Expr, right: &Expr) -> Operand {
        let base = self.classify(right);
        if base == Operand::Literal || literal_only(right) {
            return Operand::Literal;
        }
        let rh = self.hint_of(right);
        if rh == Some(Hint::Float) || self.hint_of(left) == Some(Hint::Float) {
            return Operand::Float;
        }
        if base == Operand::Const {
            return Operand::Const;
        }
        match rh {
            Some(Hint::NonZero) => Operand::NonZero,
            Some(Hint::Guarded) => Operand::Guarded,
            _ => Operand::Runtime,
        }
    }

    /// The hint of the innermost binding `name` in scope (`None` when unbound, or bound without
    /// a hint, which shadows any outer binding).
    fn local(&self, name: &str) -> Option<Hint> {
        self.locals
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .flatten()
    }

    /// Binds every identifier of `pat` (a pattern without a type: shadows with no hint).
    fn bind_pat(&mut self, pat: &syn::Pat, hint: Option<Hint>) {
        let scope = self.locals.last_mut().expect("a scope is always open");
        match pat {
            syn::Pat::Ident(i) => {
                scope.insert(i.ident.to_string(), hint);
            }
            syn::Pat::Type(t) => {
                let h = type_hint(&t.ty).or(hint);
                self.bind_pat(&t.pat, h);
            }
            other => {
                struct Idents<'s>(&'s mut BTreeMap<String, Option<Hint>>);
                impl<'ast> Visit<'ast> for Idents<'_> {
                    fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
                        self.0.insert(i.ident.to_string(), None);
                        visit::visit_pat_ident(self, i);
                    }
                }
                Idents(scope).visit_pat(other);
            }
        }
    }

    /// Binds a function's or closure's parameters (by their declared types).
    fn bind_params(&mut self, params: &[syn::Pat]) {
        for p in params {
            self.bind_pat(p, None);
        }
    }

    /// The hint of a call to the function or method `name` (its declared return type).
    fn fn_hint(&self, name: &str) -> Option<Hint> {
        if self.types.fn_nonzero.contains(name) {
            Some(Hint::NonZero)
        } else if self.types.fn_float.contains(name) {
            Some(Hint::Float)
        } else {
            None
        }
    }

    /// What `e`'s construction says about it as a divisor ([`Hint`]).
    fn hint_of(&self, e: &Expr) -> Option<Hint> {
        match e {
            Expr::Lit(l) => matches!(l.lit, Lit::Float(_)).then_some(Hint::Float),
            Expr::Paren(p) => self.hint_of(&p.expr),
            Expr::Group(g) => self.hint_of(&g.expr),
            Expr::Reference(r) => self.hint_of(&r.expr),
            // A cast to an integer type may truncate a non-zero value to zero.
            Expr::Cast(c) => (type_hint(&c.ty) == Some(Hint::Float)).then_some(Hint::Float),
            Expr::Unary(u) => match u.op {
                syn::UnOp::Neg(_) | syn::UnOp::Deref(_) => self.hint_of(&u.expr),
                _ => None,
            },
            Expr::Path(p) if p.qself.is_none() => {
                let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                match segs.as_slice() {
                    [name] => self.local(name),
                    _ if segs.iter().any(|s| s == "f32" || s == "f64") => Some(Hint::Float),
                    _ => None,
                }
            }
            Expr::Field(f) => match &f.member {
                syn::Member::Named(name) => {
                    let name = name.to_string();
                    if self.types.nonzero.contains(&name) {
                        Some(Hint::NonZero)
                    } else if self.types.float.contains(&name) {
                        Some(Hint::Float)
                    } else {
                        None
                    }
                }
                syn::Member::Unnamed(_) => None,
            },
            Expr::MethodCall(m) => {
                let recv = self.hint_of(&m.receiver);
                let name = m.method.to_string();
                let first = m.args.first();
                if recv == Some(Hint::Float) || first.is_some_and(|a| self.hint_of(a) == Some(Hint::Float)) {
                    return (!name.starts_with("is_") && name != "to_bits" && !name.ends_with("cmp"))
                        .then_some(Hint::Float);
                }
                match name.as_str() {
                    "max" if m.args.len() == 1 && first.is_some_and(is_positive_int_literal) => {
                        Some(Hint::Guarded)
                    }
                    "clamp" if m.args.len() == 2 && first.is_some_and(is_positive_int_literal) => {
                        Some(Hint::Guarded)
                    }
                    // `NonZero*` methods that return a `NonZero*` again.
                    "get" | "unsigned_abs" | "abs" | "saturating_abs" | "saturating_mul"
                    | "saturating_pow"
                        if recv == Some(Hint::NonZero) =>
                    {
                        Some(Hint::NonZero)
                    }
                    _ => self.fn_hint(&name),
                }
            }
            Expr::Call(c) => {
                let Expr::Path(func) = &*c.func else { return None };
                let segs: Vec<String> = func.path.segments.iter().map(|s| s.ident.to_string()).collect();
                let arg = c.args.first().and_then(|a| self.hint_of(a));
                match segs.as_slice() {
                    [ty, ..] if ty == "f32" || ty == "f64" => Some(Hint::Float),
                    [.., ty, f] if ty.starts_with("NonZero") && f == "get" && arg == Some(Hint::NonZero) => {
                        Some(Hint::NonZero)
                    }
                    // `From` is lossless: the source's guard or non-zero type carries over.
                    [ty, f] if f == "from" && (is_int_type(ty) || ty.starts_with("NonZero")) => {
                        arg.filter(|h| *h != Hint::Float)
                    }
                    [name] => self.fn_hint(name),
                    _ => None,
                }
            }
            Expr::Binary(b) => match b.op {
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_) => {
                    (self.hint_of(&b.left) == Some(Hint::Float)
                        || self.hint_of(&b.right) == Some(Hint::Float))
                    .then_some(Hint::Float)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Scans macro input that does not parse as expressions, token by token.
    fn scan_tokens(&mut self, tokens: TokenStream) {
        let flat = flatten(tokens);
        let mut i = 0;
        while i + 1 < flat.len() {
            let (a, b) = (&flat[i], &flat[i + 1]);
            let pair = match (a, b) {
                (Tok::Punct(x, Spacing::Joint, _), Tok::Punct(y, _, _))
                    if x == y && (*x == '<' || *x == '>') =>
                {
                    Some((if *x == '<' { Op::Shl } else { Op::Shr }, 2))
                }
                (Tok::Punct('/', _, _), _) => Some((Op::Div, 1)),
                (Tok::Punct('%', _, _), _) => Some((Op::Rem, 1)),
                _ => None,
            };
            let Some((op, width)) = pair else {
                i += 1;
                continue;
            };
            // `<<=` / `>>=` / `/=` / `%=`: the operand follows the `=`.
            let mut next = i + width;
            let last = if width == 2 { b } else { a };
            if let (Tok::Punct(_, Spacing::Joint, _), Some(Tok::Punct('=', _, _))) = (last, flat.get(next)) {
                next += 1;
            }
            let operand = flat.get(next);
            // A `>>` closing generics (`Vec<Vec<u8>>`) is followed by punctuation or a keyword
            // that cannot start an operand.
            let starts_operand = match operand {
                Some(Tok::Ident(name, _)) => !matches!(name.as_str(), "where" | "for" | "as" | "in" | "else"),
                Some(
                    Tok::Literal(..)
                    | Tok::Open(Delimiter::Parenthesis, _)
                    | Tok::Punct('$' | '-' | '*' | '!', _, _),
                ) => true,
                _ => false,
            };
            if !starts_operand {
                i += 2;
                continue;
            }
            let (kind, text) = match operand {
                Some(Tok::Literal(text, _))
                    if op.is_division()
                        && self.divisors
                        && (text.contains('.') || text.ends_with("f32") || text.ends_with("f64")) =>
                {
                    (Operand::Float, text.clone())
                }
                Some(Tok::Literal(text, _)) if text.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
                    (Operand::Literal, text.clone())
                }
                Some(Tok::Ident(name, _)) if is_const_name(name) => (Operand::Const, name.clone()),
                Some(Tok::Punct('$', _, _)) => match flat.get(next + 1) {
                    Some(Tok::Ident(name, _)) => (Operand::Runtime, format!("${name}")),
                    _ => (Operand::Runtime, "$".to_owned()),
                },
                Some(t) => (Operand::Runtime, t.text()),
                None => (Operand::Runtime, String::new()),
            };
            if self.ops.contains(&op) {
                let span = a.span();
                let was = self.in_macro;
                self.in_macro = true;
                self.push_site(op, span, span, text, kind);
                self.in_macro = was;
            }
            i = next;
        }
    }

    fn visit_macro_input(&mut self, mac: &syn::Macro) {
        let was = self.in_macro;
        self.in_macro = true;
        let parsed = mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated);
        match parsed {
            Ok(exprs) => {
                for e in &exprs {
                    self.visit_expr(e);
                }
            }
            Err(_) => self.scan_tokens(mac.tokens.clone()),
        }
        self.in_macro = was;
    }
}

/// The typed parameters of a signature (not `self`).
fn fn_params(sig: &syn::Signature) -> Vec<syn::Pat> {
    sig.inputs
        .iter()
        .filter_map(|a| match a {
            syn::FnArg::Typed(t) => Some(syn::Pat::Type(t.clone())),
            syn::FnArg::Receiver(_) => None,
        })
        .collect()
}

/// A named constant: `SCREAMING_SNAKE_CASE` with at least one letter (`FIELD_SHIFT`, `BITS`).
fn is_const_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A token of a flattened token stream.
#[derive(Debug)]
enum Tok {
    Punct(char, Spacing, Span),
    Ident(String, Span),
    Literal(String, Span),
    Open(Delimiter, Span),
    Close(Span),
}

impl Tok {
    fn span(&self) -> Span {
        match self {
            Tok::Punct(_, _, s) | Tok::Ident(_, s) | Tok::Literal(_, s) | Tok::Open(_, s) | Tok::Close(s) => {
                *s
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Tok::Punct(c, _, _) => c.to_string(),
            Tok::Ident(t, _) | Tok::Literal(t, _) => t.clone(),
            Tok::Open(..) => "(…)".to_owned(),
            Tok::Close(_) => String::new(),
        }
    }
}

/// Flattens a token stream (groups become open/close markers).
fn flatten(tokens: TokenStream) -> Vec<Tok> {
    let mut out = Vec::new();
    for t in tokens {
        match t {
            TokenTree::Punct(p) => out.push(Tok::Punct(p.as_char(), p.spacing(), p.span())),
            TokenTree::Ident(i) => out.push(Tok::Ident(i.to_string(), i.span())),
            TokenTree::Literal(l) => out.push(Tok::Literal(l.to_string(), l.span())),
            TokenTree::Group(g) => {
                out.push(Tok::Open(g.delimiter(), g.span_open()));
                out.extend(flatten(g.stream()));
                out.push(Tok::Close(g.span_close()));
            }
        }
    }
    out
}

impl<'ast> Visit<'ast> for Scanner<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !is_test_cfg(&m.attrs) {
            visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if is_test_cfg(&f.attrs) {
            return;
        }
        self.push_generics(&f.sig.generics);
        self.locals.push(BTreeMap::new());
        self.bind_params(&fn_params(&f.sig));
        visit::visit_item_fn(self, f);
        self.locals.pop();
        self.generics.pop();
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if is_test_cfg(&f.attrs) {
            return;
        }
        self.push_generics(&f.sig.generics);
        self.locals.push(BTreeMap::new());
        self.bind_params(&fn_params(&f.sig));
        visit::visit_impl_item_fn(self, f);
        self.locals.pop();
        self.generics.pop();
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if is_test_cfg(&f.attrs) {
            return;
        }
        self.push_generics(&f.sig.generics);
        self.locals.push(BTreeMap::new());
        self.bind_params(&fn_params(&f.sig));
        visit::visit_trait_item_fn(self, f);
        self.locals.pop();
        self.generics.pop();
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        self.locals.push(BTreeMap::new());
        self.bind_params(&c.inputs.iter().cloned().collect::<Vec<_>>());
        visit::visit_expr_closure(self, c);
        self.locals.pop();
    }

    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.locals.push(BTreeMap::new());
        visit::visit_block(self, b);
        self.locals.pop();
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        visit::visit_local(self, l);
        // A guard holds only while the binding cannot be reassigned; a declared type always.
        let init = l.init.as_ref().and_then(|i| self.hint_of(&i.expr));
        let mutable = matches!(&l.pat, syn::Pat::Ident(i) if i.mutability.is_some())
            || matches!(&l.pat, syn::Pat::Type(t) if matches!(&*t.pat, syn::Pat::Ident(i) if i.mutability.is_some()));
        let hint = if mutable && init == Some(Hint::Guarded) {
            None
        } else {
            init
        };
        if let (syn::Pat::Tuple(pats), Some(syn::Expr::Tuple(inits))) =
            (&l.pat, l.init.as_ref().map(|i| &*i.expr))
            && pats.elems.len() == inits.elems.len()
        {
            let hints: Vec<Option<Hint>> = inits.elems.iter().map(|e| self.hint_of(e)).collect();
            for (p, h) in pats.elems.iter().zip(hints) {
                let mutable = matches!(p, syn::Pat::Ident(i) if i.mutability.is_some());
                self.bind_pat(
                    p,
                    if mutable && h == Some(Hint::Guarded) {
                        None
                    } else {
                        h
                    },
                );
            }
            return;
        }
        // `let Some(d) = NonZeroU32::new(..) else { .. }`.
        if let (syn::Pat::TupleStruct(ts), Some(init)) = (&l.pat, l.init.as_ref())
            && ts.path.is_ident("Some")
            && ts.elems.len() == 1
            && is_nonzero_new(&init.expr)
            && let Some(p) = ts.elems.first()
        {
            self.bind_pat(p, Some(Hint::NonZero));
            return;
        }
        let pat = l.pat.clone();
        self.bind_pat(&pat, hint);
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        // `if let Some(d) = NonZeroU32::new(..) { .. }`: `d` is non-zero in the branch.
        if let Expr::Let(l) = &*i.cond
            && let syn::Pat::TupleStruct(ts) = &*l.pat
            && ts.path.is_ident("Some")
            && ts.elems.len() == 1
            && is_nonzero_new(&l.expr)
            && let Some(p) = ts.elems.first()
        {
            self.visit_expr(&l.expr);
            self.locals.push(BTreeMap::new());
            self.bind_pat(p, Some(Hint::NonZero));
            self.visit_block(&i.then_branch);
            self.locals.pop();
            if let Some((_, e)) = &i.else_branch {
                self.visit_expr(e);
            }
            return;
        }
        visit::visit_expr_if(self, i);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if is_test_cfg(&i.attrs) {
            return;
        }
        self.push_generics(&i.generics);
        self.self_generic.push(!i.generics.params.is_empty());
        visit::visit_item_impl(self, i);
        self.self_generic.pop();
        self.generics.pop();
    }

    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        if is_test_cfg(&t.attrs) {
            return;
        }
        self.push_generics(&t.generics);
        self.self_generic.push(true);
        visit::visit_item_trait(self, t);
        self.self_generic.pop();
        self.generics.pop();
    }

    // Compile-time contexts: an out-of-range operand there is a compile error.
    fn visit_item_const(&mut self, _: &'ast syn::ItemConst) {}
    fn visit_item_static(&mut self, _: &'ast syn::ItemStatic) {}
    fn visit_impl_item_const(&mut self, _: &'ast syn::ImplItemConst) {}
    fn visit_trait_item_const(&mut self, _: &'ast syn::TraitItemConst) {}
    fn visit_expr_const(&mut self, _: &'ast syn::ExprConst) {}
    fn visit_type_array(&mut self, t: &'ast syn::TypeArray) {
        self.visit_type(&t.elem);
    }
    fn visit_variant(&mut self, v: &'ast syn::Variant) {
        self.visit_fields(&v.fields);
    }
    fn visit_expr_repeat(&mut self, r: &'ast syn::ExprRepeat) {
        self.visit_expr(&r.expr);
    }
    fn visit_generic_argument(&mut self, a: &'ast syn::GenericArgument) {
        if !matches!(a, syn::GenericArgument::Const(_)) {
            visit::visit_generic_argument(self, a);
        }
    }

    fn visit_stmt(&mut self, s: &'ast syn::Stmt) {
        let attrs: &[Attribute] = match s {
            syn::Stmt::Local(l) => &l.attrs,
            syn::Stmt::Macro(m) => &m.attrs,
            _ => &[],
        };
        if is_test_cfg(attrs) {
            return;
        }
        let line = syn::spanned::Spanned::span(s).start().line as u64;
        self.stmt_line.push(line);
        visit::visit_stmt(self, s);
        self.stmt_line.pop();
    }

    fn visit_field_value(&mut self, f: &'ast syn::FieldValue) {
        // A struct-literal field on its own line anchors like a statement.
        let line = syn::spanned::Spanned::span(f).start().line as u64;
        self.stmt_line.push(line);
        visit::visit_field_value(self, f);
        self.stmt_line.pop();
    }

    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        if let Some(op) = Op::of_bin(&b.op) {
            let start = syn::spanned::Spanned::span(&*b.left);
            self.record(op, syn::spanned::Spanned::span(&b.op), start, &b.left, &b.right);
            let line = start.start().line as u64;
            self.expr_line.push(line);
            visit::visit_expr_binary(self, b);
            self.expr_line.pop();
        } else {
            visit::visit_expr_binary(self, b);
        }
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        if let (Some(op), Some(arg)) = (Op::of_method(&m.method.to_string()), m.args.first()) {
            let start = syn::spanned::Spanned::span(&*m.receiver);
            self.record(op, m.method.span(), start, &m.receiver, arg);
        }
        visit::visit_expr_method_call(self, m);
    }

    fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
        if is_test_cfg(&m.attrs) {
            return;
        }
        if m.mac.path.is_ident("macro_rules") {
            let was = self.in_macro;
            self.in_macro = true;
            self.scan_tokens(m.mac.tokens.clone());
            self.in_macro = was;
        } else {
            self.visit_macro_input(&m.mac);
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.visit_macro_input(mac);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sites(src: &str) -> Vec<OpSite> {
        let file = SourceFile {
            krate: "k".into(),
            rel: "a.rs".into(),
            src: src.into(),
        };
        scan(&file, &[Op::Shl, Op::Shr])
    }

    #[test]
    fn classifies_operands() {
        let src = r"
const S: u32 = 3;
struct P;
impl P { const K: u32 = 1; fn f(x: u32, n: u32) -> u32 { x << Self::K | x >> n } }
impl<T> Q<T> { fn g(x: u32) -> u32 { x << Self::K } }
fn h<const N: u32>(x: u32, s: &St) -> u32 {
    (x << 3) | (x >> (S - 1)) | (x << u32::BITS - 1) | (x >> N) | (x << s.shift) | x.wrapping_shl(s.k)
        | x.checked_shl(s.k).unwrap_or(0)
}
";
        let found = sites(src);
        let kinds: Vec<(Op, Operand, &str)> =
            found.iter().map(|s| (s.op, s.kind, s.operand.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (Op::Shl, Operand::Const, "Self::K"),
                (Op::Shr, Operand::Runtime, "n"),
                (Op::Shl, Operand::Runtime, "Self::K"),
                (Op::Shl, Operand::Literal, "3"),
                (Op::Shr, Operand::Const, "(S - 1)"),
                (Op::Shl, Operand::Const, "u32::BITS - 1"),
                (Op::Shr, Operand::Runtime, "N"),
                (Op::Shl, Operand::Runtime, "s.shift"),
                (Op::Shl, Operand::Runtime, "s.k"),
            ]
        );
    }

    #[test]
    fn skips_tests_and_compile_time_contexts() {
        let src = r"
const A: u32 = 1 << B;
static T: [u8; 1 << N] = [0; 1 << N];
enum E { X = 1 << N }
fn f() -> u32 { const { 1 << N } }
#[cfg(test)]
mod tests { fn t(x: u32, n: u32) -> u32 { x << n } }
#[test]
fn t2(x: u32, n: u32) -> u32 { x << n }
fn g(x: u32, n: u32) -> u32 {
    #[cfg(test)]
    let y = x << n;
    x
}
";
        assert!(sites(src).is_empty(), "{:?}", sites(src));
    }

    #[test]
    fn scans_macro_input_and_macro_rules_bodies() {
        let src = r"
fn f(x: u32, n: u32) { debug_assert!(x << n != 0, 'm'); let v: Vec<Vec<u8>> = vec![]; }
macro_rules! m { ($e:expr, $s:ident) => { ($e >> $s) | ($e << 2) | ($e >> SHIFT) } }
fn g() -> Option<Vec<u8>> { None }
";
        let found = sites(src);
        let kinds: Vec<(Op, Operand, bool)> = found.iter().map(|s| (s.op, s.kind, s.in_macro)).collect();
        assert_eq!(
            kinds,
            vec![
                (Op::Shl, Operand::Runtime, true),
                (Op::Shr, Operand::Runtime, true),
                (Op::Shl, Operand::Literal, true),
                (Op::Shr, Operand::Const, true),
            ],
            "{found:?}"
        );
    }

    #[test]
    fn proof_comment_above_statement_expression_or_trailing() {
        let src = "fn f(x: u32, n: u32) -> u32 {\n    // SHIFT: n < 8 (checked by `new`).\n    // Second line.\n    let a = x\n        << n;\n    let b = x >> n; // SHIFT: same as above\n    x >> n\n}\n";
        let found = sites(src);
        assert_eq!(found.len(), 3);
        assert_eq!(
            proof_above(src, &found[0].anchor_lines, "SHIFT:").as_deref(),
            Some("n < 8 (checked by `new`). Second line.")
        );
        assert_eq!(
            proof_above(src, &found[1].anchor_lines, "SHIFT:").as_deref(),
            Some("same as above")
        );
        assert_eq!(proof_above(src, &found[2].anchor_lines, "SHIFT:"), None);
        // A proof ends at the next tagged comment.
        let src =
            "fn f(x: u32, n: u32) -> u32 {\n    // SHIFT: n < 8.\n    // DIV: n != 0.\n    (x >> n) / n\n}\n";
        let found = sites(src);
        assert_eq!(
            proof_above(src, &found[0].anchor_lines, "SHIFT:").as_deref(),
            Some("n < 8.")
        );
        assert_eq!(
            proof_above(src, &found[0].anchor_lines, "DIV:").as_deref(),
            Some("n != 0.")
        );
        // ... including the R5.S11 proof tags (`SAFE-ARITH:`, `SAFE-INDEX:`).
        let src = "fn f(x: u32, n: u32) -> u32 {\n    // SHIFT: n < 8.\n    // SAFE-ARITH: x < 9.\n    x.wrapping_add(1) << n\n}\n";
        let found = sites(src);
        assert_eq!(
            proof_above(src, &found[0].anchor_lines, "SHIFT:").as_deref(),
            Some("n < 8.")
        );
        assert!(is_tag_line("SAFE-INDEX: i < len.") && is_tag_line("SAFE-ARITH: x < 9."));
    }

    fn divisions(src: &str) -> Vec<(Op, Operand, String)> {
        let file = SourceFile {
            krate: "k".into(),
            rel: "a.rs".into(),
            src: src.into(),
        };
        let types = CrateTypes::of([&file]);
        scan_divisions(&file, &types)
            .into_iter()
            .map(|s| (s.op, s.kind, s.operand))
            .collect()
    }

    #[test]
    fn classifies_divisors() {
        let src = r"
const N: u32 = 4;
struct S { rows: NonZeroU16, w: f32, n: u32 }
fn f(a: u32, b: u32, d: NonZeroU32, s: &S, x: f32) -> u32 {
    let g = b.max(1);
    let mut m = b.max(1);
    let c = b.clamp(2, 9);
    let (p, q) = (b.max(1), b);
    let Some(z) = NonZeroU32::new(b) else { return 0 };
    let k = u32::from(b.max(1));
    a / 2 + a / (1 << 4) + a / N + a / d + a / d.get() + a % u32::from(s.rows.get()) + a / g
        + a / m + a / c + a / p + a / q + a / z + a / k + a / b.max(0) + a / (b.max(1) as u8 as u32)
        + (x / s.w) as u32 + (a as f32 / 2.5) as u32 + a / s.n + a.div_euclid(b) + a.rem_euclid(g)
        + a.checked_div(b).unwrap_or(0)
}
fn h(a: i64, b: i64) -> i64 {
    if let Some(d) = NonZeroI64::new(b) { a / d.get() } else { a / b }
}
";
        let found = divisions(src);
        let kinds: Vec<(Operand, &str)> = found.iter().map(|(_, k, o)| (*k, o.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (Operand::Literal, "2"),
                (Operand::Literal, "(1 << 4)"),
                (Operand::Const, "N"),
                (Operand::NonZero, "d"),
                (Operand::NonZero, "d.get()"),
                (Operand::NonZero, "u32::from(s.rows.get())"),
                (Operand::Guarded, "g"),
                (Operand::Runtime, "m"),
                (Operand::Guarded, "c"),
                (Operand::Guarded, "p"),
                (Operand::Runtime, "q"),
                (Operand::NonZero, "z"),
                (Operand::Guarded, "k"),
                (Operand::Runtime, "b.max(0)"),
                (Operand::Runtime, "(b.max(1) as u8 as u32)"),
                (Operand::Float, "s.w"),
                (Operand::Float, "2.5"),
                (Operand::Runtime, "s.n"),
                (Operand::Runtime, "b"),
                (Operand::Guarded, "g"),
                (Operand::NonZero, "d.get()"),
                (Operand::Runtime, "b"),
            ],
            "{found:?}"
        );
        assert_eq!(found.iter().filter(|(op, ..)| *op == Op::Rem).count(), 2);
    }

    #[test]
    fn division_tokens_in_macro_rules_bodies() {
        let src = r"macro_rules! m { ($a:expr, $b:expr) => { ($a / $b) % 4 + $a / LEN + $a / 2.0 } }";
        let kinds: Vec<(Op, Operand)> = divisions(src).into_iter().map(|(op, k, _)| (op, k)).collect();
        assert_eq!(
            kinds,
            vec![
                (Op::Div, Operand::Runtime),
                (Op::Rem, Operand::Literal),
                (Op::Div, Operand::Const),
                (Op::Div, Operand::Float),
            ]
        );
    }

    #[test]
    fn shift_scan_ignores_divisions() {
        assert!(sites("fn f(a: u32, b: u32) -> u32 { a / b % b }").is_empty());
    }

    #[test]
    fn module_tree_skips_test_modules() {
        let items = syn::parse_file("mod a; #[cfg(test)] mod tests; mod b { mod c; }")
            .unwrap()
            .items;
        let mut out = Vec::new();
        child_modules(&items, Path::new("/x/src/lib.rs"), Path::new("/x/src"), &mut out);
        let names: Vec<String> = out.iter().map(|(p, _)| p.display().to_string()).collect();
        assert_eq!(names, vec!["/x/src/a/mod.rs", "/x/src/b/c/mod.rs"]);
    }
}
