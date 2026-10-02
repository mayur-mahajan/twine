//! Touch calibration for resistive touch screens (XPT2046, STMPE811): three crosshair targets
//! (10 %/10 %, 90 %/50 %, 50 %/90 %) give a 3-point affine [`Calibration`] to the display's
//! **native** (unrotated) pixels — so it stays valid in every rotation, also after a rotation
//! at run time — which is logged
//! (`twine::calibrate`) so it can be pasted into the firmware; a verification page with five
//! targets then shows the error of every tap in pixels.
//!
//! The demo reads **raw** touch coordinates from a [`RawTaps`] channel the caller owns (the
//! ports pattern: `main` declares the `static`, passes `&RAW` to the input wrapper and to
//! [`app`]; tests use `TestUi::channel()`). Wrap the touch driver — configured with
//! [`Calibration::IDENTITY`] — in [`RawTouchInput`], which sends one averaged raw point per tap
//! to the channel and never clicks the UI. The wrapper does not fit the driver to the display,
//! so its readings stay raw (not clamped to the screen).
//!
//! ```
//! use twine_demos::calibration::{RawTaps, RawTouch, app, targets};
//! use twine_testing::{TestUi, by_id};
//!
//! // Firmware: `static RAW: RawTaps = RawTaps::new();` next to the touch setup, then
//! // `RawTouchInput::new(touch, &RAW)` and `build(|cx| app(cx, &RAW))`.
//! let raw: &'static RawTaps = TestUi::channel();
//! let mut t = TestUi::new(320, 240).mount(move |cx| app(cx, raw));
//! // A touch panel whose raw axes are 10 × the screen's, offset by 100.
//! for (x, y) in targets(320, 240) {
//!     raw.try_send(RawTouch { x: x * 10 + 100, y: y * 10 + 100 }).unwrap();
//!     t.run_until_idle();
//! }
//! assert!(t.find(by_id("calibration")).text().contains("Calibration"));
//! ```

use core::cell::RefCell;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use twine::hal::{
    Calibration, DisplayInfo, InputData, InputDevice, InputKind, PointerData, PollHint, TouchTransform,
};
use twine::prelude::*;

/// A raw touch point (controller units).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTouch {
    /// Raw X.
    pub x: i32,
    /// Raw Y.
    pub y: i32,
}

/// The channel of raw taps from [`RawTouchInput`] (or any other context) to [`app`]. The
/// application owns it (a `static` in firmware, `TestUi::channel()` in tests) and passes a
/// reference to both sides.
///
/// ```
/// use twine_demos::calibration::{RawTaps, RawTouch};
///
/// static RAW: RawTaps = RawTaps::new();
/// RAW.try_send(RawTouch { x: 1200, y: 3400 }).unwrap();
/// assert_eq!(RAW.try_recv(), Some(RawTouch { x: 1200, y: 3400 }));
/// ```
pub type RawTaps = Channel<RawTouch, 4>;

/// The three calibration targets of a `w × h` screen: (10 %, 10 %), (90 %, 50 %), (50 %, 90 %).
#[must_use]
pub const fn targets(w: i32, h: i32) -> [(i32, i32); 3] {
    [(w / 10, h / 10), (w * 9 / 10, h / 2), (w / 2, h * 9 / 10)]
}

/// The five verification targets: the corners at 15 % and the center.
#[must_use]
pub const fn verify_targets(w: i32, h: i32) -> [(i32, i32); 5] {
    let (x0, x1, y0, y1) = (w * 15 / 100, w * 85 / 100, h * 15 / 100, h * 85 / 100);
    [(x0, y0), (x1, y0), (w / 2, h / 2), (x0, y1), (x1, y1)]
}

/// The calibration from raw readings of the three [`targets`] of the display `info` (`None`
/// for degenerate readings, e.g. the same point three times). The targets are drawn at
/// logical positions; the calibration maps to the **native** panel pixels under them
/// (`TouchTransform::for_display(info).inverse()`), which is what the touch drivers expect.
///
/// ```
/// use twine::core::{ColorFormat, Rotation};
/// use twine::hal::{DisplayInfo, TouchTransform};
/// use twine_demos::calibration::{solve, targets};
///
/// // A landscape display (240 × 320 panel turned 90°) and a touch film reading native ×10.
/// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565).with_rotation(Rotation::Deg90);
/// let native = TouchTransform::for_display(&info).inverse();
/// let raw = targets(320, 240).map(|(x, y)| {
///     let p = native.apply(x, y);
///     (p.x * 10, p.y * 10)
/// });
/// let cal = solve(raw, &info).unwrap();
/// let (nx, ny) = cal.apply(raw[0].0, raw[0].1);
/// assert_eq!(TouchTransform::for_display(&info).apply(nx, ny), twine::core::Point::new(32, 24));
/// ```
#[must_use]
pub fn solve(raw: [(i32, i32); 3], info: &DisplayInfo) -> Option<Calibration> {
    let native = TouchTransform::for_display(info).inverse();
    let t = targets(i32::from(info.width), i32::from(info.height)).map(|(x, y)| {
        let p = native.apply(x, y);
        (p.x, p.y)
    });
    Calibration::from_points(raw, t)
}

