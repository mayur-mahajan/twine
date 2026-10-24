//! Compile-time translations: the generated test catalog (`tests/i18n/catalog.rs`, from the
//! `.toml` / `.ftl` / `.po` files next to it) and the runtime — `tr!` follows the current
//! language without rebuilding views, arguments (named, positional, plural) are formatted in
//! place, missing texts were filled at build time, and the language carries its direction and
//! font.

mod i18n {
    include!("i18n/catalog.rs");
}

use std::cell::Cell;
use std::rc::Rc;

use i18n::{Key, Lang};
use twine_extra::i18n::plural::{self, PluralCategory, PluralOperands};
use twine_extra::i18n::{BaseDir, Decimal, I18n, Language, MessageKey, expect_i18n, provide_i18n, use_i18n};
use twine_extra::tr;
use twine_testing::{TestUi, by_id, capture_logs};
use twine_view::prelude::*;

/// Mounts `labels` under `provide_i18n` (English) and returns the UI and the handle.
fn ui(app: impl FnOnce(Scope) -> Vec<WidgetView<Label>> + 'static) -> (TestUi, I18n<Lang>) {
    let slot: Rc<Cell<Option<I18n<Lang>>>> = Rc::default();
    let s = slot.clone();
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        s.set(Some(provide_i18n(cx, Lang::En)));
        column(app(cx))
    });
    t.run_until_idle();
    let i18n = slot.get().expect("provided");
    (t, i18n)
}

fn text(t: &TestUi, id: &'static str) -> String {
    t.find(by_id(id)).text()
}

#[test]
fn generated_catalog_is_up_to_date() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/i18n");
    let catalog = twine_build::i18n::Translations::from_dir(dir)
        .default_language("en")
        .languages(["en", "de", "ru", "ar", "zh", "fr", "hi", "fa", "ja"])
        .font("ar", "::twine_text::EMPTY_FONT")
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let path = format!("{dir}/catalog.rs");
    if std::env::var_os("TWINE_I18N_BLESS").is_some() {
        catalog.write(&path).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap();
    assert!(
        committed == catalog.to_rust(),
        "tests/i18n/catalog.rs is stale: regenerate it with the command in tests/i18n/en.toml \
         (or run this test with TWINE_I18N_BLESS=1)"
    );
}

#[test]
fn tr_follows_the_language() {
    let (mut t, i18n) = ui(|_| vec![label(tr!(Key::Hello)).test_id("hello")]);
    assert_eq!(text(&t, "hello"), "Hello");
    for (lang, expected) in [
        (Lang::De, "Hallo"),
        (Lang::Ru, "Привет"),
        (Lang::Ar, "مرحبا"),
        (Lang::Zh, "你好"),
    ] {
        i18n.set_language(lang);
        t.run_until_idle();
        assert_eq!(text(&t, "hello"), expected, "{lang:?}");
        assert_eq!(i18n.language(), lang);
    }
}

#[test]
fn switching_language_updates_labels_without_rebuild() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!(Key::Hello)).test_id("hello"),
            label(tr!(Key::Ok)).test_id("ok"),
            label("static").test_id("static"),
        ]
    });
    let ids = |t: &TestUi| ["hello", "ok", "static"].map(|id| t.find(by_id(id)).id());
    let before = ids(&t);
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert_eq!(before, ids(&t), "the same nodes: nothing was rebuilt");
    assert_eq!(text(&t, "hello"), "Hallo");
    // Only the label whose text changed was redrawn ("OK" is the same in German).
    let hello = t.find(by_id("hello")).coords();
    let ok = t.find(by_id("ok")).coords();
    let flushed: Vec<_> = t.flushes().iter().map(|f| f.area).collect();
    assert!(flushed.iter().any(|r| r.intersects(&hello)), "{flushed:?}");
    assert!(!flushed.iter().any(|r| r.intersects(&ok)), "{flushed:?}");
    // Setting the same language again changes nothing.
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert!(t.flushes().is_empty(), "{:?}", t.flushes());
}

