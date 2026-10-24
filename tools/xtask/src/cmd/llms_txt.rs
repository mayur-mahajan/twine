//! `cargo xtask llms-txt [--check]`: writes `llms.txt` at the workspace root — the public API of
//! every library crate, for AI coding agents, generated from the rustdoc comments.
//!
//! **How.** Each crate of [`CRATES`] is documented by `cargo +nightly rustdoc … --output-format
//! json` (rustdoc's JSON output is nightly-only) into `target/llms/doc/`, and every public item
//! reachable from the crate root is rendered as Markdown: crate and module docs, then one
//! section per type, trait, function, constant and macro with its signature and the first
//! paragraph of its docs (plus the first paragraph of its `# Errors`, `# Panics` and `# Safety`
//! sections), methods and fields as bullets. Code blocks are kept only in crate docs; hidden
//! doctest lines are dropped and intra-doc links become plain code spans.
//!
//! **Sections.** Headings are line-prefixed and full-path so an agent can grep instead of reading
//! the file: `## crate <name>`, `### mod <path>`, `#### <kind> <path>`. The preamble's index
//! gives the line of every crate and module section (computed when the file is written).
//!
//! **Features.** Each crate is documented with the features of [`CRATES`]: every user-facing
//! feature, but not the alternative build profiles (`safety`, `partitions`,
//! `display-integrity`) nor chip-, SDL- or logging-specific ones — the default profile's API.
//!
//! `--check` regenerates in memory and fails when `llms.txt` differs (or nightly is missing).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::process::Command;

use serde_json::Value;

use crate::util::{R, no_incremental, run as run_cmd, workspace_root};

/// The generated file, relative to the workspace root.
pub const OUT: &str = "llms.txt";

/// Target directory of the rustdoc JSON build (kept apart from `target/debug` so it never waits
/// for or invalidates the normal build).
const TARGET_DIR: &str = "target/llms";

/// The rustdoc JSON format this renderer was written against; another version prints a warning
/// (the renderer reads fields defensively and usually still works).
const FORMAT_VERSION: u64 = 61;

/// `(crate, features)` documented, in the order of the file: the facade first (what applications
/// import), then the crates from the UI layers down to the foundations and the tooling crates.
pub const CRATES: &[(&str, &str)] = &[
    (
        "twine",
        "full,std,embassy,drivers,platform-std,drivers-fbdev,drivers-evdev",
    ),
    (
        "twine-view",
        "std,async,svg,vector,theme-lvgl,theme-mono,theme-simple",
    ),
    ("twine-widgets", "std,gif,svg,bidi,arabic-shaping"),
    ("twine-widgets-ext", "std,vector,bidi,arabic-shaping"),
    ("twine-theme", "std,assets,lvgl,mono,simple"),
    ("twine-style", "std"),
    (
        "twine-engine",
        "std,fs,svg,perf-monitor,color-argb8888,color-i1,color-l8,color-rgb565,color-rgb565-swapped,color-rgb888,color-xrgb8888",
    ),
    ("twine-reactive", "std,async"),
    ("twine-layout", "std"),
    ("twine-anim", "std"),
    (
        "twine-render",
        "std,color-argb8888,color-i1,color-l8,color-rgb565,color-rgb565-swapped,color-rgb888,color-xrgb8888",
    ),
    ("twine-text", "std,ttf,bidi,arabic-shaping"),
    (
        "twine-image",
        "std,img-bmp,img-gif,img-jpeg,img-lz4,img-png,img-qoi",
    ),
    ("twine-vector", "std,svg"),
    ("twine-fs", "std,fs-std,fs-fat"),
    ("twine-assets", "fonts-all"),
    ("twine-extra", "std,i18n,qrcode,barcode"),
    ("twine-lottie", "std"),
    ("twine-hal", "async,platform-std"),
    ("twine-drivers", "all,std,async,testkit"),
    ("twine-embassy", ""),
    ("twine-embedded-graphics", "drawtarget"),
    ("twine-accel-stm32", "mock"),
    ("twine-sim", ""),
    ("twine-testing", "ui,engine,render"),
    ("twine-build", "font,image"),
    ("twine-macros", "assets"),
    ("twine-core", "std"),
];

/// Lines of crate docs kept (the rest is in rustdoc).
const CRATE_DOC_LINES: usize = 160;
/// Lines of module docs kept.
const MODULE_DOC_LINES: usize = 24;
/// Implementors listed under a trait.
const MAX_IMPLEMENTORS: usize = 40;

/// Generates `llms.txt` (or, with `check`, verifies it is current).
///
/// # Errors
/// When nightly is not installed, a rustdoc build fails, its JSON cannot be read, the file
/// cannot be written, or (with `check`) the file is out of date.
pub fn run(check: bool) -> R {
    if !nightly_available() {
        return Err("`cargo +nightly` is required (rustdoc JSON is nightly-only): \
                    `rustup toolchain install nightly`"
            .into());
    }
    let mut jsons = Vec::new();
    for &(name, features) in CRATES {
        jsons.push((name, features, rustdoc_json(name, features)?));
    }
    // Pass 1: every crate's documented paths, so a type of another crate is shown at the path
    // its section has in this file (not at a private defining module).
    let mut canons = Vec::new();
    let mut global = HashMap::new();
    for (name, features, json) in &jsons {
        let mut r = Renderer::new(json, name, features, &EMPTY_GLOBAL);
        r.canonical_paths();
        for (id, at) in &r.canon {
            if let Some(p) = r.paths.get(id)
                && p["crate_id"] == 0
            {
                let def: Vec<&str> = arr(&p["path"]).iter().map(s).collect();
                global.insert(def.join("::"), at.clone());
            }
        }
        canons.push(r.canon);
    }
    // Pass 2: render.
    let mut crates = Vec::new();
    for ((name, features, json), canon) in jsons.iter().zip(canons) {
        let mut r = Renderer::new(json, name, features, &global);
        r.canon = canon;
        crates.push(r.render());
    }
    let text = assemble(&crates);
    let path = workspace_root().join(OUT);
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != text {
            return Err(format!("{OUT} is out of date: run `cargo xtask llms-txt`").into());
        }
        eprintln!("{OUT}: up to date ({} lines)", text.lines().count());
        return Ok(());
    }
    std::fs::write(&path, &text)?;
    eprintln!(
        "wrote {OUT} ({} lines, {} KiB)",
        text.lines().count(),
        text.len() / 1024
    );
    Ok(())
}

