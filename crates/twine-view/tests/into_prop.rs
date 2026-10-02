//! `IntoProp<T, M>` (R1 review rework F2): every value converting into `T` is a property, as a
//! constant, a closure result or a signal/memo value, with no per-type declaration; the same
//! marker pattern for `IntoText` and `IntoOptions`; constants allocate nothing and dynamic
//! values are one boxed closure.

use twine_engine::NodeId;
use twine_style::design::ColorValue;
use twine_style::{GridSpan, StyleValue};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, capture_logs};
use twine_view::TextProp;
use twine_view::prelude::*;
use twine_widgets::label::Label;
use twine_widgets::spinbox::Spinbox;
use twine_widgets::textarea::Textarea;
use twine_widgets_ext::dropdown::Dropdown;
use twine_widgets_ext::roller::Roller;

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn take<T: 'static, M>(p: impl IntoProp<T, M>) -> Prop<T> {
    p.into_prop()
}

fn value<T: Clone>(p: &Prop<T>) -> T {
    match p {
        Prop::Static(v) => v.clone(),
        Prop::Dynamic(f) => f(),
    }
}

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn prop(t: &TestUi, id: &'static str, p: PropId) -> StyleValue {
    t.engine().style_prop(node(t, id), Part::Main, p)
}

/// A user type converting into a Twine property type through `From` only.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Brand {
    Primary,
    Danger,
}

impl From<Brand> for Color {
    fn from(b: Brand) -> Color {
        match b {
            Brand::Primary => Color::BLUE,
            Brand::Danger => Color::RED,
        }
    }
}

/// Color properties store a `ColorValue` (a color or a design element, R2.S02): a user type
/// is a color property through `From<_> for ColorValue` (conversions do not chain).
impl From<Brand> for ColorValue {
    fn from(b: Brand) -> ColorValue {
        Color::from(b).into()
    }
}

#[test]
fn user_type_converts_through_from_only() {
    let cx = rt().create_root();
    let s = cx.signal(Brand::Primary);
    let m = cx.memo(move || s.get());
    assert!(matches!(
        take::<Color, _>(Brand::Danger),
        Prop::Static(Color::RED)
    ));
    assert_eq!(value(&take::<Color, _>(s)), Color::BLUE);
    assert_eq!(value(&take::<Color, _>(s.read_only())), Color::BLUE);
    assert_eq!(value(&take::<Color, _>(m)), Color::BLUE);
    assert_eq!(value(&take::<Color, _>(move || s.get())), Color::BLUE);
    cx.dispose();

    let mut t = TestUi::new(200, 100).mount(|cx| {
        let s = cx.signal(Brand::Primary);
        cx.provide(s);
        column((
            label("a").text_color(Brand::Danger).test_id("const"),
            label("b").text_color(s).test_id("signal"),
        ))
    });
    t.run_until_idle();
    assert_eq!(
        prop(&t, "const", PropId::TextColor),
        StyleValue::Color(Color::RED)
    );
    assert_eq!(
        prop(&t, "signal", PropId::TextColor),
        StyleValue::Color(Color::BLUE)
    );
    t.root_scope()
        .expect_context::<Signal<Brand>>()
        .set(Brand::Danger);
    t.run_until_idle();
    assert_eq!(
        prop(&t, "signal", PropId::TextColor),
        StyleValue::Color(Color::RED)
    );
}

