//! A minimal, non-recursive XML tokenizer for SVG: start/end/self-closing tags with quoted
//! attributes; comments, processing instructions (`<?xml?>`), CDATA sections, text and the
//! `<!DOCTYPE>` declaration are skipped (no DTD processing). Entities are decoded on demand
//! in attribute values (`&amp; &lt; &gt; &quot; &apos;` and numeric references).
//!
//! The tokenizer never allocates and never panics; malformed input is an error.

use alloc::string::String;

use super::SvgError;

/// One tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token<'a> {
    /// `<name attrs>` or `<name attrs/>`.
    Start {
        /// Element name (with namespace prefix, if any).
        name: &'a str,
        /// The validated raw attribute text.
        attrs: Attrs<'a>,
        /// `/>`.
        self_closing: bool,
    },
    /// `</name>`.
    End {
        /// Element name.
        name: &'a str,
    },
}

/// The tokenizer state.
#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    s: &'a str,
    pos: usize,
}

/// Byte classes of the tokenizer, one table lookup per byte: [`NAME`] and [`WS`] bits.
static CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    // Slice patterns: no index in a constant.
    let mut rest: &mut [u8] = &mut t;
    let mut b = 0u8;
    while let [entry, tail @ ..] = rest {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':') || b >= 0x80 {
            *entry |= NAME;
        }
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            *entry |= WS;
        }
        b = b.wrapping_add(1);
        rest = tail;
    }
    t
};
/// [`CLASS`] bit: a name character.
const NAME: u8 = 1;
/// [`CLASS`] bit: white space.
const WS: u8 = 2;

#[inline]
fn class(c: u8) -> u8 {
    // A `u8` indexes the 256 entries: the fallback is never taken.
    CLASS.get(usize::from(c)).copied().unwrap_or(0)
}

#[inline]
fn is_name_char(c: u8) -> bool {
    class(c) & NAME != 0
}

#[inline]
fn is_ws(c: u8) -> bool {
    class(c) & WS != 0
}

impl<'a> Reader<'a> {
    /// Moves the position forward by `n` bytes that the caller has just read or found, so
    /// `pos + n ≤ len`.
    #[inline]
    pub(crate) fn bump(&mut self, n: usize) {
        // SAFE-ARITH: `pos + n ≤ len ≤ isize::MAX` (the caller's contract above).
        self.pos = self.pos.wrapping_add(n);
    }

    pub(crate) fn new(s: &'a str) -> Self {
        Self { s, pos: 0 }
    }

