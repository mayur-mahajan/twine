//! The visualizer and the level meters: how the simulated sound moves on screen, and how
//! it is drawn.
//!
//! **Ballistics.** [`Analyzer`] turns each frame's raw [`Sound`] into what the eye should
//! see, the way hardware spectrum analyzers do: a bar jumps up quickly (attack smoothed over
//! ~2 frames so it does not flicker) and falls back at a constant rate; above each bar a
//! *peak cap* holds the highest recent value for a moment and then falls with gravity
//! (accelerating). The level meters work the same way with a longer hold. The analyzer runs
//! in the player's frame clock; its output — [`Spectrum`] and [`Levels`], small `Copy`
//! values in per-mille — is written to signals, and only the canvases bound to them redraw.
//!
//! **Drawing.** Two modes of the same data ([`VizMode`]): *bars* — 32 rounded bars whose
//! gradient is fixed to the canvas (violet at the floor, coral, amber at the top, so loud
//! bands glow warmer) with a fading reflection below the baseline; and *radial* — the bands
//! mirrored into 64 spokes around a spinning record whose glow pulses with the bass. Both
//! are lines, rounded rectangles and gradients in integer pixels.

// Levels in dB and per-mille fit easily into f32 / u16.
#![allow(clippy::cast_precision_loss)]

use twine::prelude::*;
use twine::render::GradKind;

use crate::audio::{BANDS, Sound};
use crate::canvas::{Canvas, canvas};
use crate::paint::{
    circle, circle_grad, gradient, gradient_opa, hsv, line, mix, polar, ring, rrect, rrect_grad,
};
use crate::theme::palette::{ACCENT, AMBER, FAINT, GREEN, RAISED, RED, TEXT, VIOLET};

/// How the visualizer draws the spectrum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VizMode {
    /// Vertical bars with peak caps.
    #[default]
    Bars,
    /// Spokes around a spinning record.
    Radial,
}

/// One frame of the visualizer, in per-mille of full height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spectrum {
    pub bars: [u16; BANDS],
    pub peaks: [u16; BANDS],
    /// Low-frequency energy: the radial mode's pulse.
    pub bass: u16,
    /// The record's rotation, in tenths of a degree.
    pub turn: u16,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            bars: [0; BANDS],
            peaks: [0; BANDS],
            bass: 0,
            turn: 0,
        }
    }
}

/// The stereo level meters, in per-mille of their scale (−48…0 dBFS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Levels {
    pub left: u16,
    pub right: u16,
    pub peak_left: u16,
    pub peak_right: u16,
}

/// The level of an empty bar…
const FLOOR_DB: f32 = -48.0;
/// …and of a full one.
const TOP_DB: f32 = 2.0;
/// The meters' scale.
const METER_FLOOR_DB: f32 = -48.0;
/// How long a peak cap stays up before it falls.
const PEAK_HOLD_MS: u32 = 420;
/// Bars fall at this rate (full height per second).
const BAR_FALL: f32 = 1.4;
/// Peak caps accelerate down at this rate (full heights per second²).
const PEAK_GRAVITY: f32 = 3.2;
/// The meters' peak markers hold longer, like a studio meter.
const METER_HOLD_MS: u32 = 1200;

/// One falling value with a held peak above it.
#[derive(Clone, Copy, Debug, Default)]
struct Ballistic {
    value: f32,
    peak: f32,
    hold_ms: u32,
    speed: f32,
}

impl Ballistic {
    /// Moves towards `target` over `dt` ms: fast up, steady down; the peak holds, then falls.
    fn step(&mut self, target: f32, dt: u32, fall: f32, hold: u32) {
        let dt_s = dt as f32 / 1000.0;
        if target > self.value {
            // Exponential attack: about 60 % of the way per 16 ms frame.
            let k = 1.0 - 0.4f32.powf(dt as f32 / 16.0);
            self.value += (target - self.value) * k;
        } else {
            self.value = (self.value - fall * dt_s).max(target);
        }
        if self.value >= self.peak {
            self.peak = self.value;
            self.hold_ms = hold;
            self.speed = 0.0;
        } else if self.hold_ms > dt {
            self.hold_ms -= dt;
        } else {
            self.hold_ms = 0;
            self.speed += PEAK_GRAVITY * dt_s;
            self.peak = (self.peak - self.speed * dt_s).max(self.value);
        }
    }
}

