//! The calibration demo: target geometry, the 3-point solution, the raw-touch wrapper and the
//! whole flow through `TestUi`, each test with its own raw-tap channel (ports pattern).

use twine::core::Point;
use twine::core::{ColorFormat, Rotation};
use twine::hal::{DisplayInfo, TouchTransform};
use twine::hal::{InputData, InputDevice, InputKind, PointerData};
use twine_demos::calibration::{RawTaps, RawTouch, RawTouchInput, app, solve, targets, verify_targets};
use twine_testing::{TestUi, by_id};

/// Raw reading of screen point `(x, y)` for a panel mounted with swapped, mirrored axes (like a
/// landscape XPT2046 module): raw = f(screen).
fn raw_of(x: i32, y: i32) -> (i32, i32) {
    (3900 - y * 3700 / 240, 200 + x * 3700 / 320)
}

/// The `TestUi` display: 320 × 240 at `Deg0` (native = logical).
fn landscape() -> DisplayInfo {
    DisplayInfo::new(320, 240, ColorFormat::Rgb565)
}

#[test]
fn calibration_survives_rotation() {
    // Calibrated on a 240 × 320 panel turned to landscape (Deg90): the calibration maps to
    // native pixels, so after rotating the display (any rotation) a tap still lands on the
    // logical point drawn over the same physical spot.
    let info90 = DisplayInfo::new(320, 240, ColorFormat::Rgb565).with_rotation(Rotation::Deg90);
    let to_native = TouchTransform::for_display(&info90).inverse();
    // A touch film reading 10 × the native pixels + 100.
    let raw_at = |x: i32, y: i32| {
        let n = to_native.apply(x, y);
        (n.x * 10 + 100, n.y * 10 + 100)
    };
    let cal = solve(targets(320, 240).map(|(x, y)| raw_at(x, y)), &info90).unwrap();
    for rot in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        let (w, h) = if rot.swaps_axes() { (320, 240) } else { (240, 320) };
        let info = DisplayInfo::new(w, h, ColorFormat::Rgb565).with_rotation(rot);
        let screen = TouchTransform::for_display(&info);
        for (x, y) in verify_targets(320, 240) {
            let (rx, ry) = raw_at(x, y);
            let native = to_native.apply(x, y);
            let (nx, ny) = cal.apply(rx, ry);
            assert_eq!(
                screen.apply(nx, ny),
                screen.apply(native.x, native.y),
                "{rot:?}: the physical spot of ({x}, {y})"
            );
        }
    }
}

#[test]
fn calibration_math_from_demo_targets() {
    let t = targets(320, 240);
    assert_eq!(t, [(32, 24), (288, 120), (160, 216)]);
    assert_eq!(verify_targets(320, 240)[2], (160, 120));
    let raw = t.map(|(x, y)| raw_of(x, y));
    let c = solve(raw, &landscape()).expect("non-degenerate");
    // Every target and verification point maps back within ±1 px.
    for (x, y) in t.into_iter().chain(verify_targets(320, 240)) {
        let (rx, ry) = raw_of(x, y);
        let (cx, cy) = c.apply(rx, ry);
        assert!(
            (cx - x).abs() <= 1 && (cy - y).abs() <= 1,
            "({x}, {y}) -> ({cx}, {cy})"
        );
    }
    assert!(
        solve([(5, 5); 3], &landscape()).is_none(),
        "degenerate readings are rejected"
    );
}

/// A touch device replaying scripted readings; once fitted to a display, mapped like a
/// capacitive driver (swap/mirror/clamp).
struct Script(Vec<Option<(i32, i32)>>, Option<TouchTransform>);

impl InputDevice for Script {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }
    fn read(&mut self) -> InputData {
        let r = if self.0.is_empty() { None } else { self.0.remove(0) };
        InputData::Pointer(match r {
            Some((x, y)) => PointerData {
                point: self.1.map_or(Point::new(x, y), |t| t.apply(x, y)),
                pressed: true,
            },
            None => PointerData::default(),
        })
    }
    fn fit_to_display(&mut self, info: &DisplayInfo) {
        self.1 = Some(TouchTransform::for_display(info));
    }
}

#[test]
fn raw_touch_wrapper_sends_averaged_taps_to_its_channel() {
    let raw: &'static RawTaps = TestUi::channel();
    // The wrapper averages one tap and sends it on release; the engine only sees the origin.
    let mut w = RawTouchInput::new(Script(vec![Some((100, 200)), Some((102, 204)), None], None), raw);
    assert!(matches!(w.read(), InputData::Pointer(p) if p.pressed && p.point == Point::new(0, 0)));
    let _ = w.read();
    assert!(matches!(w.read(), InputData::Pointer(p) if !p.pressed));
    assert_eq!(raw.try_recv(), Some(RawTouch { x: 101, y: 202 }));
    assert_eq!(raw.try_recv(), None);
}

#[test]
fn raw_touch_wrapper_keeps_the_driver_unfitted() {
    // Raw readings must not be clamped or rotated: the wrapper does not forward the fit.
    let raw: &'static RawTaps = TestUi::channel();
    let script = Script(vec![Some((4000, 3900)), None], None);
    let mut w = RawTouchInput::new(script, raw);
    w.fit_to_display(&DisplayInfo::new(320, 240, twine::core::ColorFormat::Rgb565));
    let _ = w.read();
    let _ = w.read();
    assert_eq!(
        raw.try_recv(),
        Some(RawTouch { x: 4000, y: 3900 }),
        "not clamped to 320 × 240"
    );
}

#[test]
fn calibration_flow_through_its_own_channel() {
    let raw: &'static RawTaps = TestUi::channel();
    // The demo: three raw taps → the calibration is shown; then verification errors.
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(move |cx| app(cx, raw));
    assert_eq!(t.find(by_id("calibration")).text(), "Tap the center of the cross");
    t.assert_snapshot("calibration_first_target");
    for (x, y) in targets(320, 240) {
        let (rx, ry) = raw_of(x, y);
        raw.try_send(RawTouch { x: rx, y: ry }).unwrap();
        t.run_until_idle();
    }
    let text = t.find(by_id("calibration")).text();
    assert!(text.starts_with("Calibration { a: "), "{text}");
    assert_eq!(t.find_all(by_id("target")).len(), 5, "verification page");
    t.assert_snapshot("calibration_verify");
    let expected = solve(targets(320, 240).map(|(x, y)| raw_of(x, y)), &landscape()).unwrap();
    assert!(text.contains(&format!("div: {}", expected.div)));
    // A tap 3 px right of the center target.
    let (rx, ry) = raw_of(163, 120);
    raw.try_send(RawTouch { x: rx, y: ry }).unwrap();
    t.run_until_idle();
    let (x, y) = expected.apply(rx, ry);
    let text = t.find(by_id("calibration")).text();
    assert_eq!(
        text,
        format!("Tap at {x}, {y}: error {}, {} px", x - 160, y - 120)
    );
    assert!((x - 163).abs() <= 1 && (y - 120).abs() <= 1);
}
