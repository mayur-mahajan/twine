//! Builder tests: validation, fallbacks, `.po` plural forms, naming and the generated code.

use super::*;

fn en_toml() -> &'static str {
    r#"
hello = "Hello"
greeting = "Hello, {name}!"
moved = "{0} moved to {1}"
only-en = "Only English"
[items]
"0" = "No items"
one = "{count} item"
other = "{count} items"
"#
}

fn build(t: &Translations) -> Catalog {
    t.build().unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn three_formats_one_catalog() {
    let c = build(&
        Translations::new()
            .source("en", Format::Toml, en_toml())
            .source(
                "de",
                Format::Fluent,
                "hello = Hallo\ngreeting = Hallo, { $name }!\nitems = { $count ->\n  [one] ein Ding\n *[other] { $count } Dinge\n}\n",
            )
            .source(
                "ru",
                Format::Po,
                "msgid \"items\"\nmsgid_plural \"items\"\nmsgstr[0] \"{count} вещь\"\nmsgstr[1] \"{count} вещи\"\nmsgstr[2] \"{count} вещей\"\n\nmsgid \"moved\"\nmsgstr \"{1}: перемещено {0}\"\n",
            )
            .default_language("en"),
    );
    assert_eq!(c.languages().collect::<Vec<_>>(), ["en", "de", "ru"]);
    assert_eq!(
        c.keys().collect::<Vec<_>>(),
        ["greeting", "hello", "items", "moved", "only-en"]
    );
    assert_eq!(c.preview("de", "greeting").unwrap(), "Hallo, {name}!");
    assert_eq!(c.preview("ru", "moved").unwrap(), "{1}: перемещено {0}");
    assert_eq!(
        c.preview("ru", "items").unwrap(),
        "{count -> [one] {count} вещь | [few] {count} вещи | [many] {count} вещей | *[other] {count} вещей}"
    );
    // Missing texts fall back to the default language, with a warning per language.
    assert_eq!(c.preview("ru", "hello").unwrap(), "Hello");
    assert_eq!(c.preview("de", "only-en").unwrap(), "Only English");
    let w: Vec<String> = c.warnings().iter().map(ToString::to_string).collect();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].contains("<de.ftl>") && w[0].contains("2 missing") && w[0].contains("moved, only-en"),
        "{w:?}"
    );
    assert!(c.chars().contains(&'ш') || c.chars().contains(&'щ'));
}

#[test]
fn strict_turns_warnings_into_errors() {
    let e = Translations::new()
        .source("en", Format::Toml, "a = \"A\"\nb = \"B\"")
        .source("de", Format::Toml, "a = \"A\"\nc = \"C\"")
        .default_language("en")
        .strict(true)
        .build()
        .unwrap_err();
    let msgs = e.to_string();
    assert!(msgs.contains("`c` is not a key"), "{msgs}");
    assert!(
        msgs.contains("1 missing translation(s), shown in en: b"),
        "{msgs}"
    );
}

#[test]
fn argument_errors() {
    let e = Translations::new()
        .source("en", Format::Toml, "a = \"Hi {name}\"\nb = \"{1}\"")
        .source("de", Format::Fluent, "a = Hallo { $nme }\n")
        .default_language("en")
        .build()
        .unwrap_err();
    let msgs = e.to_string();
    assert!(msgs.contains("<de.ftl>:1: `a` uses the argument `nme`"), "{msgs}");
    assert!(msgs.contains("`b`: positional arguments must be"), "{msgs}");
    let many: String = (0..=MAX_ARGS)
        .map(|i| ["{a", &i.to_string(), "}"].concat())
        .collect();
    let e = Translations::new()
        .source("en", Format::Toml, &format!("m = \"{many}\""))
        .build()
        .unwrap_err();
    assert!(e.to_string().contains("at most 8"), "{e}");
}

#[test]
fn never_chosen_category_warns() {
    let c = build(&Translations::new().source(
        "en",
        Format::Fluent,
        "n = { $n ->\n [few] few\n *[other] many\n}\n",
    ));
    assert!(
        c.warnings()[0].message.contains("`few` is never chosen in `en`"),
        "{:?}",
        c.warnings()
    );
}