#[test]
fn named_and_positional_arguments_follow_word_order() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!(Key::Greeting, name = "Ada")).test_id("greeting"),
            label(tr!(Key::Copied, file = "a.txt", dir = "/tmp")).test_id("copied"),
            label(tr!(Key::Copied, dir = "/tmp", file = "a.txt")).test_id("copied2"),
            label(tr!(Key::Moved, "a.txt", "/tmp")).test_id("moved"),
        ]
    });
    assert_eq!(text(&t, "greeting"), "Hello, Ada!");
    assert_eq!(text(&t, "copied"), "a.txt copied to /tmp");
    assert_eq!(
        text(&t, "copied2"),
        "a.txt copied to /tmp",
        "argument order in tr! is free"
    );
    assert_eq!(text(&t, "moved"), "a.txt moved to /tmp");
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert_eq!(text(&t, "copied"), "Nach /tmp kopiert: a.txt");
    i18n.set_language(Lang::Ru);
    t.run_until_idle();
    assert_eq!(text(&t, "moved"), "/tmp: перемещено a.txt");
    assert_eq!(
        text(&t, "greeting"),
        "Hello, Ada!",
        "missing in ru: the default language"
    );
}

#[test]
fn plural_variants_follow_the_count_and_the_language() {
    let count: Rc<Cell<Option<Signal<u32>>>> = Rc::default();
    let c = count.clone();
    let (mut t, i18n) = ui(move |cx| {
        let n = cx.signal(0u32);
        c.set(Some(n));
        vec![label(tr!(Key::Items, count = n.get())).test_id("items")]
    });
    let n = count.get().unwrap();
    let mut check = |lang: Lang, cases: &[(u32, &str)]| {
        i18n.set_language(lang);
        for (value, expected) in cases {
            n.set(*value);
            t.run_until_idle();
            assert_eq!(text(&t, "items"), *expected, "{lang:?} {value}");
        }
    };
    check(
        Lang::En,
        &[(0, "No items"), (1, "1 item"), (2, "2 items"), (21, "21 items")],
    );
    check(Lang::De, &[(0, "Keine Dinge"), (1, "1 Ding"), (7, "7 Dinge")]);
    check(
        Lang::Ru,
        &[
            (1, "1 вещь"),
            (3, "3 вещи"),
            (5, "5 вещей"),
            (11, "11 вещей"),
            (21, "21 вещь"),
            (22, "22 вещи"),
        ],
    );
    check(
        Lang::Ar,
        &[
            (0, "لا عناصر"),
            (1, "عنصر واحد"),
            (2, "عنصران"),
            (3, "٣ عناصر"),
            (11, "١١ عنصرًا"),
            (100, "١٠٠ عنصر"),
        ],
    );
    check(Lang::Zh, &[(1, "1 个"), (5, "5 个")]);
}

#[test]
fn plural_on_decimals_and_non_numbers() {
    let (t, _) = ui(|_| {
        vec![
            label(tr!(Key::Items, count = "1.0")).test_id("decimal"),
            label(tr!(Key::Items, count = "lots")).test_id("text"),
            label(tr!(Key::Items, count = -1)).test_id("negative"),
        ]
    });
    assert_eq!(
        text(&t, "decimal"),
        "1.0 items",
        "CLDR: 1.0 is `other` in English"
    );
    assert_eq!(text(&t, "text"), "lots items", "not a number: `other`");
    assert_eq!(text(&t, "negative"), "-1 item", "rules use the absolute value");
}

#[test]
fn fallbacks_were_filled_at_build_time() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!(Key::OnlyEn)).test_id("only"),
            label(tr!(Key::SettingsTitle)).test_id("settings"),
        ]
    });
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert_eq!(text(&t, "only"), "Only English");
    assert_eq!(text(&t, "settings"), "Einstellungen", "Fluent attribute + term");
    i18n.set_language(Lang::Zh);
    t.run_until_idle();
    assert_eq!(text(&t, "settings"), "Settings");
    // Every key has a text in every language.
    for lang in Lang::ALL {
        assert_eq!(Language::messages(lang).len(), Key::ALL.len());
    }
}

#[test]
fn language_metadata() {
    assert_eq!(Lang::DEFAULT, Lang::En);
    assert_eq!(Lang::from_code("ar"), Some(Lang::Ar));
    assert_eq!(Lang::from_code("xx"), None);
    assert_eq!(Lang::Ar.base_dir(), BaseDir::Rtl);
    assert_eq!(Lang::Zh.base_dir(), BaseDir::Ltr);
    assert!(core::ptr::eq(
        Lang::Ar.font().unwrap(),
        &raw const twine_text::EMPTY_FONT
    ));
    assert!(Lang::En.font().is_none());
    assert_eq!(Lang::Ru.code(), "ru");
    assert_eq!(
        format!("{:?} {:?}", Lang::De, Key::SettingsTitle),
        "de settings.title"
    );
    assert_eq!(Key::Moved.args(), ["0", "1"]);
    assert_eq!(MessageKey::id(Key::OnlyEn), "only-en");
    assert_eq!(
        Lang::Ru.plural_rule()(PluralOperands::integer(3)),
        PluralCategory::Few
    );
}

