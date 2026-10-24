//! Fluent (`.ftl`) sources: the subset of Fluent 1.0 that compiles to static tables.
//!
//! Supported: messages and their attributes (key `message.attribute`), terms (`-brand`,
//! inlined), message and term references (inlined), variables (`{ $name }`; `-` in a name
//! becomes `_`), string and number literals, comments, multiline patterns (common indentation
//! removed), and **one** select expression per message on a variable with plural variants
//! (`[zero]` … `[other]` or a number such as `[0]`) and a `*` default. Anything that needs
//! run-time evaluation (functions such as `NUMBER()`, term arguments, selects on strings or
//! nested selects) is an error naming the line.

use std::collections::BTreeMap;

use super::model::{Body, Entry, Parsed, Seg, Sel, concat, push_seg, valid_arg};
use super::plural::Category;

#[derive(Clone, Debug)]
enum Elem {
    Text(String),
    /// Start of a continuation line with this indentation (removed by `finish`).
    Indent(usize),
    Newline,
    Expr(Expr),
}

#[derive(Clone, Debug)]
enum Expr {
    Lit(String),
    Var(String),
    Msg(String, Option<String>),
    Term(String, Option<String>),
    Select { var: String, variants: Vec<Variant> },
}

#[derive(Clone, Debug)]
struct Variant {
    key: VKey,
    default: bool,
    pattern: Vec<Elem>,
}

#[derive(Clone, Debug)]
enum VKey {
    Num(String),
    Ident(String),
}

#[derive(Clone, Debug, Default)]
struct Raw {
    value: Option<Vec<Elem>>,
    attrs: Vec<(String, Vec<Elem>)>,
    line: usize,
}

struct Parser {
    s: Vec<char>,
    pos: usize,
}