/// Size of a crosshair.
const CROSS: i32 = 21;

// The looks, composed once at compile time (in flash) instead of repeating the modifiers on
// every node: a bare rectangle without the theme's card border, rounding and padding, and its
// variants.

/// A bare rectangle: no border, rounding or padding.
static PLAIN: Style = style! { border: (0, Color::BLACK), radius: 0, padding: 0 };
/// A bar of a crosshair.
static BAR: Style = style! { ..PLAIN, bg: Color::hex(0xE5_39_35) };
/// A transparent layer (a crosshair's box, the verification targets).
static LAYER: Style = style! { ..PLAIN, bg_opacity: Opa::TRANSP };
/// The calibration screen: white (a measurement screen, fixed high contrast).
static SCREEN: Style = style! { ..PLAIN, bg: Color::WHITE };

/// A crosshair centered on `(x, y)` (reactive).
fn crosshair(x: impl Fn() -> i32 + Clone + 'static, y: impl Fn() -> i32 + Clone + 'static) -> impl View {
    let (x2, y2) = (x.clone(), y.clone());
    container((
        container(()).size(CROSS, 1).pos(0, CROSS / 2).style(&BAR),
        container(()).size(1, CROSS).pos(CROSS / 2, 0).style(&BAR),
    ))
    .size(CROSS, CROSS)
    .pos(move || x2() - CROSS / 2, move || y2() - CROSS / 2)
    .style(&LAYER)
    .test_id("target")
}

/// The calibration application reading raw taps from `raw`; logs the result (see the
/// [module documentation](self) for an example). Never panics.
pub fn app(cx: Scope, raw: &'static RawTaps) -> impl View {
    app_with(cx, raw, |c| {
        twine::core::info!(
            target: "twine::calibrate",
            "Calibration {{ a: {}, b: {}, c: {}, d: {}, e: {}, f: {}, div: {} }}",
            c.a,
            c.b,
            c.c,
            c.d,
            c.e,
            c.f,
            c.div
        );
    })
}

/// The calibration application; `on_done` receives the calibration (and it is shown). It is a
/// closure (it may capture e.g. a signal or a storage handle), `FnMut` because it is called
/// from the touch-message handler, which may run more than once.
/// The display (size and rotation) is read from the engine's default display (320 × 240 at
/// `Deg0` when there is none).
/// Raw touch points arrive through `raw` (see the [module documentation](self)). Never
/// panics.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
/// use twine_demos::calibration::{RawTaps, app_with};
/// use twine_testing::{TestUi, by_id};
///
/// let done = Rc::new(Cell::new(false));
/// let d = done.clone();
/// let raw: &'static RawTaps = TestUi::channel();
/// // E.g. store the calibration in flash instead of logging it.
/// let mut t = TestUi::new(320, 240).mount(move |cx| app_with(cx, raw, move |_cal| d.set(true)));
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("calibration")).text(), "Tap the center of the cross");
/// assert!(!done.get()); // no taps yet
/// ```
pub fn app_with(
    cx: Scope,
    raw: &'static RawTaps,
    mut on_done: impl FnMut(Calibration) + 'static,
) -> impl View {
    let fallback = DisplayInfo::new(320, 240, twine::core::ColorFormat::Rgb565);
    let info = twine::view::EngineAccess::with(cx, |e| e.default_display().and_then(|d| e.display_info(d)))
        .flatten()
        .unwrap_or(fallback);
    let (w, h) = (i32::from(info.width), i32::from(info.height));
    let to_screen = TouchTransform::for_display(&info);
    let step = cx.signal(0usize); // 0..3: calibration targets; 3: verification
    let cal = cx.signal(None::<Calibration>);
    let message = cx.signal(String::from("Tap the center of the cross"));
    let readings: Rc<RefCell<[(i32, i32); 3]>> = Rc::new(RefCell::new([(0, 0); 3]));
    cx.on_message(raw, move |p: RawTouch| {
        let s = step.get();
        if s < 3 {
            readings.borrow_mut()[s] = (p.x, p.y);
            if s == 2 {
                if let Some(c) = solve(*readings.borrow(), &info) {
                    on_done(c);
                    cal.set(Some(c));
                    message.set(format!(
                        "Calibration {{ a: {}, b: {}, c: {}, d: {}, e: {}, f: {}, div: {} }}",
                        c.a, c.b, c.c, c.d, c.e, c.f, c.div
                    ));
                    step.set(3);
                } else {
                    message.set("Degenerate points, again from the start".into());
                    step.set(0);
                }
            } else {
                step.set(s + 1);
            }
        } else if let Some(c) = cal.get() {
            // Verification: the error to the nearest target.
            let (nx, ny) = c.apply(p.x, p.y);
            let Point { x, y } = to_screen.apply(nx, ny);
            let (tx, ty) = verify_targets(w, h)
                .into_iter()
                .min_by_key(|(tx, ty)| (tx - x).pow(2) + (ty - y).pow(2))
                .unwrap_or((0, 0));
            message.set(format!("Tap at {x}, {y}: error {}, {} px", x - tx, y - ty));
        }
    });
    let tx = move || targets(w, h)[step.get().min(2)].0;
    let ty = move || targets(w, h)[step.get().min(2)].1;
    container((
        label(text!("{}", message.get()))
            .width(Length::Pct(90))
            .text_align(TextAlign::Center)
            .test_id("calibration")
            .align(Align::Center)
            .pos(0, 44),
        when(move || step.get() < 3, move |_| crosshair(tx, ty)),
        when(
            move || step.get() >= 3,
            move |_| {
                let t = verify_targets(w, h);
                container((
                    crosshair(move || t[0].0, move || t[0].1),
                    crosshair(move || t[1].0, move || t[1].1),
                    crosshair(move || t[2].0, move || t[2].1),
                    crosshair(move || t[3].0, move || t[3].1),
                    crosshair(move || t[4].0, move || t[4].1),
                ))
                .fill()
                .style(&LAYER)
            },
        ),
    ))
    .fill()
    .style(&SCREEN)
    .scrollable(false)
}

