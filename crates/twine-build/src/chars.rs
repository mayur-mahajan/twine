//! Character sets for glyph subsetting: which characters a generated font contains.
//!
//! A bitmap font only needs the glyphs the application shows. [`CharSource`] is anything that
//! names characters — a typed range (`'\u{20}'..='\u{7E}'`), a character, a string, a
//! translation [`Catalog`] (every character of every message in every language, and the
//! characters its numbers are written with) or a list of
//! those — and [`rust_literal_chars`] collects the characters of the string and character
//! literals of Rust sources. A font built from the catalog and the sources carries only the
//! glyphs it can ever draw, so CJK or Arabic firmware does not pay for thousands of unused
//! glyphs.
//!
//! ```
//! use std::collections::BTreeSet;
//! use twine_build::chars::{CharSource, ranges};
//!
//! let mut set = BTreeSet::new();
//! ('\u{30}'..='\u{39}').add_chars(&mut set); // digits
//! "°C".add_chars(&mut set);
//! ['%', '+'].add_chars(&mut set);
//! assert_eq!(ranges(&set), ['%'..='%', '+'..='+', '0'..='9', 'C'..='C', '°'..='°']);
//! ```

use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::i18n::Catalog;

/// A source of characters for a generated font (`twine_build::font::Font::chars_from`).
///
/// Implemented for typed ranges (`'a'..='z'`), single characters, strings, translation
/// catalogs ([`Catalog::chars`] and [`Catalog::number_chars`]), character sets, and slices, arrays and vectors of any of
/// them (and references to all of these).
///
/// ```
/// use std::collections::BTreeSet;
/// use twine_build::chars::CharSource;
///
/// let mut set = BTreeSet::new();
/// ['a'..='c', 'x'..='x'].add_chars(&mut set);
/// assert_eq!(set.into_iter().collect::<String>(), "abcx");
/// ```
pub trait CharSource {
    /// Adds the characters of this source to `set`.
    fn add_chars(&self, set: &mut BTreeSet<char>);
}

impl CharSource for RangeInclusive<char> {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        // `char` ranges skip the surrogate gap.
        set.extend(self.clone());
    }
}

impl CharSource for char {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        set.insert(*self);
    }
}

impl CharSource for str {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        set.extend(self.chars());
    }
}

impl CharSource for String {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        set.extend(self.chars());
    }
}

impl CharSource for Catalog {
    /// Every character of every message ([`Catalog::chars`]) and of the numbers each language
    /// writes ([`Catalog::number_chars`]).
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        set.extend(self.chars());
        set.extend(self.number_chars());
    }
}

impl CharSource for BTreeSet<char> {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        set.extend(self.iter().copied());
    }
}

impl<T: CharSource> CharSource for [T] {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        for s in self {
            s.add_chars(set);
        }
    }
}

impl<T: CharSource, const N: usize> CharSource for [T; N] {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        self.as_slice().add_chars(set);
    }
}

impl<T: CharSource> CharSource for Vec<T> {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        self.as_slice().add_chars(set);
    }
}

impl<T: CharSource + ?Sized> CharSource for &T {
    fn add_chars(&self, set: &mut BTreeSet<char>) {
        (**self).add_chars(set);
    }
}

/// The characters of `set` as the fewest ascending inclusive ranges (consecutive code points
/// form one range).
///
/// ```
/// use std::collections::BTreeSet;
/// use twine_build::chars::ranges;
///
/// let set: BTreeSet<char> = "abcxz".chars().collect();
/// assert_eq!(ranges(&set), ['a'..='c', 'x'..='x', 'z'..='z']);
/// ```
#[must_use]
pub fn ranges(set: &BTreeSet<char>) -> Vec<RangeInclusive<char>> {
    let mut out: Vec<RangeInclusive<char>> = Vec::new();
    for &c in set {
        match out.last_mut() {
            Some(r) if u32::from(*r.end()) + 1 == u32::from(c) => *r = *r.start()..=c,
            _ => out.push(c..=c),
        }
    }
    out
}