    fn bytes(&self) -> &'a [u8] {
        self.s.as_bytes()
    }

    fn starts_with(&self, p: &str) -> bool {
        self.bytes()
            .get(self.pos..)
            .is_some_and(|b| b.starts_with(p.as_bytes()))
    }

    /// Moves past the next occurrence of `end` (error at end of input).
    fn skip_past(&mut self, end: &str) -> Result<(), SvgError> {
        let rest = self.s.get(self.pos..).ok_or(SvgError::UnexpectedEof)?;
        match rest.find(end) {
            Some(i) => {
                // SAFE-ARITH: the sum is an offset inside the input (the found byte and the one after
                // it, or the end marker), below `isize::MAX`.
                self.bump(i.wrapping_add(end.len()));
                Ok(())
            }
            None => Err(SvgError::UnexpectedEof),
        }
    }

    fn name(&mut self) -> Result<&'a str, SvgError> {
        let b = self.bytes();
        let start = self.pos;
        while b.get(self.pos).is_some_and(|&c| is_name_char(c)) {
            self.bump(1);
        }
        if self.pos == start {
            return Err(SvgError::Malformed(start));
        }
        self.s.get(start..self.pos).ok_or(SvgError::Malformed(start))
    }

    fn skip_ws(&mut self) {
        let b = self.bytes();
        while b.get(self.pos).is_some_and(|&c| is_ws(c)) {
            self.bump(1);
        }
    }

    /// Skips `<!DOCTYPE …>` including a bracketed internal subset (not interpreted).
    fn skip_decl(&mut self) -> Result<(), SvgError> {
        let b = self.bytes();
        let mut depth = 0u32;
        let mut quote = 0u8;
        while let Some(&c) = b.get(self.pos) {
            self.bump(1);
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
                continue;
            }
            match c {
                b'"' | b'\'' => quote = c,
                b'[' => depth = depth.saturating_add(1),
                b']' => depth = depth.saturating_sub(1),
                b'>' if depth == 0 => return Ok(()),
                _ => {}
            }
        }
        Err(SvgError::UnexpectedEof)
    }

    /// The next tag, or `None` at the end of the input.
    pub(crate) fn next_token(&mut self) -> Result<Option<Token<'a>>, SvgError> {
        loop {
            let rest = self.s.get(self.pos..).unwrap_or("");
            let Some(i) = rest.find('<') else {
                self.pos = self.s.len();
                return Ok(None);
            };
            self.bump(i);
            if self.starts_with("<!--") {
                self.bump(4);
                self.skip_past("-->")?;
            } else if self.starts_with("<?") {
                self.skip_past("?>")?;
            } else if self.starts_with("<![CDATA[") {
                self.skip_past("]]>")?;
            } else if self.starts_with("<!") {
                self.skip_decl()?;
            } else if self.starts_with("</") {
                self.bump(2);
                let name = self.name()?;
                self.skip_ws();
                if !self.starts_with(">") {
                    return Err(SvgError::Malformed(self.pos));
                }
                self.bump(1);
                return Ok(Some(Token::End { name }));
            } else {
                self.bump(1);
                let name = self.name()?;
                let attr_start = self.pos;
                let attrs_end = self.scan_attrs()?;
                let attrs = Attrs {
                    s: self.s.get(attr_start..attrs_end).unwrap_or(""),
                };
                let self_closing = self.starts_with("/>");
                self.bump(if self_closing { 2 } else { 1 });
                return Ok(Some(Token::Start {
                    name,
                    attrs,
                    self_closing,
                }));
            }
        }
    }

    /// Validates the attributes of a start tag; stops at `>` or `/>` (not consumed) and
    /// returns the end of the attribute text.
    fn scan_attrs(&mut self) -> Result<usize, SvgError> {
        let b = self.bytes();
        loop {
            let had_ws = {
                let p = self.pos;
                self.skip_ws();
                self.pos > p
            };
            match b.get(self.pos) {
                None => return Err(SvgError::UnexpectedEof),
                Some(b'>') => return Ok(self.pos),
                Some(b'/') => {
                    return if b.get(self.pos.saturating_add(1)) == Some(&b'>') {
                        Ok(self.pos)
                    } else {
                        Err(SvgError::Malformed(self.pos))
                    };
                }
                Some(_) if !had_ws => return Err(SvgError::Malformed(self.pos)),
                Some(_) => {}
            }
            self.name()?;
            self.skip_ws();
            if b.get(self.pos) != Some(&b'=') {
                return Err(SvgError::Malformed(self.pos));
            }
            self.bump(1);
            self.skip_ws();
            let q = match b.get(self.pos) {
                Some(&q @ (b'"' | b'\'')) => q,
                Some(_) => return Err(SvgError::Malformed(self.pos)),
                None => return Err(SvgError::UnexpectedEof),
            };
            self.bump(1);
            match b.get(self.pos..).and_then(|r| r.iter().position(|&c| c == q)) {
                // SAFE-ARITH: the sum is an offset inside the input (the found byte and the one after
                // it, or the end marker), below `isize::MAX`.
                Some(i) => self.bump(i.wrapping_add(1)),
                None => return Err(SvgError::UnexpectedEof),
            }
        }
    }
}

/// The attributes of a start tag (validated by the reader).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attrs<'a> {
    s: &'a str,
}

impl<'a> Attrs<'a> {
    /// The raw (entity-encoded) value of attribute `name`.
    pub(crate) fn get(&self, name: &str) -> Option<&'a str> {
        self.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
}

impl<'a> Attrs<'a> {
    /// Iterates `(name, raw value)` pairs.
    pub(crate) fn iter(&self) -> AttrIter<'a> {
        AttrIter { s: self.s, pos: 0 }
    }
}

/// Iterator over validated attributes.
#[derive(Clone, Debug)]
pub(crate) struct AttrIter<'a> {
    s: &'a str,
    pos: usize,
}

impl AttrIter<'_> {
    /// Moves the position forward by `n` bytes that the caller has just read or found, so
    /// `pos + n ≤ len`.
    #[inline]
    fn bump(&mut self, n: usize) {
        // SAFE-ARITH: `pos + n ≤ len ≤ isize::MAX` (the caller's contract above).
        self.pos = self.pos.wrapping_add(n);
    }
}

