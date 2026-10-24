//! Display integrity through the `Ui` (R5.S06): heartbeat, the fail-safe screen (entered after
//! a seal without allocating), `Validity` texts and looks, `watch_fresh` aging, frame observers
//! registered at build and the off-screen re-render of a safety-critical label.
//!
//! req: SAF-016, SAF-017, SAF-018
#![cfg(feature = "display-integrity")]

use std::cell::RefCell;
use std::rc::Rc;

use twine_core::ColorFormat;
use twine_core::alloc_guard;
use twine_core::crc::Crc32;
use twine_engine::EngineError;
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal};
use twine_testing::{MemoryDisplay, MockClock};
use twine_view::INVALID_TEXT;
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn limits() -> Limits {
    Limits::DEFAULT
        .max_nodes(128)
        .max_reactive_nodes(256)
        .max_scopes(64)
        .max_pending_effects(32)
        .max_text_len(32)
        .preallocated()
}

fn ui<V: View>(clock: &MockClock, app: impl FnOnce(Scope) -> V) -> Ui {
    let panel = MemoryDisplay::new(DisplayInfo::new(120, 60, ColorFormat::Rgb565));
    Ui::builder(panel)
        .runtime(Runtime::take().unwrap())
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 20 }))
        .limits(limits())
        .try_build(app)
        .unwrap()
}

fn text_of(ui: &Ui, n: NodeId) -> String {
    ui.engine()
        .tree()
        .node(n)
        .and_then(|n| n.widget().text())
        .unwrap_or_default()
        .to_string()
}

fn state_of(ui: &Ui, n: NodeId) -> State {
    ui.engine().tree().node(n).unwrap().state()
}

#[test]
fn heartbeat_counts_updates() {
    let clock = MockClock::new();
    let mut ui = ui(&clock, |_| label("hi"));
    let b = ui.heartbeat();
    for _ in 0..3 {
        clock.advance(Duration::ms(10));
        ui.update();
    }
    assert_eq!(ui.heartbeat(), b.wrapping_add(3));
}

/// fault: `FailSafe`
/// req: SAF-016, SAF-031
#[test]
fn fail_safe_is_entered_after_seal_without_allocating() {
    let clock = MockClock::new();
    let mut ui = ui(&clock, |cx| {
        let n = cx.signal(1);
        label(text!("value {}", n.get()))
    });
    ui.set_fail_safe(|_| container(label("SERVICE")).size(120, 60).bg(Color::RED))
        .unwrap();
    ui.update();
    let normal = ui.engine().active_screen(ui.display()).unwrap();
    ui.seal();
    // The test panel logs every flush: drain the log (keeping its capacity) so the log does
    // not grow inside the checked block.
    let d = ui.display();
    let mut log = Vec::with_capacity(64);
    ui.engine_mut()
        .driver_mut::<MemoryDisplay>(d)
        .unwrap()
        .drain_flushes_into(&mut log);
    log.clear();
    assert_no_alloc_after_seal(|| {
        ui.enter_fail_safe(9);
        clock.advance(Duration::ms(50));
        ui.update();
    });
    alloc_guard::unseal(twine_core::Partition::MAIN);
    assert_eq!(ui.fail_safe(), Some(9));
    assert_ne!(ui.engine().active_screen(ui.display()), Some(normal));
    let panel = ui.engine().driver::<MemoryDisplay>(ui.display()).unwrap();
    assert_eq!(panel.pixel(119, 59), Color::RED, "the fail-safe screen is shown");
    assert_eq!(ui.fault_counts().get(FaultKind::FailSafe), 1);
    assert_eq!(ui.fault_counts().get(FaultKind::AllocAfterSeal), 0);
    // Navigation cannot leave it.
    ui.engine_mut().load_screen(normal);
    assert_ne!(ui.engine().active_screen(ui.display()), Some(normal));
    ui.leave_fail_safe();
    ui.engine_mut().load_screen(normal);
    assert_eq!(ui.engine().active_screen(ui.display()), Some(normal));
}

#[test]
fn reactive_fail_safe_view_is_refused() {
    let clock = MockClock::new();
    let mut ui = ui(&clock, |_| label("app"));
    let screens = ui.engine().screens(ui.display()).len();
    let r = ui.set_fail_safe(|cx| {
        let s = cx.signal(3);
        label(text!("{}", s.get()))
    });
    assert!(matches!(r, Err(UiError::Engine(EngineError::InvalidConfig(_)))));
    assert_eq!(
        ui.engine().screens(ui.display()).len(),
        screens,
        "the screen was removed"
    );
    ui.enter_fail_safe(1); // nothing registered: logged, nothing changes
    assert_eq!(ui.fail_safe(), None);
}

/// The value signal and the label of the validity test.
type Parts = (Signal<Validity<u16>>, NodeRef<twine_widgets::label::Label>);

