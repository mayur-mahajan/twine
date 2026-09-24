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

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

impl<'a> Reader<'a> {
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
                self.pos += i + end.len();
                Ok(())
            }
            None => Err(SvgError::UnexpectedEof),
        }
    }

    fn name(&mut self) -> Result<&'a str, SvgError> {
        let b = self.bytes();
        let start = self.pos;
        while self.pos < b.len() && is_name_char(b[self.pos]) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(SvgError::Malformed(start));
        }
        self.s.get(start..self.pos).ok_or(SvgError::Malformed(start))
    }

    fn skip_ws(&mut self) {
        let b = self.bytes();
        while self.pos < b.len() && is_ws(b[self.pos]) {
            self.pos += 1;
        }
    }

    /// Skips `<!DOCTYPE …>` including a bracketed internal subset (not interpreted).
    fn skip_decl(&mut self) -> Result<(), SvgError> {
        let b = self.bytes();
        let mut depth = 0u32;
        let mut quote = 0u8;
        while self.pos < b.len() {
            let c = b[self.pos];
            self.pos += 1;
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
                continue;
            }
            match c {
                b'"' | b'\'' => quote = c,
                b'[' => depth += 1,
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
            self.pos += i;
            if self.starts_with("<!--") {
                self.pos += 4;
                self.skip_past("-->")?;
            } else if self.starts_with("<?") {
                self.skip_past("?>")?;
            } else if self.starts_with("<![CDATA[") {
                self.skip_past("]]>")?;
            } else if self.starts_with("<!") {
                self.skip_decl()?;
            } else if self.starts_with("</") {
                self.pos += 2;
                let name = self.name()?;
                self.skip_ws();
                if !self.starts_with(">") {
                    return Err(SvgError::Malformed(self.pos));
                }
                self.pos += 1;
                return Ok(Some(Token::End { name }));
            } else {
                self.pos += 1;
                let name = self.name()?;
                let attr_start = self.pos;
                let attrs_end = self.scan_attrs()?;
                let attrs = Attrs {
                    s: self.s.get(attr_start..attrs_end).unwrap_or(""),
                };
                let self_closing = self.starts_with("/>");
                self.pos += if self_closing { 2 } else { 1 };
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
                    return if b.get(self.pos + 1) == Some(&b'>') {
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
            self.pos += 1;
            self.skip_ws();
            let q = match b.get(self.pos) {
                Some(&q @ (b'"' | b'\'')) => q,
                Some(_) => return Err(SvgError::Malformed(self.pos)),
                None => return Err(SvgError::UnexpectedEof),
            };
            self.pos += 1;
            match b.get(self.pos..).and_then(|r| r.iter().position(|&c| c == q)) {
                Some(i) => self.pos += i + 1,
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

impl<'a> Iterator for AttrIter<'a> {
    type Item = (&'a str, &'a str);
    fn next(&mut self) -> Option<Self::Item> {
        let b = self.s.as_bytes();
        while self.pos < b.len() && is_ws(b[self.pos]) {
            self.pos += 1;
        }
        let n0 = self.pos;
        while self.pos < b.len() && is_name_char(b[self.pos]) {
            self.pos += 1;
        }
        if self.pos == n0 {
            return None;
        }
        let name = self.s.get(n0..self.pos)?;
        let eq = b.get(self.pos..)?.iter().position(|&c| c == b'=')?;
        self.pos += eq + 1;
        let qo = b.get(self.pos..)?.iter().position(|&c| c == b'"' || c == b'\'')?;
        self.pos += qo;
        let q = b[self.pos];
        let v0 = self.pos + 1;
        let len = b.get(v0..)?.iter().position(|&c| c == q)?;
        self.pos = v0 + len + 1;
        Some((name, self.s.get(v0..v0 + len)?))
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
    while let Some(i) = rest.find('&') {
        buf.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 12) else {
            buf.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
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
            rest = &rest[end + 1..];
        } else {
            buf.push('&');
            rest = &rest[1..];
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