fn permille(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 1000.0).round() as u16
}

/// The visualizer's and meters' memory between frames.
#[derive(Debug, Default)]
pub struct Analyzer {
    bands: [Ballistic; BANDS],
    left: Ballistic,
    right: Ballistic,
    turn: u32,
}

impl Analyzer {
    /// Advances by `dt` ms with this frame's `sound`; `volume_db` scales the meters (the
    /// visualizer shows the signal before the volume control, like most players).
    pub fn step(&mut self, sound: &Sound, volume_db: f32, spinning: bool, dt: u32) -> (Spectrum, Levels) {
        let mut out = Spectrum::default();
        for (i, b) in self.bands.iter_mut().enumerate() {
            let target = (sound.bands[i] - FLOOR_DB) / (TOP_DB - FLOOR_DB);
            b.step(target.clamp(0.0, 1.0), dt, BAR_FALL, PEAK_HOLD_MS);
            out.bars[i] = permille(b.value);
            out.peaks[i] = permille(b.peak);
        }
        out.bass = out.bars[..5].iter().map(|&v| u32::from(v)).max().unwrap_or(0) as u16;
        if spinning {
            // 33⅓ rpm = 200° per second = 2 tenths of a degree per millisecond.
            self.turn = (self.turn + dt * 2) % 3600;
        }
        out.turn = self.turn as u16;
        let meter = |db: f32| ((db + volume_db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        self.left.step(meter(sound.left), dt, 1.6, METER_HOLD_MS);
        self.right.step(meter(sound.right), dt, 1.6, METER_HOLD_MS);
        let levels = Levels {
            left: permille(self.left.value),
            right: permille(self.right.value),
            peak_left: permille(self.left.peak),
            peak_right: permille(self.right.peak),
        };
        (out, levels)
    }

    /// Whether everything has come to rest (nothing left to animate once paused).
    pub fn at_rest(&self) -> bool {
        self.bands
            .iter()
            .chain([&self.left, &self.right])
            .all(|b| b.peak <= 0.0005)
    }
}

/// What the visualizer canvas draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub spectrum: Spectrum,
    pub mode: VizMode,
    /// The hue of the playing album (the record's label in radial mode).
    pub hue: u16,
}

/// The visualizer view.
pub fn visualizer<M>(frame: impl IntoProp<Frame, M>) -> WidgetView<Canvas<Frame>> {
    canvas(frame, paint)
}

fn paint(f: Frame, cx: &mut DrawCx<'_, '_>) {
    let a = cx.coords();
    match f.mode {
        VizMode::Bars => bars(&f, a, cx),
        VizMode::Radial => radial(&f, a, cx),
    }
}

/// The bar gradient, fixed to the canvas: violet at `floor`, coral in the middle, amber at
/// `top`. Each bar is drawn as its own rectangle, so the gradient's geometry is given
/// relative to that bar (`bar_top`): the bar shows the slice of the gradient behind it.
fn bar_gradient(bar_top: i32, top: i32, floor: i32) -> Gradient {
    Gradient::new(
        GradKind::Linear {
            start: Point::new(0, floor - bar_top),
            end: Point::new(0, top - bar_top),
        },
        &[
            twine::render::GradStop::new(VIOLET, Fraction::ZERO),
            twine::render::GradStop::new(ACCENT, Fraction::pct(60)),
            twine::render::GradStop::new(AMBER, Fraction::ONE),
        ],
    )
}

fn bars(f: &Frame, a: Rect, cx: &mut DrawCx<'_, '_>) {
    let n = BANDS as i32;
    let gap = (a.width() / 80).max(2);
    let bw = ((a.width() - (n - 1) * gap) / n).max(1);
    let x0 = a.x0 + (a.width() - (bw * n + gap * (n - 1))) / 2;
    // 78 % for the bars, the rest for their reflection.
    let floor = a.y0 + a.height() * 78 / 100;
    let top = a.y0 + 8;
    let full = floor - top;
    let p = cx.painter();
    // dB guides behind the bars.
    for k in 1..4 {
        let y = floor - full * k / 4;
        p.fill(Rect::from_xywh(a.x0, y, a.width(), 1), FAINT, Opa::P20);
    }
    let r = (bw / 2).min(4);
    for i in 0..BANDS {
        let x = x0 + i as i32 * (bw + gap);
        let h = (full * i32::from(f.spectrum.bars[i]) / 1000).max(2);
        let bar = Rect::from_xywh(x, floor - h, bw, h);
        rrect_grad(p, bar, r, &bar_gradient(floor - h, top, floor));
        // The reflection: a third as tall, fading out downwards.
        let rh = h * 35 / 100;
        if rh > 1 {
            let tint = mix(VIOLET, ACCENT, i32::from(f.spectrum.bars[i]));
            let refl = Rect::from_xywh(x, floor + 2, bw, rh);
            let fade = gradient_opa(GradKind::Ver, (tint, Opa::from_raw(70)), (tint, Opa::TRANSP));
            rrect_grad(p, refl, r, &fade);
        }
        // The peak cap, floating 3 px above where the bar was highest.
        let ph = full * i32::from(f.spectrum.peaks[i]) / 1000;
        if ph > 4 {
            let y = floor - ph - 4;
            rrect(p, Rect::from_xywh(x, y - 2, bw, 3), 1, TEXT, Opa::P90);
        }
    }
    p.fill(Rect::from_xywh(a.x0, floor, a.width(), 1), FAINT, Opa::P40);
}

fn radial(f: &Frame, a: Rect, cx: &mut DrawCx<'_, '_>) {
    let c = a.center();
    let size = a.width().min(a.height());
    let bass = i32::from(f.spectrum.bass);
    // The record, its size breathing slightly with the bass.
    let r0 = size * 23 / 100 + size * bass / 40_000;
    let max_len = size / 2 - r0 - 14;
    let p = cx.painter();
    // A glow behind everything, brighter on loud bass.
    let glow_r = r0 + max_len / 2 + 8;
    let glow = gradient_opa(
        GradKind::Radial {
            center: Point::new(glow_r, glow_r),
            radius: glow_r,
            focal: Point::new(glow_r, glow_r),
            focal_radius: r0,
        },
        (ACCENT, Opa::from_raw((30 + bass * 90 / 1000) as u8)),
        (VIOLET, Opa::TRANSP),
    );
    circle_grad(p, c, glow_r, &glow);
    // 64 spokes: the 32 bands, then mirrored, so the figure is symmetric left/right.
    let spokes = 2 * BANDS as i32;
    let circumference = 2 * 314 * r0 / 100;
    let w = (circumference / spokes * 55 / 100).max(2);
    for k in 0..spokes {
        let band = if k < BANDS as i32 { k } else { spokes - 1 - k } as usize;
        let ang = 2700 + k * 3600 / spokes + 3600 / spokes / 2;
        let v = i32::from(f.spectrum.bars[band]);
        let len = (max_len * v / 1000).max(2);
        let from = polar(c, r0 + 6, ang);
        let to = polar(c, r0 + 6 + len, ang);
        let col = if v < 600 {
            mix(VIOLET, ACCENT, v * 1000 / 600)
        } else {
            mix(ACCENT, AMBER, (v - 600) * 1000 / 400)
        };
        line(p, from, to, w, col, Opa::COVER);
        let pk = max_len * i32::from(f.spectrum.peaks[band]) / 1000;
        if pk > 6 {
            circle(p, polar(c, r0 + 6 + pk + 5, ang), (w / 2).max(1), TEXT, Opa::P80);
        }
    }
    // The record: black vinyl with grooves, a colored label and a sheen that turns with it.
    circle(p, c, r0, Color::hex(0x0C_0C_10), Opa::COVER);
    for g in (r0 * 45 / 100..r0 - 2).step_by(3) {
        ring(p, c, g, 1, Color::hex(0x1C_1D_24), Opa::COVER);
    }
    let turn = i32::from(f.spectrum.turn);
    for side in [0, 1800] {
        let sheen = polar(c, r0 * 80 / 100, turn + side);
        let inner = polar(c, r0 * 52 / 100, turn + side);
        line(p, inner, sheen, (r0 / 10).max(2), Color::WHITE, Opa::from_raw(28));
    }
    let label_r = r0 * 40 / 100;
    circle_grad(
        p,
        c,
        label_r,
        &gradient(GradKind::Ver, hsv(f.hue, 150, 250), hsv(f.hue + 30, 200, 170)),
    );
    let mark = polar(c, label_r * 60 / 100, turn);
    circle(p, mark, (label_r / 6).max(1), Color::hex(0xFF_F4_E6), Opa::P70);
    circle(p, c, (r0 / 22).max(2), Color::hex(0x0C_0C_10), Opa::COVER);
    ring(p, c, r0, 1, RAISED, Opa::COVER);
}

/// The level meters view: two LED rows, left over right.
pub fn meters<M>(levels: impl IntoProp<Levels, M>) -> WidgetView<Canvas<Levels>> {
    canvas(levels, paint_meters)
}

/// Segments per row.
const SEGMENTS: i32 = 24;

fn paint_meters(l: Levels, cx: &mut DrawCx<'_, '_>) {
    let a = cx.coords();
    let gap = 1;
    let row_h = (a.height() - 2) / 2;
    let seg_w = ((a.width() - (SEGMENTS - 1) * gap) / SEGMENTS).max(1);
    let p = cx.painter();
    for (row, (v, peak)) in [(l.left, l.peak_left), (l.right, l.peak_right)]
        .into_iter()
        .enumerate()
    {
        let y = a.y0 + row as i32 * (row_h + 2);
        let lit = i32::from(v) * SEGMENTS / 1000;
        let peak_seg = (i32::from(peak) * SEGMENTS / 1000).min(SEGMENTS - 1);
        for s in 0..SEGMENTS {
            let x = a.x0 + s * (seg_w + gap);
            // Green up to −12 dBFS, amber to −3, red above (the scale is −48…0).
            let col = match s * 48 / SEGMENTS {
                0..36 => GREEN,
                36..45 => AMBER,
                _ => RED,
            };
            let on = s < lit || (s == peak_seg && peak > 20);
            let opa = if on { Opa::COVER } else { Opa::from_raw(38) };
            p.fill(Rect::from_xywh(x, y, seg_w, row_h), col, opa);
        }
    }
}

/// The "now playing" equalizer icon of the current row: three bars following the bass, the
/// mids and the highs.
pub fn playing_icon<M>(levels: impl IntoProp<[u16; 3], M>) -> WidgetView<Canvas<[u16; 3]>> {
    canvas(levels, paint_icon)
}

/// The icon's three bars, from a spectrum.
pub fn icon_bars(s: &Spectrum) -> [u16; 3] {
    [s.bars[3], s.bars[13], s.bars[22]]
}

fn paint_icon(v: [u16; 3], cx: &mut DrawCx<'_, '_>) {
    let a = cx.coords();
    let bw = 3;
    let gap = 2;
    let x0 = a.x0 + (a.width() - (3 * bw + 2 * gap)) / 2;
    let p = cx.painter();
    for (i, lvl) in v.into_iter().enumerate() {
        let h = (3 + (a.height() - 3) * i32::from(lvl) / 1000).min(a.height());
        let x = x0 + i as i32 * (bw + gap);
        rrect(p, Rect::from_xywh(x, a.y1 - h, bw, h), 1, ACCENT, Opa::COVER);
    }
}
