//! Rotary encoder on two GPIOs (quadrature A/B) plus an optional push button:
//! [`QuadratureDecoder`] and [`GpioQuadrature`] (`InputKind::Encoder`).
//!
//! # Decoding
//!
//! [`QuadratureDecoder`] runs the standard Gray-code state machine on every edge: of the 16
//! `(previous, current)` A/B combinations, the 8 valid single-bit steps count ±1, everything
//! else (no change, or both lines changed = a missed or bouncing edge) counts 0. Contact bounce
//! on one line therefore cancels out (+1 −1). Steps are accumulated and reported in detents
//! (`steps_per_detent`, default 4 for the common EC11-style encoders).
//!
//! The decoder keeps its state in atomics (`portable-atomic`: plain loads/stores plus a
//! critical-section `fetch_add` on targets without CAS such as the RP2040), so it can live in a
//! `static` and be fed from a GPIO interrupt while the UI task reads it.
//!
//! # Usage
//!
//! Polled (call [`update`](GpioQuadrature::update) often enough not to miss edges — ≥ 1 kHz
//! for hand-turned encoders; `read()` calls it too):
//!
//! ```
//! # use core::cell::Cell;
//! # use twine_core::Instant;
//! # use twine_hal::Clock;
//! # struct Now(Cell<Instant>);
//! # impl Clock for Now { fn now(&self) -> Instant { self.0.get() } }
//! use twine_drivers::encoder::{GpioQuadrature, QuadratureDecoder};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{InputData, InputDevice};
//!
//! let rec = Recorder::new();
//! let clock = Now(Cell::new(Instant::from_millis(0)));
//! let decoder = QuadratureDecoder::new(4);
//! let mut enc = GpioQuadrature::new(&decoder, rec.quiet_pin("a"), rec.quiet_pin("b"), Some(rec.quiet_pin("btn")), &clock);
//! rec.set_level("btn", true); // released (active low)
//! assert!(matches!(enc.read(), InputData::Encoder(e) if e.diff == 0 && !e.pressed));
//! ```
//!
//! Interrupt driven (RP2040 example wiring: A = GPIO 14, B = GPIO 15, button = GPIO 13, all
//! with pull-ups, common pin to GND): put the decoder in a `static`, call
//! `DECODER.on_edge(a.is_high(), b.is_high())` from the GPIO interrupt of both lines (any
//! edge), and give the UI a [`GpioQuadrature::new_isr`] reader:
//!
//! ```
//! # use core::cell::Cell;
//! # use twine_core::Instant;
//! # use twine_hal::Clock;
//! # struct Now(Cell<Instant>);
//! # impl Clock for Now { fn now(&self) -> Instant { self.0.get() } }
//! # let clock = Now(Cell::new(Instant::from_millis(0)));
//! # let rec = twine_drivers::testkit::Recorder::new();
//! # let button = rec.quiet_pin("btn");
//! use twine_drivers::encoder::{GpioQuadrature, QuadratureDecoder};
//! use twine_hal::{InputData, InputDevice};
//!
//! static DECODER: QuadratureDecoder = QuadratureDecoder::new(4);
//!
//! /// GPIO interrupt handler of A and B (any edge), with the levels read in the handler.
//! fn on_encoder_edge(a: bool, b: bool) {
//!     DECODER.on_edge(a, b);
//! }
//!
//! // UI side:
//! let mut encoder = GpioQuadrature::new_isr(&DECODER, Some(button), &clock);
//! for (a, b) in [(true, false), (true, true), (false, true), (false, false)] {
//!     on_encoder_edge(a, b); // one detent clockwise
//! }
//! assert!(matches!(encoder.read(), InputData::Encoder(e) if e.diff == 1));
//! ```

use embedded_hal::digital::InputPin;
use portable_atomic::{AtomicI16, AtomicU8, Ordering};
use twine_core::Duration;
use twine_core::log::trace;
use twine_hal::{Clock, EncoderData, InputData, InputDevice, InputKind};

use crate::NoPin;
use crate::debounce::Debouncer;

// NOTE(P21.S05): the RP2040 firmware gains an optional `input-encoder` feature (A = GPIO 14,
// B = GPIO 15, button = GPIO 13) once the firmware crates exist.

/// Step per `(previous << 2) | current` A/B state (state = `A << 1 | B`); invalid → 0.
/// Clockwise (A leads B) is `00 → 10 → 11 → 01 → 00`.
const STEP: [i8; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];

/// Lock-free quadrature state machine (see the module docs). Safe to share between an
/// interrupt handler (`on_edge`) and the UI (`take`).
#[derive(Debug)]
pub struct QuadratureDecoder {
    state: AtomicU8,
    steps: AtomicI16,
    steps_per_detent: u8,
}

