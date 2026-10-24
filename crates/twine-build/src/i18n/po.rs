//! gettext `.po` sources, used **ID-based**: `msgid` is the message key (`"greeting"`), `msgstr`
//! its text in the file's language, with `{name}` / `{0}` placeholders (the TOML syntax, not
//! `printf`).
//!
//! Plural entries (`msgid_plural` + `msgstr[0]`, `msgstr[1]`, …) list the language's CLDR
//! categories in order — either all of them or only the integer ones (see
//! [`PluralRules`](super::PluralRules)); the choice is made on the entry's only argument, or on
//! the one named by an extracted comment `#. selector: count`. Untranslated (empty) and
//! `#, fuzzy` entries count as missing, obsolete `#~` entries are skipped, the header entry
//! (`msgid ""`) is only checked for `Plural-Forms: nplurals=N`. `msgctxt` is not supported
//! (keys must be unique).

use super::model::{Body, Entry, parse_braces};

/// The string a continuation line extends.
#[derive(Clone, Copy)]
enum Target {
    /// The `msgid`.
    Id,
    /// `msgctxt` / `msgid_plural` (not used).
    Ignored,
    /// A `msgstr` (index into `Raw::strs`).
    Str(usize),
}

#[derive(Default)]
struct Raw {
    line: usize,
    id: Option<String>,
    plural_id: bool,
    strs: Vec<(Option<usize>, String)>,
    fuzzy: bool,
    selector: Option<String>,
    ctxt: bool,
}

/// What [`parse`] found.
pub(crate) struct Po {
    pub entries: Vec<(String, Entry)>,
    /// `nplurals` of the header's `Plural-Forms`, if any.
    pub nplurals: Option<usize>,
    /// Keys of fuzzy entries (skipped).
    pub fuzzy: Vec<String>,
}

fn unescape(s: &str, line: usize) -> Result<String, (usize, String)> {
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or((line, format!("expected a quoted string, found `{s}`")))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => return Err((line, format!("unsupported escape `\\{}`", other.unwrap_or(' ')))),
        }
    }
    Ok(out)
}