#[test]
fn direction_and_font_follow_the_language() {
    let (mut t, i18n) = ui(|_| vec![label(tr!(Key::Hello)).test_id("hello")]);
    let dir = Rc::new(Cell::new(BaseDir::Ltr));
    let d = dir.clone();
    let cx = t.root_scope();
    cx.effect(move || d.set(i18n.base_dir()));
    i18n.set_language(Lang::Ar);
    t.run_until_idle();
    assert_eq!(dir.get(), BaseDir::Rtl);
    assert!(i18n.font().is_some());
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert_eq!(dir.get(), BaseDir::Ltr);
    assert!(i18n.font().is_none());
}

#[test]
fn text_of_a_key_with_arguments_warns() {
    let cx = twine_reactive::Runtime::take().unwrap().create_root();
    let i = provide_i18n(cx, Lang::De);
    assert_eq!(i.text(Key::Hello), "Hallo");
    let (s, logs) = capture_logs(|| i.text(Key::Greeting));
    assert_eq!(s, "greeting");
    assert!(
        logs.iter().any(|l| l.message.contains("has arguments")),
        "{logs:?}"
    );
}

#[test]
fn tr_without_provider_shows_the_default_language() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(200, 60).mount(|_| {
            column((
                label(tr!(Key::Hello)).test_id("x"),
                label(tr!(Key::Greeting, name = "Ada")).test_id("y"),
            ))
        });
        t.run_until_idle();
        assert_eq!(text(&t, "x"), "Hello");
        assert_eq!(text(&t, "y"), "Hello, Ada!");
    });
    assert!(
        logs.iter().any(|l| l.message.contains("outside provide_i18n")),
        "{logs:?}"
    );
}

#[test]
fn use_i18n_finds_the_provided_handle_and_names_it_when_missing() {
    let t = TestUi::new(120, 60).mount(|cx| {
        let _ = provide_i18n(cx, Lang::De);
        label("x")
    });
    let i = use_i18n::<Lang>(t.root_scope()).expect("provided by the app");
    assert_eq!(
        i.language(),
        expect_i18n::<Lang>(t.root_scope().child()).language()
    );
    let bare = TestUi::new(120, 60).mount(|_cx| label("x"));
    let Err(err) = use_i18n::<Lang>(bare.root_scope()) else {
        panic!("not provided")
    };
    assert!(err.type_name().contains("I18n"), "{}", err.type_name());
}

/// The rule functions select exactly the categories `twine_build::i18n::RULES` lists for
/// them (all, and for integers), so `.po` plural forms map to the right categories.
#[test]
fn plural_rules_match_the_generator_table() {
    use std::collections::BTreeSet;
    let name = |c: PluralCategory| match c {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    };
    let mut decimals = vec![];
    for i in 0..=30u32 {
        for f in ["0", "1", "5", "00", "10", "11", "15", "21", "50", "75"] {
            decimals.push(format!("{i}.{f}"));
        }
    }
    let integers: Vec<String> = (0..=1200u64)
        .chain([1_000_000, 2_000_000])
        .map(|n| n.to_string())
        .collect();
    assert_eq!(plural::RULES.len(), twine_build::i18n::RULES.len());
    for r in twine_build::i18n::RULES {
        let rule = plural::RULES
            .iter()
            .find(|(n, _)| *n == r.name)
            .unwrap_or_else(|| panic!("no runtime rule {}", r.name))
            .1;
        let cats = |samples: &[String]| -> BTreeSet<&str> {
            samples
                .iter()
                .map(|s| name(rule(PluralOperands::parse(s).unwrap())))
                .collect()
        };
        let int: BTreeSet<&str> = cats(&integers);
        let all: BTreeSet<&str> = int.union(&cats(&decimals)).copied().collect();
        let want_int: BTreeSet<&str> = r.integer.iter().map(|c| c.name()).collect();
        let want_all: BTreeSet<&str> = r.all.iter().map(|c| c.name()).collect();
        assert_eq!(int, want_int, "{} (integers)", r.name);
        assert_eq!(all, want_all, "{} (all)", r.name);
    }
    assert_eq!(twine_build::i18n::MAX_ARGS, twine_extra::i18n::MAX_ARGS);
}

