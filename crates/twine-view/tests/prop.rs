//! Property values and bindings.

use twine_engine::{EventCode, EventParam};
use twine_reactive::Runtime;
use twine_style::{Part, PropId};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::label::Label;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

/// Accepts any property value of type `T` (the forms are told apart by the inferred marker `M`).
fn take<T: 'static, M>(p: impl IntoProp<T, M>) -> Prop<T> {
    p.into_prop()
}

fn is_static<T>(p: &Prop<T>) -> bool {
    matches!(p, Prop::Static(_))
}

#[test]
fn prop_coherence() {
    let cx = rt().create_root();
    let s = cx.signal(1i32);
    let m = cx.memo(move || s.get() * 2);
    assert!(is_static(&take::<i32, _>(5)));
    assert!(is_static(&take::<bool, _>(true)));
    assert!(is_static(&take::<Color, _>(Color::RED)));
    assert!(is_static(&take::<Opa, _>(Opa::COVER)));
    assert!(is_static(&take::<Length, _>(Length::Pct(50))));
    assert!(is_static(&take::<&str, _>("text")));
    assert!(is_static(&take::<String, _>(String::from("owned"))));
    assert!(is_static(&take::<String, _>("converted")));
    assert!(is_static(&take::<Option<u8>, _>(Some(3u8))));
    assert!(!is_static(&take::<i32, _>(s)));
    assert!(!is_static(&take::<i32, _>(s.read_only())));
    assert!(!is_static(&take::<i32, _>(m)));
    assert!(!is_static(&take::<i32, _>(move || s.get() + 1)));
    assert!(!is_static(&take::<Color, _>(move || Color::BLUE)));
    let Prop::Dynamic(f) = take::<i32, _>(move || s.get() * 10) else {
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
        before.set(rt().stats().nodes);
        let _ = cx;
        label("x").text_color(Color::RED).padding(3).test_id("l")
    });
    // A dynamic text would have created an effect; constants create none.
    assert_eq!(rt().stats().nodes, before.get());
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
    let runs = rt().stats().effect_runs;
    t.run_until_idle();
    assert_eq!(rt().stats().effect_runs - runs, 1, "only a's binding runs");
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
    let nodes = rt().stats().nodes;
    t.engine_mut().delete(id).unwrap();
    n.set(5);
    t.run_until_idle();
    assert_eq!(rt().stats().nodes, nodes - 1, "the binding disposed itself");
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
    assert!(rt().has_pending_effects());
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

/// A user property type: no declaration, it is a property of its own type; signals, memos
/// and closures of it work like for the built-in types.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Gauge(u8);

/// A user model type: no declaration either.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Level(u8);

fn take_model<T: 'static>(m: impl IntoModel<T>) -> Model<T> {
    m.into_model()
}

#[test]
fn user_type_as_prop_without_declaration() {
    let cx = rt().create_root();
    let g = cx.signal(Gauge(1));
    assert!(matches!(take::<Gauge, _>(Gauge(3)), Prop::Static(Gauge(3))));
    assert!(is_static(&take::<Vec<Gauge>, _>(vec![Gauge(1), Gauge(2)])));
    assert!(is_static(&take::<Option<Gauge>, _>(Some(Gauge(1)))));
    assert!(is_static(&take::<Option<Gauge>, _>(Gauge(1)))); // `From<T> for Option<T>`
    assert!(is_static(&take::<(Gauge, i32), _>((Gauge(1), 2))));
    assert!(!is_static(&take::<Gauge, _>(g)));
    assert!(!is_static(&take::<Gauge, _>(move || Gauge(g.get().0 + 1))));
    // Bound to a widget through `bind`: constants apply once, signals re-run.
    cx.dispose();
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let g = cx.signal(Gauge(10));
        cx.provide(g);
        label("x")
            .bind(g, |l: &mut Label, wcx, g: Gauge| {
                l.set_max_lines(wcx, u16::from(g.0));
            })
            .bind(Gauge(2), |l: &mut Label, wcx, g: Gauge| {
                l.set_long_mode(wcx, if g.0 == 2 { LongMode::Dots } else { LongMode::Wrap });
            })
            .test_id("l")
    });
    t.run_until_idle();
    let g = t.root_scope().expect_context::<Signal<Gauge>>();
    let l = t.find(by_id("l")).id();
    assert_eq!(t.engine().widget::<Label>(l).unwrap().max_lines(), 10);
    assert_eq!(t.engine().widget::<Label>(l).unwrap().long_mode(), LongMode::Dots);
    g.set(Gauge(4));
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Label>(l).unwrap().max_lines(), 4);
}

#[test]
fn user_type_as_model_without_declaration() {
    let cx = rt().create_root();
    let s = cx.signal(Level(1));
    assert!(matches!(take_model(Level(2)), Model::Owned(Level(2))));
    assert!(matches!(take_model::<Level>(s), Model::Bound(_)));
    assert!(matches!(take_model(Some(Level(1))), Model::Owned(Some(Level(1)))));
    assert!(matches!(take_model((Level(1), 5u8)), Model::Owned(_)));
    // Integer literals infer for every integer type (no conversion in models).
    assert!(matches!(take_model::<usize>(3), Model::Owned(3)));
    assert!(matches!(take_model::<u32>(7), Model::Owned(7)));
    cx.dispose();
}

#[test]
fn constant_props_create_no_binding() {
    // Every constant of the audited setters stays `Prop::Static`: building runs no effect.
    let runs = rt().stats().effect_runs;
    let mut t = TestUi::new(320, 240).mount(|_| {
        column((
            label("a").selectable(true).max_lines(2),
            arc(10).knob(false).rotation(Angle::deg(90)),
            bar(5).animated(Duration::ms(200)),
            dropdown(["a", "b"], 0usize).text("Pick"),
            roller(["x", "y"], 0usize).mode(RollerMode::Infinite),
            buttonmatrix([[btn("1"), btn("2")]]),
            list_button((), "row").grid_col(0).grid_row(0..1),
            window("w", window_button(Symbol::Close, 30), label("c")).content_padding(0),
        ))
    });
    t.run_until_idle();
    assert_eq!(rt().stats().effect_runs, runs, "no binding for constants");
}