type PResult<T> = Result<T, String>;

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.pos).copied()
    }

    fn peek_at(&self, i: usize) -> Option<char> {
        self.s.get(self.pos + i).copied()
    }

    fn line(&self) -> usize {
        1 + self.s[..self.pos.min(self.s.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> PResult<()> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(format!("expected `{c}`, found {}", self.found()))
        }
    }

    fn found(&self) -> String {
        match self.peek() {
            Some('\n') | None => "the end of the line".to_string(),
            Some(c) => format!("`{c}`"),
        }
    }

    fn skip_inline_blank(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.pos += 1;
        }
    }

    fn skip_blank(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.pos += 1;
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == '\n' {
                break;
            }
        }
    }

    /// Fluent identifier: `[a-zA-Z][a-zA-Z0-9_-]*`.
    fn ident(&mut self) -> PResult<String> {
        let start = self.pos;
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            return Err(format!("expected an identifier, found {}", self.found()));
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            self.pos += 1;
        }
        Ok(self.s[start..self.pos].iter().collect())
    }

    /// The next line that is not blank, from `self.pos` at a line end: `(start, indent)`.
    fn next_content_line(&self) -> Option<(usize, usize)> {
        let mut p = self.pos;
        loop {
            // `p` is at a '\n' (or the start of a line after it).
            if self.s.get(p) == Some(&'\n') {
                p += 1;
            }
            let start = p;
            let mut indent = 0;
            while matches!(self.s.get(p), Some(' ' | '\t' | '\r')) {
                if self.s[p] == ' ' {
                    indent += 1;
                }
                p += 1;
            }
            match self.s.get(p) {
                None => return None,
                Some('\n') => {}
                Some(_) => return Some((start, indent)),
            }
        }
    }

    /// A pattern up to the end of its last continuation line (or a `}` when `in_variant`).
    fn pattern(&mut self, in_variant: bool) -> PResult<Vec<Elem>> {
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                '{' => {
                    self.pos += 1;
                    out.push(Elem::Expr(self.placeable()?));
                }
                '}' if in_variant => break,
                '}' => return Err("unbalanced `}` (write `{\"}\"}` for a brace)".to_string()),
                '\n' => {
                    let Some((start, indent)) = self.next_content_line() else {
                        break;
                    };
                    let first = self.s[start..].iter().find(|c| !matches!(c, ' ' | '\t' | '\r'));
                    if indent == 0 || matches!(first, Some('[' | '*' | '.' | '}')) {
                        break;
                    }
                    let newlines = self.s[self.pos..start].iter().filter(|c| **c == '\n').count();
                    out.extend(std::iter::repeat_n(Elem::Newline, newlines));
                    out.push(Elem::Indent(indent));
                    self.pos = start;
                    self.skip_inline_blank();
                }
                '\r' => self.pos += 1,
                c => {
                    self.pos += 1;
                    if let Some(Elem::Text(t)) = out.last_mut() {
                        t.push(c);
                    } else {
                        out.push(Elem::Text(c.to_string()));
                    }
                }
            }
        }
        Ok(finish(out))
    }

    /// After `{`: an inline expression or a select expression, up to and including `}`.
    fn placeable(&mut self) -> PResult<Expr> {
        self.skip_blank();
        let expr = self.inline()?;
        self.skip_blank();
        if self.peek() == Some('-') && self.peek_at(1) == Some('>') {
            self.pos += 2;
            let Expr::Var(var) = expr else {
                return Err("a select expression must choose on a variable (`{ $count -> … }`)".to_string());
            };
            let mut variants = Vec::new();
            loop {
                self.skip_blank();
                if self.eat('}') {
                    break;
                }
                let default = self.eat('*');
                self.expect('[')?;
                self.skip_blank();
                let key = if self.peek().is_some_and(|c| c.is_ascii_digit() || c == '-') {
                    VKey::Num(self.number())
                } else {
                    VKey::Ident(self.ident()?)
                };
                self.skip_blank();
                self.expect(']')?;
                self.skip_inline_blank();
                let pattern = self.pattern(true)?;
                variants.push(Variant {
                    key,
                    default,
                    pattern,
                });
            }
            match variants.iter().filter(|v| v.default).count() {
                1 => Ok(Expr::Select { var, variants }),
                0 => Err("a select expression needs one default variant (`*[other]`)".to_string()),
                _ => Err("a select expression has more than one default variant".to_string()),
            }
        } else {
            self.expect('}')?;
            Ok(expr)
        }
    }

    fn number(&mut self) -> String {
        let start = self.pos;
        self.eat('-');
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        self.s[start..self.pos].iter().collect()
    }

    fn inline(&mut self) -> PResult<Expr> {
        match self.peek() {
            Some('"') => {
                self.pos += 1;
                let mut s = String::new();
                loop {
                    match self.peek() {
                        Some('"') => {
                            self.pos += 1;
                            break;
                        }
                        Some('\\') => {
                            self.pos += 1;
                            match self.peek() {
                                Some('"') => s.push('"'),
                                Some('\\') => s.push('\\'),
                                Some(u @ ('u' | 'U')) => {
                                    let n = if u == 'u' { 4 } else { 6 };
                                    let hex: String = self.s.iter().skip(self.pos + 1).take(n).collect();
                                    let c = u32::from_str_radix(&hex, 16)
                                        .ok()
                                        .and_then(char::from_u32)
                                        .ok_or_else(|| format!("invalid escape `\\{u}{hex}`"))?;
                                    s.push(c);
                                    self.pos += n;
                                }
                                _ => {
                                    return Err(format!(
                                        "unknown escape `\\{}` in a string literal",
                                        self.found()
                                    ));
                                }
                            }
                            self.pos += 1;
                        }
                        Some('\n') | None => return Err("unterminated string literal".to_string()),
                        Some(c) => {
                            s.push(c);
                            self.pos += 1;
                        }
                    }
                }
                Ok(Expr::Lit(s))
            }
            Some(c)
                if c.is_ascii_digit()
                    || (c == '-' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                Ok(Expr::Lit(self.number()))
            }
            Some('$') => {
                self.pos += 1;
                let name = self.ident()?.replace('-', "_");
                if !valid_arg(&name) {
                    return Err(format!("`${name}` is not a valid argument name"));
                }
                Ok(Expr::Var(name))
            }
            Some('-') => {
                self.pos += 1;
                let id = self.ident()?;
                let attr = self.attr_ref()?;
                self.skip_blank();
                if self.peek() == Some('(') {
                    return Err(format!("term arguments (`-{id}(…)`) are not supported"));
                }
                Ok(Expr::Term(id, attr))
            }
            Some('{') => {
                self.pos += 1;
                self.placeable()
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let id = self.ident()?;
                if self.peek() == Some('(') {
                    return Err(format!(
                        "functions (`{id}(…)`) are not supported: format the value in Rust and pass it as an argument"
                    ));
                }
                let attr = self.attr_ref()?;
                Ok(Expr::Msg(id, attr))
            }
            _ => Err(format!("expected an expression, found {}", self.found())),
        }
    }

    fn attr_ref(&mut self) -> PResult<Option<String>> {
        if self.eat('.') {
            Ok(Some(self.ident()?))
        } else {
            Ok(None)
        }
    }

    /// One message or term (at the start of a line, `-` already consumed for a term).
    fn entry(&mut self) -> PResult<(String, Raw)> {
        let line = self.line();
        let id = self.ident()?;
        self.skip_inline_blank();
        self.expect('=')?;
        self.skip_inline_blank();
        let value = self.pattern(false)?;
        let mut raw = Raw {
            value: (!value.is_empty()).then_some(value),
            attrs: Vec::new(),
            line,
        };
        while let Some((start, indent)) = self.next_content_line() {
            if indent == 0 || self.s[start + indent..].first() != Some(&'.') {
                break;
            }
            self.pos = start + indent + 1;
            let name = self.ident()?;
            self.skip_inline_blank();
            self.expect('=')?;
            self.skip_inline_blank();
            let p = self.pattern(false)?;
            if p.is_empty() {
                return Err(format!("attribute `.{name}` has no value"));
            }
            raw.attrs.push((name, p));
        }
        if raw.value.is_none() && raw.attrs.is_empty() {
            return Err(format!("`{id}` has no value"));
        }
        // The entry ends at the end of its line.
        if self.peek().is_some_and(|c| c != '\n') {
            return Err(format!("unexpected {} after the entry", self.found()));
        }
        Ok((id, raw))
    }
}

