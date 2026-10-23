//! The 10-band graphic equalizer: its state, its presets with eased transitions, and the
//! math of its frequency response.
//!
//! **State.** [`Equalizer`] is a `Copy` store of signals: one gain per band (tenths of a dB,
//! ±12 dB, so the sliders bind to them two-way), the selected preset and an on/off switch.
//! Dragging a slider writes its band directly and marks the preset "Custom". Choosing a
//! preset does not jump: it starts a [`Transition`] that the player's frame clock advances
//! ([`Equalizer::advance`]), moving every band along an ease-in-out curve over
//! [`TRANSITION_MS`], so the sliders, the response curve and the sound glide together.
//!
//! **Response.** Each band is a *peaking* filter (a bell around its centre frequency), the
//! standard building block of graphic equalizers; [`band_db`] evaluates one exactly and
//! [`response_db`] sums them. The curve on the equalizer page and the simulated audio (which
//! boosts or cuts each analyzer band by the response at its frequency) both use this one
//! function, so what you see is what the visualizer hears.

// Frequencies and gains are small numbers; f32 is exact enough for them.
#![allow(clippy::cast_precision_loss)]

use std::f64::consts::PI;

use twine::prelude::*;

/// Number of bands.
pub const BANDS: usize = 10;
/// Centre frequencies of the bands, one octave apart (the ISO graphic-equalizer series).
pub const FREQS: [f32; BANDS] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// The band labels under the sliders.
pub const LABELS: [&str; BANDS] = ["31", "62", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"];
/// The gain range of a band, in tenths of a dB (±12 dB).
pub const RANGE: i32 = 120;
/// How long a preset change takes to settle.
pub const TRANSITION_MS: u32 = 450;
/// The sample rate the filters are designed for (the simulated output's).
const SAMPLE_RATE: f64 = 48_000.0;
/// Bandwidth of each bell. For octave-spaced bands, Q = √2 makes neighbouring bells cross at
/// about half their gain, so a row of equal sliders gives a smooth plateau.
const Q: f64 = std::f64::consts::SQRT_2;

/// The presets' band gains (tenths of a dB), in the order of [`PRESET_NAMES`].
pub const PRESETS: [[i32; BANDS]; 9] = [
    [0; BANDS],                                  // Flat
    [50, 35, 20, -10, -25, -10, 15, 35, 45, 50], // Rock: scooped mids
    [30, 20, 10, 20, -15, -15, 0, 15, 30, 35],   // Jazz: warm lows, airy top
    [-30, -25, -10, 15, 40, 50, 40, 20, 0, -15], // Vocal: the presence range forward
    [90, 75, 50, 25, 5, 0, 0, 0, 0, 0],          // Bass Boost
    [55, 45, 10, 0, -20, 15, 5, 15, 45, 55],     // Electronic: the "smile"
    [35, 30, 20, 10, 15, 15, 25, 30, 25, 15],    // Acoustic
    [0, 0, 0, 0, 0, 10, 30, 55, 75, 90],         // Treble Boost
    [-20, -10, 0, 10, 20, 20, 10, 0, -20, -40],  // Late Night: soft lows and highs
];

/// The dropdown's options: the presets and "Custom" (shown once the user has moved a slider
/// by hand).
pub const PRESET_NAMES: [&str; PRESETS.len() + 1] = [
    "Flat",
    "Rock",
    "Jazz",
    "Vocal",
    "Bass Boost",
    "Electronic",
    "Acoustic",
    "Treble Boost",
    "Late Night",
    "Custom",
];
/// The index of "Custom" in [`PRESET_NAMES`].
pub const CUSTOM: usize = PRESETS.len();

/// A preset change in progress: every band moves from `from` to `to`.
#[derive(Clone, Copy, Debug)]
struct Transition {
    from: [i32; BANDS],
    to: [i32; BANDS],
    elapsed_ms: u32,
}

/// The equalizer's state (`Copy`: the page, the player and the audio share it).
#[derive(Clone, Copy)]
pub struct Equalizer {
    /// The gain of each band in tenths of a dB, as shown (and heard) right now.
    pub gains: [Signal<i32>; BANDS],
    /// The selected entry of [`PRESET_NAMES`].
    pub preset: Signal<usize>,
    /// Whether the equalizer is applied.
    pub enabled: Signal<bool>,
    transition: Signal<Option<Transition>>,
}

impl Equalizer {
    pub fn new(cx: Scope) -> Self {
        Self {
            gains: std::array::from_fn(|_| cx.signal(0)),
            preset: cx.signal(0),
            enabled: cx.signal(true),
            transition: cx.signal(None),
        }
    }

    /// Starts gliding to preset `index` (ignored for "Custom", which keeps the gains).
    pub fn choose(self, index: usize) {
        let Some(&to) = PRESETS.get(index) else {
            return;
        };
        self.preset.set(index);
        self.transition.set(Some(Transition {
            from: self.gains.map(|g| g.get_untracked()),
            to,
            elapsed_ms: 0,
        }));
    }

    /// The user moved band `band` by hand: stop any transition, show "Custom".
    pub fn edited(self, band: usize, value: i32) {
        self.transition.set(None);
        if self.gains[band].get_untracked() != value {
            self.gains[band].set(value);
        }
        if self.preset.get_untracked() != CUSTOM {
            self.preset.set(CUSTOM);
        }
    }

    /// Advances a running transition by `dt_ms` (called once per frame by the player clock).
    pub fn advance(self, dt_ms: u32) {
        let Some(mut tr) = self.transition.get_untracked() else {
            return;
        };
        tr.elapsed_ms = (tr.elapsed_ms + dt_ms).min(TRANSITION_MS);
        let t = ease_in_out(tr.elapsed_ms as f32 / TRANSITION_MS as f32);
        for (i, g) in self.gains.iter().enumerate() {
            let v = tr.from[i] + ((tr.to[i] - tr.from[i]) as f32 * t).round() as i32;
            if g.get_untracked() != v {
                g.set(v);
            }
        }
        self.transition.set((tr.elapsed_ms < TRANSITION_MS).then_some(tr));
    }

    /// The gains in dB, untracked (for the audio simulation); all zero when switched off.
    pub fn gains_db(self) -> [f32; BANDS] {
        if self.enabled.get_untracked() {
            self.gains.map(|g| g.get_untracked() as f32 / 10.0)
        } else {
            [0.0; BANDS]
        }
    }
}

/// Cubic ease-in-out on `t ∈ [0, 1]`: slow start, fast middle, slow landing.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// The gain in dB at frequency `f` of one peaking filter centred on `f0` with gain `gain_db`.
///
/// The filter is the "peakingEQ" biquad of Robert Bristow-Johnson's *Audio EQ Cookbook*:
///
/// ```text
///   A  = 10^(gain/40)              (square root of the linear gain)
///   w0 = 2π·f0/fs                  (centre frequency in radians per sample)
///   α  = sin(w0) / (2Q)            (bandwidth)
///
///   H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (a0 + a1·z⁻¹ + a2·z⁻²)
///   b0 = 1 + α·A    b1 = −2·cos(w0)    b2 = 1 − α·A
///   a0 = 1 + α/A    a1 = −2·cos(w0)    a2 = 1 − α/A
/// ```
///
/// Its magnitude at frequency `f` comes from evaluating `H` on the unit circle,
/// `z = e^{jw}` with `w = 2π·f/fs`. For real coefficients `c0, c1, c2`,
///
/// ```text
///   |c0 + c1·e^{−jw} + c2·e^{−2jw}|² = c0² + c1² + c2² + 2·(c0·c1 + c1·c2)·cos(w) + 2·c0·c2·cos(2w)
/// ```
///
/// so `|H|² = N/D` with that expression for the `b` (numerator `N`) and `a` (denominator `D`)
/// coefficients, and the gain is `10·log10(N/D)` dB. At `f = f0` it is exactly `gain_db`; far
/// from `f0` it falls to 0 dB; a negative gain mirrors the bell into a notch.
///
/// Evaluated in `f64`: for low bands `w0` is tiny, the coefficients approach `(1, −2, 1)` and
/// `N`, `D` are differences of nearly equal numbers (about 10⁻⁶ of their terms at 31 Hz) —
/// `f32` would lose every significant digit there.
pub fn band_db(f0: f32, gain_db: f32, f: f32) -> f32 {
    if gain_db == 0.0 {
        return 0.0;
    }
    let a = 10f64.powf(f64::from(gain_db) / 40.0);
    let w0 = 2.0 * PI * f64::from(f0) / SAMPLE_RATE;
    let alpha = w0.sin() / (2.0 * Q);
    let cos_w0 = w0.cos();
    let (b0, b1, b2) = (1.0 + alpha * a, -2.0 * cos_w0, 1.0 - alpha * a);
    let (a0, a1, a2) = (1.0 + alpha / a, -2.0 * cos_w0, 1.0 - alpha / a);
    let w = 2.0 * PI * f64::from(f) / SAMPLE_RATE;
    let (c1, c2) = (w.cos(), (2.0 * w).cos());
    let power = |x0: f64, x1: f64, x2: f64| {
        x0 * x0 + x1 * x1 + x2 * x2 + 2.0 * (x0 * x1 + x1 * x2) * c1 + 2.0 * x0 * x2 * c2
    };
    (10.0 * (power(b0, b1, b2) / power(a0, a1, a2)).log10()) as f32
}

/// The equalizer's total gain in dB at frequency `f`. The bands are in series (the signal
/// passes through every filter), so their linear gains multiply — which is a *sum* in dB.
pub fn response_db(gains_db: &[f32; BANDS], f: f32) -> f32 {
    FREQS
        .iter()
        .zip(gains_db)
        .map(|(&f0, &g)| band_db(f0, g, f))
        .sum()
}

/// Number of points of the response curve.
pub const CURVE_POINTS: usize = 97;
/// The curve's frequency span: 20 Hz to 20 kHz, the range of hearing (log-spaced).
pub const CURVE_LO_HZ: f32 = 20.0;
pub const CURVE_HI_HZ: f32 = 20_000.0;

/// The frequency of curve point `i` (logarithmic axis: equal steps are equal ratios).
pub fn curve_hz(i: usize) -> f32 {
    let t = i as f32 / (CURVE_POINTS - 1) as f32;
    CURVE_LO_HZ * (CURVE_HI_HZ / CURVE_LO_HZ).powf(t)
}

/// What the response graph draws: the curve in tenths of a dB at each of the
/// [`CURVE_POINTS`] frequencies, the band gains (for the handles on the curve) and whether
/// the equalizer is on. Computed by a memo whenever a gain changes — up to once a frame
/// during a preset transition, never while the gains rest — so the widget only maps numbers
/// to pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Response {
    pub curve: [i16; CURVE_POINTS],
    pub gains: [i16; BANDS],
    pub enabled: bool,
}

impl Default for Response {
    fn default() -> Self {
        Self::of([0; BANDS], true)
    }
}

impl Response {
    /// The response of `gains` (tenths of a dB).
    pub fn of(gains: [i32; BANDS], enabled: bool) -> Self {
        let db = gains.map(|g| g as f32 / 10.0);
        Self {
            curve: std::array::from_fn(|i| (response_db(&db, curve_hz(i)) * 10.0).round() as i16),
            gains: gains.map(|g| g as i16),
            enabled,
        }
    }
}

/// The response of the equalizer as a memo (re-run when a gain or the switch changes).
pub fn response_memo(cx: Scope, eq: Equalizer) -> Memo<Response> {
    cx.memo(move || Response::of(eq.gains.map(|g| g.get()), eq.enabled.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_peaks_at_its_centre() {
        assert!((band_db(1000.0, 6.0, 1000.0) - 6.0).abs() < 0.01);
        assert!(band_db(1000.0, 6.0, 20.0).abs() < 0.1);
        assert!((band_db(1000.0, -6.0, 1000.0) + 6.0).abs() < 0.01);
    }
}
