//! CLDR plural rules known to the generator: which runtime rule
//! (`twine_extra::i18n::plural::<name>`) each language uses and which categories it can select.

use core::fmt;

/// A CLDR plural category (`zero`, `one`, `two`, `few`, `many`, `other`), in CLDR order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// `zero`.
    Zero,
    /// `one`.
    One,
    /// `two`.
    Two,
    /// `few`.
    Few,
    /// `many`.
    Many,
    /// `other` (every language has it; the catch-all).
    Other,
}

impl Category {
    /// Every category, in CLDR order.
    pub const ALL: [Category; 6] = [
        Category::Zero,
        Category::One,
        Category::Two,
        Category::Few,
        Category::Many,
        Category::Other,
    ];

    /// The CLDR name (`"one"`, …).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Category::Zero => "zero",
            Category::One => "one",
            Category::Two => "two",
            Category::Few => "few",
            Category::Many => "many",
            Category::Other => "other",
        }
    }

    /// The category named `name` (`"one"` → [`Category::One`]).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The variant name of the runtime enum `PluralCategory` (`"One"`).
    pub(crate) const fn variant(self) -> &'static str {
        match self {
            Category::Zero => "Zero",
            Category::One => "One",
            Category::Two => "Two",
            Category::Few => "Few",
            Category::Many => "Many",
            Category::Other => "Other",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The plural rule of a group of languages: the runtime function that implements it and the
/// categories it selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluralRules {
    /// Name of the runtime rule, `twine_extra::i18n::plural::<name>` (named after a language
    /// that uses it). Also what [`Translations::plural_rules`](super::Translations::plural_rules)
    /// takes to choose the rule of a language explicitly.
    pub name: &'static str,
    /// Every category the rule selects for some number (integers and decimals), in CLDR order.
    pub all: &'static [Category],
    /// The categories it selects for integers, in CLDR order (what a gettext `.po` file with
    /// integer-only plural forms lists).
    pub integer: &'static [Category],
    /// The base language codes (lower case, without region or script) that use the rule.
    pub languages: &'static [&'static str],
}

use Category::{Few, Many, One, Other, Two, Zero};