/// Removes the common indentation of continuation lines, leading line breaks and trailing
/// blanks.
fn finish(mut elems: Vec<Elem>) -> Vec<Elem> {
    let min = elems
        .iter()
        .filter_map(|e| match e {
            Elem::Indent(n) => Some(*n),
            _ => None,
        })
        .min()
        .unwrap_or(0);
    while matches!(elems.first(), Some(Elem::Newline)) {
        elems.remove(0);
    }
    while matches!(elems.last(), Some(Elem::Newline | Elem::Indent(_))) {
        elems.pop();
    }
    if let Some(Elem::Text(t)) = elems.last_mut() {
        let trimmed = t.trim_end().len();
        t.truncate(trimmed);
        if t.is_empty() {
            elems.pop();
        }
    }
    elems
        .into_iter()
        .map(|e| match e {
            Elem::Indent(n) => Elem::Text(" ".repeat(n - min)),
            Elem::Newline => Elem::Text("\n".to_string()),
            e => e,
        })
        .collect()
}

/// A flattened pattern piece: segments, or the (single) plural choice.
enum Piece {
    Seg(Seg),
    Select {
        var: String,
        variants: Vec<(VKey, bool, Vec<Seg>)>,
    },
}

struct Resolver<'a> {
    msgs: &'a BTreeMap<String, Raw>,
    terms: &'a BTreeMap<String, Raw>,
    stack: Vec<String>,
}