#[test]
fn po_forms_must_match_the_language() {
    let po = "msgid \"n\"\nmsgid_plural \"n\"\nmsgstr[0] \"{n} a\"\nmsgstr[1] \"{n} b\"\n";
    let e = Translations::new()
        .source("en", Format::Toml, "[n]\none = \"{n}\"\nother = \"{n}\"")
        .source("ru", Format::Po, po)
        .default_language("en")
        .build()
        .unwrap_err();
    assert!(e.to_string().contains("2 plural forms, but `ru` has 4"), "{e}");
    // Czech: three integer forms (one, few, other) are accepted; decimals use `other`.
    let cs = "msgid \"n\"\nmsgid_plural \"n\"\nmsgstr[0] \"a\"\nmsgstr[1] \"b {n}\"\nmsgstr[2] \"c\"\n";
    let c = build(
        &Translations::new()
            .source("en", Format::Toml, "[n]\none = \"{n}\"\nother = \"{n}\"")
            .source("cs", Format::Po, cs)
            .default_language("en"),
    );
    assert_eq!(
        c.preview("cs", "n").unwrap(),
        "{n -> [one] a | [few] b {n} | *[other] c}"
    );
}

#[test]
fn languages_order_fallbacks_and_overrides() {
    let c = build(
        &Translations::new()
            .source("en", Format::Toml, "a = \"A\"\nb = \"B\"\nc = \"C\"")
            .source("de", Format::Toml, "a = \"A-de\"\nb = \"B-de\"")
            .source("de-AT", Format::Toml, "a = \"A-at\"")
            .source("xx", Format::Toml, "a = \"A-xx\"")
            .languages(["de", "de-AT", "en", "xx"])
            .default_language("en")
            .plural_rules("xx", "other")
            .base_dir("xx", Dir::Rtl)
            .fallback("xx", "de-AT")
            .font("xx", "crate::FONT"),
    );
    assert_eq!(c.languages().collect::<Vec<_>>(), ["de", "de-AT", "en", "xx"]);
    assert_eq!(
        c.preview("de-AT", "b").unwrap(),
        "B-de",
        "regional → base language"
    );
    assert_eq!(c.preview("de-AT", "c").unwrap(), "C", "→ default");
    assert_eq!(c.preview("xx", "b").unwrap(), "B-de", "explicit fallback chain");
    let rust = c.to_rust();
    assert!(rust.contains("pub const DEFAULT: Lang = Lang::En;"), "{rust}");
    assert!(rust.contains("Lang::Xx => Some(&crate::FONT),"), "{rust}");
    assert!(
        rust.contains("Lang::Xx => ::twine_extra::i18n::BaseDir::Rtl,"),
        "{rust}"
    );
    assert!(
        rust.contains("Lang::De | Lang::DeAt | Lang::En => ::twine_extra::i18n::plural::en,"),
        "{rust}"
    );
}

#[test]
fn configuration_errors() {
    let two = || {
        Translations::new()
            .source("en", Format::Toml, "a = \"A\"")
            .source("de", Format::Toml, "a = \"B\"")
    };
    assert!(
        two()
            .build()
            .unwrap_err()
            .to_string()
            .contains("choose the default language")
    );
    let e = two().default_language("fr").build().unwrap_err();
    assert!(e.to_string().contains("`fr` has no translation file"), "{e}");
    let e = Translations::new()
        .source("xx", Format::Toml, "a = \"A\"")
        .build()
        .unwrap_err();
    assert!(
        e.to_string().contains("no CLDR plural rule is known for `xx`"),
        "{e}"
    );
    let e = Translations::new()
        .source("config", Format::Toml, "a = \"A\"")
        .build()
        .unwrap_err();
    assert!(e.to_string().contains("not a language code"), "{e}");
    let e = Translations::new()
        .source("en", Format::Toml, "a-b = \"A\"\na_b = \"B\"")
        .build()
        .unwrap_err();
    assert!(e.to_string().contains("both become `Key::AB`"), "{e}");
    let e = two().default_language("en").font("fr", "X").build().unwrap_err();
    assert!(e.to_string().contains("font for `fr`"), "{e}");
}

#[test]
fn rtl_defaults() {
    assert_eq!(default_dir("ar"), Dir::Rtl);
    assert_eq!(default_dir("he"), Dir::Rtl);
    assert_eq!(default_dir("fa-IR"), Dir::Rtl);
    assert_eq!(default_dir("en"), Dir::Ltr);
    assert_eq!(default_dir("pa-Arab"), Dir::Rtl);
    assert_eq!(default_dir("ku-Latn"), Dir::Ltr);
}