#[test]
fn numbers_follow_the_language() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!(Key::Total, amount = Decimal::new(123_456_789, 2))).test_id("total"),
            label(tr!(Key::Total, amount = -1_234_567)).test_id("negative"),
            label(tr!(Key::Total, amount = Decimal::new(125, 3).percent())).test_id("percent"),
        ]
    });
    let cases: [(Lang, &str, &str, &str); 9] = [
        (
            Lang::En,
            "Total: 1,234,567.89",
            "Total: -1,234,567",
            "Total: 12.5%",
        ),
        (
            Lang::De,
            "Total: 1.234.567,89",
            "Total: -1.234.567",
            "Total: 12,5\u{A0}%",
        ),
        (
            Lang::Fr,
            "Total : 1\u{202F}234\u{202F}567,89",
            "Total : -1\u{202F}234\u{202F}567",
            "Total : 12,5\u{202F}%",
        ),
        (
            Lang::Ru,
            "Total: 1\u{A0}234\u{A0}567,89",
            "Total: -1\u{A0}234\u{A0}567",
            "Total: 12,5\u{A0}%",
        ),
        (
            Lang::Hi,
            "Total: 12,34,567.89",
            "Total: -12,34,567",
            "Total: 12.5%",
        ),
        (
            Lang::Ja,
            "Total: 1,234,567.89",
            "Total: -1,234,567",
            "Total: 12.5%",
        ),
        (
            Lang::Zh,
            "Total: 1,234,567.89",
            "Total: -1,234,567",
            "Total: 12.5%",
        ),
        (
            Lang::Ar,
            "Total: ١٬٢٣٤٬٥٦٧٫٨٩",
            "Total: \u{61C}-١٬٢٣٤٬٥٦٧",
            "Total: ١٢٫٥\u{66A}\u{61C}",
        ),
        (
            Lang::Fa,
            "Total: ۱٬۲۳۴٬۵۶۷٫۸۹",
            "Total: \u{200E}\u{2212}۱٬۲۳۴٬۵۶۷",
            "Total: ۱۲٫۵\u{66A}",
        ),
    ];
    for (lang, total, negative, percent) in cases {
        i18n.set_language(lang);
        t.run_until_idle();
        assert_eq!(text(&t, "total"), total, "{lang:?}");
        assert_eq!(text(&t, "negative"), negative, "{lang:?}");
        assert_eq!(text(&t, "percent"), percent, "{lang:?}");
    }
}

#[test]
fn plural_operands_come_from_the_formatted_number() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!(Key::Items, count = Decimal::from(1))).test_id("one"),
            label(tr!(Key::Items, count = Decimal::new(10, 1))).test_id("one_point_zero"),
            label(tr!(
                Key::Items,
                count = Decimal::new(10, 1).max_fraction_digits(0)
            ))
            .test_id("rounded"),
        ]
    });
    assert_eq!(text(&t, "one"), "1 item");
    assert_eq!(
        text(&t, "one_point_zero"),
        "1.0 items",
        "v = 1: `other` in English"
    );
    assert_eq!(text(&t, "rounded"), "1 item", "the shown digits decide");
    i18n.set_language(Lang::De);
    t.run_until_idle();
    assert_eq!(text(&t, "one_point_zero"), "1,0 Dinge");
    i18n.set_language(Lang::Ru);
    t.run_until_idle();
    assert_eq!(
        text(&t, "one_point_zero"),
        "1,0 вещей",
        "decimals: `other` (the last form)"
    );
    i18n.set_language(Lang::Ar);
    t.run_until_idle();
    assert_eq!(text(&t, "one"), "عنصر واحد");
}

#[test]
fn i18n_number_localizes_outside_messages() {
    let cx = twine_reactive::Runtime::take().unwrap().create_root();
    let i = provide_i18n(cx, Lang::Fa);
    assert_eq!(i.number(1_234).to_string(), "۱٬۲۳۴");
    i.set_language(Lang::Hi);
    assert_eq!(i.number(Decimal::new(123_456_789, 0)).to_string(), "12,34,56,789");
    assert_eq!(Lang::Fr.number_format().group, "\u{202F}");
}
