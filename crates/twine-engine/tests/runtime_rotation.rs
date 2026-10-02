//! Power and rotation at run time (R3.S07): rotation through the driver (hardware) or in
//! software, re-checked like a new display and refused when it does not fit; brightness and
//! sleep on a mock driver; inactivity and the idle timeout; no allocation for a rotation with
//! the same buffers.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{boxed, white_screen};
use twine_core::fault::FaultKind;
use twine_core::{Color, ColorFormat, Duration, Fraction, Instant, Point, Rect, Rotation};
use twine_engine::{
    BufferMode, BufferSpec, DisplayControlFault, DisplayId, Engine, EngineConfig, NodeId, Wake,
};
use twine_hal::{
    ControlError, DisplayDriver, DisplayInfo, DrawBufferMem, InputData, InputDevice, InputKind, PointerData,
    PollHint,
};
use twine_style::{Align, Length};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, leak_buffer};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const W: u16 = 64;
const H: u16 = 32;

fn t(ms: u64) -> Instant {
    Instant::from_millis(ms)
}

/// An asymmetric scene whose layout depends on the screen size: a fixed box, and a box of
/// 50 % × 25 % of the screen at the bottom right.
fn scene(e: &mut Engine) -> NodeId {
    let s = white_screen(e);
    boxed(e, s, Rect::from_xywh(2, 3, 12, 6), Color::RED);
    let b = boxed(e, s, Rect::from_xywh(0, 0, 1, 1), Color::BLUE);
    e.set_size(b, Length::pct(50), Length::pct(25));
    e.align(b, Align::BottomRight, 0, 0);
    b
}

fn engine(panel: impl DisplayDriver + 'static, buffers: BufferMode) -> (Engine, DisplayId, NodeId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e.add_display(panel, buffers).unwrap();
    let b = scene(&mut e);
    e.step(t(0));
    (e, d, b)
}

fn pixels(e: &Engine, d: DisplayId) -> Vec<u8> {
    e.driver::<MemoryDisplay>(d).unwrap().to_rgb888()
}

fn alloc_buffers() -> BufferMode {
    BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 })
}

#[test]
fn hardware_rotation_matches_a_display_built_rotated() {
    for rot in [
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
        Rotation::Deg0,
    ] {
        let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).with_rotation_control();
        let (mut e, d, b) = engine(panel, alloc_buffers());
        // Away and back, so `Deg0` is a real rotation too.
        e.set_rotation(d, Rotation::Deg180).unwrap();
        e.step(t(100));
        e.set_rotation(d, rot).unwrap();
        e.step(t(200));
        let info = e.display_info(d).unwrap();
        assert_eq!((info.rotation, info.hw_rotation), (rot, true));
        assert_eq!(e.driver::<MemoryDisplay>(d).unwrap().display_info(), info);

        let (lw, lh) = if rot.swaps_axes() { (H, W) } else { (W, H) };
        let built = MemoryDisplay::new(
            DisplayInfo::new(lw, lh, ColorFormat::Rgb565)
                .with_rotation(rot)
                .with_hw_rotation(true),
        );
        let (r, rd, rb) = engine(built, alloc_buffers());
        assert_eq!(e.coords(b), r.coords(rb), "{rot:?}: layout");
        assert_eq!(
            e.coords(e.active_screen(d).unwrap()),
            Rect::new(0, 0, i32::from(lw), i32::from(lh))
        );
        assert_eq!(pixels(&e, d), pixels(&r, rd), "{rot:?}: pixels");
        assert!(e.take_faults().is_empty());
    }
}

#[test]
fn unsupported_hardware_rotation_falls_back_to_software() {
    for rot in [Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
        // No rotation control: the driver answers `Unsupported`.
        let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
        let (mut e, d, b) = engine(panel, alloc_buffers());
        e.reserve_rotation(d).unwrap();
        e.set_rotation(d, rot).unwrap();
        e.step(t(100));
        let info = e.display_info(d).unwrap();
        assert_eq!((info.rotation, info.hw_rotation), (rot, false));
        // The panel did not change: flush areas stay native.
        assert_eq!(
            e.driver::<MemoryDisplay>(d).unwrap().display_info().rotation,
            Rotation::Deg0
        );

        let (lw, lh) = if rot.swaps_axes() { (H, W) } else { (W, H) };
        let built = MemoryDisplay::new(DisplayInfo::new(lw, lh, ColorFormat::Rgb565).with_rotation(rot));
        let (r, rd, rb) = engine(built, alloc_buffers());
        assert_eq!(e.coords(b), r.coords(rb), "{rot:?}: layout");
        assert_eq!(pixels(&e, d), pixels(&r, rd), "{rot:?}: pixels (native layout)");
        assert!(e.take_faults().is_empty());
    }
}