impl<'a> Iterator for AttrIter<'a> {
    type Item = (&'a str, &'a str);
    fn next(&mut self) -> Option<Self::Item> {
        let b = self.s.as_bytes();
        while b.get(self.pos).is_some_and(|&c| is_ws(c)) {
            self.bump(1);
        }
        let n0 = self.pos;
        while b.get(self.pos).is_some_and(|&c| is_name_char(c)) {
            self.bump(1);
        }
        if self.pos == n0 {
            return None;
        }
        let name = self.s.get(n0..self.pos)?;
        let eq = b.get(self.pos..)?.iter().position(|&c| c == b'=')?;
        // SAFE-ARITH: the sum is an offset inside the input (the found byte and the one after
        // it, or the end marker), below `isize::MAX`.
        self.bump(eq.wrapping_add(1));
        let qo = b.get(self.pos..)?.iter().position(|&c| c == b'"' || c == b'\'')?;
        self.bump(qo);
        let q = *b.get(self.pos)?;
        self.bump(1);
        let v0 = self.pos;
        let len = b.get(v0..)?.iter().position(|&c| c == q)?;
        // SAFE-ARITH: the sum is an offset inside the input (the found byte and the one after
        // it, or the end marker), below `isize::MAX`.
        self.bump(len.wrapping_add(1));
        // The quote follows the value: `v0 + len < self.pos`.
        let v1 = self.pos.wrapping_sub(1);
        Some((name, self.s.get(v0..v1)?))
    }
}

/// Decodes entities in `v`, allocating only when it contains one.
pub(crate) fn decode(v: &str) -> alloc::borrow::Cow<'_, str> {
    if v.contains('&') {
        let mut buf = String::new();
        unescape(v, &mut buf);
        alloc::borrow::Cow::Owned(buf)
    } else {
        alloc::borrow::Cow::Borrowed(v)
    }
}

/// Decodes entities in `v` (into `buf` when needed). Unknown entities are kept literally.
pub(crate) fn unescape<'b>(v: &'b str, buf: &'b mut String) -> &'b str {
    if !v.contains('&') {
        return v;
    }
    buf.clear();
    let mut rest = v;
    // The positions below are those of the ASCII bytes `&` and `;`: char boundaries, so the
    // `get`s never fall back.
    while let Some(i) = rest.find('&') {
        let (text, from_amp) = rest.split_at_checked(i).unwrap_or((rest, ""));
        buf.push_str(text);
        rest = from_amp;
        let after_amp = rest.get(1..).unwrap_or_default();
        let Some(end) = rest.find(';').filter(|&e| e <= 12) else {
            buf.push('&');
            rest = after_amp;
            continue;
        };
        let ent = rest.get(1..end).unwrap_or_default();
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent.strip_prefix('#').and_then(|n| {
                let code = match n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                    Some(h) => u32::from_str_radix(h, 16).ok(),
                    None => n.parse::<u32>().ok(),
                };
                code.and_then(char::from_u32)
            }),
        };
        if let Some(c) = ch {
            buf.push(c);
            rest = rest.get(end.saturating_add(1)..).unwrap_or_default();
        } else {
            buf.push('&');
            rest = after_amp;
        }
    }
    buf.push_str(rest);
    buf.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(s: &str) -> Result<alloc::vec::Vec<Token<'_>>, SvgError> {
        let mut r = Reader::new(s);
        let mut v = alloc::vec::Vec::new();
        while let Some(t) = r.next_token()? {
            v.push(t);
        }
        Ok(v)
    }

    #[test]
    fn tags_attrs_and_skipped_content() {
        let s = "<?xml version=\"1.0\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" [<!ENTITY x \"y\">]>\
                 <!-- c --><svg a='1' b = \"x&amp;y\"><![CDATA[ <g> ]]>text<g/></svg>";
        let t = tokens(s).unwrap();
        assert_eq!(t.len(), 3);
        let Token::Start {
            name,
            attrs,
            self_closing,
        } = t[0]
        else {
            panic!()
        };
        assert_eq!((name, self_closing), ("svg", false));
        assert_eq!(attrs.get("a"), Some("1"));
        let mut buf = String::new();
        assert_eq!(unescape(attrs.get("b").unwrap(), &mut buf), "x&y");
        assert!(matches!(
            t[1],
            Token::Start {
                name: "g",
                self_closing: true,
                ..
            }
        ));
        assert_eq!(t[2], Token::End { name: "svg" });
    }

    #[test]
    fn malformed_is_an_error() {
        for s in [
            "<svg a=1>",
            "<svg a='1>",
            "<svg a='1'b='2'>",
            "<svg",
            "<!-- x",
            "</svg",
            "< svg>",
            "<svg/ >",
        ] {
            assert!(tokens(s).is_err(), "{s}");
        }
    }

    #[test]
    fn entities() {
        let mut b = String::new();
        assert_eq!(unescape("&#65;&#x42;&lt;&bogus;&", &mut b), "AB<&bogus;&");
    }
}
