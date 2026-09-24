//! API conformance: the examples of the public API guide, verbatim where they are complete
//! programs, wrapped in functions where they are fragments. If one of these stops compiling,
//! the public API changed.
//!
//! NOTE(P19.S01): msgbox (§6 modal example), chart (§8) and the other complex widgets of §3.5
//! (dropdown, roller, list, menu, tabview, …) are added with their views.
// The guide's code is kept verbatim: public items without docs, unused parameters.
#![allow(
    missing_docs,
    clippy::needless_pass_by_value,
    clippy::unreadable_literal,
    clippy::cast_precision_loss,
    clippy::semicolon_if_nothing_returned,
    unused_variables,
    dead_code
)]

use twine::prelude::*;

// ---- §1 Hello, counter ---------------------------------------------------------------------

pub fn counter(cx: Scope) -> impl View {
    let count = cx.signal(0u32);

    column((
        label(text!("Clicked {} times", count.get()))
            .font(&fonts::MONTSERRAT_20)
            .test_id("count"),
        button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
        button(label("Reset"))
            .disabled(move || count.get() == 0)
            .on_click(move || count.set(0)),
    ))
    .gap(12)
    .padding(16)
    .align_items(FlexAlign::Center)
    .size(Length::Pct(100), Length::Pct(100))
}

// ---- §1.2 Thermostat ------------------------------------------------------------------------

mod thermostat {
    use twine::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct SensorMsg {
        pub celsius: f32,
    }

    pub static SENSOR: Channel<SensorMsg, 8> = Channel::new();

    pub fn thermostat(cx: Scope) -> impl View {
        let current = cx.signal(200i32); // tenths of a degree
        let target = cx.signal(215i32);
        cx.on_message(&SENSOR, move |m: SensorMsg| {
            current.set((m.celsius * 10.0) as i32)
        });
        let shown = cx.tween(move || current.get(), Duration::ms(400), Easing::EaseOut);
        let heating = cx.memo(move || current.get() < target.get());

        column((
            label(text!("{:.1} °C", shown.get() as f32 / 10.0))
                .font(&fonts::MONTSERRAT_20)
                .test_id("current"),
            row((
                button(label("-")).on_click(move || target.update(|t| *t -= 5)),
                label(text!("Target {:.1} °C", target.get() as f32 / 10.0)).test_id("target"),
                button(label("+")).on_click(move || target.update(|t| *t += 5)),
            ))
            .gap(8)
            .align_items(FlexAlign::Center),
            label(text!("{}", if heating.get() { "Heating" } else { "Idle" }))
                .text_color(move || {
                    if heating.get() {
                        Color::hex(0xE5_39_35)
                    } else {
                        Color::hex(0x75_75_75)
                    }
                })
                .test_id("state"),
        ))
        .gap(12)
        .padding(16)
        .align_items(FlexAlign::Center)
        .size(Length::Pct(100), Length::Pct(100))
    }
}

#[test]
fn s1_counter_and_thermostat() {
    use twine_testing::{TestUi, by_id, by_text};
    let mut t = TestUi::new(320, 240).mount(counter);
    t.find(by_text("Click me")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "Clicked 1 times");

    let mut t = TestUi::new(320, 240).mount(thermostat::thermostat);
    thermostat::SENSOR
        .try_send(thermostat::SensorMsg { celsius: 22.5 })
        .ok();
    t.find(by_text("+")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("current")).text(), "22.5 °C");
    assert_eq!(t.find(by_id("target")).text(), "Target 22.0 °C");
    assert_eq!(t.find(by_id("state")).text(), "Idle");
}

// ---- §2 Reactive primitives ----------------------------------------------------------------

#[test]
fn s2_reactive_primitives() {
    let cx = twine::reactive::create_root();
    let s: Signal<i32> = cx.signal(1);
    let m: Memo<i32> = cx.memo(move || s.get() * 2);
    let _e: EffectId = cx.effect(move || {
        let _ = m.get();
    });
    cx.on_cleanup(|| {});
    let child = cx.child();
    cx.provide(5u8);
    assert_eq!(child.use_context::<u8>(), Some(5));
    assert_eq!(child.expect_context::<u8>(), 5);
    assert_eq!(s.get_untracked(), 1);
    s.with(|v| assert_eq!(*v, 1));
    s.with_untracked(|v| assert_eq!(*v, 1));
    s.set(2);
    s.set_if_changed(2);
    s.update(|v| *v += 1);
    let r: ReadSignal<i32> = s.read_only();
    let w: WriteSignal<i32> = s.write_only();
    let (r2, w2) = s.split();
    w.set(4);
    w2.set_if_changed(4);
    assert_eq!(r.get() + r2.get(), 8);
    assert_eq!(s.map(|v| v * 10).get(), 40);
    assert_eq!(m.get(), 8);
    assert_eq!(batch(|| 3), 3);
    assert_eq!(untrack(|| s.get()), 4);
    assert_eq!(s.try_get(), Some(4));
    assert!(s.is_alive());
    child.dispose();
    cx.dispose();
}