/// `cargo +nightly` in the workspace: the rustup proxy (not `$CARGO`, which is pinned to the
/// stable toolchain) resolves `+nightly`.
fn nightly_cargo() -> Command {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root())
        .env_remove("RUSTUP_TOOLCHAIN")
        .arg("+nightly");
    cmd
}

/// Whether `cargo +nightly` runs.
#[must_use]
pub fn nightly_available() -> bool {
    nightly_cargo()
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// One rendered crate: its body lines and the headings to index.
struct CrateDoc {
    /// Body lines (headings included).
    lines: Vec<String>,
    /// `(line index in lines, index entry text)` of the crate and each module.
    index: Vec<(usize, String)>,
}

/// Builds and reads the rustdoc JSON of `name`.
fn rustdoc_json(name: &str, features: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let mut cmd = nightly_cargo();
    no_incremental(&mut cmd);
    cmd.args(["rustdoc", "-q", "-p", name, "--lib", "--target-dir", TARGET_DIR]);
    if !features.is_empty() {
        cmd.args(["--features", features]);
    }
    cmd.args(["--", "-Zunstable-options", "--output-format", "json"]);
    run_cmd(&mut cmd)?;
    let file = workspace_root()
        .join(TARGET_DIR)
        .join("doc")
        .join(format!("{}.json", name.replace('-', "_")));
    let json: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
    let version = json["format_version"].as_u64().unwrap_or(0);
    if version != FORMAT_VERSION {
        crate::util::warn(&format!(
            "{name}: rustdoc JSON format {version}, renderer written for {FORMAT_VERSION}"
        ));
    }
    Ok(json)
}

/// Preamble, index and every crate's body as one file.
fn assemble(crates: &[CrateDoc]) -> String {
    let mut head = vec![
        "# Twine — public API reference for AI coding agents".to_owned(),
        String::new(),
        "> Generated by `cargo xtask llms-txt` from the rustdoc comments — do not edit by hand.".to_owned(),
        "> Twine is a declarative, signal-driven GUI library in Rust for microcontrollers and".to_owned(),
        "> desktops (`no_std` + `alloc`). Applications depend on the facade crate `twine`, which".to_owned(),
        "> re-exports the other crates (`twine::view` = `twine_view`, `twine::widgets` =".to_owned(),
        "> `twine_widgets`, …) and offers `twine::prelude::*`.".to_owned(),
        String::new(),
        "## How to use this file".to_owned(),
        String::new(),
        "This file is large: do not read it whole. Find the section you need, then read only it.".to_owned(),
        String::new(),
        "1. Pick a crate or module from the **Index** below; each entry gives the line where its".to_owned(),
        "   section starts (`L1234`). Read from that line (e.g. offset 1234, limit 200).".to_owned(),
        "2. Or search the headings, which always start a line and use full paths:".to_owned(),
        "   - `^## crate twine_view` — a crate (its docs: concepts, features, an example)".to_owned(),
        "   - `^### mod twine_view::modifiers` — a module".to_owned(),
        "   - `^#### struct .*::Ui$`, `^#### trait .*::View$`, `^#### fn .*::run$`, `^#### enum`,".to_owned(),
        "     `^#### const`, `^#### macro`, `^#### type` — one item".to_owned(),
        "   - a method or field: search `fn <name>(` (methods) or `` `<name>: `` (fields).".to_owned(),
        "3. An item section is: `#### <kind> <path>`, a signature in a code span, the first".to_owned(),
        "   paragraph of its docs, then bullets for fields/variants/methods (signature — summary),"
            .to_owned(),
        "   with `Errors:`, `Panics:`, `Safety:` notes and the traits it implements.".to_owned(),
        "4. Paths are where an item is defined or first re-exported; through the facade the same".to_owned(),
        "   item is `twine::<module>::…` (see the facade's re-export list at the top).".to_owned(),
        "5. Documented with each crate's user-facing features, in the default build profile (the".to_owned(),
        "   `safety` profile removes some items and widens handles; see `twine_core`'s docs).".to_owned(),
        String::new(),
        "## Index".to_owned(),
        String::new(),
    ];
    let entries: usize = crates.iter().map(|c| c.index.len()).sum();
    // The body starts after the head, the index entries and one blank line.
    let body_start = head.len() + entries + 1;
    let mut offset = body_start;
    for c in crates {
        for (line, text) in &c.index {
            head.push(format!("{} (L{})", text, offset + line + 1));
        }
        offset += c.lines.len();
    }
    head.push(String::new());
    let mut out = head.join("\n");
    out.push('\n');
    for c in crates {
        for l in &c.lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// The map read when a JSON object is missing.
static EMPTY: std::sync::LazyLock<serde_json::Map<String, Value>> =
    std::sync::LazyLock::new(serde_json::Map::new);

/// The global path map of pass 1 (empty).
static EMPTY_GLOBAL: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);

/// Renders one crate's rustdoc JSON.
struct Renderer<'a> {
    index: &'a serde_json::Map<String, Value>,
    paths: &'a serde_json::Map<String, Value>,
    root: String,
    krate: String,
    features: String,
    lines: Vec<String>,
    idx: Vec<(usize, String)>,
    /// The path each reachable item and module is documented at: its shortest public path,
    /// definitions winning over re-exports at the same depth (see [`Renderer::canonical_paths`]).
    canon: HashMap<String, String>,
    /// Items already rendered.
    done: HashSet<String>,
    /// Defining path (`crate::module::Name`) → documented path, for every crate.
    global: &'a HashMap<String, String>,
}

/// The `id` of `v` as a map key (`"123"`).
fn id_key(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The single key of a `{"kind": {...}}` object and its payload.
fn tagged(v: &Value) -> (&str, &Value) {
    match v {
        Value::Object(m) => m
            .iter()
            .next()
            .map_or(("", &Value::Null), |(k, v)| (k.as_str(), v)),
        Value::String(s) => (s.as_str(), &Value::Null),
        _ => ("", &Value::Null),
    }
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

impl<'a> Renderer<'a> {
    fn new(json: &'a Value, krate: &str, features: &str, global: &'a HashMap<String, String>) -> Self {
        Self {
            global,
            index: json["index"].as_object().unwrap_or(&EMPTY),
            paths: json["paths"].as_object().unwrap_or(&EMPTY),
            root: id_key(&json["root"]).unwrap_or_default(),
            krate: krate.replace('-', "_"),
            features: features.to_owned(),
            lines: Vec::new(),
            idx: Vec::new(),
            canon: HashMap::new(),
            done: HashSet::new(),
        }
    }

    fn item(&self, id: &str) -> Option<&'a Value> {
        self.index.get(id)
    }

    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    fn render(mut self) -> CrateDoc {
        let root = self.root.clone();
        let Some(item) = self.item(&root) else {
            return CrateDoc {
                lines: self.lines,
                index: self.idx,
            };
        };
        if self.canon.is_empty() {
            self.canonical_paths();
        }
        let docs = s(&item["docs"]).to_owned();
        self.idx.push((
            self.lines.len(),
            format!("- crate `{}` — {}", self.krate, short(&first_sentence(&docs))),
        ));
        self.push(format!("## crate {}", self.krate));
        self.push(String::new());
        if self.features.is_empty() {
            self.push("Documented with the default features.".to_owned());
        } else {
            self.push(format!("Documented with features: `{}`.", self.features));
        }
        self.push(String::new());
        for l in doc_block(&docs, CRATE_DOC_LINES, true) {
            self.push(l);
        }
        self.push(String::new());
        let path = self.krate.clone();
        self.module_body(&root, &path);
        CrateDoc {
            lines: self.lines,
            index: self.idx,
        }
    }

    /// The public entries of module `id`: its items, with glob re-exports of local modules
    /// replaced by that module's entries (as Rust resolves them).
    fn entries(&self, id: &str, seen: &mut HashSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        if !seen.insert(id.to_owned()) {
            return out;
        }
        let Some(m) = self.item(id) else { return out };
        for it in arr(&m["inner"]["module"]["items"]).iter().filter_map(id_key) {
            let Some(item) = self.item(&it) else { continue };
            if item["visibility"] != "public" {
                continue;
            }
            if let ("use", u) = tagged(&item["inner"])
                && u["is_glob"].as_bool().unwrap_or(false)
                && let Some(t) = self.local(&u["id"])
                && tagged(&self.index[&t]["inner"]).0 == "module"
            {
                out.extend(self.entries(&t, seen));
                continue;
            }
            out.push(it);
        }
        out
    }

    /// The id of a local item (`crate_id` 0) referenced by `v`.
    fn local(&self, v: &Value) -> Option<String> {
        let id = id_key(v)?;
        (self.item(&id)?["crate_id"] == 0).then_some(id)
    }

    /// Fills [`Renderer::canon`]: breadth-first from the root, so every item gets its
    /// shortest public path; at each depth the items defined in the modules are placed before
    /// re-exports, so `pub use crate::slider::Slider` in a prelude does not move `Slider` out
    /// of `slider`.
    fn canonical_paths(&mut self) {
        let mut level = vec![(self.root.clone(), self.krate.clone())];
        self.canon.insert(self.root.clone(), self.krate.clone());
        while !level.is_empty() {
            let mut next = Vec::new();
            for (m, p) in &level {
                for e in self.entries(m, &mut HashSet::new()) {
                    let item = &self.index[&e];
                    let kind = tagged(&item["inner"]).0;
                    if matches!(kind, "use" | "impl") || self.canon.contains_key(&e) {
                        continue;
                    }
                    let full = format!("{p}::{}", s(&item["name"]));
                    if kind == "module" {
                        next.push((e.clone(), full.clone()));
                    }
                    self.canon.insert(e, full);
                }
            }
            for (m, p) in &level {
                for e in self.entries(m, &mut HashSet::new()) {
                    let (kind, u) = tagged(&self.index[&e]["inner"]);
                    if kind != "use" || u["is_glob"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let Some(t) = self.local(&u["id"]) else { continue };
                    if self.canon.contains_key(&t) {
                        continue;
                    }
                    let full = format!("{p}::{}", s(&u["name"]));
                    if tagged(&self.index[&t]["inner"]).0 == "module" {
                        next.push((t.clone(), full.clone()));
                    }
                    self.canon.insert(t, full);
                }
            }
            level = next;
        }
    }

    fn render_module(&mut self, id: &str, path: &str) {
        let Some(m) = self.item(id) else { return };
        let docs = s(&m["docs"]).to_owned();
        self.idx.push((
            self.lines.len(),
            format!("  - mod `{path}` — {}", short(&first_sentence(&docs))),
        ));
        self.push(format!("### mod {path}"));
        self.push(String::new());
        let block = doc_block(&docs, MODULE_DOC_LINES, false);
        if !block.is_empty() {
            for l in block {
                self.push(l);
            }
            self.push(String::new());
        }
        self.module_body(id, path);
    }

    /// Renders the items whose canonical path is in module `id` (at `path`), lists the other
    /// re-exports, then renders the submodules.
    fn module_body(&mut self, id: &str, path: &str) {
        let mut children = Vec::new();
        let mut reexports = Vec::new();
        for e in self.entries(id, &mut HashSet::new()) {
            let item = &self.index[&e];
            let (kind, u) = tagged(&item["inner"]);
            match kind {
                "impl" => {}
                "use" => {
                    let name = s(&u["name"]).to_owned();
                    let glob = u["is_glob"].as_bool().unwrap_or(false);
                    match self.local(&u["id"]) {
                        Some(_) if glob => reexports.push(format!("`pub use {}::*`", s(&u["source"]))),
                        Some(t) => {
                            let here = format!("{path}::{name}");
                            let at = self.canon.get(&t).cloned().unwrap_or_default();
                            if at != here {
                                reexports.push(format!("`{name}` — same as `{at}`"));
                            } else if tagged(&self.index[&t]["inner"]).0 == "module" {
                                children.push((t, here));
                            } else {
                                self.render_item(&t, path, Some(&name));
                            }
                        }
                        None => reexports.push(self.external_reexport(u, &name, glob, path)),
                    }
                }
                _ => {
                    let here = format!("{path}::{}", s(&item["name"]));
                    let at = self.canon.get(&e).cloned().unwrap_or_default();
                    if at != here {
                        reexports.push(format!("`{}` — same as `{at}`", s(&item["name"])));
                    } else if kind == "module" {
                        children.push((e, here));
                    } else {
                        self.render_item(&e, path, None);
                    }
                }
            }
        }
        if !reexports.is_empty() {
            self.push(format!("Re-exports in `{path}`:"));
            for r in reexports {
                self.push(format!("- {r}"));
            }
            self.push(String::new());
        }
        for (mid, mpath) in children {
            self.render_module(&mid, &mpath);
        }
    }

    /// A re-export of another crate's item: `` `Name` = `other::Name` ``, or of a whole crate.
    fn external_reexport(&self, u: &Value, name: &str, glob: bool, path: &str) -> String {
        // A whole crate (the facade's `pub use twine_view as view`): point at its section.
        if let Some(p) = id_key(&u["id"]).and_then(|t| self.paths.get(&t))
            && p["kind"] == "module"
            && arr(&p["path"]).len() == 1
        {
            let krate = s(&arr(&p["path"])[0]);
            return format!("`{path}::{name}` → crate `{krate}` (section `## crate {krate}`)");
        }
        let shown = id_key(&u["id"]).and_then(|t| self.paths.get(&t)).map_or_else(
            || s(&u["source"]).to_owned(),
            |p| {
                let def = arr(&p["path"]).iter().map(s).collect::<Vec<_>>().join("::");
                self.global.get(&def).cloned().unwrap_or(def)
            },
        );
        if glob {
            return format!("`pub use {shown}::*`");
        }
        let tail = shown.rsplit("::").next().unwrap_or(&shown);
        if tail == name {
            format!("`{name}` = `{shown}`")
        } else {
            format!("`{name}` = `{shown}` (renamed)")
        }
    }

    /// Renders item `id` as `#### <kind> <path>::<name>`.
    fn render_item(&mut self, id: &str, path: &str, rename: Option<&str>) {
        let Some(item) = self.item(id) else { return };
        if !self.done.insert(id.to_owned()) {
            return;
        }
        let name = rename.map_or_else(|| s(&item["name"]).to_owned(), str::to_owned);
        let full = format!("{path}::{name}");
        let docs = s(&item["docs"]).to_owned();
        let (kind, inner) = tagged(&item["inner"]);
        match kind {
            "struct" | "union" | "enum" => self.render_type(kind, &full, &name, inner, &docs),
            "trait" => self.render_trait(&full, &name, inner, &docs),
            "function" => {
                self.push(format!("#### fn {full}"));
                self.push(format!("`{}`", self.fn_sig(&name, inner)));
                self.summary(&docs);
            }
            "constant" => {
                let value = s(&inner["const"]["expr"]);
                let ty = self.ty(&inner["type"]);
                self.push(format!("#### const {full}"));
                if value.len() <= 40 && !value.is_empty() && value != "_" {
                    self.push(format!("`pub const {name}: {ty} = {value}`"));
                } else {
                    self.push(format!("`pub const {name}: {ty}`"));
                }
                self.summary(&docs);
            }
            "static" => {
                let ty = self.ty(&inner["type"]);
                let m = if inner["is_mutable"].as_bool().unwrap_or(false) {
                    "mut "
                } else {
                    ""
                };
                self.push(format!("#### static {full}"));
                self.push(format!("`pub static {m}{name}: {ty}`"));
                self.summary(&docs);
            }
            "type_alias" => {
                let (g, w) = self.generics(&inner["generics"]);
                self.push(format!("#### type {full}"));
                self.push(format!("`pub type {name}{g} = {}{w}`", self.ty(&inner["type"])));
                self.summary(&docs);
            }
            "macro" => {
                self.push(format!("#### macro {full}!"));
                let src = inner.as_str().unwrap_or("");
                let rules: Vec<&str> = src.lines().take(12).collect();
                self.push("```rust".to_owned());
                for l in rules {
                    self.push(l.to_owned());
                }
                self.push("```".to_owned());
                self.summary(&docs);
            }
            "proc_macro" => {
                let k = s(&inner["kind"]);
                let shown = match k {
                    "derive" => format!("#[derive({name})]"),
                    "attr" => format!("#[{name}]"),
                    _ => format!("{name}!(…)"),
                };
                self.push(format!("#### macro {full}"));
                self.push(format!("`{shown}`"));
                self.summary(&docs);
            }
            "primitive" | "extern_crate" | "struct_field" | "variant" | "impl" | "use" | "module" => {}
            other => {
                self.push(format!("#### {other} {full}"));
                self.summary(&docs);
            }
        }
    }

    /// First paragraph of `docs` plus its `Errors`/`Panics`/`Safety` notes, then a blank line.
    fn summary(&mut self, docs: &str) {
        let first = first_paragraph(docs);
        if !first.is_empty() {
            self.push(first);
        }
        for (title, text) in notes(docs) {
            self.push(format!("{title}: {text}"));
        }
        self.push(String::new());
    }

    fn render_type(&mut self, kind: &str, full: &str, name: &str, inner: &Value, docs: &str) {
        let (g, w) = self.generics(&inner["generics"]);
        self.push(format!("#### {kind} {full}{g}"));
        if !w.is_empty() {
            self.push(format!("`{}`", w.trim_start()));
        }
        let first = first_paragraph(docs);
        if !first.is_empty() {
            self.push(first);
        }
        // Fields.
        match kind {
            "struct" | "union" => {
                let (sk, sv) = tagged(&inner["kind"]);
                let fields: Vec<Option<String>> = match sk {
                    "plain" => arr(&sv["fields"]).iter().map(id_key).collect(),
                    "tuple" => arr(sv).iter().map(id_key).collect(),
                    _ => arr(&inner["fields"]).iter().map(id_key).collect(),
                };
                let shown: Vec<String> = fields
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| {
                        let fi = self.item(f.as_deref()?)?;
                        if fi["visibility"] != "public" {
                            return None;
                        }
                        let fname = if sk == "tuple" {
                            i.to_string()
                        } else {
                            s(&fi["name"]).to_owned()
                        };
                        let ty = self.ty(&fi["inner"]["struct_field"]);
                        Some(bullet(&format!("{fname}: {ty}"), s(&fi["docs"])))
                    })
                    .collect();
                if !shown.is_empty() {
                    self.push("Fields:".to_owned());
                    for l in shown {
                        self.push(l);
                    }
                }
            }
            "enum" => {
                let vars: Vec<String> = arr(&inner["variants"])
                    .iter()
                    .filter_map(id_key)
                    .filter_map(|v| {
                        let vi = self.item(&v)?;
                        Some(bullet(&self.variant(vi), s(&vi["docs"])))
                    })
                    .collect();
                if !vars.is_empty() {
                    self.push("Variants:".to_owned());
                    for l in vars {
                        self.push(l);
                    }
                }
            }
            _ => {}
        }
        self.impls(inner, name);
        for (title, text) in notes(docs) {
            self.push(format!("{title}: {text}"));
        }
        self.push(String::new());
    }

    /// Inherent methods/constants and the implemented traits of a type.
    fn impls(&mut self, inner: &Value, self_name: &str) {
        let mut methods = Vec::new();
        let mut traits = Vec::new();
        for imp in arr(&inner["impls"]).iter().filter_map(id_key) {
            let Some(i) = self.item(&imp) else { continue };
            let ii = &i["inner"]["impl"];
            if ii["is_synthetic"].as_bool().unwrap_or(false) || !ii["blanket_impl"].is_null() {
                continue;
            }
            if ii["trait"].is_null() {
                for m in arr(&ii["items"]).iter().filter_map(id_key) {
                    let Some(mi) = self.item(&m) else { continue };
                    if mi["visibility"] != "public" {
                        continue;
                    }
                    let mname = s(&mi["name"]);
                    let (k, v) = tagged(&mi["inner"]);
                    let sig = match k {
                        "function" => self.fn_sig(mname, v),
                        "assoc_const" => format!("const {mname}: {}", self.ty(&v["type"])),
                        "assoc_type" => format!("type {mname}"),
                        _ => continue,
                    };
                    methods.push(method_bullet(&sig, s(&mi["docs"])));
                }
            } else {
                let t = &ii["trait"];
                let tn = s(&t["path"]);
                let tn = tn.rsplit("::").next().unwrap_or(tn);
                // Compiler-internal marker of `#[derive(PartialEq)]`.
                if tn == "StructuralPartialEq" {
                    continue;
                }
                let neg = if ii["is_negative"].as_bool().unwrap_or(false) {
                    "!"
                } else {
                    ""
                };
                traits.push(format!("{neg}{tn}{}", self.args(&t["args"])));
            }
        }
        if !methods.is_empty() {
            self.push(format!("Methods of `{self_name}`:"));
            for m in methods {
                for l in m {
                    self.push(l);
                }
            }
        }
        if !traits.is_empty() {
            self.push(format!("Implements: {}", traits.join(", ")));
        }
    }

    fn render_trait(&mut self, full: &str, name: &str, inner: &Value, docs: &str) {
        let (g, w) = self.generics(&inner["generics"]);
        let bounds = self.bounds(arr(&inner["bounds"]));
        let sup = if bounds.is_empty() {
            String::new()
        } else {
            format!(": {bounds}")
        };
        self.push(format!("#### trait {full}{g}"));
        self.push(format!("`pub trait {name}{g}{sup}{w}`"));
        let first = first_paragraph(docs);
        if !first.is_empty() {
            self.push(first);
        }
        let mut items = Vec::new();
        for m in arr(&inner["items"]).iter().filter_map(id_key) {
            let Some(mi) = self.item(&m) else { continue };
            let mname = s(&mi["name"]);
            let (k, v) = tagged(&mi["inner"]);
            let sig = match k {
                "function" => {
                    let sig = self.fn_sig(mname, v);
                    let sig = sig.strip_prefix("pub ").unwrap_or(&sig).to_owned();
                    if v["has_body"].as_bool().unwrap_or(false) {
                        format!("{sig} (provided)")
                    } else {
                        sig
                    }
                }
                "assoc_const" => format!("const {mname}: {}", self.ty(&v["type"])),
                "assoc_type" => {
                    let b = self.bounds(arr(&v["bounds"]));
                    if b.is_empty() {
                        format!("type {mname}")
                    } else {
                        format!("type {mname}: {b}")
                    }
                }
                _ => continue,
            };
            items.push(method_bullet(&sig, s(&mi["docs"])));
        }
        if !items.is_empty() {
            self.push("Items:".to_owned());
            for m in items {
                for l in m {
                    self.push(l);
                }
            }
        }
        let implementors: Vec<String> = arr(&inner["implementations"])
            .iter()
            .filter_map(id_key)
            .filter_map(|i| {
                let ii = &self.item(&i)?["inner"]["impl"];
                if ii["is_synthetic"].as_bool().unwrap_or(false) {
                    return None;
                }
                Some(self.ty(&ii["for"]))
            })
            .collect();
        if !implementors.is_empty() {
            let more = implementors.len().saturating_sub(MAX_IMPLEMENTORS);
            let mut list = implementors
                .into_iter()
                .take(MAX_IMPLEMENTORS)
                .collect::<Vec<_>>()
                .join(", ");
            if more > 0 {
                let _ = write!(list, ", … ({more} more)");
            }
            self.push(format!("Implemented by: {list}"));
        }
        for (title, text) in notes(docs) {
            self.push(format!("{title}: {text}"));
        }
        self.push(String::new());
    }

    fn variant(&self, v: &Value) -> String {
        let name = s(&v["name"]);
        let (k, kv) = tagged(&v["inner"]["variant"]["kind"]);
        let field = |f: &Value| -> Option<(String, String)> {
            let fi = self.item(&id_key(f)?)?;
            Some((s(&fi["name"]).to_owned(), self.ty(&fi["inner"]["struct_field"])))
        };
        match k {
            "tuple" => {
                let tys: Vec<String> = arr(kv).iter().filter_map(field).map(|(_, t)| t).collect();
                format!("{name}({})", tys.join(", "))
            }
            "struct" => {
                let fs: Vec<String> = arr(&kv["fields"])
                    .iter()
                    .filter_map(field)
                    .map(|(n, t)| format!("{n}: {t}"))
                    .collect();
                format!("{name} {{ {} }}", fs.join(", "))
            }
            _ => match v["inner"]["variant"]["discriminant"]["expr"].as_str() {
                Some(d) => format!("{name} = {d}"),
                None => name.to_owned(),
            },
        }
    }

    /// `pub [const] [async] [unsafe] fn name<G>(args) -> R where …`.
    fn fn_sig(&self, name: &str, f: &Value) -> String {
        let h = &f["header"];
        let mut q = String::from("fn ");
        if h["is_unsafe"].as_bool().unwrap_or(false) {
            q.insert_str(0, "unsafe ");
        }
        if h["is_async"].as_bool().unwrap_or(false) {
            q.insert_str(0, "async ");
        }
        if h["is_const"].as_bool().unwrap_or(false) {
            q.insert_str(0, "const ");
        }
        let (g, w) = self.generics(&f["generics"]);
        let args: Vec<String> = arr(&f["sig"]["inputs"])
            .iter()
            .map(|inp| {
                let n = s(&inp[0]);
                let t = &inp[1];
                if n == "self" {
                    let (k, v) = tagged(t);
                    match k {
                        "generic" if s(v) == "Self" => "self".to_owned(),
                        "borrowed_ref" if v["type"]["generic"] == "Self" => {
                            let lt = v["lifetime"]
                                .as_str()
                                .map(|l| format!("{l} "))
                                .unwrap_or_default();
                            let m = if v["is_mutable"].as_bool().unwrap_or(false) {
                                "mut "
                            } else {
                                ""
                            };
                            format!("&{lt}{m}self")
                        }
                        _ => format!("self: {}", self.ty(t)),
                    }
                } else {
                    format!("{n}: {}", self.ty(t))
                }
            })
            .collect();
        let out = &f["sig"]["output"];
        let ret = if out.is_null() {
            String::new()
        } else {
            format!(" -> {}", self.ty(out))
        };
        format!("pub {q}{name}{g}({}){ret}{w}", args.join(", "))
    }

    /// `<params>` and ` where …` of a generics object (synthetic `impl Trait` params skipped).
    fn generics(&self, g: &Value) -> (String, String) {
        let params: Vec<String> = arr(&g["params"])
            .iter()
            .filter_map(|p| {
                let name = s(&p["name"]);
                let (k, v) = tagged(&p["kind"]);
                match k {
                    "lifetime" => {
                        let o: Vec<&str> = arr(&v["outlives"]).iter().map(s).collect();
                        Some(if o.is_empty() {
                            name.to_owned()
                        } else {
                            format!("{name}: {}", o.join(" + "))
                        })
                    }
                    "type" => {
                        if v["is_synthetic"].as_bool().unwrap_or(false) {
                            return None;
                        }
                        let b = self.bounds(arr(&v["bounds"]));
                        let mut out = if b.is_empty() {
                            name.to_owned()
                        } else {
                            format!("{name}: {b}")
                        };
                        if !v["default"].is_null() {
                            let _ = write!(out, " = {}", self.ty(&v["default"]));
                        }
                        Some(out)
                    }
                    "const" => Some(format!("const {name}: {}", self.ty(&v["type"]))),
                    _ => None,
                }
            })
            .collect();
        let preds: Vec<String> = arr(&g["where_predicates"])
            .iter()
            .filter_map(|p| {
                let (k, v) = tagged(p);
                match k {
                    "bound_predicate" => {
                        let b = self.bounds(arr(&v["bounds"]));
                        Some(format!("{}: {b}", self.ty(&v["type"])))
                    }
                    "lifetime_predicate" => {
                        let o: Vec<&str> = arr(&v["outlives"]).iter().map(s).collect();
                        Some(format!("{}: {}", s(&v["lifetime"]), o.join(" + ")))
                    }
                    "eq_predicate" => Some(format!("{} = {}", self.ty(&v["lhs"]), self.term(&v["rhs"]))),
                    _ => None,
                }
            })
            .collect();
        let g = if params.is_empty() {
            String::new()
        } else {
            format!("<{}>", params.join(", "))
        };
        let w = if preds.is_empty() {
            String::new()
        } else {
            format!(" where {}", preds.join(", "))
        };
        (g, w)
    }

    fn term(&self, t: &Value) -> String {
        let (k, v) = tagged(t);
        match k {
            "type" => self.ty(v),
            "constant" => s(&v["expr"]).to_owned(),
            _ => "_".to_owned(),
        }
    }

    fn bounds(&self, bs: &[Value]) -> String {
        bs.iter()
            .map(|b| {
                let (k, v) = tagged(b);
                match k {
                    "trait_bound" => {
                        let m = match s(&v["modifier"]) {
                            "maybe" => "?",
                            "maybe_const" => "~const ",
                            _ => "",
                        };
                        let hr = hrtb(arr(&v["generic_params"]));
                        format!(
                            "{hr}{m}{}{}",
                            self.path_of(&v["trait"]),
                            self.args(&v["trait"]["args"])
                        )
                    }
                    "outlives" => s(v).to_owned(),
                    "use" => "use<..>".to_owned(),
                    _ => String::new(),
                }
            })
            .filter(|b| !b.is_empty())
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// The path of a `{path, id, args}` reference: a local item's documented path, another
    /// workspace crate's full path, the bare name for `core`/`alloc`/`std` items and generics.
    fn path_of(&self, v: &Value) -> String {
        let written = s(&v["path"]);
        let Some(id) = id_key(&v["id"]) else {
            return written.to_owned();
        };
        if let Some(at) = self.canon.get(&id) {
            return at.clone();
        }
        if let Some(p) = self.paths.get(&id) {
            let segs: Vec<&str> = arr(&p["path"]).iter().map(s).collect();
            if let Some(first) = segs.first()
                && !matches!(*first, "core" | "alloc" | "std")
                && p["crate_id"] != 0
            {
                let def = segs.join("::");
                return self.global.get(&def).cloned().unwrap_or(def);
            }
        }
        written.rsplit("::").next().unwrap_or(written).to_owned()
    }

    /// Generic arguments `<A, B, Item = C>` / `(A) -> B`.
    fn args(&self, a: &Value) -> String {
        if a.is_null() {
            return String::new();
        }
        let (k, v) = tagged(a);
        match k {
            "angle_bracketed" => {
                let mut parts: Vec<String> = arr(&v["args"])
                    .iter()
                    .map(|g| {
                        let (gk, gv) = tagged(g);
                        match gk {
                            "lifetime" => s(gv).to_owned(),
                            "type" => self.ty(gv),
                            "const" => s(&gv["expr"]).to_owned(),
                            _ => "_".to_owned(),
                        }
                    })
                    .collect();
                for c in arr(&v["constraints"]) {
                    let name = s(&c["name"]);
                    let (bk, bv) = tagged(&c["binding"]);
                    parts.push(match bk {
                        "equality" => format!("{name}{} = {}", self.args(&c["args"]), self.term(bv)),
                        "constraint" => format!("{name}: {}", self.bounds(arr(bv))),
                        _ => name.to_owned(),
                    });
                }
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("<{}>", parts.join(", "))
                }
            }
            "parenthesized" => {
                let ins: Vec<String> = arr(&v["inputs"]).iter().map(|t| self.ty(t)).collect();
                let out = if v["output"].is_null() {
                    String::new()
                } else {
                    format!(" -> {}", self.ty(&v["output"]))
                };
                format!("({}){out}", ins.join(", "))
            }
            "return_type_notation" => "(..)".to_owned(),
            _ => String::new(),
        }
    }

    /// A type as Rust source.
    fn ty(&self, t: &Value) -> String {
        let (k, v) = tagged(t);
        match k {
            "resolved_path" => format!("{}{}", self.path_of(v), self.args(&v["args"])),
            "dyn_trait" => {
                let traits: Vec<String> = arr(&v["traits"])
                    .iter()
                    .map(|p| {
                        let hr = hrtb(arr(&p["generic_params"]));
                        format!(
                            "{hr}{}{}",
                            self.path_of(&p["trait"]),
                            self.args(&p["trait"]["args"])
                        )
                    })
                    .collect();
                let lt = v["lifetime"]
                    .as_str()
                    .map(|l| format!(" + {l}"))
                    .unwrap_or_default();
                format!("dyn {}{lt}", traits.join(" + "))
            }
            "generic" | "primitive" => s(v).to_owned(),
            "tuple" => {
                let ts: Vec<String> = arr(v).iter().map(|t| self.ty(t)).collect();
                if ts.len() == 1 {
                    format!("({},)", ts[0])
                } else {
                    format!("({})", ts.join(", "))
                }
            }
            "slice" => format!("[{}]", self.ty(v)),
            "array" => format!("[{}; {}]", self.ty(&v["type"]), s(&v["len"])),
            "pat" => self.ty(&v["type"]),
            "impl_trait" => format!("impl {}", self.bounds(arr(v))),
            "raw_pointer" => {
                let m = if v["is_mutable"].as_bool().unwrap_or(false) {
                    "mut"
                } else {
                    "const"
                };
                format!("*{m} {}", self.ty(&v["type"]))
            }
            "borrowed_ref" => {
                let lt = v["lifetime"]
                    .as_str()
                    .map(|l| format!("{l} "))
                    .unwrap_or_default();
                let m = if v["is_mutable"].as_bool().unwrap_or(false) {
                    "mut "
                } else {
                    ""
                };
                format!("&{lt}{m}{}", self.ty(&v["type"]))
            }
            "qualified_path" => {
                let name = s(&v["name"]);
                let st = self.ty(&v["self_type"]);
                if v["trait"].is_null() || st == "Self" {
                    format!("{st}::{name}{}", self.args(&v["args"]))
                } else {
                    format!(
                        "<{st} as {}>::{name}{}",
                        self.path_of(&v["trait"]),
                        self.args(&v["args"])
                    )
                }
            }
            "function_pointer" => {
                let ins: Vec<String> = arr(&v["sig"]["inputs"]).iter().map(|i| self.ty(&i[1])).collect();
                let out = &v["sig"]["output"];
                let ret = if out.is_null() {
                    String::new()
                } else {
                    format!(" -> {}", self.ty(out))
                };
                format!("fn({}){ret}", ins.join(", "))
            }
            _ => "_".to_owned(),
        }
    }
}

/// `for<'a> ` of higher-ranked bounds (empty without parameters).
fn hrtb(params: &[Value]) -> String {
    let names: Vec<&str> = params.iter().map(|p| s(&p["name"])).collect();
    if names.is_empty() {
        String::new()
    } else {
        format!("for<{}> ", names.join(", "))
    }
}

/// A one-line bullet: the signature in a code span, then the summary.
fn bullet(sig: &str, docs: &str) -> String {
    let first = first_paragraph(docs);
    if first.is_empty() {
        format!("- `{sig}`")
    } else {
        format!("- `{sig}` — {first}")
    }
}

/// A method bullet plus indented `Errors:`/`Panics:`/`Safety:` lines.
fn method_bullet(sig: &str, docs: &str) -> Vec<String> {
    let mut v = vec![bullet(sig, docs)];
    for (title, text) in notes(docs) {
        v.push(format!("  {title}: {text}"));
    }
    v
}

/// Shortens an index summary to one line.
fn short(s: &str) -> String {
    const MAX: usize = 110;
    if s.chars().count() <= MAX {
        return s.to_owned();
    }
    let cut: String = s.chars().take(MAX).collect();
    format!("{}…", cut.trim_end())
}

/// The first sentence of the first paragraph.
fn first_sentence(docs: &str) -> String {
    let p = first_paragraph(docs);
    match p.find(". ") {
        Some(i) => p[..=i].to_owned(),
        None => p,
    }
}

/// The first prose paragraph (code blocks and headings skipped), on one line, links flattened.
fn first_paragraph(docs: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut fence = false;
    for line in docs.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            fence = !fence;
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if fence {
            continue;
        }
        if t.is_empty() {
            if out.is_empty() {
                continue;
            }
            break;
        }
        if t.starts_with('#') && out.is_empty() {
            continue;
        }
        out.push(t);
    }
    flatten_links(&out.join(" "))
}

/// The first paragraph of each `# Errors` / `# Panics` / `# Safety` section.
fn notes(docs: &str) -> Vec<(&'static str, String)> {
    let mut res = Vec::new();
    for (title, heading) in [("Errors", "errors"), ("Panics", "panics"), ("Safety", "safety")] {
        let mut lines = docs.lines();
        let found = lines.by_ref().any(|l| {
            let t = l.trim();
            t.starts_with('#') && t.trim_start_matches('#').trim().eq_ignore_ascii_case(heading)
        });
        if found {
            let rest: Vec<&str> = lines.collect();
            let text = first_paragraph(&rest.join("\n"));
            if !text.is_empty() {
                res.push((title, text));
            }
        }
    }
    res
}

/// Crate/module docs as lines: headings demoted to bold, hidden doctest lines dropped, links
/// flattened; code blocks kept only when `code`; at most `max` lines.
fn doc_block(docs: &str, max: usize, code: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    let mut truncated = false;
    for line in docs.lines() {
        let t = line.trim_start();
        if t.starts_with("```") {
            fence = !fence;
            if code {
                out.push(if fence {
                    "```rust".to_owned()
                } else {
                    "```".to_owned()
                });
            }
            continue;
        }
        if fence {
            if !code || t == "#" || t.starts_with("# ") {
                continue;
            }
            out.push(line.to_owned());
        } else if let Some(h) = t.strip_prefix('#') {
            out.push(format!("**{}**", h.trim_start_matches('#').trim()));
        } else {
            out.push(flatten_links(line));
        }
        if out.len() >= max {
            truncated = true;
            break;
        }
    }
    if fence && code && truncated {
        out.push("```".to_owned());
    }
    if truncated {
        out.push("(… see the full docs in rustdoc)".to_owned());
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Rewrites rustdoc links outside code spans: `[`X`](…)`, `[`X`]`, `[text][ref]` and
/// `[text](url)` become their text.
fn flatten_links(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut code = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            code = !code;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '[' && !code {
            // Find the matching `]`, skipping code spans.
            let mut j = i + 1;
            let mut depth = 1;
            let mut in_code = false;
            while j < chars.len() {
                match chars[j] {
                    '`' => in_code = !in_code,
                    '[' if !in_code => depth += 1,
                    ']' if !in_code => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j < chars.len() {
                let text: String = chars[i + 1..j].iter().collect();
                let mut k = j + 1;
                // A `(target)` or `[ref]` right after the text belongs to the link.
                if k < chars.len() && (chars[k] == '(' || chars[k] == '[') {
                    let close = if chars[k] == '(' { ')' } else { ']' };
                    if let Some(end) = chars[k..].iter().position(|&ch| ch == close) {
                        k += end + 1;
                    }
                }
                out.push_str(&flatten_links(&text));
                i = k;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_become_code_spans_and_text() {
        assert_eq!(
            flatten_links("See [`Rect`] and [the docs](crate::x)."),
            "See `Rect` and the docs."
        );
        assert_eq!(
            flatten_links("[`Ui::new`](crate::Ui::new) builds"),
            "`Ui::new` builds"
        );
        assert_eq!(flatten_links("a `[u8; 4]` array"), "a `[u8; 4]` array");
        assert_eq!(flatten_links("[label][ref] x"), "label x");
    }

    #[test]
    fn first_paragraph_skips_code_and_joins_lines() {
        let d = "Builds a UI.\nIt runs once.\n\nMore text.\n```\ncode\n```";
        assert_eq!(first_paragraph(d), "Builds a UI. It runs once.");
        assert_eq!(first_sentence(d), "Builds a UI.");
    }

    #[test]
    fn notes_take_the_first_paragraph_of_each_section() {
        let d = "Does it.\n\n# Errors\nWhen the [`Limits`] are hit.\n\nDetails.\n\n# Panics\nNever panics.";
        let n = notes(d);
        assert_eq!(
            n,
            vec![
                ("Errors", "When the `Limits` are hit.".to_owned()),
                ("Panics", "Never panics.".to_owned())
            ]
        );
    }

    #[test]
    fn doc_blocks_demote_headings_and_hide_doctest_lines() {
        let d = "# Title\nText\n```\n# use x;\nlet a = 1;\n```";
        assert_eq!(
            doc_block(d, 50, true),
            vec!["**Title**", "Text", "```rust", "let a = 1;", "```"]
        );
        assert_eq!(doc_block(d, 50, false), vec!["**Title**", "Text"]);
    }

    #[test]
    fn every_documented_crate_is_a_workspace_library() {
        let root = workspace_root();
        for (name, _) in CRATES {
            assert!(
                root.join("crates").join(name).join("Cargo.toml").is_file(),
                "{name}"
            );
        }
    }
}
