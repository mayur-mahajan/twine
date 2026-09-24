//! The `Ui` runtime: update order, `Wake`, themes.

use std::cell::RefCell;
use std::rc::Rc;

use twine_anim::Anim;
use twine_core::{ColorFormat, Duration, Instant, Point};
use twine_engine::{DrawCx, EventCode, EventFilter, EventResult, Widget, WidgetClass};
use twine_hal::{DisplayInfo, PollHint};
use twine_testing::{FlushRecord, MemoryDisplay, MockClock, MockPointer};
use twine_view::prelude::*;

type Log = Rc<RefCell<Vec<&'static str>>>;

/// A widget that logs its drawing.
struct Probe(Log);
static PROBE_CLASS: WidgetClass = WidgetClass::new("probe");
impl Widget for Probe {
    fn class(&self) -> &'static WidgetClass {
        &PROBE_CLASS
    }
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        self.0.borrow_mut().push("draw");
        cx.draw_base(Part::Main);
    }
}

fn ui(app: impl FnOnce(Scope) -> AnyView) -> (Ui, MockClock, MockPointer) {
    let clock = MockClock::new();
    let pointer = MockPointer::new();
    pointer.set_poll_hint(PollHint::Interrupt);
    let display = MemoryDisplay::new(DisplayInfo::new(160, 120, ColorFormat::Rgb565));
    let ui = Ui::builder(display)
        .clock(clock.clone())
        .input(pointer.clone())
        .theme(DefaultTheme::light())
        .build(app);
    (ui, clock, pointer)
}

fn flushes(ui: &mut Ui) -> Vec<FlushRecord> {
    let d = ui.display();
    let mut out = Vec::new();
    if let Some(m) = ui.engine_mut().driver_mut::<MemoryDisplay>(d) {
        m.drain_flushes_into(&mut out);
    }
    out
}

static ORDER_CH: Channel<u8, 4> = Channel::new();

#[test]
fn update_order_is_as_documented() {
    let log: Log = Rc::default();
    let l = log.clone();
    let (mut ui, clock, pointer) = ui(move |cx| {
        let w = cx.signal(40);
        cx.provide(w);
        let l1 = l.clone();
        cx.on_message(&ORDER_CH, move |_| {
            l1.borrow_mut().push("message");
            w.set(60);
        });
        let l2 = l.clone();
        let l3 = l.clone();
        widget_view(move || Probe(l.clone()))
            .width(w)
            .height(30)
            .clickable(true)
            .on_press(move || l2.borrow_mut().push("input"))
            .on_event(EventCode::SizeChanged, move |_| l3.borrow_mut().push("layout"))
            .into_any()
    });
    let lb = log.clone();
    let w = ui.root_scope().expect_context::<Signal<i32>>();
    ui.root_scope().effect(move || {
        w.get();
        lb.borrow_mut().push("effect");
    });
    ui.update();
    let probe = ui
        .engine()
        .tree()
        .descendants(ui.engine().active_screen(ui.display()).unwrap())
        .nth(1)
        .unwrap();
    // A binding reading a signal written by every step.
    let lt = log.clone();
    ui.engine_mut()
        .timer_add(Duration::ms(10), move |_, _| lt.borrow_mut().push("timer"));
    let la = log.clone();
    ui.engine_mut()
        .anim_start_fn(Anim::new(0, 10).duration(Duration::ms(100)), move |_, _| {
            la.borrow_mut().push("anim");
        });
    let le = log.clone();
    ui.engine_mut()
        .add_event_handler(probe, EventFilter::Code(EventCode::Pressed), move |_, _| {
            le.borrow_mut().push("input");
            EventResult::Continue
        });
    ui.update(); // the timer starts counting now
    log.borrow_mut().clear();
    clock.advance(Duration::ms(20));
    ORDER_CH.try_send(1).unwrap();
    pointer.press(Point::new(5, 5));
    ui.notify_input();
    ui.update();
    let got: Vec<&str> = log.borrow().clone();
    let first = |what: &str| {
        got.iter()
            .position(|x| *x == what)
            .unwrap_or_else(|| panic!("no {what} in {got:?}"))
    };
    let order = ["message", "input", "timer", "anim", "effect", "layout", "draw"];
    for w in order.windows(2) {
        assert!(first(w[0]) < first(w[1]), "{} before {}: {got:?}", w[0], w[1]);
    }
}

#[test]
fn idle_after_build_and_first_frame() {
    let (mut ui, clock, _p) = ui(|_| column((label("a"), button(label("b")))).into_any());
    let w = ui.update();
    assert!(!flushes(&mut ui).is_empty(), "the first update renders");
    if let Wake::At(t) = w {
        clock.set(t);
        ui.update();
    }
    let _ = flushes(&mut ui);
    clock.advance(Duration::ms(100));
    assert_eq!(ui.update(), Wake::Idle);
    assert!(flushes(&mut ui).is_empty());
}

#[test]
fn wake_at_when_timer_pending() {
    let (mut ui, clock, _p) = ui(|cx| {
        cx.interval(Duration::ms(500), || {});
        label("tick").into_any()
    });
    let mut w = ui.update();
    while w == Wake::Now {
        w = ui.update();
    }
    match w {
        Wake::At(t) => assert!(t <= clock.now_instant() + Duration::ms(500), "{t:?}"),
        other => panic!("expected Wake::At, got {other:?}"),
    }
}

trait Now {
    fn now_instant(&self) -> Instant;
}
impl Now for MockClock {
    fn now_instant(&self) -> Instant {
        twine_hal::Clock::now(self)
    }
}

#[test]
fn wake_now_when_waker_flag_set() {
    let (mut ui, clock, _p) = ui(|_| label("x").into_any());
    ui.update();
    let w = ui.waker();
    ui.engine_mut().timer_add(Duration::ms(10), move |e, id| {
        w.wake();
        e.timer_remove(id);
    });
    ui.update(); // the timer starts counting now
    clock.advance(Duration::ms(10));
    assert_eq!(ui.update(), Wake::Now);
    // The flag is consumed by the next update (which notifies the inputs).
    clock.advance(Duration::ms(100));
    assert_eq!(ui.update(), Wake::Idle);
}

#[test]
fn set_theme_rerenders_once() {
    let (mut ui, clock, _p) = ui(|_| button(label("b")).into_any());
    ui.update();
    clock.advance(Duration::ms(100));
    ui.update();
    let _ = flushes(&mut ui);
    ui.set_theme(DefaultTheme::dark());
    clock.advance(Duration::ms(100));
    let w = ui.update();
    let f = flushes(&mut ui);
    let px: u32 = f.iter().map(|r| r.area.area() as u32).sum();
    assert_eq!(px, 160 * 120, "the whole display once");
    if let Wake::At(t) = w {
        clock.set(t);
        ui.update();
    }
    clock.advance(Duration::ms(100));
    assert_eq!(ui.update(), Wake::Idle);
    assert!(flushes(&mut ui).is_empty());
}

#[test]
fn missing_clock_is_an_error() {
    let display = MemoryDisplay::new(DisplayInfo::new(32, 32, ColorFormat::Rgb565));
    let r = Ui::builder(display).try_build(|_| label("x"));
    assert!(r.is_err());
}
