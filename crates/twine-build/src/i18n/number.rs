//! CLDR number symbols and patterns (default numbering system, CLDR 44) for the languages the
//! generator knows: what each `Lang` writes as `twine_extra::i18n::NumberFormat`.

/// How one language writes numbers (the generator's copy of the runtime's `NumberFormat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NumberSymbols {
    /// Decimal separator.
    pub decimal: &'static str,
    /// Grouping separator.
    pub group: &'static str,
    /// Digits of the group next to the decimal separator (0: no grouping).
    pub primary_group: u8,
    /// Digits of the further groups (2 for Indian grouping).
    pub secondary_group: u8,
    /// Grouping starts at `primary_group + min_grouping` integer digits.
    pub min_grouping: u8,
    /// Minus sign (with bidi marks where CLDR has them).
    pub minus: &'static str,
    /// Text before a percentage.
    pub percent_prefix: &'static str,
    /// Text after a percentage.
    pub percent_suffix: &'static str,
    /// Digit zero of the default numbering system.
    pub zero: char,
}

/// CLDR's root locale (`1,234.5`, `-1`, `12%`, ASCII digits).
pub const ROOT: NumberSymbols = NumberSymbols {
    decimal: ".",
    group: ",",
    primary_group: 3,
    secondary_group: 3,
    min_grouping: 1,
    minus: "-",
    percent_prefix: "",
    percent_suffix: "%",
    zero: '0',
};

const NBSP: &str = "\u{A0}";
const NNBSP: &str = "\u{202F}";
const MINUS: &str = "\u{2212}";

const fn latin(decimal: &'static str, group: &'static str, percent_suffix: &'static str) -> NumberSymbols {
    NumberSymbols {
        decimal,
        group,
        percent_suffix,
        ..ROOT
    }
}

/// `1,234.5`, `12%` (English, Chinese, Japanese, Korean, Thai, Hebrew-less defaults …).
const EN: NumberSymbols = ROOT;
/// `1.234,5`, `12 %` (German, Austrian).
const DE: NumberSymbols = latin(",", ".", "\u{A0}%");
/// `1 234,5`, `12 %` with narrow no-break spaces (French).
const FR: NumberSymbols = latin(",", NNBSP, "\u{202F}%");
/// `1 234,5`, `12 %` with no-break spaces (Russian, Ukrainian, Czech, Slovak, Bulgarian).
const RU: NumberSymbols = latin(",", NBSP, "\u{A0}%");
/// `12,34,567.5` (Hindi, Bengali in Latin digits, English in India).
const IN: NumberSymbols = NumberSymbols {
    secondary_group: 2,
    ..ROOT
};

