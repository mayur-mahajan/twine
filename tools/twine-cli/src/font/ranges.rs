//! `--range` parsing: `0x20-0x7F`, `0xB0`, `32-126`, `chars:°•`.

use std::collections::BTreeSet;

use super::Result;

fn parse_num(s: &str) -> Result<u32> {
    let s = s.trim();
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        s.parse::<u32>()
    };
    v.map_err(|_| format!("invalid code point `{s}` (use decimal or 0x-prefixed hex)").into())
}

fn to_char(v: u32) -> Result<char> {
    char::from_u32(v).ok_or_else(|| format!("U+{v:04X} is not a Unicode scalar value").into())
}

/// Parses one `--range` spec into the characters it names.
///
/// ```
/// use twine_cli::font::parse_range;
/// assert_eq!(parse_range("0x41-0x43").unwrap(), vec!['A', 'B', 'C']);
/// assert_eq!(parse_range("chars:°•").unwrap(), vec!['°', '•']);
/// ```
pub fn parse_range(spec: &str) -> Result<Vec<char>> {
    if let Some(chars) = spec.strip_prefix("chars:") {
        if chars.is_empty() {
            return Err("`chars:` range lists no characters".into());
        }
        return Ok(chars.chars().collect());
    }
    let (a, b) = if let Some((a, b)) = spec.split_once('-') {
        (parse_num(a)?, parse_num(b)?)
    } else {
        let v = parse_num(spec)?;
        (v, v)
    };
    if a > b {
        return Err(format!("range `{spec}` is reversed").into());
    }
    if b - a > 0x10000 {
        return Err(format!("range `{spec}` is larger than 65 536 code points").into());
    }
    (a..=b)
        .filter(|v| !(0xD800..=0xDFFF).contains(v))
        .map(to_char)
        .collect()
}

/// Parses and merges several specs: duplicates removed, sorted ascending.
///
/// ```
/// use twine_cli::font::merge_ranges;
/// let specs = ["0x43".to_string(), "0x41-0x42".to_string(), "chars:BA".to_string()];
/// assert_eq!(merge_ranges(&specs).unwrap(), vec!['A', 'B', 'C']);
/// ```
pub fn merge_ranges(specs: &[String]) -> Result<Vec<char>> {
    let mut set = BTreeSet::new();
    for s in specs {
        set.extend(parse_range(s)?);
    }
    Ok(set.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(parse_range("32-34").unwrap(), vec![' ', '!', '"']);
        assert_eq!(parse_range("0xB0").unwrap(), vec!['°']);
        assert!(parse_range("0x7F-0x20").is_err());
        assert!(parse_range("zz").is_err());
        assert!(parse_range("chars:").is_err());
        assert_eq!(
            parse_range("0xD7FF-0xE000").unwrap().len(),
            2,
            "surrogates skipped"
        );
    }
}
