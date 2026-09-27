//! Translations: `tr!` follows the current language without rebuilding views, missing keys fall
//! back to the key with one warning, and `{}` arguments are formatted in place.

use std::cell::Cell;
use std::rc::Rc;

use twine_extra::i18n::{I18n, Translations, keys_sorted, provide_i18n, rows_complete};
use twine_extra::{tr, translations};
use twine_testing::{TestUi, by_id, capture_logs};
use twine_view::prelude::*;

static TEXTS: Translations = translations! {
    langs: ["en", "de", "zh"],
    "count" => ["{} items", "{} Dinge", "{} 个"],
    "hello" => ["Hello", "Hallo", "你好"],
    "ok" => ["OK", "OK", "确定"],
    "partial" => ["Only English", "", ""],
};

/// Mounts `labels` under `provide_i18n` and returns the UI and the `I18n` handle.
fn ui(app: impl FnOnce(Scope) -> Vec<WidgetView<Label>> + 'static) -> (TestUi, I18n) {
    let slot: Rc<Cell<Option<I18n>>> = Rc::default();
    let s = slot.clone();
    let mut t = TestUi::new(240, 160).mount(move |cx| {
        s.set(Some(provide_i18n(cx, &TEXTS, "en")));
        column(app(cx))
    });
    t.run_until_idle();
    let i18n = slot.get().expect("provided");
    (t, i18n)
}

#[test]
fn tr_returns_current_language() {
    let (mut t, i18n) = ui(|_| vec![label(tr!("hello")).test_id("hello")]);
    assert_eq!(t.find(by_id("hello")).text(), "Hello");
    assert_eq!(i18n.language(), "en");
    assert!(i18n.set_language("zh"));
    t.run_until_idle();
    assert_eq!(t.find(by_id("hello")).text(), "你好");
    assert!(!i18n.set_language("xx"), "unknown codes are rejected");
    assert_eq!(i18n.language(), "zh");
}

#[test]
fn switching_language_updates_labels_without_rebuild() {
    let (mut t, i18n) = ui(|_| {
        vec![
            label(tr!("hello")).test_id("hello"),
            label(tr!("ok")).test_id("ok"),
            label("static").test_id("static"),
        ]
    });
    let ids: Vec<_> = ["hello", "ok", "static"]
        .iter()
        .map(|id| t.find(by_id(id)).id())
        .collect();
    i18n.set_language("de");
    t.run_until_idle();
    let after: Vec<_> = ["hello", "ok", "static"]
        .iter()
        .map(|id| t.find(by_id(id)).id())
        .collect();
    assert_eq!(ids, after, "the same nodes: nothing was rebuilt");
    assert_eq!(t.find(by_id("hello")).text(), "Hallo");
    // Only the label whose text changed was redrawn ("OK" is the same in German).
    let hello = t.find(by_id("hello")).coords();
    let ok = t.find(by_id("ok")).coords();
    let flushed: Vec<_> = t.flushes().iter().map(|f| f.area).collect();
    assert!(flushed.iter().any(|r| r.intersects(&hello)), "{flushed:?}");
    assert!(!flushed.iter().any(|r| r.intersects(&ok)), "{flushed:?}");
}

#[test]
fn missing_key_returns_key_and_warns_once() {
    let ((t, i18n), logs) = capture_logs(|| {
        let (mut t, i18n) = ui(|_| {
            vec![
                label(tr!("no.such.key")).test_id("a"),
                label(tr!("no.such.key")).test_id("b"),
                label(tr!("partial")).test_id("p"),
            ]
        });
        i18n.set_language("de");
        t.run_until_idle();
        (t, i18n)
    });
    assert_eq!(t.find(by_id("a")).text(), "no.such.key");
    assert_eq!(
        t.find(by_id("p")).text(),
        "partial",
        "an empty translation counts as missing"
    );
    let warns = |needle: &str| {
        logs.iter()
            .filter(|l| l.level == log::Level::Warn && l.message.contains(needle))
            .count()
    };
    assert_eq!(warns("no.such.key\" for en"), 1, "{logs:?}");
    assert_eq!(warns("\"partial\" for de"), 1, "{logs:?}");
    assert_eq!(i18n.table().lookup("partial", 0), Some("Only English"));
}

#[test]
fn tr_with_args() {
    let n: Rc<Cell<Option<Signal<u32>>>> = Rc::default();
    let s = n.clone();
    let (mut t, i18n) = ui(move |cx| {
        let count = cx.signal(3u32);
        s.set(Some(count));
        vec![label(tr!("count", count.get())).test_id("count")]
    });
    assert_eq!(t.find(by_id("count")).text(), "3 items");
    n.get().unwrap().set(5);
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "5 items");
    i18n.set_language("zh");
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "5 个");
}

#[test]
fn tr_without_provider_shows_the_key() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(120, 60).mount(|_| label(tr!("hello")).test_id("x"));
        t.run_until_idle();
        assert_eq!(t.find(by_id("x")).text(), "hello");
    });
    assert!(
        logs.iter().any(|l| l.message.contains("outside provide_i18n")),
        "{logs:?}"
    );
}

#[test]
fn translations_const_check_accepts_sorted_keys() {
    const OK: bool = keys_sorted(&["a", "b", "b.c", "bc"]);
    const DUP: bool = keys_sorted(&["a", "a"]);
    const DESC: bool = keys_sorted(&["b", "a"]);
    const _: () = assert!(OK && !DUP && !DESC);
    assert!(rows_complete(2, &[&["a", "b"], &["c", "d"]]));
    assert!(!rows_complete(2, &[&["a"]]));
    assert_eq!(TEXTS.lookup("hello", 1), Some("Hallo"));
    assert_eq!(TEXTS.lookup("zzz", 0), None);
    assert_eq!(TEXTS.lang_index("zh"), Some(2));
}
