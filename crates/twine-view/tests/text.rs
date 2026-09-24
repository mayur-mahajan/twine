//! Text properties, `text!` and two-way models.

use std::cell::Cell;
use std::rc::Rc;

use twine_engine::State;
use twine_reactive::debug_stats;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A counter label and a button incrementing it by `step`.
fn counter_app(step: i32) -> impl FnOnce(Scope) -> Flex {
    move |cx| {
        let n = cx.signal(1000i32);
        column((
            label(text!("n = {}", n.get())).test_id("n"),
            // A plain clickable label: a themed button's press transitions would allocate.
            label("inc")
                .clickable(true)
                .on_click(move || n.update(|v| *v += step)),
        ))
    }
}

#[test]
fn text_macro_no_alloc_after_warmup() {
    let mut t = TestUi::new(200, 80).mount(counter_app(1));
    t.run_until_idle();
    let btn = t.find(by_text("inc")).coords().center();
    // Warm up: the label's second buffer, the event and effect queues.
    for _ in 0..3 {
        t.tap(btn);
        t.run_until_idle();
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..100 {
            t.tap(btn);
            t.run_until_idle();
        }
    });
    assert_eq!(t.find(by_id("n")).text(), "n = 1103");
    assert_eq!(stats.allocs, 0, "{stats:?}");
}

#[test]
fn text_same_value_no_invalidate() {
    let mut t = TestUi::new(200, 80).mount(counter_app(0));
    t.run_until_idle();
    let btn = t.find(by_text("inc")).coords().center();
    let label = t.find(by_id("n")).coords();
    t.tap(btn);
    t.run_until_idle();
    // Nothing changes: the binding runs and writes the same text, nothing is invalidated.
    t.press(btn);
    t.release();
    assert!(
        !t.invalidations().iter().any(|(r, _)| r.intersects(&label)),
        "{:?}",
        t.invalidations().to_vec()
    );
    t.run_until_idle();
    assert_eq!(t.find(by_id("n")).text(), "n = 1000");
}

#[test]
fn into_text_all_kinds() {
    let mut t = TestUi::new(300, 200).mount(|cx| {
        let name = cx.signal(String::from("sig"));
        let n = cx.signal(2);
        let m = cx.memo(move || format!("memo {}", n.get()));
        cx.provide((name, n));
        column((
            label("static").test_id("a"),
            label(String::from("owned")).test_id("b"),
            label(name).test_id("c"),
            label(name.read_only()).test_id("d"),
            label(m).test_id("e"),
            label(move || format!("fn {}", n.get())).test_id("f"),
            label(text!("macro {}", n.get())).test_id("g"),
        ))
    });
    t.run_until_idle();
    let texts = |t: &TestUi| {
        ["a", "b", "c", "d", "e", "f", "g"]
            .iter()
            .map(|id| t.find(by_id(id)).text())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        texts(&t),
        ["static", "owned", "sig", "sig", "memo 2", "fn 2", "macro 2"]
    );
    let (name, n) = t.root_scope().expect_context::<(Signal<String>, Signal<i32>)>();
    name.set("new".into());
    n.set(3);
    t.run_until_idle();
    assert_eq!(
        texts(&t),
        ["static", "owned", "new", "new", "memo 3", "fn 3", "macro 3"]
    );
}

#[test]
fn model_two_way_no_loop() {
    let mut t = TestUi::new(200, 80).mount(|cx| {
        let on = cx.signal(false);
        cx.provide(on);
        button(label("toggle")).checkable(true).checked(on).test_id("b")
    });
    t.run_until_idle();
    let on = t.root_scope().expect_context::<Signal<bool>>();
    let runs = debug_stats().effect_runs;
    t.find(by_id("b")).click();
    t.run_until_idle();
    assert!(on.get_untracked(), "the click wrote back");
    assert!(t.find(by_id("b")).state().contains(State::CHECKED));
    let n = debug_stats().effect_runs - runs;
    assert_eq!(n, 1, "the display binding runs once and changes nothing");
    assert_eq!(debug_stats().loop_cuts, 0);
    // Signal → widget.
    on.set(false);
    t.run_until_idle();
    assert!(!t.find(by_id("b")).state().contains(State::CHECKED));
}

#[test]
fn model_owned_reports_via_on_change() {
    let seen: Rc<Cell<Option<bool>>> = Rc::default();
    let s = seen.clone();
    let mut t = TestUi::new(200, 80).mount(move |_cx| {
        button(label("toggle"))
            .checkable(true)
            .checked(true)
            .on_change(move |v| s.set(Some(v)))
            .test_id("b")
    });
    t.run_until_idle();
    assert!(t.find(by_id("b")).state().contains(State::CHECKED));
    t.find(by_id("b")).click();
    t.run_until_idle();
    assert_eq!(seen.get(), Some(false));
}
