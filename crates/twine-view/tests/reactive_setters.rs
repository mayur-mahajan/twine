//! Every widget setter accepts a signal (`IntoProp` / `IntoText` everywhere): one test per
//! widget view file, each binding every setter of its views to signals and checking that
//! changes reach the widgets.

use twine_engine::{NodeId, ObjFlags};
use twine_style::{Part, PropId, Selector};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;
use twine_widgets::animimg::AnimImg;
use twine_widgets::arc::Arc;
use twine_widgets::bar::Bar;
use twine_widgets::buttonmatrix::ButtonMatrix;
use twine_widgets::image::Image;
use twine_widgets::image_button::ImageButton;
use twine_widgets::keyboard::Keyboard;
use twine_widgets::led::Led;
use twine_widgets::line::Line;
use twine_widgets::spinner::Spinner;
use twine_widgets::textarea::AcceptedChars;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn has_flag(t: &TestUi, n: NodeId, f: ObjFlags) -> bool {
    t.engine().has_flag(n, f)
}

fn text_of(t: &TestUi, n: NodeId) -> String {
    t.engine()
        .widget::<Label>(n)
        .map(|l| l.text().to_string())
        .unwrap_or_default()
}

// ---- core.rs ----------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct CoreSig {
    on: Signal<bool>,
    lines: Signal<u16>,
    mode: Signal<LongMode>,
    angle: Signal<Angle>,
    align: Signal<ImageAlign>,
}

#[test]
fn core_setters_accept_signals() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let s = CoreSig {
            on: cx.signal(true),
            lines: cx.signal(1),
            mode: cx.signal(LongMode::Wrap),
            angle: cx.signal(Angle::deg(0)),
            align: cx.signal(ImageAlign::Center),
        };
        cx.provide(s);
        column((
            label("select me")
                .long_mode(s.mode)
                .max_lines(s.lines)
                .selectable(s.on)
                .test_id("l"),
            button(label("b")).checkable(s.on).test_id("b"),
            image(Symbol::Ok)
                .rotation(s.angle)
                .scale(move || {
                    if s.on.get() {
                        Scale::ONE
                    } else {
                        Scale::from_raw_256(128)
                    }
                })
                .pivot(move || Point::new(0, 0))
                .antialias(s.on)
                .inner_align(s.align)
                .test_id("i"),
        ))
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<CoreSig>();
    let (l, b, i) = (node(&t, "l"), node(&t, "b"), node(&t, "i"));
    assert!(has_flag(&t, l, ObjFlags::CLICKABLE));
    assert!(has_flag(&t, b, ObjFlags::CHECKABLE));
    s.on.set(false);
    s.lines.set(3);
    s.mode.set(LongMode::Dots);
    s.angle.set(Angle::deg(90));
    t.run_until_idle();
    assert!(!has_flag(&t, l, ObjFlags::CLICKABLE), "selectable(false)");
    assert!(!has_flag(&t, b, ObjFlags::CHECKABLE));
    let e = t.engine();
    let w = e.widget::<Label>(l).unwrap();
    assert_eq!((w.max_lines(), w.long_mode()), (3, LongMode::Dots));
    assert_eq!(e.widget::<Image>(i).unwrap().rotation(), Angle::deg(90));
}

// ---- controls.rs ------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct CtlSig {
    v: Signal<i32>,
    range: Signal<core::ops::RangeInclusive<i32>>,
    on: Signal<bool>,
    dur: Signal<Duration>,
    color: Signal<Color>,
    bright: Signal<Fraction>,
    angle: Signal<Angle>,
    width: Signal<i32>,
    pts: Signal<Vec<Point>>,
    text: Signal<String>,
}

