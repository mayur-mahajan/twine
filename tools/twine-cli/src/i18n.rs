//! `twine i18n`: compiles translation files (Fluent `.ftl`, gettext `.po`, TOML) into the Rust
//! catalog of `twine_extra::i18n` — the command-line form of `twine_build::i18n` (which build
//! scripts use directly).
//!
//! ```no_run
//! use twine_cli::i18n::{I18nArgs, run};
//!
//! let args = I18nArgs {
//!     inputs: vec!["i18n".into()],
//!     out: "src/i18n.rs".into(),
//!     default: Some("en".into()),
//!     languages: Vec::new(),
//!     font: vec!["zh=crate::fonts::CJK".into()],
//!     dir: Vec::new(),
//!     plural: Vec::new(),
//!     fallback: Vec::new(),
//!     strict: true,
//!     runtime: None,
//! };
//! run(&args).unwrap();
//! ```

use std::path::PathBuf;

use twine_build::i18n::{Dir, Translations};

/// Arguments of `twine i18n`.
#[derive(Clone, Debug, clap::Args)]
pub struct I18nArgs {
    /// Translation directories (every `<language>.ftl`, `.po` and `.toml` in them) or files.
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    /// The Rust file to write.
    #[arg(short, long)]
    pub out: PathBuf,
    /// The default language (required with several languages unless `--languages` is given:
    /// then its first entry).
    #[arg(long)]
    pub default: Option<String>,
    /// The languages to compile, in `Lang` order (comma-separated; default: all, the default
    /// first).
    #[arg(long, value_delimiter = ',')]
    pub languages: Vec<String>,
    /// A language's font as a Rust path: `zh=crate::fonts::CJK` (repeatable).
    #[arg(long)]
    pub font: Vec<String>,
    /// A language's base direction: `xx=rtl` or `xx=ltr` (repeatable).
    #[arg(long)]
    pub dir: Vec<String>,
    /// A language's plural rules by name: `xx=ru` (repeatable; see `twine_build::i18n::RULES`).
    #[arg(long)]
    pub plural: Vec<String>,
    /// A language's fallback: `de-AT=de` (repeatable).
    #[arg(long)]
    pub fallback: Vec<String>,
    /// Treat warnings (missing translations, unknown keys, …) as errors.
    #[arg(long)]
    pub strict: bool,
    /// The runtime module path the generated code names (default `::twine_extra::i18n`).
    #[arg(long)]
    pub runtime: Option<String>,
}

fn pairs(list: &[String], what: &str) -> Result<Vec<(String, String)>, String> {
    list.iter()
        .map(|p| {
            p.split_once('=')
                .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
                .ok_or_else(|| format!("--{what} {p:?}: expected `<language>=<value>`"))
        })
        .collect()
}

/// The [`Translations`] the arguments describe.
///
/// # Errors
/// A malformed `--font` / `--dir` / `--plural` / `--fallback` value.
pub fn translations(args: &I18nArgs) -> Result<Translations, String> {
    let mut t = Translations::new();
    for input in &args.inputs {
        t = if input.is_dir() {
            t.dir(input)
        } else {
            t.file(input)
        };
    }
    if let Some(d) = &args.default {
        t = t.default_language(d);
    }
    if !args.languages.is_empty() {
        t = t.languages(args.languages.iter().cloned());
    }
    for (code, path) in pairs(&args.font, "font")? {
        t = t.font(&code, &path);
    }
    for (code, dir) in pairs(&args.dir, "dir")? {
        let dir = match dir.as_str() {
            "ltr" => Dir::Ltr,
            "rtl" => Dir::Rtl,
            _ => return Err(format!("--dir {code}={dir}: expected `ltr` or `rtl`")),
        };
        t = t.base_dir(&code, dir);
    }
    for (code, rules) in pairs(&args.plural, "plural")? {
        t = t.plural_rules(&code, &rules);
    }
    for (code, fallback) in pairs(&args.fallback, "fallback")? {
        t = t.fallback(&code, &fallback);
    }
    if let Some(r) = &args.runtime {
        t = t.runtime_path(r);
    }
    Ok(t.strict(args.strict))
}

/// Builds the catalog, prints its warnings to stderr and writes `args.out` (only when it
/// changed).
///
/// # Errors
/// Every problem found in the translations, or an I/O error.
pub fn run(args: &I18nArgs) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = translations(args)?.build()?;
    for w in catalog.warnings() {
        eprintln!("warning: {w}");
    }
    catalog.write(&args.out)?;
    println!(
        "i18n: {} ({} languages, {} keys)",
        args.out.display(),
        catalog.languages().count(),
        catalog.keys().count()
    );
    Ok(())
}
