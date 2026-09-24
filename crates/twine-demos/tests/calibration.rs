//! The calibration demo: target geometry, the 3-point solution, the raw-touch wrapper and the
//! whole flow through `TestUi` with a mock raw-touch channel.

use twine::core::Point;
use twine::hal::{InputData, InputDevice, InputKind, PointerData};
use twine_demos::calibration::{RAW, RawTouch, RawTouchInput, app, solve, targets, verify_targets};
use twine_testing::{TestUi, by_id};

/// Raw reading of screen point `(x, y)` for a panel mounted with swapped, mirrored axes (like a
/// landscape XPT2046 module): raw = f(screen).
fn raw_of(x: i32, y: i32) -> (i32, i32) {
    (3900 - y * 3700 / 240, 200 + x * 3700 / 320)
}

#[test]
fn calibration_math_from_demo_targets() {
    let t = targets(320, 240);
    assert_eq!(t, [(32, 24), (288, 120), (160, 216)]);
    assert_eq!(verify_targets(320, 240)[2], (160, 120));
    let raw = t.map(|(x, y)| raw_of(x, y));
    let c = solve(raw, 320, 240).expect("non-degenerate");
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
        solve([(5, 5); 3], 320, 240).is_none(),
        "degenerate readings are rejected"
    );
}

/// A touch device replaying scripted readings.
struct Script(Vec<Option<(i32, i32)>>);

impl InputDevice for Script {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }
    fn read(&mut self) -> InputData {
        let r = if self.0.is_empty() { None } else { self.0.remove(0) };
        InputData::Pointer(match r {
            Some((x, y)) => PointerData {
                point: Point::new(x, y),
                pressed: true,
            },
            None => PointerData::default(),
        })
    }
}

#[test]
fn calibration_flow_and_raw_touch_wrapper() {
    // The wrapper averages one tap and sends it on release; the engine only sees the origin.
    let mut w = RawTouchInput::new(Script(vec![Some((100, 200)), Some((102, 204)), None]));
    assert!(matches!(w.read(), InputData::Pointer(p) if p.pressed && p.point == Point::new(0, 0)));
    let _ = w.read();
    assert!(matches!(w.read(), InputData::Pointer(p) if !p.pressed));
    assert_eq!(RAW.try_recv(), Some(RawTouch { x: 101, y: 202 }));
    assert_eq!(RAW.try_recv(), None);

    // The demo: three raw taps → the calibration is shown; then verification errors.
    let mut t = TestUi::new(320, 240).mount(app);
    assert_eq!(t.find(by_id("calibration")).text(), "Tap the center of the cross");
    t.assert_snapshot("calibration_first_target");
    for (x, y) in targets(320, 240) {
        let (rx, ry) = raw_of(x, y);
        RAW.try_send(RawTouch { x: rx, y: ry }).unwrap();
        t.run_until_idle();
    }
    let text = t.find(by_id("calibration")).text();
    assert!(text.starts_with("Calibration { a: "), "{text}");
    assert_eq!(t.find_all(by_id("target")).len(), 5, "verification page");
    t.assert_snapshot("calibration_verify");
    let expected = solve(targets(320, 240).map(|(x, y)| raw_of(x, y)), 320, 240).unwrap();
    assert!(text.contains(&format!("div: {}", expected.div)));
    // A tap 3 px right of the center target.
    let (rx, ry) = raw_of(163, 120);
    RAW.try_send(RawTouch { x: rx, y: ry }).unwrap();
    t.run_until_idle();
    let (x, y) = expected.apply(rx, ry);
    let text = t.find(by_id("calibration")).text();
    assert_eq!(
        text,
        format!("Tap at {x}, {y}: error {}, {} px", x - 160, y - 120)
    );
    assert!((x - 163).abs() <= 1 && (y - 120).abs() <= 1);
}