#[test]
fn validity_texts_and_looks() {
    let clock = MockClock::new();
    let cell: Rc<RefCell<Option<Parts>>> = Rc::default();
    let c2 = cell.clone();
    let mut ui = ui(&clock, move |cx| {
        let v = cx.signal(Validity::Valid(42u16));
        let r = cx.node_ref();
        *c2.borrow_mut() = Some((v, r));
        label(v).validity(v).node_ref(r)
    });
    ui.update();
    let (v, r) = cell.borrow_mut().take().unwrap();
    let n = r.get_untracked().unwrap();
    assert_eq!(text_of(&ui, n), "42");
    assert!(!state_of(&ui, n).intersects(State::STALE | State::INVALID));
    v.set(Validity::Stale {
        value: 41,
        age: Duration::ms(900),
    });
    ui.update();
    assert_eq!(text_of(&ui, n), "41");
    assert!(state_of(&ui, n).contains(State::STALE));
    v.set(Validity::Failed);
    ui.update();
    assert_eq!(text_of(&ui, n), INVALID_TEXT);
    let s = state_of(&ui, n);
    assert!(s.contains(State::INVALID) && !s.contains(State::STALE));
    v.set(Validity::Valid(7));
    ui.update();
    assert_eq!(text_of(&ui, n), "7");
    assert!(!state_of(&ui, n).intersects(State::STALE | State::INVALID));
}

static SPEED: Latest<Validity<u16>> = Latest::new(Validity::NoData);

#[test]
fn watch_fresh_ages_values_on_the_clock() {
    let clock = MockClock::new();
    let seen: Rc<RefCell<Option<ReadSignal<Validity<u16>>>>> = Rc::default();
    let s2 = seen.clone();
    let mut ui = ui(&clock, move |cx| {
        let v = cx.watch_fresh(&SPEED, Duration::ms(500));
        *s2.borrow_mut() = Some(v);
        label(v).validity(v)
    });
    let v = seen.borrow_mut().take().unwrap();
    ui.update();
    assert_eq!(v.get(), Validity::NoData);
    SPEED.set(Validity::Valid(88));
    ui.update();
    assert_eq!(v.get(), Validity::Valid(88));
    // Fresh values keep it valid.
    for i in 0..3u16 {
        clock.advance(Duration::ms(300));
        SPEED.set(Validity::Valid(90 + i));
        ui.update();
        assert_eq!(v.get(), Validity::Valid(90 + i));
    }
    // No new value for longer than 500 ms: stale, with its age; and it keeps aging.
    clock.advance(Duration::ms(520));
    ui.update();
    let Validity::Stale { value, age } = v.get() else {
        panic!("stale expected, got {:?}", v.get());
    };
    assert_eq!(value, 92);
    assert!(age >= Duration::ms(500), "{age}");
    clock.advance(Duration::ms(500));
    ui.update();
    assert!(matches!(v.get(), Validity::Stale { age, .. } if age >= Duration::ms(1000)));
    // A failure reported by the producer, then a fresh value again.
    SPEED.set(Validity::Failed);
    ui.update();
    assert_eq!(v.get(), Validity::Failed);
    SPEED.set(Validity::Valid(1));
    ui.update();
    assert_eq!(v.get(), Validity::Valid(1));
    SPEED.set(Validity::NoData);
}

#[derive(Clone, Default)]
struct Chunks(Rc<RefCell<(u32, u32)>>);

impl FrameObserver for Chunks {
    fn chunk(&mut self, _: &FrameChunk<'_>) {
        self.0.borrow_mut().0 += 1;
    }
    fn frame_done(&mut self, r: &FrameReport) {
        let mut s = self.0.borrow_mut();
        assert_eq!(u32::from(r.stats.chunks), s.0);
        s.0 = 0;
        s.1 += 1;
    }
}

#[test]
fn frame_observer_registered_at_build_and_critical_label_cross_check() {
    let clock = MockClock::new();
    let obs = Chunks::default();
    let cell: Rc<RefCell<Option<NodeRef<twine_widgets::label::Label>>>> = Rc::default();
    let c2 = cell.clone();
    let panel = MemoryDisplay::new(DisplayInfo::new(120, 60, ColorFormat::Rgb565));
    let mut ui = Ui::builder(panel)
        .runtime(Runtime::take().unwrap())
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 20 }))
        .limits(limits())
        .frame_observer(obs.clone())
        .theme(&lvgl::THEME)
        .try_build(move |cx| {
            let r = cx.node_ref();
            *c2.borrow_mut() = Some(r);
            column((
                label("normal"),
                label("88 km/h").safety_critical(true).node_ref(r),
            ))
        })
        .unwrap();
    ui.update();
    assert_eq!(obs.0.borrow().1, 1, "one frame observed");
    let n = cell.borrow_mut().take().unwrap().get_untracked().unwrap();
    let e = ui.engine_mut();
    assert_eq!(e.critical_nodes().collect::<Vec<_>>(), [n]);
    let report = e.critical_report(n).unwrap();
    assert_eq!(report.value, Crc32::of(b"88 km/h"));
    assert!(report.drawn_frame.is_some());
    let mut buf = vec![0u8; report.region.unwrap().buffer_bytes];
    let check = e.check_region(n, &mut buf).unwrap();
    assert!(
        check.ok(),
        "the text re-rendered without the glyph cache matches the panel"
    );
}