#[test]
fn integer_literals_infer() {
    // `i32` and the types converting from it (`Length`, `Radius`, `GridSpan`), `u8` (the only
    // integer type converting into `u8`), also as closure results.
    assert!(matches!(take::<Length, _>(4), Prop::Static(Length::Px(4))));
    assert!(matches!(take::<i32, _>(-3), Prop::Static(-3)));
    assert!(matches!(take::<u8, _>(200), Prop::Static(200)));
    assert!(matches!(take::<i64, _>(7), Prop::Static(7)));
    assert_eq!(value(&take::<Length, _>(|| 8)), Length::Px(8));
    assert_eq!(value(&take::<u8, _>(|| 9)), 9);
    // Options: `Some(T)` and `T` itself (`From<T> for Option<T>`), and `None`.
    assert!(matches!(
        take::<Option<Length>, _>(Some(Length::Px(5))),
        Prop::Static(Some(_))
    ));
    assert!(matches!(
        take::<Option<Length>, _>(Length::Px(5)),
        Prop::Static(Some(_))
    ));
    assert!(matches!(take::<Option<Length>, _>(None), Prop::Static(None)));

    let t = TestUi::new(320, 240).mount(|_| {
        column((
            label("a")
                .padding(4)
                .radius(3)
                .flex_grow(2)
                .max_lines(3)
                .test_id("label"),
            roller(["a", "b", "c", "d"], 0).visible_rows(3).test_id("roller"),
            spinbox(5).digits(3, 1).step(10).test_id("spin"),
            textarea(String::new()).max_length(24).test_id("ta"),
        ))
    });
    assert_eq!(
        prop(&t, "label", PropId::PaddingTop),
        StyleValue::Length(Length::Px(4))
    );
    assert_eq!(prop(&t, "label", PropId::FlexGrow), StyleValue::Int(2));
    let e = t.engine();
    assert_eq!(e.widget::<Label>(node(&t, "label")).unwrap().max_lines(), 3);
    assert_eq!(
        e.widget::<Roller>(node(&t, "roller"))
            .unwrap()
            .visible_row_count(),
        3
    );
    let spin = e.widget::<Spinbox>(node(&t, "spin")).unwrap();
    assert_eq!((spin.digit_count(), spin.step()), (3, 10));
    assert_eq!(e.widget::<Textarea>(node(&t, "ta")).unwrap().max_length(), 24);
}

#[test]
fn count_setters_clamp_and_warn_out_of_range_values() {
    let (mut t, logs) = capture_logs(|| {
        let mut t = TestUi::new(320, 240).mount(|cx| {
            let lines = cx.signal(2);
            cx.provide(lines);
            column((
                label("a").max_lines(-1).test_id("neg"),
                label("b").max_lines(1 << 20).test_id("big"),
                label("c").max_lines(lines).test_id("dyn"),
                spinbox(5).step(-4).test_id("spin"),
                textarea(String::new()).max_length(-1).test_id("ta"),
                row(label("d").flex_grow(-3).test_id("grow")).size(200, 40),
            ))
        });
        t.run_until_idle();
        t
    });
    let e = t.engine();
    assert_eq!(e.widget::<Label>(node(&t, "neg")).unwrap().max_lines(), 0);
    assert_eq!(e.widget::<Label>(node(&t, "big")).unwrap().max_lines(), u16::MAX);
    assert_eq!(e.widget::<Label>(node(&t, "dyn")).unwrap().max_lines(), 2);
    assert_eq!(e.widget::<Spinbox>(node(&t, "spin")).unwrap().step(), 1);
    assert_eq!(e.widget::<Textarea>(node(&t, "ta")).unwrap().max_length(), 0);
    drop(e);
    // A negative weight is no grow: the label keeps its own width.
    assert!(t.engine().coords(node(&t, "grow")).width() < 200);
    let warned = |target: &str, msg: &str| {
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.target == target && l.message == msg)
    };
    assert!(
        warned("twine::view", "max_lines was -1, setting it to 0"),
        "{logs:?}"
    );
    assert!(
        warned("twine::view", "max_lines was 1048576, setting it to 65535"),
        "{logs:?}"
    );
    assert!(warned("twine::view", "step was -4, setting it to 1"), "{logs:?}");
    assert!(
        warned("twine::view", "max_length was -1, setting it to 0"),
        "{logs:?}"
    );
    assert!(
        warned("twine::layout", "flex_grow was -3, setting it to 0"),
        "{logs:?}"
    );
    // In-range values (also dynamic ones) warn nothing.
    assert!(!logs.iter().any(|l| l.message.contains("was 2")), "{logs:?}");
    let ((), logs) = capture_logs(|| {
        t.root_scope().expect_context::<Signal<i32>>().set(-5);
        t.run_until_idle();
    });
    assert_eq!(
        t.engine().widget::<Label>(node(&t, "dyn")).unwrap().max_lines(),
        0
    );
    assert!(
        logs.iter()
            .any(|l| l.message == "max_lines was -5, setting it to 0"),
        "a dynamic value is checked on every run: {logs:?}"
    );
}