impl QuadratureDecoder {
    /// A decoder reporting one detent every `steps_per_detent` valid steps (≥ 1; 4 for most
    /// encoders, 2 or 1 for half/quarter-step types). Starts in state `00`.
    #[must_use]
    pub const fn new(steps_per_detent: u8) -> Self {
        Self {
            state: AtomicU8::new(0),
            steps: AtomicI16::new(0),
            steps_per_detent: if steps_per_detent == 0 {
                1
            } else {
                steps_per_detent
            },
        }
    }

    /// Feeds the current levels of A and B (call on every edge of either line).
    pub fn on_edge(&self, a: bool, b: bool) {
        let cur = u8::from(a) << 1 | u8::from(b);
        let prev = self.state.load(Ordering::Relaxed);
        if cur == prev {
            return;
        }
        self.state.store(cur, Ordering::Relaxed);
        let step = STEP[usize::from(prev << 2 | cur)];
        if step != 0 {
            self.steps.fetch_add(i16::from(step), Ordering::Relaxed);
        }
    }

    /// Takes the whole detents turned since the last call (positive = clockwise); partial
    /// steps are kept.
    pub fn take(&self) -> i16 {
        let spd = i16::from(self.steps_per_detent);
        let detents = self.steps.load(Ordering::Relaxed) / spd;
        if detents != 0 {
            self.steps.fetch_sub(detents * spd, Ordering::Relaxed);
        }
        detents
    }
}

/// Rotary encoder with optional push button, implementing [`InputDevice`] (`Encoder`).
///
/// The button is active low by default (switch to GND with pull-up) and debounced with the
/// clock given at construction (default 20 ms).
#[derive(Debug)]
pub struct GpioQuadrature<'d, A, B, BTN, C> {
    decoder: &'d QuadratureDecoder,
    a: A,
    b: B,
    poll_pins: bool,
    btn: Option<BTN>,
    btn_active_high: bool,
    button: Debouncer,
    clock: C,
}

impl<'d, A: InputPin, B: InputPin, BTN: InputPin, C: Clock> GpioQuadrature<'d, A, B, BTN, C> {
    /// A polled encoder: [`update`](Self::update) (and `read`) sample A and B.
    #[must_use]
    pub fn new(decoder: &'d QuadratureDecoder, a: A, b: B, btn: Option<BTN>, clock: C) -> Self {
        Self {
            decoder,
            a,
            b,
            poll_pins: true,
            btn,
            btn_active_high: false,
            button: Debouncer::default(),
            clock,
        }
    }
}

impl<'d, BTN: InputPin, C: Clock> GpioQuadrature<'d, NoPin, NoPin, BTN, C> {
    /// An encoder whose A/B edges are fed to `decoder` from an interrupt handler
    /// ([`QuadratureDecoder::on_edge`]); `update` only samples the button.
    #[must_use]
    pub fn new_isr(decoder: &'d QuadratureDecoder, btn: Option<BTN>, clock: C) -> Self {
        Self {
            decoder,
            a: NoPin,
            b: NoPin,
            poll_pins: false,
            btn,
            btn_active_high: false,
            button: Debouncer::default(),
            clock,
        }
    }
}

impl<A: InputPin, B: InputPin, BTN: InputPin, C: Clock> GpioQuadrature<'_, A, B, BTN, C> {
    /// Sets the button debounce time.
    #[must_use]
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.button = Debouncer::new(debounce);
        self
    }

    /// Treats the button as active high (switch to VCC with pull-down).
    #[must_use]
    pub fn with_button_active_high(mut self, active_high: bool) -> Self {
        self.btn_active_high = active_high;
        self
    }

    /// Samples A/B (polled mode) and the button. Call from a periodic task (≥ 1 kHz when
    /// polling A/B) or rely on `read`.
    pub fn update(&mut self) {
        if self.poll_pins {
            // A read error counts as "low"; the state machine ignores the invalid transitions
            // this may cause.
            let a = self.a.is_high().unwrap_or(false);
            let b = self.b.is_high().unwrap_or(false);
            self.decoder.on_edge(a, b);
        }
        if let Some(btn) = self.btn.as_mut() {
            let raw = btn.is_high().is_ok_and(|high| high == self.btn_active_high);
            self.button.update(raw, self.clock.now());
        }
    }

    /// Returns the pins and the clock.
    #[must_use]
    pub fn release(self) -> (A, B, Option<BTN>, C) {
        (self.a, self.b, self.btn, self.clock)
    }
}