// ---- §2.1 Sending values from interrupts / other tasks --------------------------------------

mod sensor {
    use twine::prelude::*;

    #[derive(Clone, Copy)]
    pub struct SensorMsg {
        pub celsius: f32,
    }

    static SENSOR: Channel<SensorMsg, 8> = Channel::new(); // ISR/task safe, no alloc

    fn app(cx: Scope) -> impl View {
        let temp = cx.signal(0.0f32);
        cx.on_message(&SENSOR, move |msg: SensorMsg| temp.set(msg.celsius));
        label(text!("{:.1} °C", temp.get()))
    }

    #[test]
    fn s2_1_channel() {
        let mut t = twine_testing::TestUi::new(120, 40).mount(app);
        // In an ISR or another embassy task (Send + Sync):
        SENSOR.try_send(SensorMsg { celsius: 21.7 }).ok(); // also wakes the UI loop
        t.run_until_idle();
        assert_eq!(t.find(twine_testing::by_class("label")).text(), "21.7 °C");
    }
}

// ---- §3 Views, §3.1 property values -----------------------------------------------------

struct Pair(&'static str, &'static str);

impl View for Pair {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let root = cx.create(twine::engine::Obj);
        cx.with_parent(root, |cx| {
            cx.build_seq((label(self.0), label(self.1)));
        });
        root
    }
}

#[test]
fn s3_views_and_props() {
    fn takes_prop<T: 'static>(p: impl IntoProp<T>) -> Prop<T> {
        p.into_prop()
    }
    let cx = twine::reactive::create_root();
    let s = cx.signal(Color::RED);
    let _: Prop<Color> = takes_prop(Color::BLUE);
    let _: Prop<Color> = takes_prop(s);
    let _: Prop<Color> = takes_prop(s.read_only());
    let _: Prop<Color> = takes_prop(cx.memo(move || s.get()));
    let _: Prop<Color> = takes_prop(move || s.get());
    let name = cx.signal(String::from("a"));
    let _ = (
        label("static"),
        label(String::from("owned")),
        label(text!("{}", 1)),
        label(name),
        label(cx.memo(move || name.get())),
        label(move || name.get()),
    );
    let any: AnyView = Pair("a", "b").into_any();
    let seq = (
        label("x"),
        vec![label("y")],
        [label("z")],
        Some(label("w")),
        (),
        any,
    );
    let _ = column(seq);
    cx.dispose();
}

// ---- §3.5 Widgets: basic controls, text entry ----------------------------------------------

static FRAMES: [ImageSource; 2] = [
    ImageSource::Symbol(symbols::PLAY),
    ImageSource::Symbol(symbols::PAUSE),
];
static POINTS: [Point; 3] = [Point::new(0, 10), Point::new(20, 0), Point::new(40, 10)];
static MAP: [&str; 5] = ["1", "2", "\n", "3", "4"];
static KB_MAP: [&str; 3] = ["a", "b", "c"];
static KB_CTRL: [BtnCtrl; 3] = [BtnCtrl::empty(), BtnCtrl::empty(), BtnCtrl::empty()];

fn basic_controls(cx: Scope) -> impl View {
    let level = cx.signal(40);
    let low = cx.signal(10);
    let on = cx.signal(false);
    let pts = cx.signal(vec![Point::new(0, 0), Point::new(30, 20)]);
    column((
        image_button(
            ImageSource::Symbol(symbols::PLAY),
            ImageSource::Symbol(symbols::PLAY),
        )
        .checked_images(
            ImageSource::Symbol(symbols::PAUSE),
            ImageSource::Symbol(symbols::PAUSE),
        )
        .disabled_image(ImageSource::Symbol(symbols::STOP)),
        animimg(&FRAMES, Duration::ms(500))
            .repeat(Repeat::Infinite)
            .playing(on),
        arc(level)
            .range(0..=100)
            .angles(Angle::deg(0), Angle::deg(90))
            .bg_angles(Angle::deg(0), Angle::deg(270))
            .mode(ArcMode::Normal)
            .rotation(Angle::deg(135))
            .knob(true)
            .change_rate(720),
        bar(level)
            .range(0..=100)
            .mode(BarMode::Range)
            .start_value(low)
            .orientation(Orientation::Horizontal)
            .animated(Duration::ms(200)),
        slider(level)
            .range(0..=100)
            .mode(SliderMode::Range)
            .left_value(low)
            .orientation(Orientation::Auto),
        switch(on).orientation(Orientation::Horizontal),
        checkbox("Enabled", on),
        led(on).color(Color::RED).brightness(200),
        line(pts).y_invert(false).width(2).rounded(true).dash(4, 2),
        line_static(&POINTS),
        spinner().period(Duration::ms(1000)).arc_angle(Angle::deg(200)),
    ))
}