/// Wraps a touch driver that reports raw coordinates: every tap is averaged and sent to the
/// [`RawTaps`] channel on release. The engine sees the press state (so it keeps reading while
/// pressed) at the screen's origin, where the calibration screen has nothing clickable.
///
/// It deliberately does **not** forward
/// [`fit_to_display`](twine::hal::InputDevice::fit_to_display): the wrapped driver is never
/// fitted to the display, so its readings stay raw (not clamped or rotated). A full channel
/// drops the tap (the user taps again).
#[derive(Debug)]
pub struct RawTouchInput<T> {
    inner: T,
    raw: &'static RawTaps,
    sum: (i64, i64, i64),
}

impl<T> RawTouchInput<T> {
    /// Wraps `inner` (configure it with [`Calibration::IDENTITY`]); taps go to `raw`. Never
    /// panics.
    ///
    /// ```
    /// use twine::core::Point;
    /// use twine::hal::{InputData, InputDevice, InputKind, PointerData};
    /// use twine_demos::calibration::{RawTaps, RawTouch, RawTouchInput};
    ///
    /// struct Held;
    /// impl InputDevice for Held {
    ///     fn kind(&self) -> InputKind { InputKind::Pointer }
    ///     fn read(&mut self) -> InputData {
    ///         InputData::Pointer(PointerData { point: Point::new(1200, 3400), pressed: true })
    ///     }
    /// }
    ///
    /// static RAW: RawTaps = RawTaps::new();
    /// let mut touch = RawTouchInput::new(Held, &RAW);
    /// assert!(matches!(touch.read(), InputData::Pointer(p) if p.pressed && p.point == Point::new(0, 0)));
    /// assert_eq!(RAW.try_recv(), None); // sent on release
    /// ```
    pub const fn new(inner: T, raw: &'static RawTaps) -> Self {
        Self {
            inner,
            raw,
            sum: (0, 0, 0),
        }
    }
}

impl<T: InputDevice> InputDevice for RawTouchInput<T> {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        let pressed = match self.inner.read() {
            InputData::Pointer(p) if p.pressed => {
                self.sum = (
                    self.sum.0 + i64::from(p.point.x),
                    self.sum.1 + i64::from(p.point.y),
                    self.sum.2 + 1,
                );
                true
            }
            _ if self.sum.2 > 0 => {
                let n = self.sum.2;
                let raw = RawTouch {
                    x: (self.sum.0 / n) as i32,
                    y: (self.sum.1 / n) as i32,
                };
                self.sum = (0, 0, 0);
                twine::core::info!(target: "twine::calibrate", "raw tap {} {} ({} samples)", raw.x, raw.y, n);
                if self.raw.try_send(raw).is_err() {
                    twine::core::warn!(target: "twine::calibrate", "raw tap dropped (channel full)");
                }
                false
            }
            _ => false,
        };
        InputData::Pointer(PointerData {
            point: Point::new(0, 0),
            pressed,
        })
    }

    fn poll_hint(&self) -> PollHint {
        self.inner.poll_hint()
    }

    fn rearm(&mut self) {
        self.inner.rearm();
    }

    fn health(&self) -> twine::hal::DeviceHealth {
        self.inner.health()
    }
}

#[cfg(feature = "async")]
impl<T: twine::hal::AsyncInputWait> twine::hal::AsyncInputWait for RawTouchInput<T> {
    async fn wait_for_interrupt(&mut self) {
        self.inner.wait_for_interrupt().await;
    }
}
