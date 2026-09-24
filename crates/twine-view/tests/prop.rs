//! Property values and bindings.

use twine_engine::{EventCode, EventParam};
use twine_reactive::debug_stats;
use twine_style::{Part, PropId};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::label::Label;

/// Accepts any property value (the coherence check of `IntoProp`: constants, signals, memos
/// and closures of the same `T` all work).
fn take<T: 'static>(p: impl IntoProp<T>) -> Prop<T> {
    p.into_prop()
}

fn is_static<T>(p: &Prop<T>) -> bool {
    matches!(p, Prop::Static(_))
}

#[test]
fn prop_coherence() {
    let cx = twine_reactive::create_root();
    let s = cx.signal(1i32);
    let m = cx.memo(move || s.get() * 2);
    assert!(is_static(&take(5i32)));
    assert!(is_static(&take(true)));
    assert!(is_static(&take(Color::RED)));
    assert!(is_static(&take(Opa::COVER)));
    assert!(is_static(&take(Length::Pct(50))));
    assert!(is_static(&take("text")));
    assert!(is_static(&take(String::from("owned"))));
    assert!(is_static(&take(Some(3u8))));
    assert!(!is_static(&take(s)));
    assert!(!is_static(&take(s.read_only())));
    assert!(!is_static(&take(m)));
    assert!(!is_static(&take(move || s.get() + 1)));
    assert!(!is_static(&take(move || Color::BLUE)));
    let Prop::Dynamic(f) = take::<i32>(move || s.get() * 10) else {
        unreachable!()
    };
    s.set(4);
    assert_eq!(f(), 40);
    cx.dispose();
}

#[test]
fn static_prop_applies_once_no_effect_created() {
    let before = std::cell::Cell::new(0);
    let t = TestUi::new(100, 60).mount(|cx| {
        before.set(debug_stats().nodes);
        let _ = cx;
        label("x").text_color(Color::RED).padding(3).test_id("l")
    });
    // A dynamic text would have created an effect; constants create none.
    assert_eq!(debug_stats().nodes, before.get());
    let id = t.find(by_id("l")).id();
    assert_eq!(
        t.engine().style_color(id, Part::Main, PropId::TextColor),
        Color::RED
    );
}

#[test]
fn dynamic_prop_updates_on_signal_change() {
    let mut t = TestUi::new(100, 60).mount(|cx| {
        let red = cx.signal(true);
        cx.provide(red);
        label("x")
            .text_color(move || if red.get() { Color::RED } else { Color::BLUE })
            .test_id("l")
    });
    t.run_until_idle();
    let id = t.find(by_id("l")).id();
    assert_eq!(
        t.engine().style_color(id, Part::Main, PropId::TextColor),
        Color::RED
    );
    let red = t.root_scope().expect_context::<Signal<bool>>();
    red.set(false);
    t.run_until_idle();
    assert_eq!(
        t.engine().style_color(id, Part::Main, PropId::TextColor),
        Color::BLUE
    );
}

#[test]
fn binding_runs_only_for_its_signal() {
    let mut t = TestUi::new(200, 60).mount(|cx| {
        let a = cx.signal(1);
        let b = cx.signal(2);
        cx.provide((a, b));
        row((label(text!("{}", a.get())), label(text!("{}", b.get()))))
    });
    t.run_until_idle();
    let (a, _b) = t.root_scope().expect_context::<(Signal<i32>, Signal<i32>)>();
    a.set(10); // outside the Ui: the binding defers itself to the next update
    let runs = debug_stats().effect_runs;
    t.run_until_idle();
    assert_eq!(debug_stats().effect_runs - runs, 1, "only a's binding runs");
}

#[test]
fn binding_disposes_when_node_deleted() {
    let mut t = TestUi::new(100, 60).mount(|cx| {
        let n = cx.signal(0);
        cx.provide(n);
        label(text!("{}", n.get())).test_id("l")
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    let id = t.find(by_id("l")).id();
    let nodes = debug_stats().nodes;
    t.engine_mut().delete(id).unwrap();
    n.set(5);
    t.run_until_idle();
    assert_eq!(debug_stats().nodes, nodes - 1, "the binding disposed itself");
    n.set(6);
    t.run_until_idle();
}

#[test]
fn binding_outside_update_is_deferred() {
    let mut t = TestUi::new(100, 60).mount(|cx| {
        let n = cx.signal(0);
        cx.provide(n);
        label(text!("{}", n.get())).test_id("l")
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    n.set(7); // no engine here: the binding waits
    let id = t.find(by_id("l")).id();
    assert_eq!(t.engine().widget::<Label>(id).unwrap().text(), "0");
    assert!(twine_reactive::has_pending_effects());
    t.update();
    assert_eq!(t.engine().widget::<Label>(id).unwrap().text(), "7");
    t.run_until_idle();
}

#[test]
fn handler_writes_run_bindings_in_the_same_update() {
    let mut t = TestUi::new(100, 60).mount(|cx| {
        let n = cx.signal(0);
        label(text!("{}", n.get()))
            .clickable(true)
            .on_click(move || n.update(|v| *v += 1))
            .test_id("l")
    });
    t.run_until_idle();
    let id = t.find(by_id("l")).id();
    t.engine_mut()
        .send_event(id, EventCode::Clicked, EventParam::None);
    // Sent outside an update: the binding is deferred, then applied by the next update.
    t.update();
    assert_eq!(t.find(by_id("l")).text(), "1");
}
