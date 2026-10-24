//! The sweep waveform: the moving ECG, pleth and respiration traces of the monitor screen.
//!
//! **What it looks like.** Like a real bedside monitor (and unlike a scrolling chart), the
//! trace is *swept*: a write head moves left to right, overwriting the previous sweep, with a
//! short blank **erase gap** just ahead of it so the eye can tell new data from old. Nothing
//! scrolls, so nothing has to be redrawn except the columns the head just passed.
//!
//! **Data flow.** The patient simulator (`store.rs`) appends one *column* per tick to a
//! [`Trace`]: the min/max envelope of the samples that fell into that column (decimation that
//! keeps a 90 ms QRS spike as tall as it really is). It then bumps the trace's `head` signal —
//! the count of columns written. The waveform view binds that signal to
//! [`Waveform::set_head`]; nothing else about the trace is reactive.
//!
//! **Partial redraw.** `set_head` invalidates exactly two slivers per new column: the column
//! just written (plus the pen width) and the column that the erase gap just swallowed. At
//! 62.5 columns per second that is a few hundred pixels per frame out of 153 600, and the
//! engine's partial renderer draws only those. With `F2` in the simulator you can watch the
//! redrawn areas: three thin bars marching across the screen.
//!
//! **Cheap drawing.** [`Waveform::draw`] only visits the columns inside the clip area. Each
//! column is one vertical `fill` from the lowest to the highest value of this column joined to
//! the previous one, so steep slopes stay connected without any line rasterisation, anti-
//! aliasing or floating point; a pen of `line_width` pixels is a fill that wide. The pleth can
//! also be `filled`: a translucent fill from the trace down to the baseline.
//!
//! **No allocation at 60 Hz.** The ring of columns is allocated once at start-up; writing a
//! column, setting the signal, invalidating and drawing allocate nothing.

use core::ops::RangeInclusive;

use twine::engine::recipe;
use twine::prelude::*;
use twine::reactive::StoredValue;

/// Columns a trace remembers: more than the widest strip, so the whole sweep is always there.
pub const RING_COLUMNS: usize = 512;
/// Width of the blank erase gap ahead of the write head, in columns.
pub const ERASE_GAP: i32 = 12;

/// The min/max of the samples that fell into one column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Envelope {
    /// Lowest sample.
    pub lo: i16,
    /// Highest sample.
    pub hi: i16,
}

impl Envelope {
    /// An envelope holding only `v`.
    pub fn of(v: i32) -> Self {
        let v = v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        Self { lo: v, hi: v }
    }

    /// Widens the envelope to include `v`.
    pub fn add(&mut self, v: i32) {
        let e = Self::of(v);
        self.lo = self.lo.min(e.lo);
        self.hi = self.hi.max(e.hi);
    }
}

/// The last [`RING_COLUMNS`] columns of one waveform.
#[derive(Debug)]
pub struct Ring {
    cols: Box<[Envelope; RING_COLUMNS]>,
}

impl Ring {
    /// Column number `n` (counting from the first column ever written), if still held.
    fn get(&self, n: u32, written: u32) -> Option<Envelope> {
        (n < written && written - n <= RING_COLUMNS as u32).then(|| self.cols[n as usize % RING_COLUMNS])
    }
}

/// One waveform channel: a ring of columns shared by the simulator (writer) and the waveform
/// widget (reader), and a signal counting the columns written.
///
/// `Copy`, so it can sit in the application's store and be captured by any closure. The ring
/// is a [`StoredValue`]: owned by the scope that created the trace (dropped with it), shared
/// through a `Copy` handle, and not reactive — writing it notifies nobody; the `head` signal
/// is the only thing the UI subscribes to.
#[derive(Clone, Copy)]
pub struct Trace {
    ring: StoredValue<Ring>,
    /// Columns written so far (the write head). Bound to [`Waveform::set_head`].
    pub head: Signal<u32>,
}

impl Trace {
    /// A new, empty trace whose head signal lives in `cx`.
    pub fn new(cx: Scope) -> Self {
        let ring = cx.stored_value(Ring {
            cols: Box::new([Envelope::default(); RING_COLUMNS]),
        });
        Self {
            ring,
            head: cx.signal(0),
        }
    }