#[test]
fn software_rotation_without_reserved_memory_is_refused() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let (mut e, d, _) = engine(panel, alloc_buffers());
    let before = pixels(&e, d);
    e.set_rotation(d, Rotation::Deg90).unwrap();
    e.step(t(100));
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg0, "kept");
    assert!(e.take_faults().contains(FaultKind::DisplayControl));
    let rec = e.last_fault(FaultKind::DisplayControl).unwrap();
    assert_eq!(
        DisplayControlFault::from_code(rec.code),
        Some(DisplayControlFault::RotationNotReserved)
    );
    assert_eq!(rec.display, Some(d));
    assert_eq!(pixels(&e, d), before, "nothing redrawn");
}

/// A panel whose hardware rotation reports a description the engine must refuse.
struct OddPanel {
    inner: MemoryDisplay,
    format_after: ColorFormat,
    rotations: Rc<RefCell<Vec<Rotation>>>,
}

impl DisplayDriver for OddPanel {
    type Error = twine_testing::MemoryDisplayError;
    fn info(&self) -> DisplayInfo {
        self.inner.info()
    }
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        self.inner.begin_flush(area, buf)
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.inner.poll_flush()
    }
    fn set_rotation(&mut self, rotation: Rotation) -> Result<DisplayInfo, ControlError<Self::Error>> {
        self.rotations.borrow_mut().push(rotation);
        let info = self.inner.set_rotation(rotation)?;
        Ok(DisplayInfo {
            format: self.format_after,
            ..info
        })
    }
}

#[test]
fn rotation_checks_the_draw_format_again_and_undoes_a_refused_hardware_rotation() {
    let rotations = Rc::new(RefCell::new(Vec::new()));
    let panel = OddPanel {
        inner: MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).with_rotation_control(),
        // `A8` can never be drawn into.
        format_after: ColorFormat::A8,
        rotations: rotations.clone(),
    };
    let (mut e, d, _) = engine(panel, alloc_buffers());
    e.set_rotation(d, Rotation::Deg90).unwrap();
    e.step(t(100));
    let faults = e.take_faults();
    assert!(faults.contains(FaultKind::FormatDisabled), "{faults:?}");
    assert_eq!(
        DisplayControlFault::from_code(e.last_fault(FaultKind::DisplayControl).unwrap().code),
        Some(DisplayControlFault::RotationRefused)
    );
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg0);
    // The panel was turned back.
    assert_eq!(*rotations.borrow(), [Rotation::Deg90, Rotation::Deg0]);
}

#[test]
fn rotation_that_does_not_fit_the_buffers_is_refused() {
    // One buffer of exactly one 32-px row: a 64-px-wide rotated screen does not fit.
    let panel = MemoryDisplay::new(DisplayInfo::new(H, W, ColorFormat::Rgb565)).with_rotation_control();
    let (mut e, d, _) = engine(
        panel,
        BufferMode::Partial {
            a: leak_buffer(usize::from(H) * 2),
            b: None,
        },
    );
    e.set_rotation(d, Rotation::Deg90).unwrap();
    e.step(t(100));
    assert_eq!(
        DisplayControlFault::from_code(e.last_fault(FaultKind::DisplayControl).unwrap().code),
        Some(DisplayControlFault::RotationRefused)
    );
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg0);
    assert_eq!(
        e.driver::<MemoryDisplay>(d).unwrap().display_info().rotation,
        Rotation::Deg0,
        "undone"
    );
}

#[test]
fn rotation_waits_for_a_flush_in_flight() {
    // A DMA-like panel: the buffer comes back on the 3rd poll.
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565))
        .with_rotation_control()
        .with_latency(3);
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e
        .add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 32 }))
        .unwrap();
    scene(&mut e);
    e.step(t(0)); // one chunk, still in flight
    assert!(e.flush_pending());
    e.set_rotation(d, Rotation::Deg90).unwrap();
    let wake = e.step(t(1));
    if e.flush_pending() {
        assert_eq!(
            e.display_info(d).unwrap().rotation,
            Rotation::Deg0,
            "not while in flight"
        );
        assert!(matches!(wake, Wake::At(_) | Wake::Now), "{wake:?}");
    }
    for ms in 2..10 {
        e.step(t(ms));
    }
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg90);
}

