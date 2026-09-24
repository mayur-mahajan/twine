//! Touch calibration for resistive touch screens (XPT2046, STMPE811): three crosshair targets
//! (10 %/10 %, 90 %/50 %, 50 %/90 %) give a 3-point affine [`Calibration`], which is logged
//! (`twine::calibrate`) so it can be pasted into the firmware; a verification page with five
//! targets then shows the error of every tap in pixels.
//!
//! The demo reads **raw** touch coordinates from [`RAW`]: wrap the touch driver — configured
//! with [`Calibration::IDENTITY`] and a large screen size so nothing is clamped — in
//! [`RawTouchInput`], which sends one averaged raw point per tap to [`RAW`] and never clicks the
//! UI.
//!
//! ```
//! use twine_demos::calibration::{RAW, RawTouch, app, targets};
//! use twine_testing::{TestUi, by_id};
//!
//! let mut t = TestUi::new(320, 240).mount(app);
//! // A touch panel whose raw axes are 10 × the screen's, offset by 100.
//! for (x, y) in targets(320, 240) {
//!     RAW.try_send(RawTouch { x: x * 10 + 100, y: y * 10 + 100 }).unwrap();
//!     t.run_until_idle();
//! }
//! assert!(t.find(by_id("calibration")).text().contains("Calibration"));
//! ```

use core::cell::RefCell;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use twine::hal::{Calibration, InputData, InputDevice, InputKind, PointerData, PollHint};
use twine::prelude::*;

/// A raw touch point (controller units).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTouch {
    /// Raw X.
    pub x: i32,
    /// Raw Y.
    pub y: i32,
}

/// Raw taps, sent by [`RawTouchInput`] (or by any other context).
pub static RAW: Channel<RawTouch, 4> = Channel::new();

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

/// The calibration from raw readings of the three [`targets`] of a `w × h` screen (`None` for
/// degenerate readings, e.g. the same point three times).
#[must_use]
pub fn solve(raw: [(i32, i32); 3], w: i32, h: i32) -> Option<Calibration> {
    Calibration::from_points(raw, targets(w, h))
}

/// Size of a crosshair.
const CROSS: i32 = 21;

/// A crosshair centered on `(x, y)` (reactive).
fn crosshair(x: impl Fn() -> i32 + Clone + 'static, y: impl Fn() -> i32 + Clone + 'static) -> impl View {
    let (x2, y2) = (x.clone(), y.clone());
    container((
        container(())
            .size(CROSS, 1)
            .pos(0, CROSS / 2)
            .bg(Color::hex(0xE5_39_35))
            .border(0, Color::BLACK)
            .radius(0)
            .padding(0),
        container(())
            .size(1, CROSS)
            .pos(CROSS / 2, 0)
            .bg(Color::hex(0xE5_39_35))
            .border(0, Color::BLACK)
            .radius(0)
            .padding(0),
    ))
    .size(CROSS, CROSS)
    .pos(move || x2() - CROSS / 2, move || y2() - CROSS / 2)
    .bg_opa(Opa::TRANSP)
    .border(0, Color::BLACK)
    .padding(0)
    .test_id("target")
}

/// The calibration application; logs the result.
pub fn app(cx: Scope) -> impl View {
    app_with(cx, |c| {
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

/// The calibration application; `on_done` receives the calibration (and it is shown).
pub fn app_with(cx: Scope, on_done: fn(Calibration)) -> impl View {
    let (w, h) = twine::view::EngineAccess::with(|e| {
        e.default_display()
            .and_then(|d| e.display_info(d))
            .map_or((320, 240), |i| (i32::from(i.width), i32::from(i.height)))
    })
    .unwrap_or((320, 240));
    let step = cx.signal(0usize); // 0..3: calibration targets; 3: verification
    let cal = cx.signal(None::<Calibration>);
    let message = cx.signal(String::from("Tap the center of the cross"));
    let raw: Rc<RefCell<[(i32, i32); 3]>> = Rc::new(RefCell::new([(0, 0); 3]));
    cx.on_message(&RAW, move |p: RawTouch| {
        let s = step.get();
        if s < 3 {
            raw.borrow_mut()[s] = (p.x, p.y);
            if s == 2 {
                if let Some(c) = solve(*raw.borrow(), w, h) {
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
            let (x, y) = c.apply(p.x, p.y);
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
                .size(Length::Pct(100), Length::Pct(100))
                .bg_opa(Opa::TRANSP)
                .border(0, Color::BLACK)
                .padding(0)
            },
        ),
    ))
    .size(Length::Pct(100), Length::Pct(100))
    .padding(0)
    .radius(0)
    .border(0, Color::BLACK)
    .bg(Color::WHITE)
    .scroll_dir(Dir::NONE)
}

/// Wraps a touch driver that reports raw coordinates: every tap is averaged and sent to
/// [`RAW`] on release. The engine sees the press state (so it keeps reading while pressed) at
/// the screen's origin, where the calibration screen has nothing clickable.
#[derive(Debug)]
pub struct RawTouchInput<T> {
    inner: T,
    sum: (i64, i64, i64),
}

impl<T> RawTouchInput<T> {
    /// Wraps `inner` (identity calibration, no clamping).
    pub const fn new(inner: T) -> Self {
        Self {
            inner,
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
                let _ = RAW.try_send(raw);
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
}

#[cfg(feature = "async")]
impl<T: twine::hal::AsyncInputWait> twine::hal::AsyncInputWait for RawTouchInput<T> {
    async fn wait_for_interrupt(&mut self) {
        self.inner.wait_for_interrupt().await;
    }
}