    /// Appends one column and moves the head signal past it (one `u32` write: the bound
    /// waveform's setter runs, nothing is allocated).
    pub fn push(self, col: Envelope) {
        let n = self.head.get_untracked();
        self.ring.with_mut(|r| r.cols[n as usize % RING_COLUMNS] = col);
        self.head.set(n.wrapping_add(1));
    }
}

// ---- The widget -----------------------------------------------------------------------------

/// The class of [`Waveform`]. `Main` is the strip's background; [`Part::Indicator`] is the
/// trace: `line_color`, `line_opacity`, `line_width` (the pen), and `bg_opacity` (the fill
/// under a [`filled`](WaveformView::filled) trace).
pub static WAVEFORM_CLASS: WidgetClass = WidgetClass::new("waveform").parts(&[Part::Main, Part::Indicator]);

/// The theme's look for waveforms: a 2 px pen and a faint fill. The trace colour is not set
/// here — each strip gives its own (`design` elements of `theme.rs`, so it follows the mode).
static TRACE_STYLE: Style = style! { line_width: 2, bg_opacity: Opa::P20 };

recipe! {
    /// The look of waveforms in the application theme (listed in `theme.rs`).
    pub static WAVEFORM_RECIPE for WAVEFORM_CLASS {
        Part::Indicator => { ..TRACE_STYLE },
    }
}

/// A sweep waveform reading one [`Trace`] (see the module docs).
#[derive(Debug)]
pub struct Waveform {
    ring: StoredValue<Ring>,
    /// Columns written, as of the last `set_head`.
    head: u32,
    /// The strip width the invalidation was computed for (0: not laid out yet).
    width: i32,
    /// Value at the bottom and at the top of the strip.
    lo: i32,
    hi: i32,
    /// Fill between the trace and the zero line.
    filled: bool,
}

impl Waveform {
    fn new(trace: Trace) -> Self {
        Self {
            ring: trace.ring,
            head: 0,
            width: 0,
            lo: -1000,
            hi: 1000,
            filled: false,
        }
    }

    /// Moves the write head to `head` columns written. Idempotent. Invalidates only the new
    /// columns and the columns the erase gap now covers; a jump of a whole sweep (or the first
    /// call after a layout change) invalidates the strip once.
    pub fn set_head(&mut self, cx: &mut WidgetCx<'_>, head: u32) {
        if head == self.head {
            return;
        }
        let area = cx.content_area();
        let w = area.width();
        let advanced = head.wrapping_sub(self.head);
        let old = self.head;
        self.head = head;
        if w <= ERASE_GAP + 2 || w != self.width || head < old || advanced >= w as u32 {
            self.width = w;
            cx.invalidate_for("waveform.head");
            return;
        }
        let pen = cx.measure().line_dsc(Part::Indicator).width.max(1);
        let sliver = |cx: &mut WidgetCx<'_>, col: i32, extra: i32| {
            let x = area.x0 + col.rem_euclid(w);
            cx.invalidate_area(Rect::new(x, area.y0, x + pen + extra, area.y1));
        };
        for n in old..head {
            let col = (n % w as u32) as i32;
            // The new column (its pen spills `pen - 1` px to the right) ...
            sliver(cx, col, 0);
            // ... and the one the gap just erased, with its right neighbour, which loses
            // its connection to it.
            sliver(cx, col + ERASE_GAP, 1);
        }
    }

    /// Sets the value range shown (bottom..=top). Idempotent.
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, r: RangeInclusive<i32>) {
        let (lo, hi) = r.into_inner();
        if (lo, hi) != (self.lo, self.hi) && hi > lo {
            (self.lo, self.hi) = (lo, hi);
            cx.invalidate_for("waveform.range");
        }
    }

    /// Fills under the trace or not. Idempotent.
    pub fn set_filled(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if on != self.filled {
            self.filled = on;
            cx.invalidate_for("waveform.filled");
        }
    }

    /// The column shown at strip x `x` (0-based) for a strip `w` wide: its column number and
    /// whether it joins the column to its left. `None` in the erase gap or before any data.
    fn column_at(&self, x: i32, w: i32) -> Option<(u32, bool)> {
        let last = self.head.checked_sub(1)?;
        let head_x = (last % w as u32) as i32;
        // How many columns ago `x` was written (0: the head itself).
        let age = (head_x - x).rem_euclid(w);
        let oldest = w - ERASE_GAP - 1;
        if age > oldest || age as u32 > last {
            return None;
        }
        Some((last - age as u32, age < oldest))
    }
}