/// A pointer that remembers what it was fitted to and reports a fixed native point mapped
/// like a capacitive driver.
struct Touch {
    fits: Rc<RefCell<Vec<DisplayInfo>>>,
    native: Point,
    pressed: bool,
    transform: twine_hal::TouchTransform,
}

impl InputDevice for Touch {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }
    fn read(&mut self) -> InputData {
        InputData::Pointer(PointerData {
            point: self.transform.apply(self.native.x, self.native.y),
            pressed: self.pressed,
        })
    }
    fn poll_hint(&self) -> PollHint {
        PollHint::Periodic
    }
    fn fit_to_display(&mut self, info: &DisplayInfo) {
        self.fits.borrow_mut().push(*info);
        self.transform = twine_hal::TouchTransform::for_display(info);
    }
}

#[test]
fn input_devices_are_fitted_again_after_a_rotation() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).with_rotation_control();
    let (mut e, d, _) = engine(panel, alloc_buffers());
    let fits = Rc::new(RefCell::new(Vec::new()));
    e.add_input(
        Touch {
            fits: fits.clone(),
            native: Point::new(60, 2),
            pressed: false,
            transform: twine_hal::TouchTransform::PASS_THROUGH,
        },
        d,
    )
    .unwrap();
    e.set_rotation(d, Rotation::Deg90).unwrap();
    e.step(t(100));
    let fits = fits.borrow();
    assert_eq!(fits.len(), 2);
    assert_eq!(fits[1], e.display_info(d).unwrap());
    // Native (60, 2) on a 64 × 32 panel turned 90°: logical (32 − 1 − 2, 60) = (29, 60).
    assert_eq!(
        twine_hal::TouchTransform::for_display(&fits[1]).apply(60, 2),
        Point::new(29, 60)
    );
}

#[test]
fn brightness_and_sleep_on_a_mock_driver() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565))
        .with_power_control()
        .with_sleep_settle(Duration::ms(120));
    let (mut e, d, b) = engine(panel, alloc_buffers());
    e.set_display_brightness(d, Fraction::pct(30)).unwrap();
    e.step(t(100));
    assert_eq!(e.display_brightness(d), Some(Fraction::pct(30)));
    assert_eq!(
        e.driver::<MemoryDisplay>(d).unwrap().brightness(),
        Some(Fraction::pct(30))
    );

    // Asleep: changes are not drawn and do not keep the engine awake.
    e.set_display_sleep(d, true).unwrap();
    e.step(t(200));
    assert!(e.display_asleep(d));
    assert!(e.driver::<MemoryDisplay>(d).unwrap().is_asleep());
    let flushes = e.driver::<MemoryDisplay>(d).unwrap().flushes().len();
    e.set_size(b, Length::pct(40), Length::pct(25));
    // The settle time of `SLPIN` (120 ms here) passes first.
    let mut now = 200;
    let wake = loop {
        now += 50;
        let w = e.step(t(now));
        if now >= 400 {
            break w;
        }
    };
    assert!(wake.is_idle(), "{wake:?}");
    assert_eq!(
        e.driver::<MemoryDisplay>(d).unwrap().flushes().len(),
        flushes,
        "nothing drawn asleep"
    );
    assert_eq!(e.driver::<MemoryDisplay>(d).unwrap().flushes_while_asleep(), 0);

    // Wake: nothing is sent before the panel's settle time, then the change is drawn.
    e.set_display_sleep(d, false).unwrap();
    let wake = e.step(t(1_000));
    assert!(!e.display_asleep(d));
    assert_eq!(wake, Wake::At(t(1_120)));
    assert_eq!(e.driver::<MemoryDisplay>(d).unwrap().flushes().len(), flushes);
    e.step(t(1_120));
    assert!(e.driver::<MemoryDisplay>(d).unwrap().flushes().len() > flushes);
    assert!(e.take_faults().is_empty());
}