/// The characters of the string and character literals in the Rust source `src`: what an
/// application can show from its own code (labels, button texts, format strings).
///
/// Recognised: string literals with their escapes (`"…\u{B0}…"`, line continuations), raw
/// strings (`r"…"`, `r#"…"#`), character literals (`'°'`, `'\n'`) and the literal parts of
/// format strings. Byte and C strings (`b"…"`, `c"…"`), comments (doc comments included) and
/// lifetimes are skipped; control characters (newlines, tabs) are not glyphs and are left out.
/// The result over-approximates (a format string's `{name}` placeholder adds its letters),
/// which only costs a few glyphs. Malformed input never fails: scanning stops at the end of
/// the text.
///
/// ```
/// use twine_build::chars::rust_literal_chars;
///
/// let src = r##"
///     // A comment: ignored ✗
///     let t = label("Température\u{A0}°C");
///     let raw = r#"«ok»"#;
///     let unit = 'µ';
///     fn f<'a>(x: &'a str) {}
///     let bytes = b"\xFF ignored";
/// "##;
/// let chars: String = rust_literal_chars(src).into_iter().collect();
/// assert_eq!(chars, "CTaekmoprtu\u{A0}«°µ»é");
/// ```
#[must_use]
pub fn rust_literal_chars(src: &str) -> BTreeSet<char> {
    let mut set = BTreeSet::new();
    let mut lx = Lexer {
        s: src.chars().collect(),
        i: 0,
    };
    while let Some(c) = lx.peek(0) {
        let prev_ident = lx.i > 0 && lx.s.get(lx.i - 1).is_some_and(|&p| is_ident_char(p));
        match c {
            '/' if lx.peek(1) == Some('/') => lx.skip_line(),
            '/' if lx.peek(1) == Some('*') => lx.skip_block_comment(),
            '"' => {
                lx.i += 1;
                lx.string(Some(&mut set));
            }
            'r' if !prev_ident && matches!(lx.peek(1), Some('"' | '#')) => {
                lx.i += 1;
                lx.raw_string(Some(&mut set));
            }
            'b' | 'c' if !prev_ident => {
                // Byte and C strings (`b"…"`, `br#"…"#`, `c"…"`, `b'x'`): skipped.
                match (lx.peek(1), lx.peek(2)) {
                    (Some('"'), _) => {
                        lx.i += 2;
                        lx.string(None);
                    }
                    (Some('r'), Some('"' | '#')) => {
                        lx.i += 2;
                        lx.raw_string(None);
                    }
                    (Some('\''), _) if c == 'b' => {
                        lx.i += 1;
                        lx.char_literal(None);
                    }
                    _ => lx.skip_ident(),
                }
            }
            '\'' => lx.char_literal(Some(&mut set)),
            c if is_ident_char(c) => lx.skip_ident(),
            _ => lx.i += 1,
        }
    }
    set.retain(|c| !c.is_control());
    set
}

/// The characters of the string and character literals of every `.rs` file under `path` (a
/// file or a directory, searched recursively in name order), with the files read (for
/// `cargo:rerun-if-changed`). See [`rust_literal_chars`].
///
/// ```no_run
/// let (chars, files) = twine_build::chars::rust_source_chars("src".as_ref()).unwrap();
/// println!("{} characters in {} files", chars.len(), files.len());
/// ```
///
/// # Errors
/// When `path` or a file or directory below it cannot be read, or a `.rs` file is not UTF-8.
pub fn rust_source_chars(path: &Path) -> std::io::Result<(BTreeSet<char>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    collect_rs(path, &mut files)?;
    let mut set = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", f.display())))?;
        set.extend(rust_literal_chars(&text));
    }
    Ok((set, files))
}

