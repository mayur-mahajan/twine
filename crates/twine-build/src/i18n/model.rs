//! The generator's message model (shared by the three source formats), the `{name}` placeholder
//! syntax of TOML and `.po` files, and identifier conversion.

use super::plural::Category;

/// A piece of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seg {
    /// Literal text.
    Text(String),
    /// The value of an argument (by name; positional arguments are named `"0"`, `"1"`, …).
    Arg(String),
}

/// What a plural variant matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Sel {
    /// Exactly this non-negative integer (checked before the categories).
    Exact(u64),
    /// A CLDR category.
    Cat(Category),
}

/// The body of one message in one language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// A text with placeholders.
    Pattern(Vec<Seg>),
    /// A choice on the plural category (or exact value) of the argument `selector`.
    Plural {
        selector: String,
        variants: Vec<(Sel, Vec<Seg>)>,
        other: Vec<Seg>,
    },
    /// The plural forms of a `.po` entry (`msgstr[0]`, …), mapped to categories once the
    /// language's plural rule is known; `selector` is `None` until resolved from the arguments
    /// (or given by a `#. selector:` comment).
    Forms {
        selector: Option<String>,
        forms: Vec<Vec<Seg>>,
    },
}

impl Body {
    /// Every segment of the body.
    pub(crate) fn segments(&self) -> Box<dyn Iterator<Item = &Seg> + '_> {
        match self {
            Body::Pattern(s) => Box::new(s.iter()),
            Body::Plural { variants, other, .. } => {
                Box::new(variants.iter().flat_map(|(_, s)| s.iter()).chain(other.iter()))
            }
            Body::Forms { forms, .. } => Box::new(forms.iter().flatten()),
        }
    }

    /// The argument names the body uses (placeholders and the plural selector), sorted and
    /// unique.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .segments()
            .filter_map(|s| match s {
                Seg::Arg(a) => Some(a.clone()),
                Seg::Text(_) => None,
            })
            .collect();
        match self {
            Body::Plural { selector, .. }
            | Body::Forms {
                selector: Some(selector),
                ..
            } => v.push(selector.clone()),
            _ => {}
        }
        sort_args(&mut v);
        v
    }
}

/// Sorts argument names canonically: positional ones (`"0"`, `"1"`, …) by number first, then
/// named ones alphabetically; removes duplicates.
pub(crate) fn sort_args(v: &mut Vec<String>) {
    v.sort_by(|a, b| match (a.parse::<u32>(), b.parse::<u32>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => core::cmp::Ordering::Less,
        (Err(_), Ok(_)) => core::cmp::Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    });
    v.dedup();
}

/// One message as read from a source file.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub body: Body,
    /// 1-based line in the source file (0 when unknown, e.g. TOML).
    pub line: usize,
}

/// What a source parser returns: `(key, entry)` pairs, or the errors as `(line, message)`.
pub(crate) type Parsed = Result<Vec<(String, Entry)>, Vec<(usize, String)>>;

/// Appends `seg`, merging adjacent texts and dropping empty ones.
pub(crate) fn push_seg(out: &mut Vec<Seg>, seg: Seg) {
    match seg {
        Seg::Text(t) if t.is_empty() => {}
        Seg::Text(t) => {
            if let Some(Seg::Text(last)) = out.last_mut() {
                last.push_str(&t);
            } else {
                out.push(Seg::Text(t));
            }
        }
        arg @ Seg::Arg(_) => out.push(arg),
    }
}

/// Concatenates segment lists.
pub(crate) fn concat(parts: &[&[Seg]]) -> Vec<Seg> {
    let mut out = Vec::new();
    for p in parts {
        for s in *p {
            push_seg(&mut out, s.clone());
        }
    }
    out
}

/// Whether `name` is a valid argument name: a Rust identifier (`name`, `user_name`) or a
/// position (`0`, `1`, … without leading zeros).
pub(crate) fn valid_arg(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some('0') => name.len() == 1,
        Some(c) if c.is_ascii_digit() => name.chars().all(|c| c.is_ascii_digit()),
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            name != "_" && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

/// Parses the placeholder syntax of TOML and `.po` messages: `{name}` and `{0}` insert an
/// argument, `{{` and `}}` are literal braces.
///
/// # Errors
/// A message for an unclosed `{`, a lone `}` or an invalid argument name.
pub(crate) fn parse_braces(text: &str) -> Result<Vec<Seg>, String> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                lit.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                lit.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => return Err(format!("unclosed `{{` in {text:?} (write `{{{{` for a brace)")),
                    }
                }
                let name = name.trim();
                if !valid_arg(name) {
                    return Err(format!(
                        "invalid placeholder `{{{name}}}` in {text:?}: use a name (`{{count}}`) or a position (`{{0}}`)"
                    ));
                }
                push_seg(&mut out, Seg::Text(core::mem::take(&mut lit)));
                out.push(Seg::Arg(name.to_string()));
            }
            '}' => return Err(format!("lone `}}` in {text:?} (write `}}}}` for a brace)")),
            c => lit.push(c),
        }
    }
    push_seg(&mut out, Seg::Text(lit));
    Ok(out)
}

/// The Rust variant name of an identifier: `"items-count"`, `"items_count"` and
/// `"items.count"` all give `ItemsCount`, `"pt-BR"` gives `PtBr`.
///
/// # Errors
/// When the result is not a valid enum variant name (empty, starts with a digit, `Self`).
pub(crate) fn camel(id: &str) -> Result<String, String> {
    let mut out = String::new();
    // Words: split at separators and at lower-case → upper-case steps (`okButton`).
    let mut word_start = true;
    let mut prev_lower = false;
    for c in id.chars() {
        if !c.is_ascii_alphanumeric() {
            word_start = true;
            prev_lower = false;
            continue;
        }
        if prev_lower && c.is_ascii_uppercase() {
            word_start = true;
        }
        out.push(if word_start {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        });
        word_start = false;
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) || out == "Self" {
        return Err(format!(
            "`{id}` does not make a Rust name (it must start with a letter and contain letters or digits)"
        ));
    }
    if !id.is_ascii() {
        return Err(format!("`{id}` is not ASCII: identifiers become Rust names"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces() {
        assert_eq!(
            parse_braces("{n} of {total} {{x}}").unwrap(),
            vec![
                Seg::Arg("n".into()),
                Seg::Text(" of ".into()),
                Seg::Arg("total".into()),
                Seg::Text(" {x}".into()),
            ]
        );
        assert_eq!(parse_braces("{ 0 }").unwrap(), vec![Seg::Arg("0".into())]);
        assert!(parse_braces("{oops").is_err());
        assert!(parse_braces("a } b").is_err());
        assert!(parse_braces("{01}").is_err());
        assert!(parse_braces("{a-b}").is_err());
    }

    #[test]
    fn names() {
        assert_eq!(camel("items-count").unwrap(), "ItemsCount");
        assert_eq!(camel("settings.title").unwrap(), "SettingsTitle");
        assert_eq!(camel("pt_BR").unwrap(), "PtBr");
        assert_eq!(camel("OK").unwrap(), "Ok");
        assert_eq!(camel("okButton").unwrap(), "OkButton");
        assert!(camel("1st").is_err());
        assert!(camel("self").is_err());
        assert!(camel("--").is_err());
    }

    #[test]
    fn canonical_arg_order() {
        let mut v: Vec<String> = ["name", "1", "count", "0", "name"].map(String::from).to_vec();
        sort_args(&mut v);
        assert_eq!(v, ["0", "1", "count", "name"]);
    }
}