/// Languages (lower-case tags, `_` read as `-`; full tag first, then the base language) and
/// their symbols.
pub const NUMBERS: &[(&str, NumberSymbols)] = &[
    ("en", EN),
    ("en-in", IN),
    ("zh", EN),
    ("ja", EN),
    ("ko", EN),
    ("th", EN),
    ("ms", EN),
    ("fil", EN),
    ("ga", EN),
    ("cy", EN),
    (
        "ur",
        NumberSymbols {
            minus: "\u{200E}-",
            ..EN
        },
    ),
    (
        "he",
        NumberSymbols {
            minus: "\u{200E}-",
            ..EN
        },
    ),
    (
        "iw",
        NumberSymbols {
            minus: "\u{200E}-",
            ..EN
        },
    ),
    ("hi", IN),
    (
        "bn",
        NumberSymbols {
            zero: '\u{9E6}',
            ..IN
        },
    ),
    ("gu", IN),
    ("kn", EN),
    ("ta", IN),
    ("te", IN),
    (
        "mr",
        NumberSymbols {
            zero: '\u{966}',
            ..IN
        },
    ),
    (
        "ne",
        NumberSymbols {
            zero: '\u{966}',
            ..IN
        },
    ),
    ("de", DE),
    ("de-ch", latin(".", "\u{2019}", "%")),
    ("de-at", latin(",", NBSP, "\u{A0}%")),
    ("nl", latin(",", ".", "%")),
    ("it", latin(",", ".", "%")),
    ("pt", latin(",", ".", "%")),
    (
        "pt-pt",
        NumberSymbols {
            min_grouping: 2,
            ..latin(",", NBSP, "%")
        },
    ),
    (
        "es",
        NumberSymbols {
            min_grouping: 2,
            ..latin(",", ".", "\u{A0}%")
        },
    ),
    ("ca", latin(",", ".", "\u{A0}%")),
    ("ro", latin(",", ".", "\u{A0}%")),
    ("el", latin(",", ".", "%")),
    ("id", latin(",", ".", "%")),
    ("vi", latin(",", ".", "%")),
    ("da", latin(",", ".", "\u{A0}%")),
    (
        "tr",
        NumberSymbols {
            percent_prefix: "%",
            percent_suffix: "",
            ..latin(",", ".", "")
        },
    ),
    ("fr", FR),
    ("fr-ch", latin(",", NNBSP, "%")),
    ("ru", RU),
    ("uk", RU),
    ("be", RU),
    ("bg", RU),
    ("cs", RU),
    ("sk", RU),
    (
        "pl",
        NumberSymbols {
            min_grouping: 2,
            ..latin(",", NBSP, "%")
        },
    ),
    ("hu", latin(",", NBSP, "%")),
    (
        "hr",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", ".", "\u{A0}%")
        },
    ),
    ("sr", latin(",", ".", "%")),
    (
        "sl",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", ".", "\u{A0}%")
        },
    ),
    (
        "lt",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    ("lv", latin(",", NBSP, "%")),
    (
        "et",
        NumberSymbols {
            minus: MINUS,
            min_grouping: 2,
            ..latin(",", NBSP, "%")
        },
    ),
    (
        "fi",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    (
        "sv",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    (
        "nb",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    (
        "no",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    (
        "nn",
        NumberSymbols {
            minus: MINUS,
            ..latin(",", NBSP, "\u{A0}%")
        },
    ),
    ("is", latin(",", ".", "%")),
    ("mk", latin(",", ".", "%")),
    // Arabic: Arabic-Indic digits (`arab`), ALM before the minus, `٪` + ALM.
    (
        "ar",
        NumberSymbols {
            decimal: "\u{66B}",
            group: "\u{66C}",
            minus: "\u{61C}-",
            percent_suffix: "\u{66A}\u{61C}",
            zero: '\u{660}',
            ..ROOT
        },
    ),
    // Persian: Extended Arabic-Indic digits (`arabext`), LRM + minus sign.
    (
        "fa",
        NumberSymbols {
            decimal: "\u{66B}",
            group: "\u{66C}",
            minus: "\u{200E}\u{2212}",
            percent_suffix: "\u{66A}",
            zero: '\u{6F0}',
            ..ROOT
        },
    ),
];

/// The symbols of language `code`: the full tag (lower case, `_` read as `-`), then its base
/// language; `None` when neither is known.
#[must_use]
pub fn numbers_for(code: &str) -> Option<NumberSymbols> {
    let tag = code.to_ascii_lowercase().replace('_', "-");
    let base = tag.split('-').next().unwrap_or("");
    [tag.as_str(), base]
        .into_iter()
        .find_map(|c| NUMBERS.iter().find(|(l, _)| *l == c).map(|(_, s)| *s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        assert_eq!(numbers_for("en-IN").unwrap().secondary_group, 2);
        assert_eq!(numbers_for("en_GB").unwrap(), ROOT);
        assert_eq!(numbers_for("fr").unwrap().group, "\u{202F}");
        assert_eq!(numbers_for("ar").unwrap().zero, '٠');
        assert_eq!(numbers_for("fa").unwrap().zero, '۰');
        assert!(numbers_for("xx").is_none());
        let mut seen = std::collections::BTreeSet::new();
        assert!(
            NUMBERS.iter().all(|(l, _)| seen.insert(*l)),
            "a language listed twice"
        );
    }
}