#[test]
fn generated_code_is_deterministic_and_complete() {
    let t = Translations::new().source("en", Format::Toml, en_toml()).source(
        "ar",
        Format::Toml,
        "hello = \"مرحبا\"",
    );
    let a = build(&t.clone().default_language("en")).to_rust();
    let b = build(&t.default_language("en")).to_rust();
    assert_eq!(a, b);
    for needle in [
        "pub enum Key {",
        "Key::Greeting => &[\"name\"],",
        "Key::Moved => &[\"0\", \"1\"],",
        "Key::Hello | Key::OnlyEn => &[],",
        "pub enum Lang {",
        "Lang::Ar => ::twine_extra::i18n::BaseDir::Rtl,",
        "Lang::Ar => ::twine_extra::i18n::plural::ar,",
        "rt::Message::Plural(&rt::Plural { arg: 0, variants: &[rt::Variant { selector: rt::Selector::Exact(0), segments: &[rt::Segment::Text(\"No items\")] }, rt::Variant { selector: rt::Selector::Category(rt::PluralCategory::One), segments: &[rt::Segment::Arg(0), rt::Segment::Text(\" item\")] }], other: &[rt::Segment::Arg(0), rt::Segment::Text(\" items\")] })",
        "rt::Message::Text(\"مرحبا\")",
    ] {
        assert!(a.contains(needle), "missing {needle:?} in\n{a}");
    }
}

#[test]
fn reads_a_directory() {
    let dir = std::env::temp_dir().join(format!("twine-build-i18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("en.toml"), "a = \"A\"").unwrap();
    std::fs::write(dir.join("de.ftl"), "a = B\n").unwrap();
    std::fs::write(dir.join("README.md"), "ignored").unwrap();
    let c = Translations::from_dir(&dir).default_language("en").build();
    let _ = std::fs::remove_dir_all(&dir);
    let c = c.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.languages().collect::<Vec<_>>(), ["en", "de"]);
    assert_eq!(c.inputs().len(), 3, "the directory and two files");
    assert!(c.to_rust().contains("from en.toml, de.ftl"));
}

#[test]
fn number_formats_per_language() {
    let c = build(
        &Translations::new()
            .source("en", Format::Toml, "a = \"{n}\"")
            .source("fa", Format::Toml, "a = \"{n}\"")
            .source("xx", Format::Toml, "a = \"{n}\"")
            .plural_rules("xx", "other")
            .default_language("en"),
    );
    let rust = c.to_rust();
    assert!(
        rust.contains("const NUMBER_FORMATS: &'static [::twine_extra::i18n::NumberFormat] = &["),
        "{rust}"
    );
    assert!(rust.contains("zero: '۰'"), "{rust}");
    assert!(
        c.warnings()
            .iter()
            .any(|w| w.message.contains("no CLDR number symbols are known for `xx`")),
        "{:?}",
        c.warnings()
    );
    let symbols = NumberSymbols {
        decimal: "·",
        ..ROOT_NUMBERS
    };
    let c = build(
        &Translations::new()
            .source("xx", Format::Toml, "a = \"{n}\"")
            .plural_rules("xx", "other")
            .number_symbols("xx", symbols),
    );
    assert!(c.warnings().is_empty(), "{:?}", c.warnings());
    assert!(c.to_rust().contains("decimal: \"·\""));
}

/// The number characters follow each language's numbering system; bidi marks are left out,
/// and the message characters stay apart from them.
#[test]
fn number_chars_follow_each_language() {
    let c = build(
        &Translations::new()
            .source("en", Format::Toml, "a = \"n {n}\"")
            .source("ar", Format::Toml, "a = \"ع {n}\"")
            .source("fa", Format::Toml, "a = \"ع {n}\"")
            .default_language("en"),
    );
    let n = c.number_chars();
    for d in 0..10 {
        for zero in ['0', '\u{660}', '\u{6F0}'] {
            let digit = char::from_u32(u32::from(zero) + d).unwrap();
            assert!(n.contains(&digit), "U+{:04X}", u32::from(digit));
        }
    }
    // Arabic decimal and group separators and percent sign.
    for s in ['\u{66B}', '\u{66C}', '\u{66A}'] {
        assert!(n.contains(&s), "U+{:04X}", u32::from(s));
    }
    for mark in ['\u{61C}', '\u{200E}', '\u{200F}'] {
        assert!(!n.contains(&mark));
    }
    assert_eq!(c.chars().into_iter().collect::<String>(), " nع");
    let mut all = std::collections::BTreeSet::new();
    crate::chars::CharSource::add_chars(&c, &mut all);
    assert!(all.is_superset(&n) && all.is_superset(&c.chars()));
}