#[test]
fn controls_setters_accept_signals() {
    let mut t = TestUi::new(320, 480).mount(|cx| {
        let s = CtlSig {
            v: cx.signal(10),
            range: cx.signal(0..=100),
            on: cx.signal(true),
            dur: cx.signal(Duration::ms(100)),
            color: cx.signal(Color::RED),
            bright: cx.signal(Fraction::from_raw(200)),
            angle: cx.signal(Angle::deg(0)),
            width: cx.signal(2),
            pts: cx.signal(vec![Point::new(0, 0), Point::new(10, 10)]),
            text: cx.signal(String::from("check")),
        };
        cx.provide(s);
        column((
            bar(s.v)
                .range(s.range)
                .mode(move || BarMode::Normal)
                .start_value(s.v)
                .orientation(move || Orientation::Horizontal)
                .animated(s.dur)
                .test_id("bar"),
            slider(s.v)
                .range(s.range)
                .mode(move || BarMode::Normal)
                .orientation(move || Orientation::Horizontal),
            switch(s.on).orientation(move || Orientation::Auto),
            checkbox(s.text, s.on).test_id("cb"),
            arc(s.v)
                .range(s.range)
                .angles(s.angle, s.angle)
                .bg_angles(s.angle, move || Angle::deg(270))
                .mode(move || ArcMode::Normal)
                .rotation(s.angle)
                .knob(s.on)
                .change_rate(move || AngularSpeed::deg_per_s(360))
                .test_id("arc"),
            led(s.on).color(s.color).brightness(s.bright).test_id("led"),
            line(s.pts)
                .y_invert(s.on)
                .width(s.width)
                .rounded(s.on)
                .dash(s.width, s.width)
                .test_id("line"),
            spinner().period(s.dur).arc_angle(s.angle).test_id("sp"),
        ))
    });
    // The spinner runs forever: a few updates instead of waiting for idle.
    let period = t.engine().config().refr_period;
    t.advance(period);
    let s = t.root_scope().expect_context::<CtlSig>();
    let arc_node = node(&t, "arc");
    let knob_styles = |t: &TestUi| {
        t.engine()
            .tree()
            .node(arc_node)
            .unwrap()
            .styles()
            .entries()
            .iter()
            .filter(|e| e.selector.part == Part::Knob)
            .count()
    };
    let knob = knob_styles(&t);
    assert!(knob > 0, "the theme styles the knob");
    s.v.set(40);
    s.range.set(0..=50);
    s.on.set(false);
    s.dur.set(Duration::ms(2000));
    s.color.set(Color::BLUE);
    s.bright.set(Fraction::from_raw(150));
    s.angle.set(Angle::deg(30));
    s.width.set(5);
    s.pts
        .set(vec![Point::new(0, 0), Point::new(20, 5), Point::new(40, 0)]);
    s.text.set(String::from("changed"));
    t.advance(period);
    t.advance(Duration::ms(3000));
    {
        let e = t.engine();
        let bar_w = e.widget::<Bar>(node(&t, "bar")).unwrap();
        assert_eq!((bar_w.value(), bar_w.max()), (40, 50));
        let a = e.widget::<Arc>(arc_node).unwrap();
        assert_eq!((a.value(), a.max(), a.rotation()), (40, 50, Angle::deg(30)));
        assert!(!e.widget::<Led>(node(&t, "led")).unwrap().is_on());
        assert_eq!(e.widget::<Led>(node(&t, "led")).unwrap().color(), Color::BLUE);
        let l = e.widget::<Line>(node(&t, "line")).unwrap();
        assert_eq!((l.points().len(), l.y_invert()), (3, false));
        assert_eq!(e.style_i32(node(&t, "line"), Part::Main, PropId::LineWidth), 5);
        let sp = e.widget::<Spinner>(node(&t, "sp")).unwrap();
        assert_eq!(sp.period(), Duration::ms(2000));
        assert_eq!(
            e.style_i32(node(&t, "bar"), Part::Main, PropId::AnimDuration),
            2000
        );
    }
    assert_eq!(t.find(by_id("cb")).text(), "changed");
    // knob(false): no knob styles, not clickable; knob(true) puts them back.
    assert_eq!(knob_styles(&t), 0);
    assert!(!has_flag(&t, arc_node, ObjFlags::CLICKABLE));
    s.on.set(true);
    t.advance(period);
    assert_eq!(knob_styles(&t), knob);
    assert!(has_flag(&t, arc_node, ObjFlags::CLICKABLE));
}

