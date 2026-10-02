//! Power and rotation at run time through the `Ui` (R3.S07): `Ui::set_rotation` lays out and
//! draws exactly what a `Ui` built in that rotation shows and fits touch to it; display
//! commands arrive through a channel from another thread; the idle timeout reaches the run
//! loop as `LoopEvent::Inactive`.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use twine_core::ColorFormat;
use twine_drivers::NoPin;
use twine_drivers::testkit::Recorder;
use twine_drivers::touch::Ft6x36;
use twine_engine::EngineConfig;
use twine_hal::DisplayInfo;
use twine_testing::{MemoryDisplay, MockClock, MockPlatform};
use twine_view::prelude::*;
use twine_view::run::{self, LoopEvent};

/// A screen whose layout depends on its size: a title, a 50 %-wide bar at the bottom.
fn scene(_cx: Scope) -> impl View {
    container((
        label("Rotate").test_id("title").align(Align::TopMid).pos(0, 4),
        container(())
            .width(Length::pct(50))
            .height(20)
            .align(Align::BottomRight)
            .test_id("bar"),
    ))
    .fill()
}

fn ui(panel: MemoryDisplay, clock: &MockClock) -> Ui {
    Ui::builder(panel)
        .runtime(Runtime::current_thread())
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 16 }))
        .theme(DefaultTheme::light())
        .reserve_rotation()
        .build(scene)
}

fn pixels(ui: &Ui) -> Vec<u8> {
    ui.engine()
        .driver::<MemoryDisplay>(ui.display())
        .unwrap()
        .to_rgb888()
}

/// Updates until idle, advancing the clock to every deadline (frames are at most one per
/// `refr_period`).
fn settle(ui: &mut Ui, clock: &MockClock) {
    for _ in 0..20 {
        match ui.update() {
            Wake::At(t) => clock.set(t),
            Wake::Now => {}
            Wake::Idle | Wake::IdleFor(_) => return,
        }
    }
    panic!("not idle");
}

#[test]
fn set_rotation_matches_a_ui_built_in_that_rotation() {
    for (hw, rot) in [
        (true, Rotation::Deg90),
        (true, Rotation::Deg270),
        (false, Rotation::Deg90),
        (false, Rotation::Deg180),
    ] {
        let mut native = MemoryDisplay::new(DisplayInfo::new(160, 120, ColorFormat::Rgb565));
        if hw {
            native = native.with_rotation_control();
        }
        let clock = MockClock::new();
        let mut rotated = ui(native, &clock);
        settle(&mut rotated, &clock);
        clock.advance(Duration::ms(100));
        rotated.set_rotation(rot).unwrap();
        settle(&mut rotated, &clock);
        let info = rotated.display_info();
        assert_eq!(
            (info.rotation, info.hw_rotation),
            (rot, hw && rot != Rotation::Deg0)
        );

        let (w, h) = if rot.swaps_axes() { (120, 160) } else { (160, 120) };
        let built = DisplayInfo::new(w, h, ColorFormat::Rgb565)
            .with_rotation(rot)
            .with_hw_rotation(hw);
        let clock = MockClock::new();
        let mut reference = ui(MemoryDisplay::new(built), &clock);
        settle(&mut reference, &clock);
        assert_eq!(rotated.display_info().width, reference.display_info().width);
        let (a, b) = (pixels(&rotated), pixels(&reference));
        let pw = rotated
            .engine()
            .driver::<MemoryDisplay>(rotated.display())
            .unwrap()
            .panel_size()
            .0 as usize;
        let diffs: Vec<(usize, usize)> = (0..a.len() / 3)
            .filter(|i| a[i * 3..i * 3 + 3] != b[i * 3..i * 3 + 3])
            .map(|i| (i % pw, i / pw))
            .collect();
        assert!(
            diffs.is_empty(),
            "hw {hw}, {rot:?}: {} px differ, first {:?} last {:?}",
            diffs.len(),
            diffs.first(),
            diffs.last()
        );
        assert!(rotated.take_faults().is_empty());
    }
}

/// Answers the `FT6x36`'s 5-byte status read with `regs`.
fn ft_bus(regs: Rc<RefCell<[u8; 5]>>) -> Recorder {
    let rec = Recorder::new();
    rec.set_i2c_responder(move |_addr, _written, rx| {
        let r = regs.borrow();
        for (b, v) in rx.iter_mut().zip(r.iter()) {
            *b = *v;
        }
    });
    rec
}

fn tap(ui: &mut Ui, clock: &MockClock, regs: &Rc<RefCell<[u8; 5]>>, raw: (u8, u8)) {
    *regs.borrow_mut() = [0x01, 0x80, raw.0, 0x00, raw.1];
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
    *regs.borrow_mut() = [0; 5];
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
}