#[test]
fn signals_of_convertible_types() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let w = cx.signal(40i32); // `i32` into `Length`
        let v = cx.signal(7u8); // `u8` into `i32`
        let d = cx.signal(Duration::ms(250)); // `Duration` into `DurationMs`
        cx.provide((w, v));
        column((
            label("a")
                .width(w)
                .height(cx.memo(move || w.get() / 2))
                .test_id("l"),
            bar(v).anim_duration(d).test_id("bar"),
        ))
    });
    t.run_until_idle();
    assert_eq!(prop(&t, "l", PropId::Width), StyleValue::Length(Length::Px(40)));
    assert_eq!(prop(&t, "l", PropId::Height), StyleValue::Length(Length::Px(20)));
    let (w, v) = t.root_scope().expect_context::<(Signal<i32>, Signal<u8>)>();
    w.set(60);
    v.set(9);
    t.run_until_idle();
    assert_eq!(prop(&t, "l", PropId::Width), StyleValue::Length(Length::Px(60)));
    assert_eq!(prop(&t, "l", PropId::Height), StyleValue::Length(Length::Px(30)));
    let bar = node(&t, "bar");
    assert_eq!(
        t.engine().widget::<twine_widgets::bar::Bar>(bar).unwrap().value(),
        9
    );
}

#[test]
fn grid_spans_from_integers_ranges_and_signals() {
    assert_eq!(value(&take::<GridSpan, _>(1)), GridSpan::cell(1));
    assert_eq!(value(&take::<GridSpan, _>(0..2)), GridSpan::new(0, 2));
    assert_eq!(value(&take::<GridSpan, _>(1..=2)), GridSpan::new(1, 2));
    assert_eq!(value(&take::<GridSpan, _>(3u8)), GridSpan::cell(3));
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let n = cx.signal(1i32);
        cx.provide(n);
        grid(
            grid_tracks![fr(1), fr(1), fr(1)],
            grid_tracks![px(20), px(20)],
            (
                label("a").grid_col(0..2).grid_row(1).test_id("range"),
                label("b").grid_col(n).grid_row(0..=1).test_id("signal"),
                label("c")
                    .grid_col(move || 0..n.get())
                    .grid_row(0)
                    .test_id("closure"),
            ),
        )
        .size(300, 100)
    });
    t.run_until_idle();
    let cell = |t: &TestUi, id| {
        (
            prop(t, id, PropId::GridCellColumn),
            prop(t, id, PropId::GridCellColumnSpan),
        )
    };
    assert_eq!(cell(&t, "range"), (StyleValue::Int(0), StyleValue::Int(2)));
    assert_eq!(cell(&t, "signal"), (StyleValue::Int(1), StyleValue::Int(1)));
    assert_eq!(cell(&t, "closure"), (StyleValue::Int(0), StyleValue::Int(1)));
    assert_eq!(prop(&t, "signal", PropId::GridCellRowSpan), StyleValue::Int(2));
    t.root_scope().expect_context::<Signal<i32>>().set(2);
    t.run_until_idle();
    assert_eq!(cell(&t, "signal"), (StyleValue::Int(2), StyleValue::Int(1)));
    assert_eq!(cell(&t, "closure"), (StyleValue::Int(0), StyleValue::Int(2)));
}