// ---- images.rs --------------------------------------------------------------------------------

static FRAMES_A: [ImageSource; 2] = [
    ImageSource::symbol(Symbol::Play),
    ImageSource::symbol(Symbol::Pause),
];
static FRAMES_B: [ImageSource; 1] = [ImageSource::symbol(Symbol::Stop)];

#[derive(Clone, Copy)]
struct ImgSig {
    src: Signal<ImageSource>,
    side: Signal<Option<ImageSource>>,
    frames: Signal<&'static [ImageSource]>,
    period: Signal<Duration>,
    playing: Signal<bool>,
}

#[test]
fn images_setters_accept_signals() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let s = ImgSig {
            src: cx.signal(ImageSource::symbol(Symbol::Play)),
            side: cx.signal(None),
            frames: cx.signal(&FRAMES_A[..]),
            period: cx.signal(Duration::ms(400)),
            playing: cx.signal(false),
        };
        cx.provide(s);
        column((
            image_button(s.src, s.src)
                .checked_images(s.src, s.src)
                .disabled_image(s.src)
                .three_slice(ImageButtonState::Released, s.side, s.src, s.side)
                .test_id("ib"),
            animimg(s.frames, s.period)
                .repeat(move || Repeat::Forever)
                .playing(s.playing)
                .test_id("anim"),
        ))
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<ImgSig>();
    s.src.set(ImageSource::symbol(Symbol::Pause));
    s.side.set(Some(ImageSource::symbol(Symbol::Left)));
    s.frames.set(&FRAMES_B[..]);
    s.period.set(Duration::ms(800));
    t.run_until_idle();
    let e = t.engine();
    let ib = e.widget::<ImageButton>(node(&t, "ib")).unwrap();
    let [l, m, r] = ib.src(ImageButtonState::Released);
    assert_eq!(m, Some(&ImageSource::symbol(Symbol::Pause)));
    assert_eq!(
        (l, r),
        (
            Some(&ImageSource::symbol(Symbol::Left)),
            Some(&ImageSource::symbol(Symbol::Left))
        )
    );
    assert_eq!(
        ib.src(ImageButtonState::Disabled)[1],
        Some(&ImageSource::symbol(Symbol::Pause))
    );
    let a = e.widget::<AnimImg>(node(&t, "anim")).unwrap();
    assert_eq!((a.frames().len(), a.period()), (1, Duration::ms(800)));
}

// ---- lists.rs ---------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ListSig {
    header: Signal<String>,
    icon: Signal<Option<ImageSource>>,
    text: Signal<String>,
}

#[test]
fn lists_setters_accept_signals() {
    let mut t = TestUi::new(240, 200).mount(|cx| {
        let s = ListSig {
            header: cx.signal(String::from("Files")),
            icon: cx.signal(Some(ImageSource::symbol(Symbol::File))),
            text: cx.signal(String::from("a.txt")),
        };
        cx.provide(s);
        list((
            list_text(s.header).test_id("h"),
            list_button(s.icon, s.text).test_id("b"),
        ))
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<ListSig>();
    let b = node(&t, "b");
    let img = t
        .engine()
        .tree()
        .children(b)
        .find(|&c| t.engine().widget::<Image>(c).is_some())
        .unwrap();
    assert!(!has_flag(&t, img, ObjFlags::HIDDEN));
    s.header.set(String::from("Docs"));
    s.text.set(String::from("b.txt"));
    s.icon.set(None);
    t.run_until_idle();
    assert_eq!(text_of(&t, node(&t, "h")), "Docs");
    assert!(has_flag(&t, img, ObjFlags::HIDDEN), "no icon: hidden");
    s.icon.set(Some(ImageSource::symbol(Symbol::Save)));
    t.run_until_idle();
    assert!(!has_flag(&t, img, ObjFlags::HIDDEN));
    assert_eq!(
        t.engine().widget::<Image>(img).unwrap().src(),
        Some(&ImageSource::symbol(Symbol::Save))
    );
    assert_eq!(t.find_all(twine_testing::by_text("b.txt")).len(), 1);
}

// ---- menus.rs ---------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct MenuSig {
    title: Signal<String>,
    mode: Signal<MenuHeaderMode>,
    back: Signal<bool>,
}

#[test]
fn menus_setters_accept_signals() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let s = MenuSig {
            title: cx.signal(String::from("Home")),
            mode: cx.signal(MenuHeaderMode::TopFixed),
            back: cx.signal(true),
        };
        cx.provide(s);
        menu(menu_page(label("root")).title(s.title))
            .header_mode(s.mode)
            .root_back_button(s.back)
            .test_id("m")
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<MenuSig>();
    let m = node(&t, "m");
    let header_title = || {
        let e = t.engine();
        let title = e.widget::<Menu>(m).unwrap().main_header_title();
        e.widget::<Label>(title).unwrap().text().to_string()
    };
    assert_eq!(header_title(), "Home");
    // A dynamic title updates the header of the loaded page.
    s.title.set(String::from("Start"));
    s.mode.set(MenuHeaderMode::BottomFixed);
    s.back.set(false);
    t.run_until_idle();
    let e = t.engine();
    let w = e.widget::<Menu>(m).unwrap();
    assert_eq!(e.widget::<Label>(w.main_header_title()).unwrap().text(), "Start");
    assert_eq!(
        (w.mode_header(), w.mode_root_back_button()),
        (MenuHeaderMode::BottomFixed, false)
    );
}

