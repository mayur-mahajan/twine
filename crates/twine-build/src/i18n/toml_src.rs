//! TOML sources: `key = "text"` with `{name}` / `{0}` placeholders; a table groups keys
//! (`[settings]` + `title = …` is the key `settings.title`); a table with an `other` entry is a
//! plural message whose entries are CLDR categories (`zero` … `other`) or exact numbers
//! (`"0" = …`), chosen on the message's only argument or on the one named by `selector`.
//! An empty text counts as missing.

use super::model::{Body, Entry, Parsed, Sel, parse_braces};
use super::plural::Category;

/// Parses a TOML file into `(id, entry)` pairs, or returns the errors as `(line, message)`
/// (line 0: TOML keys carry no line numbers once parsed; messages name the key).
pub(crate) fn parse(src: &str) -> Parsed {
    let table: toml::Table = src.parse().map_err(|e: toml::de::Error| {
        let line = e
            .span()
            .map_or(0, |s| 1 + src[..s.start.min(src.len())].matches('\n').count());
        vec![(line, e.message().to_string())]
    })?;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    walk("", &table, &mut out, &mut errors);
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

fn walk(
    prefix: &str,
    table: &toml::Table,
    out: &mut Vec<(String, Entry)>,
    errors: &mut Vec<(usize, String)>,
) {
    for (k, v) in table {
        let id = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(s) if s.is_empty() => {}
            toml::Value::String(s) => match parse_braces(s) {
                Ok(segs) => out.push((
                    id,
                    Entry {
                        body: Body::Pattern(segs),
                        line: 0,
                    },
                )),
                Err(e) => errors.push((0, format!("`{id}`: {e}"))),
            },
            toml::Value::Table(t) if t.contains_key("other") => match plural(&id, t) {
                Ok(Some(body)) => out.push((id, Entry { body, line: 0 })),
                Ok(None) => {}
                Err(e) => errors.push((0, format!("`{id}`: {e}"))),
            },
            toml::Value::Table(t) => walk(&id, t, out, errors),
            _ => errors.push((0, format!("`{id}`: expected a text, a group or a plural table"))),
        }
    }
}

fn plural(id: &str, t: &toml::Table) -> Result<Option<Body>, String> {
    let mut selector = None;
    let mut variants = Vec::new();
    let mut other = None;
    for (k, v) in t {
        let toml::Value::String(s) = v else {
            return Err(format!("`{id}.{k}` must be a text"));
        };
        if k == "selector" {
            selector = Some(s.clone());
            continue;
        }
        if s.is_empty() {
            return Ok(None); // untranslated
        }
        let segs = parse_braces(s)?;
        let sel = if let Ok(n) = k.parse::<u64>() {
            Sel::Exact(n)
        } else {
            Sel::Cat(Category::from_name(k).ok_or_else(|| {
                format!("`{k}` is not a plural category (`zero`, `one`, `two`, `few`, `many`, `other`) or a number")
            })?)
        };
        if sel == Sel::Cat(Category::Other) {
            other = Some(segs);
        } else {
            variants.push((sel, segs));
        }
    }
    let other = other.unwrap_or_default();
    let mut body = Body::Plural {
        selector: selector.clone().unwrap_or_default(),
        variants,
        other,
    };
    if selector.is_none() {
        // The only argument the variants use.
        let args = body
            .args()
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>();
        match args.as_slice() {
            [one] => {
                if let Body::Plural { selector, .. } = &mut body {
                    selector.clone_from(one);
                }
            }
            [] => return Err("a plural message needs an argument to choose on".to_string()),
            _ => {
                return Err(format!(
                    "the variants use several arguments ({}): name the one to choose on with `selector = \"…\"`",
                    args.join(", ")
                ));
            }
        }
    }
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::model::Seg;

    #[test]
    fn groups_plurals_and_missing() {
        let v = parse(
            r#"
hello = "Hello"
empty = ""
[settings]
title = "Settings"
[items]
"0" = "No items"
one = "{n} item"
other = "{n} items"
[moved]
selector = "count"
one = "{count} file moved to {dir}"
other = "{count} files moved to {dir}"
"#,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let ids: Vec<_> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(ids, ["hello", "items", "moved", "settings.title"]);
        let Body::Plural {
            selector,
            variants,
            other,
        } = &v[1].1.body
        else {
            panic!()
        };
        assert_eq!(selector, "n");
        assert_eq!(variants[0], (Sel::Exact(0), vec![Seg::Text("No items".into())]));
        assert_eq!(other[0], Seg::Arg("n".into()));
        assert_eq!(v[2].1.body.args(), ["count", "dir"]);
    }

    #[test]
    fn errors() {
        assert!(parse("a = 1").is_err());
        assert!(parse("[p]\nother = \"{a} {b}\"").is_err(), "ambiguous selector");
        assert!(
            parse("[p]\nsome = \"x {n}\"\nother = \"{n}\"").is_err(),
            "unknown category"
        );
        let e = parse("a = \"x\"\nb = \n").unwrap_err();
        assert_eq!(e[0].0, 2, "{e:?}");
    }
}