#[test]
fn touch_maps_to_the_new_rotation() {
    let regs = Rc::new(RefCell::new([0u8; 5]));
    let rec = ft_bus(regs.clone());
    let clock = MockClock::new();
    let rt = Runtime::current_thread();
    let (landscape_hit, portrait_hit) = (rt.create_root().signal(0u32), rt.create_root().signal(0u32));
    // A 240 × 320 panel, built in portrait. Native (10, 20) is logical (10, 20) in portrait and
    // logical (299, 10) in landscape (`Deg90`).
    let panel = MemoryDisplay::new(DisplayInfo::new(240, 320, ColorFormat::Rgb565)).with_rotation_control();
    let mut ui = Ui::builder(panel)
        .runtime(rt)
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .input(Ft6x36::new(rec.i2c(), None::<NoPin>))
        .build(move |_| {
            container((
                button(label(""))
                    .pos(290, 0)
                    .size(20, 20)
                    .on_click(move || landscape_hit.update(|n| *n += 1)),
                button(label(""))
                    .pos(0, 10)
                    .size(20, 20)
                    .on_click(move || portrait_hit.update(|n| *n += 1)),
            ))
            .fill()
        });
    tap(&mut ui, &clock, &regs, (10, 20));
    assert_eq!((portrait_hit.get(), landscape_hit.get()), (1, 0), "portrait");
    ui.set_rotation(Rotation::Deg90).unwrap();
    let _ = ui.update();
    assert_eq!((ui.display_info().width, ui.display_info().height), (320, 240));
    tap(&mut ui, &clock, &regs, (10, 20));
    assert_eq!(
        (portrait_hit.get(), landscape_hit.get()),
        (1, 1),
        "landscape after rotation"
    );
}

#[test]
fn display_commands_from_another_thread() {
    static COMMANDS: Channel<DisplayCmd, 4> = Channel::new();
    let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565))
        .with_power_control()
        .with_rotation_control();
    let mut ui = Ui::builder(panel)
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .display_commands(&COMMANDS)
        .build(|_| label("hi"));
    let _ = ui.update();
    std::thread::spawn(|| {
        COMMANDS
            .try_send(DisplayCmd::Brightness(Fraction::pct(10)))
            .unwrap();
        COMMANDS.try_send(DisplayCmd::Rotate(Rotation::Deg90)).unwrap();
        COMMANDS.try_send(DisplayCmd::Sleep).unwrap();
    })
    .join()
    .unwrap();
    assert!(ui.waker().is_set(), "a send wakes the UI");
    let _ = ui.update();
    assert_eq!(
        ui.engine().display_brightness(ui.display()),
        Some(Fraction::pct(10))
    );
    assert_eq!(ui.display_info().rotation, Rotation::Deg90);
    assert!(ui.display_asleep());
    let panel = ui.engine().driver::<MemoryDisplay>(ui.display()).unwrap();
    assert!(panel.is_asleep());
    assert_eq!(panel.brightness(), Some(Fraction::pct(10)));
    COMMANDS.try_send(DisplayCmd::Wake).unwrap();
    let _ = ui.update();
    assert!(!ui.display_asleep());
    assert!(ui.take_faults().is_empty());
}

/// The payload that ends a run loop.
struct Stop;

#[test]
fn idle_timeout_reaches_the_run_loop_as_inactive() {
    let mut platform = MockPlatform::new();
    let ui =
        Ui::builder(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control())
            .runtime(Runtime::current_thread())
            .platform(&platform)
            .config(EngineConfig {
                idle_timeout: Some(Duration::secs(30)),
                ..EngineConfig::default()
            })
            .buffers(BufferMode::alloc(BufferSpec::default()))
            .build(|_| label("hi"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let result = catch_unwind(AssertUnwindSafe(|| -> () {
        run::blocking_with(ui, &mut platform, |ui, event| {
            seen.borrow_mut().push(event);
            if let LoopEvent::Inactive(d) = event {
                assert_eq!(d, Duration::secs(30));
                assert_eq!(ui.inactive_for(), Duration::secs(30));
                // The policy is the application's: e.g. put the display to sleep.
                ui.set_display_sleep(true);
            }
            if event == LoopEvent::Woken && ui.display_asleep() {
                resume_unwind(Box::new(Stop));
            }
        })
    }));
    match result {
        Err(p) if p.is::<Stop>() => {}
        Err(p) => resume_unwind(p),
        Ok(()) => unreachable!(),
    }
    let events = events.borrow();
    // The first idle wait has the timeout as its deadline (one wake-up, no polling).
    assert!(
        events
            .iter()
            .any(|e| matches!(e, LoopEvent::Idle(Some(t)) if *t == Instant::from_millis(30_000)))
    );
    assert!(events.iter().any(|e| matches!(e, LoopEvent::Inactive(_))));
}