// ---- selection.rs -----------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct SelSig {
    names: Signal<Vec<String>>,
    dir: Signal<Side>,
    text: Signal<String>,
    on: Signal<bool>,
    mode: Signal<RollerMode>,
    rows: Signal<u8>,
}

#[test]
fn selection_setters_accept_signals() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let s = SelSig {
            names: cx.signal(vec![String::from("a"), String::from("b")]),
            dir: cx.signal(Side::Bottom),
            text: cx.signal(String::from("Pick")),
            on: cx.signal(true),
            mode: cx.signal(RollerMode::Normal),
            rows: cx.signal(3),
        };
        cx.provide(s);
        row((
            dropdown(s.names, 0usize)
                .dir(s.dir)
                .symbol(move || Some(ImageSource::symbol(Symbol::Down)))
                .text(s.text)
                .highlight(s.on)
                .test_id("dd"),
            roller(s.names, 0usize)
                .mode(s.mode)
                .visible_rows(s.rows)
                .test_id("r"),
        ))
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<SelSig>();
    s.names
        .set(vec![String::from("x"), String::from("y"), String::from("z")]);
    s.dir.set(Side::Top);
    s.text.set(String::from("Choose"));
    s.on.set(false);
    s.mode.set(RollerMode::Infinite);
    s.rows.set(5);
    t.run_until_idle();
    let e = t.engine();
    let d = e.widget::<Dropdown>(node(&t, "dd")).unwrap();
    assert_eq!(
        (d.options(), d.dir(), d.text(), d.selected_highlight()),
        ("x\ny\nz", Side::Top, Some("Choose"), false)
    );
    let r = e.widget::<Roller>(node(&t, "r")).unwrap();
    assert_eq!(
        (r.options(), r.mode(), r.visible_row_count()),
        ("x\ny\nz", RollerMode::Infinite, 5)
    );
}

// ---- span.rs ----------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct SpanSig {
    mode: Signal<SpanMode>,
    overflow: Signal<SpanOverflow>,
    n: Signal<i32>,
    text: Signal<String>,
    color: Signal<Color>,
}