impl<A: InputPin, B: InputPin, BTN: InputPin, C: Clock> InputDevice for GpioQuadrature<'_, A, B, BTN, C> {
    fn kind(&self) -> InputKind {
        InputKind::Encoder
    }

    /// Returns the detents since the previous read and the debounced button state.
    fn read(&mut self) -> InputData {
        self.update();
        let diff = self.decoder.take();
        if diff != 0 {
            trace!(target: "twine::driver", "encoder diff {}", diff);
        }
        InputData::Encoder(EncoderData {
            diff,
            pressed: self.button.level(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Recorder, RecordingPin};
    use core::cell::Cell;
    use twine_core::Instant;

    struct TestClock(Cell<Instant>);
    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.0.get()
        }
    }

    const CW: [(bool, bool); 4] = [(true, false), (true, true), (false, true), (false, false)];

    fn feed(d: &QuadratureDecoder, seq: impl IntoIterator<Item = (bool, bool)>) {
        for (a, b) in seq {
            d.on_edge(a, b);
        }
    }

    #[test]
    fn quadrature_cw_sequence_positive_diff() {
        let d = QuadratureDecoder::new(4);
        feed(&d, CW);
        feed(&d, CW);
        assert_eq!(d.take(), 2);
        assert_eq!(d.take(), 0);
        // Half a detent is kept for later.
        feed(&d, CW[..2].iter().copied());
        assert_eq!(d.take(), 0);
        feed(&d, CW[2..].iter().copied());
        assert_eq!(d.take(), 1);
    }

    #[test]
    fn quadrature_ccw_negative() {
        let d = QuadratureDecoder::new(4);
        let ccw = CW.iter().rev().skip(1).copied().chain([(false, false)]);
        feed(&d, ccw);
        assert_eq!(d.take(), -1);
        let d1 = QuadratureDecoder::new(1);
        feed(&d1, [(false, true), (true, true)]);
        assert_eq!(d1.take(), -2);
    }

    #[test]
    fn quadrature_bounce_ignored() {
        let d = QuadratureDecoder::new(4);
        // A bounces between 0 and 1 (single-line chatter cancels out).
        feed(&d, [(true, false), (false, false), (true, false), (false, false)]);
        assert_eq!(d.steps.load(Ordering::Relaxed), 0);
        // Invalid transitions (both lines change) count nothing.
        feed(&d, [(true, true), (false, false), (true, true)]);
        assert_eq!(d.steps.load(Ordering::Relaxed), 0);
        assert_eq!(d.take(), 0);
        assert_eq!(QuadratureDecoder::new(0).steps_per_detent, 1);
    }

    #[test]
    fn polled_encoder_reads_pins() {
        let rec = Recorder::new();
        let clock = TestClock(Cell::new(Instant::from_millis(0)));
        let dec = QuadratureDecoder::new(4);
        let mut e = GpioQuadrature::new(
            &dec,
            rec.quiet_pin("a"),
            rec.quiet_pin("b"),
            None::<RecordingPin>,
            &clock,
        );
        for (a, b) in CW {
            rec.set_level("a", a);
            rec.set_level("b", b);
            e.update();
        }
        assert_eq!(
            e.read(),
            InputData::Encoder(EncoderData {
                diff: 1,
                pressed: false
            })
        );
        assert_eq!(e.kind(), InputKind::Encoder);
        let _ = e.release();
    }

    #[test]
    fn button_debounce() {
        let rec = Recorder::new();
        let clock = TestClock(Cell::new(Instant::from_millis(0)));
        let dec = QuadratureDecoder::new(4);
        let mut e =
            GpioQuadrature::new_isr(&dec, Some(rec.quiet_pin("btn")), &clock).with_debounce(Duration::ms(20));
        let pressed =
            |e: &mut GpioQuadrature<'_, _, _, _, _>| matches!(e.read(), InputData::Encoder(d) if d.pressed);
        rec.set_level("btn", true);
        assert!(!pressed(&mut e));
        // Bouncing contact: low for 5 ms, high, low …
        for (t, level) in [(1, false), (6, true), (8, false), (12, true), (15, false)] {
            clock.0.set(Instant::from_millis(t));
            rec.set_level("btn", level);
            assert!(!pressed(&mut e), "t = {t}");
        }
        clock.0.set(Instant::from_millis(34));
        assert!(!pressed(&mut e));
        clock.0.set(Instant::from_millis(35));
        assert!(pressed(&mut e));
        // ISR-fed rotation.
        feed(&dec, CW);
        assert_eq!(
            e.read(),
            InputData::Encoder(EncoderData {
                diff: 1,
                pressed: true
            })
        );
        // Active-high button.
        let mut e =
            GpioQuadrature::new_isr(&dec, Some(rec.quiet_pin("btn")), &clock).with_button_active_high(true);
        rec.set_level("btn", true);
        clock.0.set(Instant::from_millis(100));
        let _ = e.read();
        clock.0.set(Instant::from_millis(200));
        assert!(pressed(&mut e));
    }
}