fn text_entry(cx: Scope) -> impl View {
    let text = cx.signal(String::new());
    let qty = cx.signal(5);
    let name = cx.signal(String::from("Ada"));
    let ta: NodeRef<Textarea> = cx.node_ref();
    column((
        buttonmatrix(&MAP)
            .ctrl(0, BtnCtrl::CHECKABLE)
            .one_checked(true)
            .on_select(|idx: usize| {}),
        textarea(text)
            .placeholder("Type here")
            .one_line(true)
            .password(false)
            .max_length(32)
            .accepted_chars("abc")
            .cursor_click_pos(true)
            .text_selection(true)
            .on_ready(|| {})
            .node_ref(ta),
        keyboard(ta)
            .mode(KeyboardMode::TextLower)
            .custom_map(KeyboardMode::User1, &KB_MAP, &KB_CTRL)
            .popovers(true),
        spinbox(qty).range(0..=99).digits(2, 0).step(1).rollover(false),
        spangroup((
            span("Hello, "),
            span(name)
                .font(&fonts::MONTSERRAT_20)
                .text_color(Color::BLUE)
                .text_decor(TextDecor::UNDERLINE),
        ))
        .mode(SpanMode::Break)
        .overflow(SpanOverflow::Ellipsis)
        .indent(8)
        .max_lines(2),
    ))
}

#[test]
fn s3_5_widgets() {
    // The spinner runs forever: a few frames instead of waiting for idle.
    let mut t = TestUi::new(320, 480).mount(basic_controls);
    t.advance(Duration::ms(100));
    let mut t = TestUi::new(320, 480).mount(text_entry);
    t.advance(Duration::ms(100));
}

// ---- §3.3 Containers and §3.4 control flow ------------------------------------------------

static COLS: [GridTrack; 2] = [GridTrack::Px(40), GridTrack::Fr(1)];
static ROWS: [GridTrack; 1] = [GridTrack::Content];

fn containers_and_flow(cx: Scope) -> impl View {
    let on = cx.signal(true);
    let items = cx.signal(vec![1u32, 2, 3]);
    column((
        row((label("a"), spacer(), label("b"))).justify(FlexAlign::SpaceBetween),
        flex(FlexFlow::RowWrap, (label("c"), label("d")))
            .align_items(FlexAlign::Center)
            .align_content(FlexAlign::Start),
        grid(
            &COLS,
            &ROWS,
            (label("e").grid_cell(0, 1, 0, 1), label("f").grid_cell(1, 1, 0, 1)),
        )
        .column_align(GridAlign::Start)
        .row_align(GridAlign::Start),
        container(label("g")),
        stack((label("h"), label("i"))),
        scroll_view(Dir::VER, label("j")),
        when(move || on.get(), |_cx| label("on")).otherwise(|_cx| label("off")),
        dynamic(move |_cx| {
            if on.get() {
                label("x").into_any()
            } else {
                button(label("y")).into_any()
            }
        }),
        for_each(move || items.get(), |i| *i, |_cx, i| label(format!("{i}"))),
        virtual_list(|| 100, 20, |_cx, i| label(format!("row {i}"))).height(60),
    ))
}

#[test]
fn s3_3_containers_and_control_flow() {
    let mut t = twine_testing::TestUi::new(240, 400).mount(containers_and_flow);
    t.run_until_idle();
    assert_eq!(t.find(twine_testing::by_text("on")).text(), "on");
}

// ---- §4 Styles ------------------------------------------------------------------------------

pub static CARD: Style = style! {
    bg_color: Color::WHITE,
    bg_opa: Opa::COVER,
    radius: 8,
    pad_all: 12,              // shorthand → pad_top/bottom/left/right
    shadow_width: 12,
    shadow_opa: Opa::P30,
};
pub static CARD_PRESSED: Style = style! { bg_color: Color::hex(0xEEEEEE), transform_scale: 250 };