#[test]
fn span_setters_accept_signals() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let s = SpanSig {
            mode: cx.signal(SpanMode::Break),
            overflow: cx.signal(SpanOverflow::Clip),
            n: cx.signal(0),
            text: cx.signal(String::from("one")),
            color: cx.signal(Color::RED),
        };
        cx.provide(s);
        spangroup((
            span(s.text)
                .font(move || &twine_assets::fonts::MONTSERRAT_14)
                .text_color(s.color)
                .text_opacity(move || Opa::COVER)
                .text_decoration(move || TextDecor::UNDERLINE)
                .letter_spacing(s.n),
            span("!"),
        ))
        .mode(s.mode)
        .overflow(s.overflow)
        .indent(s.n)
        .max_lines(move || s.n.get() + 2)
        .width(200)
        .test_id("g")
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<SpanSig>();
    s.text.set(String::from("two"));
    s.n.set(4);
    s.mode.set(SpanMode::Expand);
    s.overflow.set(SpanOverflow::Ellipsis);
    t.run_until_idle();
    let g = node(&t, "g");
    let e = t.engine();
    let w = e.widget::<twine_widgets::spangroup::SpanGroup>(g).unwrap();
    assert_eq!(
        (
            w.mode(&MeasureCx::new(&e, g)),
            w.overflow(),
            w.indent(),
            w.max_lines()
        ),
        (SpanMode::Expand, SpanOverflow::Ellipsis, 4, 6)
    );
    let first = w.span_ids().next().unwrap();
    assert_eq!(w.span(first).unwrap().text(), "two");
}

// ---- tabs.rs ----------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct TabSig {
    title: Signal<String>,
    dir: Signal<Side>,
    size: Signal<i32>,
    on: Signal<bool>,
}

#[test]
fn tabs_setters_accept_signals() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let s = TabSig {
            title: cx.signal(String::from("One")),
            dir: cx.signal(Side::Top),
            size: cx.signal(40),
            on: cx.signal(true),
        };
        cx.provide(s);
        tabview(0usize, (tab(s.title, label("1")), tab("Two", label("2"))))
            .bar_position(s.dir)
            .bar_size(s.size)
            .animated(s.on)
            .test_id("tv")
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<TabSig>();
    s.title.set(String::from("First"));
    s.dir.set(Side::Bottom);
    s.size.set(30);
    s.on.set(false);
    t.run_until_idle();
    let tv = node(&t, "tv");
    let e = t.engine();
    let w = e.widget::<Tabview>(tv).unwrap();
    assert_eq!(
        (w.tab_bar_position(), w.tab_bar_size(), w.animated()),
        (Side::Bottom, 30, false)
    );
    let b = w.tab_button(&e, 0).unwrap();
    let l = e
        .tree()
        .children(b)
        .find(|&c| e.widget::<Label>(c).is_some())
        .unwrap();
    assert_eq!(e.widget::<Label>(l).unwrap().text(), "First");
}

// ---- text_input.rs ----------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct InputSig {
    key: Signal<String>,
    on: Signal<bool>,
    mode: Signal<KeyboardMode>,
    hint: Signal<String>,
    chars: Signal<String>,
    max: Signal<i32>,
    range: Signal<core::ops::RangeInclusive<i32>>,
    digits: Signal<u8>,
}