impl Widget for Waveform {
    fn class(&self) -> &'static WidgetClass {
        &WAVEFORM_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let area = cx.content_area();
        let (w, h) = (area.width(), area.height());
        if w <= ERASE_GAP + 2 || h < 4 {
            return;
        }
        let Some(clip) = cx.clip().intersection(&area) else {
            return;
        };
        let pen = cx.line_dsc(Part::Indicator);
        let fill_opa = cx.rect_dsc(Part::Indicator).base.bg_opa;
        let width = pen.width.max(1);
        // Value → y: `hi` at the top row, `lo` at the bottom one (integer, clamped).
        let span = (self.hi - self.lo).max(1);
        let y_of = |v: i32| area.y1 - 1 - ((v - self.lo) * (h - width) / span).clamp(0, h - width);
        let base_y = y_of(0);
        // The ring is a scope-owned `StoredValue`: borrowed for the loop, nothing copied.
        self.ring.with(|ring| {
            // Start `width - 1` columns left of the clip: their pen spills into it.
            for x in (clip.x0 - area.x0 - (width - 1)).max(0)..clip.x1 - area.x0 {
                let Some((n, joined)) = self.column_at(x, w) else {
                    continue;
                };
                let Some(cur) = ring.get(n, self.head) else {
                    continue;
                };
                // Join this column to the previous one: span from the lower of (this low,
                // previous high) to the higher of (this high, previous low).
                let (mut lo, mut hi) = (i32::from(cur.lo), i32::from(cur.hi));
                if let Some(prev) = joined.then(|| ring.get(n.wrapping_sub(1), self.head)).flatten() {
                    lo = lo.min(i32::from(prev.hi));
                    hi = hi.max(i32::from(prev.lo));
                }
                let (top, bottom) = (y_of(hi), y_of(lo) + width);
                let x0 = area.x0 + x;
                if self.filled && bottom < base_y {
                    cx.painter()
                        .fill(Rect::new(x0, bottom, x0 + 1, base_y), pen.color, fill_opa);
                }
                cx.painter()
                    .fill(Rect::new(x0, top, x0 + width, bottom), pen.color, pen.opa);
            }
        });
    }
}

// ---- The view -------------------------------------------------------------------------------

/// A sweep waveform of `trace` (see the module docs). Give it a size and a trace colour:
/// `waveform(ecg).range(-400..=1300).part(Part::Indicator, |s| s.line_color(theme::HR))`.
pub fn waveform(trace: Trace) -> WaveformView {
    WaveformView(
        widget_view(move || Waveform::new(trace)).bind_after_children(trace.head, Waveform::set_head),
    )
}

/// The view of a [`Waveform`]: its own modifiers plus every [`ViewExt`] and [`StyleExt`] one.
#[derive(Debug)]
pub struct WaveformView(WidgetView<Waveform>);

impl WaveformView {
    /// The values at the bottom and the top of the strip (default `-1000..=1000`).
    #[must_use]
    pub fn range(self, r: RangeInclusive<i32>) -> Self {
        Self(self.0.bind(r, Waveform::set_range))
    }

    /// Fills between the trace and zero with the `Indicator`'s `bg_opacity` (pleth style).
    #[must_use]
    pub fn filled(self, on: bool) -> Self {
        Self(self.0.bind(on, Waveform::set_filled))
    }
}

impl View for WaveformView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for WaveformView {
    type Widget = Waveform;

    fn push_op(self, op: twine::view::BuildOp) -> Self {
        Self(self.0.push_op(op))
    }
}