impl Resolver<'_> {
    fn reference(&mut self, term: bool, id: &str, attr: Option<&str>) -> PResult<Vec<Piece>> {
        let (map, sigil) = if term { (self.terms, "-") } else { (self.msgs, "") };
        let name = match attr {
            Some(a) => format!("{sigil}{id}.{a}"),
            None => format!("{sigil}{id}"),
        };
        if self.stack.contains(&name) {
            return Err(format!("`{name}` references itself"));
        }
        let raw = map.get(id).ok_or_else(|| format!("unknown reference `{name}`"))?;
        let pattern = match attr {
            None => raw.value.as_ref(),
            Some(a) => raw.attrs.iter().find(|(n, _)| n == a).map(|(_, p)| p),
        }
        .ok_or_else(|| format!("`{name}` has no value"))?;
        self.stack.push(name);
        let out = self.flatten(pattern);
        self.stack.pop();
        out
    }

    fn flatten(&mut self, elems: &[Elem]) -> PResult<Vec<Piece>> {
        let mut out = Vec::new();
        for e in elems {
            match e {
                Elem::Text(t) | Elem::Expr(Expr::Lit(t)) => out.push(Piece::Seg(Seg::Text(t.clone()))),
                Elem::Indent(_) | Elem::Newline => {}
                Elem::Expr(Expr::Var(v)) => out.push(Piece::Seg(Seg::Arg(v.clone()))),
                Elem::Expr(Expr::Msg(id, attr)) => out.extend(self.reference(false, id, attr.as_deref())?),
                Elem::Expr(Expr::Term(id, attr)) => out.extend(self.reference(true, id, attr.as_deref())?),
                Elem::Expr(Expr::Select { var, variants }) => {
                    let mut vs = Vec::new();
                    for v in variants {
                        let mut segs = Vec::new();
                        for p in self.flatten(&v.pattern)? {
                            match p {
                                Piece::Seg(s) => push_seg(&mut segs, s),
                                Piece::Select { .. } => {
                                    return Err("nested select expressions are not supported".to_string());
                                }
                            }
                        }
                        vs.push((v.key.clone(), v.default, segs));
                    }
                    out.push(Piece::Select {
                        var: var.clone(),
                        variants: vs,
                    });
                }
            }
        }
        Ok(out)
    }

    fn body(&mut self, elems: &[Elem]) -> PResult<Body> {
        let pieces = self.flatten(elems)?;
        let selects = pieces
            .iter()
            .filter(|p| matches!(p, Piece::Select { .. }))
            .count();
        if selects > 1 {
            return Err("at most one select expression per message is supported".to_string());
        }
        let mut prefix = Vec::new();
        let mut suffix = Vec::new();
        let mut select = None;
        for p in pieces {
            match p {
                Piece::Seg(s) if select.is_none() => push_seg(&mut prefix, s),
                Piece::Seg(s) => push_seg(&mut suffix, s),
                Piece::Select { var, variants } => select = Some((var, variants)),
            }
        }
        let Some((selector, variants)) = select else {
            return Ok(Body::Pattern(prefix));
        };
        let mut out = Vec::new();
        let mut other = Vec::new();
        for (key, default, segs) in variants {
            let full = concat(&[&prefix, &segs, &suffix]);
            let sel = match &key {
                VKey::Num(n) => Sel::Exact(n.parse::<u64>().map_err(|_| {
                    format!("variant key `[{n}]`: only non-negative integers are supported as numbers")
                })?),
                VKey::Ident(name) => Sel::Cat(Category::from_name(name).ok_or_else(|| {
                    format!(
                        "variant key `[{name}]`: only plural variants are supported \
                         (`zero`, `one`, `two`, `few`, `many`, `other` or a number)"
                    )
                })?),
            };
            if default {
                other.clone_from(&full);
            }
            if sel != Sel::Cat(Category::Other) {
                out.push((sel, full));
            }
        }
        Ok(Body::Plural {
            selector,
            variants: out,
            other,
        })
    }
}