#[test]
fn option_lists_from_any_iterable_and_reactive_lists() {
    static CITIES: &[&str] = &["Oslo", "Lima"];
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let n = cx.signal(3usize);
        let names = cx.signal(vec!["x", "y"]); // a `Vec<&'static str>` signal
        cx.provide(n);
        column((
            dropdown(CITIES, 0).test_id("slice"),
            dropdown(vec![String::from("a"), String::from("b")], 0).test_id("vec"),
            roller((1..=6).filter(|i| i % 2 == 0).map(|i| format!("{i} h")), 0).test_id("iter"),
            roller(names, 0).test_id("signal"),
            roller(move || (0..n.get()).map(|i| i.to_string()).collect::<Vec<_>>(), 0).test_id("closure"),
        ))
    });
    t.run_until_idle();
    let dd = |t: &TestUi, id| {
        t.engine()
            .widget::<Dropdown>(node(t, id))
            .unwrap()
            .options()
            .to_string()
    };
    let ro = |t: &TestUi, id| {
        t.engine()
            .widget::<Roller>(node(t, id))
            .unwrap()
            .options()
            .to_string()
    };
    assert_eq!(dd(&t, "slice"), "Oslo\nLima");
    assert_eq!(dd(&t, "vec"), "a\nb");
    assert_eq!(ro(&t, "iter"), "2 h\n4 h\n6 h");
    assert_eq!(ro(&t, "signal"), "x\ny");
    assert_eq!(ro(&t, "closure"), "0\n1\n2");
    t.root_scope().expect_context::<Signal<usize>>().set(2);
    t.run_until_idle();
    assert_eq!(ro(&t, "closure"), "0\n1");
}

#[test]
fn texts_from_static_closures_signals_and_references() {
    let cx = rt().create_root();
    let on = cx.signal(true);
    let word = cx.signal("Hi");
    let owned = String::from("copied");
    // A closure choosing a `'static` text is stored without copying.
    assert!(matches!(
        (move || if on.get() { "On" } else { "Off" }).into_text(),
        TextProp::StaticFn(_)
    ));
    assert!(matches!((move || Symbol::Ok).into_text(), TextProp::StaticFn(_)));
    assert!(matches!(word.into_text(), TextProp::Write(_)));
    assert!(matches!((&owned).into_text(), TextProp::Owned(ref s) if s == "copied"));
    cx.dispose();

    let mut t = TestUi::new(200, 100).mount(|cx| {
        let on = cx.signal(true);
        let word = cx.signal("Hi");
        cx.provide((on, word));
        column((
            label(move || if on.get() { "On" } else { "Off" }).test_id("static_fn"),
            label(word).test_id("signal"),
        ))
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("static_fn")).text(), "On");
    assert_eq!(t.find(by_id("signal")).text(), "Hi");
    let (on, word) = t
        .root_scope()
        .expect_context::<(Signal<bool>, Signal<&'static str>)>();
    on.set(false);
    word.set("Bye");
    t.run_until_idle();
    assert_eq!(t.find(by_id("static_fn")).text(), "Off");
    assert_eq!(t.find(by_id("signal")).text(), "Bye");
}

#[test]
fn constants_allocate_nothing_and_dynamic_values_one_box() {
    let cx = rt().create_root();
    let s = cx.signal(3i32);
    let ((), stats) = count_allocs(|| {
        assert!(matches!(take::<Length, _>(4), Prop::Static(_)));
        assert!(matches!(take::<Color, _>(Brand::Primary), Prop::Static(_)));
        assert!(matches!(take::<GridSpan, _>(0..2), Prop::Static(_)));
        assert!(matches!(take::<Icon, _>(Symbol::Ok), Prop::Static(_)));
        assert!(matches!(take::<Icon, _>(()), Prop::Static(Icon(None))));
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
    for (name, make) in [
        (
            "signal",
            Box::new(move || take::<Length, _>(s)) as Box<dyn Fn() -> Prop<Length>>,
        ),
        (
            "closure",
            Box::new(move || take::<Length, _>(move || s.get() * 2)),
        ),
        ("read signal", Box::new(move || take::<Length, _>(s.read_only()))),
    ] {
        let (p, stats) = count_allocs(&*make);
        assert!(matches!(p, Prop::Dynamic(_)), "{name}");
        assert_eq!(stats.allocs, 1, "{name}: one boxed closure, {stats:?}");
        let ((), stats) =
            count_allocs(|| assert_eq!(value(&p), Length::Px(if name == "closure" { 6 } else { 3 })));
        assert_eq!(stats.allocs, 0, "{name}: reading converts in place, {stats:?}");
    }
    cx.dispose();
}