/// Parses a `.po` file, or returns the errors as `(line, message)`.
pub(crate) fn parse(src: &str) -> Result<Po, Vec<(usize, String)>> {
    let mut raws: Vec<Raw> = Vec::new();
    let mut cur = Raw::default();
    // Which string a continuation line (`"…"`) extends.
    let mut target: Option<Target> = None;
    let mut errors = Vec::new();
    let flush = |cur: &mut Raw, raws: &mut Vec<Raw>| {
        if cur.id.is_some() {
            raws.push(core::mem::take(cur));
        } else {
            *cur = Raw::default();
        }
    };
    let mut id_text = String::new();
    for (i, line) in src.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        if line.is_empty() {
            if cur.id.is_some() {
                cur.id = Some(core::mem::take(&mut id_text));
            }
            flush(&mut cur, &mut raws);
            target = None;
            continue;
        }
        if line.starts_with("#~") {
            continue;
        }
        if let Some(flags) = line.strip_prefix("#,") {
            cur.fuzzy |= flags.split(',').any(|f| f.trim() == "fuzzy");
            continue;
        }
        if let Some(c) = line.strip_prefix("#.") {
            if let Some(sel) = c.trim().strip_prefix("selector:") {
                cur.selector = Some(sel.trim().to_string());
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (kw, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let string = |s: &str| unescape(s, n);
        match kw {
            _ if line.starts_with('"') => match (target, string(line)) {
                (Some(Target::Id), Ok(s)) => id_text.push_str(&s),
                (Some(Target::Ignored), Ok(_)) => {}
                (Some(Target::Str(t)), Ok(s)) => {
                    if let Some((_, text)) = cur.strs.get_mut(t) {
                        text.push_str(&s);
                    }
                }
                (None, Ok(_)) => errors.push((n, "string outside of an entry".to_string())),
                (_, Err(e)) => errors.push(e),
            },
            "msgctxt" => {
                cur.ctxt = true;
                if cur.line == 0 {
                    cur.line = n;
                }
                target = Some(Target::Ignored);
            }
            "msgid" => {
                if cur.id.is_some() {
                    // A new entry without a blank line in between.
                    cur.id = Some(core::mem::take(&mut id_text));
                    flush(&mut cur, &mut raws);
                }
                if cur.line == 0 {
                    cur.line = n;
                }
                match string(rest) {
                    Ok(s) => {
                        id_text = s;
                        cur.id = Some(String::new());
                    }
                    Err(e) => errors.push(e),
                }
                target = Some(Target::Id);
            }
            "msgid_plural" => {
                cur.plural_id = true;
                target = Some(Target::Ignored);
            }
            _ if kw.starts_with("msgstr") => {
                let index = if kw == "msgstr" {
                    None
                } else if let Some(i) = kw
                    .strip_prefix("msgstr[")
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    Some(i)
                } else {
                    errors.push((n, format!("unknown keyword `{kw}`")));
                    continue;
                };
                match string(rest) {
                    Ok(s) => {
                        target = Some(Target::Str(cur.strs.len()));
                        cur.strs.push((index, s));
                    }
                    Err(e) => errors.push(e),
                }
            }
            _ => errors.push((n, format!("unknown keyword `{kw}`"))),
        }
    }
    if cur.id.is_some() {
        cur.id = Some(id_text);
    }
    flush(&mut cur, &mut raws);

    let mut po = Po {
        entries: Vec::new(),
        nplurals: None,
        fuzzy: Vec::new(),
    };
    for raw in raws {
        let id = raw.id.clone().unwrap_or_default();
        let line = raw.line;
        if raw.ctxt {
            errors.push((
                line,
                format!("`{id}`: `msgctxt` is not supported (use unique keys)"),
            ));
            continue;
        }
        if id.is_empty() {
            // The header.
            let header = raw.strs.first().map_or("", |(_, s)| s.as_str());
            po.nplurals = header
                .lines()
                .find_map(|l| l.trim().strip_prefix("Plural-Forms:"))
                .and_then(|pf| pf.split(';').find_map(|p| p.trim().strip_prefix("nplurals=")))
                .and_then(|n| n.trim().parse().ok());
            continue;
        }
        if raw.fuzzy {
            po.fuzzy.push(id);
            continue;
        }
        let parse_text = |s: &str| parse_braces(s).map_err(|e| (line, format!("`{id}`: {e}")));
        let body = if raw.plural_id {
            if let Some(k) = raw
                .strs
                .iter()
                .enumerate()
                .position(|(k, (index, _))| *index != Some(k))
            {
                errors.push((line, format!("`{id}`: expected `msgstr[{k}]`")));
                continue;
            }
            if raw.strs.is_empty() || raw.strs.iter().any(|(_, s)| s.is_empty()) {
                continue; // untranslated
            }
            match raw
                .strs
                .iter()
                .map(|(_, s)| parse_text(s))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(forms) => Body::Forms {
                    selector: raw.selector.clone(),
                    forms,
                },
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            }
        } else {
            match raw.strs.as_slice() {
                [(None, s)] if s.is_empty() => continue, // untranslated
                [(None, s)] => match parse_text(s) {
                    Ok(segs) => Body::Pattern(segs),
                    Err(e) => {
                        errors.push(e);
                        continue;
                    }
                },
                _ => {
                    errors.push((line, format!("`{id}`: expected one `msgstr`")));
                    continue;
                }
            }
        };
        po.entries.push((id, Entry { body, line }));
    }
    if errors.is_empty() { Ok(po) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::model::Seg;

    const SRC: &str = r#"
msgid ""
msgstr ""
"Language: ru\n"
"Plural-Forms: nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : 1);\n"

#: src/main.rs
msgid "hello"
msgstr "Привет"

#. selector: count
msgid "items"
msgid_plural "items"
msgstr[0] "{count} файл"
msgstr[1] "{count} файла"
msgstr[2] "{count} файлов"

msgid "long"
msgstr ""
"two "
"lines\n"

#, fuzzy
msgid "unsure"
msgstr "?"

msgid "todo"
msgstr ""

#~ msgid "old"
#~ msgstr "alt"
"#;

    #[test]
    fn entries_header_and_skips() {
        let po = parse(SRC).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(po.nplurals, Some(3));
        assert_eq!(po.fuzzy, ["unsure"]);
        let ids: Vec<_> = po.entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(ids, ["hello", "items", "long"]);
        assert_eq!(
            po.entries[2].1.body,
            Body::Pattern(vec![Seg::Text("two lines\n".into())])
        );
        let Body::Forms { selector, forms } = &po.entries[1].1.body else {
            panic!("plural")
        };
        assert_eq!(selector.as_deref(), Some("count"));
        assert_eq!(forms.len(), 3);
        assert_eq!(po.entries[1].1.line, 12);
    }

    #[test]
    fn errors_name_lines() {
        let Err(e) = parse("msgid \"a\"\nmsgstr \"{x\"\n\nmsgctxt \"c\"\nmsgid \"b\"\nmsgstr \"B\"\n") else {
            panic!("errors expected")
        };
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e[0].0, 1);
    }
}