fn card(title: &'static str) -> impl View {
    container(label(title))
        .style(&CARD)
        .style_for(Selector::state(State::PRESSED), &CARD_PRESSED)
        .transition(&SMOOTH)
}
pub static SMOOTH: TransitionDsc = TransitionDsc::new(
    &[PropId::BgColor, PropId::TransformScaleX, PropId::TransformScaleY],
    Duration::ms(150),
    Easing::EaseOut,
);

#[test]
fn s4_styles() {
    let mut t = twine_testing::TestUi::new(200, 100).mount(|_| card("Card"));
    t.run_until_idle();
    let _ = Selector::part(Part::Indicator).with_state(State::PRESSED);
}

// ---- §5 Themes ------------------------------------------------------------------------------

fn s5_themes<D: twine::hal::DisplayDriver + 'static>(display: D, clock: impl twine::hal::Clock + 'static) {
    let app = |_cx: Scope| label("themed");
    let ui = Ui::builder(display)
        .clock(clock)
        .theme(DefaultTheme::new(
            Palette::Blue,
            Palette::Red,
            ThemeMode::Dark,
            &fonts::MONTSERRAT_14,
        ))
        .build(app);
    let mut ui = ui;
    // Switch at runtime: ui.set_theme(...) or from a view: use_theme(cx).set(...)
    ui.set_theme(DefaultTheme::light());
}

#[test]
fn s5_themes_run() {
    use twine::core::ColorFormat;
    use twine::hal::DisplayInfo;
    let display = twine_testing::MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    s5_themes(display, twine_testing::MockClock::new());
}

// ---- §6 Navigation, screens, modals -------------------------------------------------------

mod navigation {
    use twine::prelude::*;

    pub fn app(cx: Scope) -> impl View {
        navigator(cx, home) // provides Navigator context
    }
    fn home(cx: Scope) -> impl View {
        let nav = use_navigator(cx);
        button(label("Settings"))
            .on_click(move || nav.push(settings, ScreenAnim::MoveLeft(Duration::ms(300))))
    }
    fn settings(cx: Scope) -> impl View {
        let nav = use_navigator(cx);
        column((
            label("Settings"),
            button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(Duration::ms(300)))),
        ))
    }

    /// NOTE(P19.S01): the guide's modal shows a `msgbox`; any view works.
    pub fn modal(cx: Scope) {
        let modal = cx.show_modal(|cx| label("This cannot be undone"));
        modal.close();
    }
}

#[test]
fn s6_navigation() {
    use twine_testing::{TestUi, by_text};
    let mut t = TestUi::new(240, 160).mount(navigation::app);
    t.find(by_text("Settings")).click();
    t.run_until_idle();
    t.find(by_text("Back")).click();
    t.run_until_idle();
    assert_eq!(t.find_all(by_text("Settings")).len(), 1);
    let _ = ScreenAnim::FadeIn(Duration::ms(100)).delay(Duration::ms(50));
    let t = TestUi::new(240, 160).mount(|cx| {
        navigation::modal(cx);
        label("x")
    });
    drop(t);
}

// ---- §7 Animation from the declarative layer --------------------------------------------------

static GEAR: twine::image::Image = twine::image::Image::new_static(
    twine::image::ImageHeader::new(twine::core::ColorFormat::Rgb565, 2, 2),
    &[0; 8],
);

fn animation(cx: Scope) -> impl View {
    let content = label("content");
    let clock = cx.signal(0u32);
    let toast = cx.signal(Some("saved"));

    let open = cx.signal(false);
    // Tween: a ReadSignal that animates toward the source value whenever it changes.
    let h = cx.tween(
        move || if open.get() { 200 } else { 48 },
        Duration::ms(250),
        Easing::EaseInOut,
    );
    let _ = container(content).height(h);

    // Free-running animation value:
    let (angle, ctl) = cx.animation(
        Anim::new(0, 3600)
            .duration(Duration::ms(1000))
            .repeat(Repeat::Infinite)
            .easing(Easing::Linear),
    );
    let _ = image(ImageSource::from(&GEAR)).rotation(move || Angle::decideg(angle.get()));
    ctl.pause();
    ctl.resume();
    ctl.restart();

    // Timers
    cx.interval(Duration::ms(1000), move || clock.update(|t| *t += 1));
    cx.timeout(Duration::ms(3000), move || toast.set(None));
    label("animated")
}

#[test]
fn s7_animation() {
    let mut t = twine_testing::TestUi::new(120, 80).mount(animation);
    t.advance(Duration::ms(100));
}