#[test]
fn unsupported_controls_raise_display_control_faults() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)); // no controls
    let (mut e, d, _) = engine(panel, alloc_buffers());
    e.set_display_brightness(d, Fraction::HALF).unwrap();
    e.step(t(100));
    assert_eq!(e.display_brightness(d), None);
    let code = e.last_fault(FaultKind::DisplayControl).unwrap().code;
    assert_eq!(
        DisplayControlFault::from_code(code),
        Some(DisplayControlFault::BrightnessUnsupported)
    );
    // A sleep still stops drawing.
    e.set_display_sleep(d, true).unwrap();
    e.step(t(200));
    assert!(e.display_asleep(d));
    let code = e.last_fault(FaultKind::DisplayControl).unwrap().code;
    assert_eq!(
        DisplayControlFault::from_code(code),
        Some(DisplayControlFault::SleepUnsupported)
    );
    // Idempotent requests do nothing.
    assert!(!e.display_requests_pending(d));
    e.set_display_sleep(d, true).unwrap();
    assert!(!e.display_requests_pending(d));
    e.set_rotation(d, Rotation::Deg0).unwrap();
    assert!(!e.display_requests_pending(d));
}

#[test]
fn rotation_with_the_same_buffers_allocates_nothing() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).with_rotation_control();
    let (mut e, d, _) = engine(panel, alloc_buffers());
    // Warm up: the layout pass and the renderer's caches see both geometries once.
    for (i, rot) in [Rotation::Deg90, Rotation::Deg0].into_iter().enumerate() {
        e.set_rotation(d, rot).unwrap();
        e.step(t(100 * (i as u64 + 1)));
    }
    let ((), stats) = count_allocs(|| {
        e.set_rotation(d, Rotation::Deg90).unwrap();
        e.step(t(1_000));
        e.set_rotation(d, Rotation::Deg0).unwrap();
        e.step(t(2_000));
    });
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg0);
    assert_eq!(stats.allocs, 0, "{stats:?}");
}

#[test]
fn software_rotation_after_reserve_allocates_nothing() {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let (mut e, d, _) = engine(panel, alloc_buffers());
    let ((), reserve) = count_allocs(|| e.reserve_rotation(d).unwrap());
    assert!(reserve.allocs > 0, "reserving allocates (once, at start-up)");
    for (i, rot) in [Rotation::Deg270, Rotation::Deg0].into_iter().enumerate() {
        e.set_rotation(d, rot).unwrap();
        e.step(t(100 * (i as u64 + 1)));
    }
    let ((), stats) = count_allocs(|| {
        e.set_rotation(d, Rotation::Deg90).unwrap();
        e.step(t(1_000));
    });
    assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg90);
    assert_eq!(stats.allocs, 0, "{stats:?}");
}

#[test]
fn inactive_for_and_the_idle_timeout() {
    let config = EngineConfig {
        idle_timeout: Some(Duration::secs(5)),
        ..EngineConfig::default()
    };
    let mut e = Engine::new(config).unwrap();
    let d = e
        .add_display(
            MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)),
            alloc_buffers(),
        )
        .unwrap();
    scene(&mut e);
    // Otherwise idle: one wake-up when the timeout passes, no polling before.
    assert_eq!(e.step(t(0)), Wake::At(t(5_000)));
    assert_eq!(e.step(t(2_000)), Wake::At(t(5_000)));
    assert_eq!(e.inactive_for(t(2_000)), Duration::secs(2));
    assert_eq!(e.step(t(5_000)), Wake::IdleFor(Duration::secs(5)));
    assert_eq!(e.step(t(9_000)), Wake::IdleFor(Duration::secs(9)));
    // User input restarts it.
    let fits = Rc::new(RefCell::new(Vec::new()));
    let touch = e
        .add_input(
            Touch {
                fits,
                native: Point::new(10, 10),
                pressed: true,
                transform: twine_hal::TouchTransform::PASS_THROUGH,
            },
            d,
        )
        .unwrap();
    let _ = touch;
    e.step(t(10_000));
    assert_eq!(e.inactive_for(t(10_000)), Duration::ZERO);
    e.trigger_activity(t(20_000));
    assert_eq!(e.inactive_for(t(21_000)), Duration::secs(1));
    // Without an idle timeout nothing changes: `Idle`, no extra wake-up.
    let mut plain = Engine::new(EngineConfig::default()).unwrap();
    plain
        .add_display(
            MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)),
            alloc_buffers(),
        )
        .unwrap();
    plain.step(t(0));
    assert_eq!(plain.step(t(60_000)), Wake::Idle);
}

#[test]
fn framebuffer_displays_cannot_rotate_at_run_time() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let fb = twine_testing::MockFramebufferDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565), true, 0);
    let d = e.add_framebuffer_display(fb, BufferMode::full()).unwrap();
    assert!(matches!(
        e.set_rotation(d, Rotation::Deg90),
        Err(twine_engine::EngineError::InvalidConfig(_))
    ));
}
