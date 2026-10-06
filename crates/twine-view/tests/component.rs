//! `#[component]` (R4.S01): required props as arguments, optional props with defaults as
//! methods, reactive props, children, the build scope, callbacks, modifiers on the root, and the
//! cost of the generated builder against the same view written by hand.

use core::ops::RangeInclusive;
use std::cell::Cell;
use std::rc::Rc;

use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::bar::Bar;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A captioned meter: one required prop, two optional ones.
#[component]
fn meter(
    /// The value shown.
    value: Prop<i32>,
    /// The scale.
    #[prop(default = 0..=100)]
    range: Prop<RangeInclusive<i32>>,
    /// The caption above the bar.
    #[prop(default)]
    caption: TextProp,
) -> impl ViewExt<Widget = Obj> {
    column((
        label(caption).test_id("caption"),
        bar(value).range(range).test_id("bar"),
    ))
}

fn bar_value(t: &TestUi) -> i32 {
    let id = t.find(by_id("bar")).id();
    t.engine().widget::<Bar>(id).unwrap().value()
}

#[test]
fn required_and_default_props() {
    let mut t = TestUi::new(200, 100).mount(|_| meter(150));
    t.run_until_idle();
    assert_eq!(bar_value(&t), 100, "clamped into the default range 0..=100");
    assert_eq!(t.find(by_id("caption")).text(), "", "`#[prop(default)]` text");
}

#[test]
fn optional_props_are_methods() {
    let mut t = TestUi::new(200, 100).mount(|_| meter(150).range(0..=200).caption("rpm"));
    t.run_until_idle();
    assert_eq!(bar_value(&t), 150);
    assert_eq!(t.find(by_id("caption")).text(), "rpm");
}

#[test]
fn props_are_reactive() {
    let v: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
    let v2 = v.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let n = cx.signal(10);
        v2.set(Some(n));
        let max = cx.signal(100);
        meter(n)
            .range(move || 0..=max.get())
            .caption(text!("{} of {}", n.get(), max.get()))
    });
    t.run_until_idle();
    assert_eq!(bar_value(&t), 10);
    assert_eq!(t.find(by_id("caption")).text(), "10 of 100");
    let n = v.get().unwrap();
    n.set(42);
    t.run_until_idle();
    assert_eq!(bar_value(&t), 42);
    assert_eq!(t.find(by_id("caption")).text(), "42 of 100");
    // An unchanged value does nothing (the bindings call idempotent setters).
    n.set(42);
    t.assert_idle();
}

#[test]
fn modifiers_apply_to_the_root() {
    let r: Rc<Cell<Option<NodeRef<Obj>>>> = Rc::default();
    let r2 = r.clone();
    let clicks = Rc::new(Cell::new(0));
    let c2 = clicks.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let nr = cx.node_ref();
        r2.set(Some(nr));
        meter(5)
            .padding(7)
            .bg(Color::RED)
            .clickable(true)
            .on_click(move || c2.set(c2.get() + 1))
            .node_ref(nr)
            .test_id("meter")
    });
    t.run_until_idle();
    let root = t.find(by_id("meter")).id();
    assert_eq!(
        r.get().unwrap().get(),
        Some(root),
        "`node_ref` is the root, typed `Obj`"
    );
    let bar = t.find(by_id("bar")).id();
    {
        let e = t.engine();
        assert_eq!(e.style_i32(root, Part::Main, PropId::PaddingTop), 7);
        assert_eq!(e.style_color(root, Part::Main, PropId::BgColor), Color::RED);
        // The root is the column: the label and the bar are its children.
        assert_eq!(e.tree().parent(bar), Some(root));
    }
    t.find(by_id("meter")).click();
    t.run_until_idle();
    assert_eq!(clicks.get(), 1);
}

/// A card with required children (`impl ViewSeq`: a type parameter, not boxed).
#[component]
fn titled(
    title: TextProp,
    content: impl ViewSeq,
    #[prop(default = Color::BLUE)] title_color: Prop<twine_style::design::ColorValue>,
) -> impl ViewExt<Widget = Card> {
    card((label(title).text_color(title_color).test_id("title"), content))
}

#[test]
fn children_and_color_props() {
    let mut t = TestUi::new(200, 100).mount(|_| {
        row((
            titled("A", (label("a1").test_id("a1"), label("a2"))).test_id("ca"),
            titled("B", label("b1")).title_color(Color::GREEN),
        ))
    });
    t.run_until_idle();
    let ca = t.find(by_id("ca")).id();
    let a1 = t.find(by_id("a1")).id();
    let e = t.engine();
    assert_eq!(e.tree().parent(a1), Some(ca));
    assert_eq!(e.tree().node(ca).unwrap().child_count(), 3);
    let titles = t.find_all(by_id("title"));
    assert_eq!(
        e.style_color(titles[0].id(), Part::Main, PropId::TextColor),
        Color::BLUE
    );
    assert_eq!(
        e.style_color(titles[1].id(), Part::Main, PropId::TextColor),
        Color::GREEN
    );
}

/// A component with its own state (the build scope), a plain prop, an `into` prop, a callback
/// and an optional callback.
#[component]
fn stepper(
    cx: Scope,
    #[prop(default = 1)] step: i32,
    #[prop(default, into)] name: String,
    #[prop(default)] on_step: Option<Box<dyn FnMut(i32)>>,
) -> impl ViewExt<Widget = Button> {
    let n = cx.signal(0);
    let mut on_step = on_step;
    button(label(text!("{} {}", name, n.get())).test_id("count")).on_click(move || {
        n.update(|n| *n += step);
        if let Some(f) = on_step.as_mut() {
            f(n.get_untracked());
        }
    })
}