// ---- §8 Imperative escape hatch ----------------------------------------------------------------
// NOTE(P19.S10): the guide's example drives a `chart`; the same with a label:

static SAMPLES: Channel<u32, 4> = Channel::new();

fn escape_hatch(cx: Scope) -> impl View {
    let scope_ref: NodeRef<Label> = cx.node_ref();
    cx.on_message(&SAMPLES, move |s: u32| {
        scope_ref.with_mut(|l, cx| l.set_text(cx, "sample"));
    });
    label("waiting").node_ref(scope_ref)
}

#[test]
fn s8_escape_hatch() {
    let mut t = twine_testing::TestUi::new(120, 40).mount(escape_hatch);
    SAMPLES.try_send(1).unwrap();
    t.run_until_idle();
    assert_eq!(t.find(twine_testing::by_class("label")).text(), "sample");
}

// ---- §9 Custom widgets --------------------------------------------------------------------

pub struct Gauge {
    value: i32,
}
impl Widget for Gauge {
    fn class(&self) -> &'static WidgetClass {
        &GAUGE_CLASS
    }
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let dsc = cx.arc_dsc(Part::Indicator);
        let center = cx.content_area().center();
        cx.painter().arc(
            center,
            40,
            Angle::deg(135),
            Angle::deg(135 + self.value * 27 / 10),
            &dsc,
        );
    }
    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        EventResult::Continue
    }
}
pub static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge").parts(&[Part::Main, Part::Indicator]);

impl Gauge {
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        if self.value == v {
            return;
        } // P3
        self.value = v;
        cx.invalidate();
    }
}

// Declarative wrapper:
pub fn gauge(value: impl IntoProp<i32>) -> impl View {
    widget_view(|| Gauge { value: 0 }).bind(value, |g: &mut Gauge, cx, v| g.set_value(cx, v))
}

#[test]
fn s9_custom_widget() {
    let mut t = twine_testing::TestUi::new(120, 120).mount(|cx| {
        let v = cx.signal(50);
        cx.provide(v);
        container(gauge(v)).size(100, 100)
    });
    t.run_until_idle();
    let v = t.root_scope().expect_context::<Signal<i32>>();
    v.set(60);
    t.run_until_idle();
    t.assert_idle();
}

// ---- §10.1 Blocking super-loop ---------------------------------------------------------------

fn wait_for_interrupt() {}
fn sleep_until_or_interrupt(_t: Instant) {}

fn super_loop<D: twine::hal::DisplayDriver + 'static>(
    display: D,
    touch: impl twine::hal::InputDevice + 'static,
    clock: impl twine::hal::Clock + 'static,
    buf_a: &'static mut [u8],
    buf_b: &'static mut [u8],
    app: fn(Scope) -> Flex,
    rounds: usize,
) {
    let mut ui = Ui::builder(display) // impl DisplayDriver
        .buffers(BufferMode::partial_double(buf_a, buf_b))
        .input(touch) // impl InputDevice (any number)
        .clock(clock) // impl Clock
        .theme(DefaultTheme::light())
        .build(app);
    for _ in 0..rounds {
        match ui.update() {
            Wake::Idle => wait_for_interrupt(), // touch IRQ, channel send, etc.
            Wake::At(t) => sleep_until_or_interrupt(t),
            Wake::Now => {} // more work pending (e.g. DMA done)
        }
    }
}

#[test]
fn s10_1_super_loop() {
    use twine::core::ColorFormat;
    use twine::hal::DisplayInfo;
    let leak = |n: usize| -> &'static mut [u8] { Box::leak(vec![0u8; n].into_boxed_slice()) };
    let display = twine_testing::MemoryDisplay::new(DisplayInfo::new(160, 120, ColorFormat::Rgb565));
    let app: fn(Scope) -> Flex = |_| column(label("loop"));
    super_loop(
        display,
        twine_testing::MockPointer::new(),
        twine_testing::MockClock::new(),
        leak(160 * 20 * 2),
        leak(160 * 20 * 2),
        app,
        5,
    );
}

// ---- §10.3 Headless tests ---------------------------------------------------------------------

use twine_testing::{TestUi, by_id, by_text};

#[test]
fn counter_increments() {
    let mut t = TestUi::new(320, 240).mount(counter);
    t.find(by_text("Click me")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "Clicked 1 times");
    t.assert_snapshot("counter_clicked_once");
    assert!(
        t.last_frame().dirty_px < 320 * 40,
        "only the label + button should redraw"
    );
}