/// The rules the generator knows (CLDR 44 cardinal rules, compact-exponent operand `e` = 0).
/// Each one matches a function of `twine_extra::i18n::plural`; a test there checks that the
/// categories listed here are exactly the ones the function selects.
pub const RULES: &[PluralRules] = &[
    PluralRules {
        name: "other",
        all: &[Other],
        integer: &[Other],
        languages: &[
            "bm", "bo", "dz", "id", "ig", "ii", "in", "ja", "jbo", "jv", "jw", "kde", "kea", "km", "ko",
            "lkt", "lo", "ms", "my", "nqo", "osa", "sah", "ses", "sg", "su", "th", "to", "tpi", "vi", "wo",
            "yo", "yue", "zh",
        ],
    },
    PluralRules {
        name: "en",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &[
            "ast", "de", "en", "et", "fi", "fy", "gl", "ia", "io", "ji", "lij", "nl", "sc", "sv", "sw", "ur",
            "yi",
        ],
    },
    PluralRules {
        name: "tr",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &[
            "af", "an", "az", "bg", "ce", "chr", "ckb", "dv", "ee", "el", "eo", "eu", "fo", "fur", "gsw",
            "ha", "haw", "hu", "ka", "kk", "kl", "ks", "ku", "ky", "lb", "lg", "ml", "mn", "mr", "nb", "nd",
            "ne", "nn", "no", "nr", "ny", "om", "or", "os", "pap", "ps", "rm", "sd", "sn", "so", "sq", "ss",
            "st", "syr", "ta", "te", "tk", "tn", "tr", "ts", "ug", "uz", "ve", "vo", "xh",
        ],
    },
    PluralRules {
        name: "fr",
        all: &[One, Many, Other],
        integer: &[One, Many, Other],
        languages: &["fr", "pt"],
    },
    PluralRules {
        name: "es",
        all: &[One, Many, Other],
        integer: &[One, Many, Other],
        languages: &["es"],
    },
    PluralRules {
        name: "it",
        all: &[One, Many, Other],
        integer: &[One, Many, Other],
        languages: &["ca", "it", "pt-pt", "vec"],
    },
    PluralRules {
        name: "ru",
        all: &[One, Few, Many, Other],
        integer: &[One, Few, Many],
        languages: &["ru", "uk"],
    },
    PluralRules {
        name: "be",
        all: &[One, Few, Many, Other],
        integer: &[One, Few, Many],
        languages: &["be"],
    },
    PluralRules {
        name: "pl",
        all: &[One, Few, Many, Other],
        integer: &[One, Few, Many],
        languages: &["pl"],
    },
    PluralRules {
        name: "cs",
        all: &[One, Few, Many, Other],
        integer: &[One, Few, Other],
        languages: &["cs", "sk"],
    },
    PluralRules {
        name: "ar",
        all: &[Zero, One, Two, Few, Many, Other],
        integer: &[Zero, One, Two, Few, Many, Other],
        languages: &["ar", "ars"],
    },
    PluralRules {
        name: "he",
        all: &[One, Two, Other],
        integer: &[One, Two, Other],
        languages: &["he", "iw"],
    },
    PluralRules {
        name: "hi",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &["am", "as", "bn", "doi", "fa", "gu", "hi", "kn", "pcm", "zu"],
    },
    PluralRules {
        name: "da",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &["da"],
    },
    PluralRules {
        name: "is",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &["is"],
    },
    PluralRules {
        name: "lt",
        all: &[One, Few, Many, Other],
        integer: &[One, Few, Other],
        languages: &["lt"],
    },
    PluralRules {
        name: "lv",
        all: &[Zero, One, Other],
        integer: &[Zero, One, Other],
        languages: &["lv", "prg"],
    },
    PluralRules {
        name: "ro",
        all: &[One, Few, Other],
        integer: &[One, Few, Other],
        languages: &["mo", "ro"],
    },
    PluralRules {
        name: "sl",
        all: &[One, Two, Few, Other],
        integer: &[One, Two, Few, Other],
        languages: &["sl"],
    },
    PluralRules {
        name: "hr",
        all: &[One, Few, Other],
        integer: &[One, Few, Other],
        languages: &["bs", "hr", "sh", "sr"],
    },
    PluralRules {
        name: "mk",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &["mk"],
    },
    PluralRules {
        name: "fil",
        all: &[One, Other],
        integer: &[One, Other],
        languages: &["fil", "tl"],
    },
    PluralRules {
        name: "ga",
        all: &[One, Two, Few, Many, Other],
        integer: &[One, Two, Few, Many, Other],
        languages: &["ga"],
    },
    PluralRules {
        name: "cy",
        all: &[Zero, One, Two, Few, Many, Other],
        integer: &[Zero, One, Two, Few, Many, Other],
        languages: &["cy"],
    },
];

/// The rule named `name` (see [`PluralRules::name`]).
#[must_use]
pub fn rules_named(name: &str) -> Option<&'static PluralRules> {
    RULES.iter().find(|r| r.name == name)
}

/// The rule of language `code` (a BCP 47 tag such as `"pt-PT"` or `"zh_Hant"`): the full tag
/// first (lower case, `_` read as `-`), then its base language.
#[must_use]
pub fn rules_for(code: &str) -> Option<&'static PluralRules> {
    let tag = code.to_ascii_lowercase().replace('_', "-");
    let base = tag.split('-').next().unwrap_or("");
    [tag.as_str(), base]
        .into_iter()
        .find_map(|c| RULES.iter().find(|r| r.languages.contains(&c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_one_rule() {
        let mut seen = std::collections::BTreeSet::new();
        for r in RULES {
            assert!(
                r.all.contains(&Other) && r.all.windows(2).all(|w| w[0] < w[1]),
                "{}",
                r.name
            );
            assert!(r.integer.iter().all(|c| r.all.contains(c)), "{}", r.name);
            for l in r.languages {
                assert!(seen.insert(*l), "{l} listed twice");
            }
        }
    }

    #[test]
    fn lookup_by_tag() {
        assert_eq!(rules_for("pt-PT").map(|r| r.name), Some("it"));
        assert_eq!(rules_for("pt_BR").map(|r| r.name), Some("fr"));
        assert_eq!(rules_for("zh-Hant").map(|r| r.name), Some("other"));
        assert_eq!(rules_for("RU").map(|r| r.name), Some("ru"));
        assert!(rules_for("xx").is_none());
        assert_eq!(Category::from_name("few"), Some(Few));
    }
}