#[test]
fn scope_plain_into_and_callback_props() {
    let seen = Rc::new(Cell::new(0));
    let s2 = seen.clone();
    let mut t = TestUi::new(200, 100).mount(move |_| {
        stepper()
            .step(5)
            .name("n")
            .on_step(move |v| s2.set(v))
            .test_id("s")
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "n 0");
    t.find(by_id("s")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "n 5");
    assert_eq!(seen.get(), 5);
}

/// Reports when the scope it was built in is disposed.
#[component]
fn watched(cx: Scope, disposed: Rc<Cell<bool>>) -> impl View {
    cx.on_cleanup(move || disposed.set(true));
    label("watched")
}

/// The scope a component receives is the one it is built in: disposed with its `when` branch.
#[test]
fn component_scope_is_the_build_scope() {
    let show: Rc<Cell<Option<Signal<bool>>>> = Rc::default();
    let s2 = show.clone();
    let disposed = Rc::new(Cell::new(false));
    let d2 = disposed.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let on = cx.signal(true);
        s2.set(Some(on));
        column(when(move || on.get(), move |_| watched(d2.clone())))
    });
    t.run_until_idle();
    assert!(!disposed.get());
    show.get().unwrap().set(false);
    t.run_until_idle();
    assert!(
        disposed.get(),
        "the component's scope is disposed with the branch"
    );
}

/// A required `Box<dyn Fn>` prop, a generic prop, and a `View`-only component.
#[component]
fn picker<T: Clone + Into<i32> + 'static>(items: Vec<T>, on_pick: Box<dyn Fn(i32)>) -> impl View {
    let first = items.first().cloned().map_or(0, Into::into);
    button(label("pick").test_id("pick")).on_click(move || on_pick(first))
}

#[test]
fn boxed_generic_and_view_only() {
    let got = Rc::new(Cell::new(0));
    let g2 = got.clone();
    let mut t = TestUi::new(200, 100).mount(move |_| picker(vec![7u8, 9], move |v| g2.set(v)));
    t.run_until_idle();
    t.find(by_id("pick")).click();
    t.run_until_idle();
    assert_eq!(got.get(), 7);
}

/// `meter` written by hand, for the cost comparison.
fn meter_by_hand<M>(value: impl IntoProp<i32, M>) -> impl ViewExt<Widget = Obj> {
    column((
        label("").test_id("caption"),
        bar(value).range(0..=100).test_id("bar"),
    ))
}

/// A harness whose mount does not lease from the process-wide waker pool: the pool is shared
/// with the tests running in parallel, so a pooled mount allocates a heap waker or not
/// depending on how many of their UIs are alive (see `TestUi::with_waker`).
fn harness(waker: &'static UiWaker) -> TestUi {
    TestUi::new(200, 100).with_waker(waker)
}

/// The generated builder costs what the hand-written function does: creating it allocates
/// nothing for constant props, and building it allocates as much as the same view written by
/// hand (no modifiers: the modifier list stays empty).
#[test]
fn builder_is_zero_cost() {
    static WAKER: UiWaker = UiWaker::new();
    let ((), created) = count_allocs(|| {
        let v = meter(3).range(0..=10).caption("x");
        core::mem::forget(v);
    });
    assert_eq!(created.allocs, 0, "constant props allocate nothing");

    // Warm-up: one-time initialisation (thread-locals, logger) is not counted.
    drop(harness(&WAKER).mount(|_| meter_by_hand(3)));
    let (_, generated) = count_allocs(|| harness(&WAKER).mount(|_| meter(3).range(0..=100).caption("")));
    let (_, by_hand) = count_allocs(|| harness(&WAKER).mount(|_| meter_by_hand(3)));
    assert_eq!(generated.allocs, by_hand.allocs);
    assert_eq!(generated.bytes, by_hand.bytes);
}

/// With modifiers, the generated builder keeps them in one `Vec` until the body's view exists:
/// one allocation more than the same modifiers written on the view by hand, at build time.
#[test]
fn modifiers_cost_one_vec() {
    static WAKER: UiWaker = UiWaker::new();
    drop(harness(&WAKER).mount(|_| meter_by_hand(3).padding(4)));
    let (_, generated) =
        count_allocs(|| harness(&WAKER).mount(|_| meter(3).caption("").padding(4).bg(Color::RED)));
    let (_, by_hand) = count_allocs(|| harness(&WAKER).mount(|_| meter_by_hand(3).padding(4).bg(Color::RED)));
    assert_eq!(generated.allocs, by_hand.allocs + 1);
}

#[test]
fn prop_get_and_defaults() {
    assert_eq!(Prop::<i32>::default().get(), 0);
    assert!(matches!(TextProp::default(), TextProp::Static("")));
    assert!(matches!(Model::<u8>::default(), Model::Owned(0)));
    let p: Prop<i32> = (|| 4).into_prop();
    assert_eq!(p.get(), 4);
}

#[test]
fn view_event_codes_reach_the_component_root() {
    let pressed = Rc::new(Cell::new(false));
    let p2 = pressed.clone();
    let mut t = TestUi::new(200, 100).mount(move |_| {
        meter(1)
            .clickable(true)
            .on_event(EventCode::Pressed, move |_| p2.set(true))
            .test_id("m")
    });
    t.run_until_idle();
    t.find(by_id("m")).click();
    t.run_until_idle();
    assert!(pressed.get());
}