#[test]
fn text_input_setters_accept_signals() {
    let mut t = TestUi::new(320, 480).mount(|cx| {
        let s = InputSig {
            key: cx.signal(String::from("A")),
            on: cx.signal(true),
            mode: cx.signal(KeyboardMode::TextLower),
            hint: cx.signal(String::from("Name")),
            chars: cx.signal(String::from("abc")),
            max: cx.signal(8),
            range: cx.signal(0..=99),
            digits: cx.signal(2),
        };
        cx.provide(s);
        let ta: NodeRef<Textarea> = cx.node_ref();
        column((
            buttonmatrix([[btn(s.key), btn("B")]])
                .one_checked(s.on)
                .test_id("m"),
            textarea(String::new())
                .placeholder(s.hint)
                .one_line(s.on)
                .password(s.on)
                .max_length(s.max)
                .accepted_chars(s.chars)
                .cursor_click_pos(s.on)
                .text_selection(s.on)
                .node_ref(ta)
                .test_id("ta"),
            keyboard(ta).mode(s.mode).popovers(s.on).test_id("kb"),
            spinbox(5)
                .range(s.range)
                .digits(s.digits, move || 0u8)
                .step(move || 1)
                .rollover(s.on)
                .test_id("sb"),
        ))
    });
    // The keyboard makes the textarea's cursor blink: never idle.
    t.advance(Duration::ms(100));
    let s = t.root_scope().expect_context::<InputSig>();
    s.key.set(String::from("Z"));
    s.on.set(false);
    s.mode.set(KeyboardMode::Number);
    s.hint.set(String::from("Code"));
    s.chars.set(String::from("0123"));
    s.max.set(4);
    s.range.set(0..=500);
    s.digits.set(3);
    // The keyboard makes the textarea's cursor blink: never idle.
    t.advance(Duration::ms(100));
    let e = t.engine();
    let m = e.widget::<ButtonMatrix>(node(&t, "m")).unwrap();
    assert_eq!((m.btn_text(0), m.one_checked()), (Some("Z"), false));
    let ta = e.widget::<Textarea>(node(&t, "ta")).unwrap();
    assert_eq!(
        (
            ta.placeholder_text(),
            ta.accepted_chars().map(AcceptedChars::as_str),
            ta.max_length(),
            ta.one_line()
        ),
        ("Code", Some("0123"), 4, false)
    );
    let kb = e.widget::<Keyboard>(node(&t, "kb")).unwrap();
    assert_eq!((kb.mode(), kb.popovers()), (KeyboardMode::Number, false));
    let sb = e.widget::<Spinbox>(node(&t, "sb")).unwrap();
    assert_eq!(
        (sb.range(), sb.digit_count(), sb.rollover()),
        ((0, 500), 3, false)
    );
}

// ---- windows.rs -------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct WinSig {
    title: Signal<String>,
    h: Signal<i32>,
    pad: Signal<i32>,
    icon: Signal<ImageSource>,
    w: Signal<i32>,
    close: Signal<bool>,
    ok: Signal<String>,
}

#[test]
fn windows_setters_accept_signals() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let s = WinSig {
            title: cx.signal(String::from("Win")),
            h: cx.signal(40),
            pad: cx.signal(4),
            icon: cx.signal(ImageSource::symbol(Symbol::Close)),
            w: cx.signal(30),
            close: cx.signal(true),
            ok: cx.signal(String::from("OK")),
        };
        cx.provide(s);
        let _ = cx.show_modal(move |_, _| {
            msgbox(s.title, s.title)
                .buttons([s.ok])
                .close_button(s.close)
                .test_id("mb")
        });
        window(s.title, window_button(s.icon, s.w).test_id("wb"), label("body"))
            .header_height(s.h)
            .content_padding(s.pad)
            .test_id("w")
    });
    t.run_until_idle();
    let s = t.root_scope().expect_context::<WinSig>();
    let close = t.find(twine_testing::by_class("msgbox_header_button")).id();
    assert!(!has_flag(&t, close, ObjFlags::HIDDEN));
    s.title.set(String::from("Window"));
    s.h.set(50);
    s.pad.set(0);
    s.icon.set(ImageSource::symbol(Symbol::Ok));
    s.w.set(44);
    s.close.set(false);
    s.ok.set(String::from("Fine"));
    t.run_until_idle();
    assert!(
        has_flag(&t, close, ObjFlags::HIDDEN),
        "close_button(false) hides it"
    );
    let footer = t.find(twine_testing::by_class("msgbox_footer_button")).id();
    let footer_label = t.engine().tree().children(footer).next().unwrap();
    assert_eq!(text_of(&t, footer_label), "Fine");
    assert_eq!(
        t.find_all(twine_testing::by_text("Window")).len(),
        3,
        "window and msgbox title, msgbox text"
    );
    let (w, wb) = (node(&t, "w"), node(&t, "wb"));
    let e = t.engine();
    let content = e
        .widget::<twine_widgets_ext::window::Window>(w)
        .unwrap()
        .content();
    assert_eq!(e.style_i32(content, Part::Main, PropId::PaddingTop), 0);
    assert_eq!(e.coords(wb).width(), 44);
    let img = e.tree().children(wb).next().unwrap();
    assert_eq!(
        e.widget::<Image>(img).unwrap().src(),
        Some(&ImageSource::symbol(Symbol::Ok))
    );
    let _ = Selector::MAIN;
}