/// Parses a Fluent file into `(id, entry)` pairs (attributes as `message.attribute`), or the
/// errors as `(line, message)`.
pub(crate) fn parse(src: &str) -> Parsed {
    let mut p = Parser {
        s: src.chars().collect(),
        pos: 0,
    };
    let mut msgs: BTreeMap<String, Raw> = BTreeMap::new();
    let mut terms: BTreeMap<String, Raw> = BTreeMap::new();
    let mut order = Vec::new();
    let mut errors = Vec::new();
    while let Some(c) = p.peek() {
        match c {
            '\n' | '\r' => p.pos += 1,
            '#' => p.skip_line(),
            ' ' | '\t' => {
                let start = p.pos;
                p.skip_inline_blank();
                if p.peek().is_some_and(|c| c != '\n') {
                    p.pos = start;
                    errors.push((p.line(), "indented line outside of a message".to_string()));
                }
                p.skip_line();
            }
            _ => {
                let term = p.eat('-');
                let line = p.line();
                match p.entry() {
                    Ok((id, raw)) => {
                        let map = if term { &mut terms } else { &mut msgs };
                        if map.insert(id.clone(), raw).is_some() {
                            errors.push((
                                line,
                                format!("`{}{id}` is defined twice", if term { "-" } else { "" }),
                            ));
                        } else if !term {
                            order.push(id);
                        }
                    }
                    Err(e) => {
                        errors.push((p.line(), e));
                        // Skip to the next entry (a line starting in column 0).
                        p.skip_line();
                        while p.peek().is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
                            p.skip_line();
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut r = Resolver {
        msgs: &msgs,
        terms: &terms,
        stack: Vec::new(),
    };
    for id in &order {
        let raw = &msgs[id];
        if let Some(v) = &raw.value {
            r.stack = vec![id.clone()];
            match r.body(v) {
                Ok(body) => out.push((id.clone(), Entry { body, line: raw.line })),
                Err(e) => errors.push((raw.line, format!("`{id}`: {e}"))),
            }
        }
        for (name, p) in &raw.attrs {
            let full = format!("{id}.{name}");
            r.stack = vec![full.clone()];
            match r.body(p) {
                Ok(body) => out.push((full, Entry { body, line: raw.line })),
                Err(e) => errors.push((raw.line, format!("`{full}`: {e}"))),
            }
        }
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(src: &str) -> Body {
        let mut v = parse(src).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(v.len(), 1, "{v:?}");
        v.remove(0).1.body
    }

    fn text(s: &str) -> Seg {
        Seg::Text(s.into())
    }

    fn arg(s: &str) -> Seg {
        Seg::Arg(s.into())
    }

    #[test]
    fn simple_and_variables() {
        assert_eq!(one("hello = Hello\n"), Body::Pattern(vec![text("Hello")]));
        assert_eq!(
            one("greet = Hello, { $user-name }!"),
            Body::Pattern(vec![text("Hello, "), arg("user_name"), text("!")])
        );
        assert_eq!(one("b = {\"{\"}x{ 42 }"), Body::Pattern(vec![text("{x42")]));
    }

    #[test]
    fn multiline_removes_common_indent() {
        let b = one("m =\n    first\n      second\n\n    third\n# c\n");
        assert_eq!(b, Body::Pattern(vec![text("first\n  second\n\nthird")]));
    }

    #[test]
    fn terms_and_references_are_inlined() {
        let v = parse("-brand = Twine\nabout = About { -brand }\ntitle = { about }: { $n }\n").unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].1.body, Body::Pattern(vec![text("About Twine: "), arg("n")]));
    }

    #[test]
    fn select_with_prefix_and_suffix() {
        let b = one(
            "items = You have { $count ->\n    [0] no items\n    [one] one item\n   *[other] { $count } items\n } left\n",
        );
        assert_eq!(
            b,
            Body::Plural {
                selector: "count".into(),
                variants: vec![
                    (Sel::Exact(0), vec![text("You have no items left")]),
                    (Sel::Cat(Category::One), vec![text("You have one item left")]),
                ],
                other: vec![text("You have "), arg("count"), text(" items left")],
            }
        );
    }

    #[test]
    fn attributes_become_keys() {
        let v = parse("login = Log in\n    .title = Log in to { -x }\n-x = X\n").unwrap();
        let ids: Vec<_> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(ids, ["login", "login.title"]);
    }

    #[test]
    fn unsupported_constructs_are_errors_with_lines() {
        let e = parse("ok = fine\nbad = { NUMBER($n) }\n").unwrap_err();
        assert_eq!(e[0].0, 2, "{e:?}");
        assert!(parse("g = { $g ->\n [male] he\n *[other] they\n}\n").is_err());
        assert!(parse("n = { $a ->\n [one] { $b ->\n *[other] x\n }\n *[other] y\n}\n").is_err());
        assert!(parse("loop = { loop }\n").is_err());
        assert!(parse("x = { missing }\n").is_err());
        assert!(parse("d = a\nd = b\n").is_err());
        assert!(
            parse("s = { $n ->\n [one] a\n [other] b\n}\n").is_err(),
            "no default"
        );
    }
}