fn collect_rs(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    if meta.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect_rs(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// A cursor over the characters of a Rust source.
struct Lexer {
    s: Vec<char>,
    i: usize,
}

impl Lexer {
    fn peek(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek(0) {
            self.i += 1;
            if c == '\n' {
                break;
            }
        }
    }

    /// Skips a (nested) block comment starting at `/*`.
    fn skip_block_comment(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.peek(0) {
            if c == '/' && self.peek(1) == Some('*') {
                depth += 1;
                self.i += 2;
            } else if c == '*' && self.peek(1) == Some('/') {
                self.i += 2;
                depth -= 1;
                if depth == 0 {
                    return;
                }
            } else {
                self.i += 1;
            }
        }
    }

    fn skip_ident(&mut self) {
        while self.peek(0).is_some_and(is_ident_char) {
            self.i += 1;
        }
    }

    /// The body of a string literal after its opening `"`, up to and including the closing one.
    fn string(&mut self, mut out: Option<&mut BTreeSet<char>>) {
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '"' => return,
                '\\' => {
                    if let Some(e) = self.escape() {
                        if let Some(o) = out.as_deref_mut() {
                            o.insert(e);
                        }
                    }
                }
                c => {
                    if let Some(o) = out.as_deref_mut() {
                        o.insert(c);
                    }
                }
            }
        }
    }

    /// A raw string after its `r`: `#…#"…"#…#`.
    fn raw_string(&mut self, mut out: Option<&mut BTreeSet<char>>) {
        let mut hashes = 0;
        while self.peek(0) == Some('#') {
            hashes += 1;
            self.i += 1;
        }
        if self.peek(0) != Some('"') {
            return; // `r#ident` (a raw identifier), not a string
        }
        self.i += 1;
        while let Some(c) = self.peek(0) {
            if c == '"' && (1..=hashes).all(|k| self.peek(k) == Some('#')) {
                self.i += 1 + hashes;
                return;
            }
            if let Some(o) = out.as_deref_mut() {
                o.insert(c);
            }
            self.i += 1;
        }
    }

    /// At a `'`: a character literal (`'x'`, `'\n'`, `'\u{B0}'`) or a lifetime / label (`'a`).
    fn char_literal(&mut self, out: Option<&mut BTreeSet<char>>) {
        self.i += 1; // the opening quote
        match (self.peek(0), self.peek(1)) {
            (Some('\\'), _) => {
                self.i += 1;
                let e = self.escape();
                if self.peek(0) == Some('\'') {
                    self.i += 1;
                    if let (Some(o), Some(e)) = (out, e) {
                        o.insert(e);
                    }
                }
            }
            (Some(c), Some('\'')) => {
                self.i += 2;
                if let Some(o) = out {
                    o.insert(c);
                }
            }
            // A lifetime or label: the identifier is skipped by the caller's loop.
            _ => {}
        }
    }

    /// An escape after its `\`: the character it stands for (`None` for a line continuation or
    /// an invalid escape).
    fn escape(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.i += 1;
        match c {
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            '0' => Some('\0'),
            '\\' | '\'' | '"' => Some(c),
            'x' => {
                let hex: String = (0..2).filter_map(|k| self.peek(k)).collect();
                self.i += hex.len();
                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
            }
            'u' if self.peek(0) == Some('{') => {
                self.i += 1;
                let mut hex = String::new();
                while let Some(h) = self.peek(0) {
                    self.i += 1;
                    if h == '}' {
                        break;
                    }
                    if h != '_' {
                        hex.push(h);
                    }
                }
                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
            }
            '\n' | '\r' => {
                // Line continuation: the newline and the next line's leading whitespace.
                while self.peek(0).is_some_and(char::is_whitespace) {
                    self.i += 1;
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(src: &str) -> String {
        rust_literal_chars(src).into_iter().collect()
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(lit(r#"f("a\"b\\c")"#), "\"\\abc");
        assert_eq!(lit(r#""\x41\u{1F600}\u{00_B0}""#), "A°😀");
        assert_eq!(lit("\"a\\\n    b\""), "ab");
        assert_eq!(lit(r#""tab\there""#), "abehrt");
    }

    #[test]
    fn raw_strings_and_raw_identifiers() {
        assert_eq!(lit(r###"r##"a"#b"##"###), "\"#ab");
        assert_eq!(lit("let r#type = 1; r\"x\""), "x");
    }

    #[test]
    fn chars_lifetimes_and_labels() {
        assert_eq!(lit("'x' '\\'' '\\u{263A}'"), "'x☺");
        assert_eq!(
            lit("fn f<'a, 'b>(x: &'a T) -> &'b U { 'outer: loop { break 'outer; } }"),
            ""
        );
        assert_eq!(lit("impl<'a> X<'a> { const C: char = 'é'; }"), "é");
    }

    #[test]
    fn comments_and_byte_strings_are_skipped() {
        assert_eq!(
            lit("// \"no\"\n/* \"no\" /* nested \"no\" */ \"no\" */ \"yes\""),
            "esy"
        );
        assert_eq!(lit("b\"no\" br#\"no\"# b'n' c\"no\" cr\"no\" \"ok\""), "ko");
        assert_eq!(lit("abc\"ok\" rb\"ok\""), "ko");
    }

    #[test]
    fn unterminated_input_ends_the_scan() {
        assert_eq!(lit("\"abc"), "abc");
        assert_eq!(lit("r#\"abc"), "abc");
        assert_eq!(lit("'"), "");
        assert_eq!(lit("\"\\u{"), "");
    }

    #[test]
    fn ranges_merge_consecutive_code_points() {
        let set: BTreeSet<char> = ['a', 'b', 'c', 'e', '\u{D7FF}', '\u{E000}'].into_iter().collect();
        assert_eq!(
            ranges(&set),
            [
                'a'..='c',
                'e'..='e',
                '\u{D7FF}'..='\u{D7FF}',
                '\u{E000}'..='\u{E000}'
            ]
        );
        assert_eq!(ranges(&BTreeSet::new()), []);
    }

    #[test]
    fn sources_are_read_recursively() {
        let dir = std::env::temp_dir().join(format!("twine-build-chars-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.rs"), "const A: &str = \"ab\";").unwrap();
        std::fs::write(dir.join("sub/b.rs"), "const B: char = 'z';").unwrap();
        std::fs::write(dir.join("c.txt"), "\"ignored\"").unwrap();
        let (set, files) = rust_source_chars(&dir).unwrap();
        assert_eq!(set.into_iter().collect::<String>(), "abz");
        assert_eq!(files.len(), 2);
        assert!(rust_source_chars(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
