//! The trend plot: one parameter over the last 1, 4 or 12 hours (Trends screen).
//!
//! **Data.** The plot reads the store's trend history (one point every 10 s, 4320 points for
//! 12 h) while drawing; it holds no copy. Its view binds two signals: the selected span (a
//! setter that invalidates the plot) and the history's revision (bumped when a point is
//! appended, every 10 s), plus the alarm limits so the shaded "out of limits" bands follow the
//! Setup screen live.
//!
//! **Decimation.** 12 h of points is ~4300 values for ~400 pixel columns. Each column draws
//! the min/max envelope of the points that fall into it — the same per-column vertical fill as
//! the sweep waveform (`waveform.rs`) — so a 3-minute desaturation 8 hours ago still shows at
//! its true depth instead of being averaged away, and drawing costs one fill per column.
//!
//! **Parts** (themed in `style_trend`, coloured per plot by the view): `Main` the plot area,
//! [`Part::Indicator`] the series (`line_color`, `line_width`, `bg_opacity` of the fill below
//! it), [`GRID`] the grid lines (`line_color`), [`BAND`] the region beyond the alarm limits
//! (`bg_color`, `bg_opacity`) and the limit lines (`line_*`, dashed).

use core::ops::RangeInclusive;

use twine::prelude::*;
use twine::reactive::StoredValue;
use twine::render::LineDsc;
use twine::theme::ClassCx;

use crate::patient::TrendPoint;
use crate::store::Trends;
use crate::theme;

/// The grid lines.
pub const GRID: Part = Part::custom::<0>();
/// The out-of-limits band and the limit lines.
pub const BAND: Part = Part::custom::<1>();

/// The class of [`TrendPlot`].
pub static TREND_CLASS: WidgetClass =
    WidgetClass::new("trend_plot").parts(&[Part::Main, Part::Indicator, GRID, BAND]);

static SERIES_STYLE: Style = style! { line_width: 2, bg_opacity: Opa::P10 };
static GRID_STYLE: Style = style! { line_color: theme::HAIRLINE, line_width: 1 };
static BAND_STYLE: Style = style! {
    bg_color: theme::ALARM_HIGH,
    bg_opacity: Opa::pct(7),
    line_color: theme::ALARM_HIGH,
    line_opacity: Opa::P60,
    line_width: 1,
    line_dash_width: 4,
    line_dash_gap: 4,
};

/// Styles trend plots in the application theme (registered in `theme.rs`).
pub fn style_trend<S>(cx: &mut ClassCx<'_, '_, S>) {
    cx.add_style(Selector::part(Part::Indicator), &SERIES_STYLE);
    cx.add_style(Selector::part(GRID), &GRID_STYLE);
    cx.add_style(Selector::part(BAND), &BAND_STYLE);
}

/// Picks the plotted value out of a trend point.
pub type Series = fn(&TrendPoint) -> i32;

/// A trend plot (see the module docs).
#[derive(Debug)]
pub struct TrendPlot {
    trends: StoredValue<Trends>,
    series: Series,
    /// Points shown (the most recent ones).
    span: usize,
    /// Value range, bottom..=top, and the grid step.
    lo: i32,
    hi: i32,
    step: i32,
    /// Alarm limits (`i32::MIN` / `i32::MAX`: none on that side).
    limits: (i32, i32),
    rev: u32,
}

impl TrendPlot {
    /// Shows the last `points` points. Idempotent.
    pub fn set_span(&mut self, cx: &mut WidgetCx<'_>, points: usize) {
        if points != self.span && points > 1 {
            self.span = points;
            cx.invalidate_for("trend.span");
        }
    }

    /// The history changed (a new point). Idempotent.
    pub fn set_rev(&mut self, cx: &mut WidgetCx<'_>, rev: u32) {
        if rev != self.rev {
            self.rev = rev;
            cx.invalidate_for("trend.rev");
        }
    }

    /// The alarm limits to shade beyond. Idempotent.
    pub fn set_limits(&mut self, cx: &mut WidgetCx<'_>, limits: (i32, i32)) {
        if limits != self.limits {
            self.limits = limits;
            cx.invalidate_for("trend.limits");
        }
    }

    /// Draws a horizontal line at value `v`, if inside the plot.
    fn hline(cx: &mut DrawCx<'_, '_>, area: Rect, y: i32, dsc: &LineDsc) {
        if (area.y0..area.y1).contains(&y) {
            cx.painter()
                .line(Point::new(area.x0, y), Point::new(area.x1 - 1, y), dsc);
        }
    }
}

impl Widget for TrendPlot {
    fn class(&self) -> &'static WidgetClass {
        &TREND_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let area = cx.content_area();
        let (w, h) = (area.width(), area.height());
        if w < 2 || h < 4 {
            return;
        }
        let span = (self.hi - self.lo).max(1);
        let y_of = |v: i32| area.y1 - 1 - ((v - self.lo) * (h - 1) / span).clamp(0, h - 1);

        // The bands beyond the limits, then the grid, then the limit lines.
        let band = cx.rect_dsc(BAND).base;
        if self.limits.1 < self.hi {
            let r = Rect::new(area.x0, area.y0, area.x1, y_of(self.limits.1));
            cx.painter().fill(r, band.bg_color, band.bg_opa);
        }
        if self.limits.0 > self.lo {
            let r = Rect::new(area.x0, y_of(self.limits.0) + 1, area.x1, area.y1);
            cx.painter().fill(r, band.bg_color, band.bg_opa);
        }
        let grid = cx.line_dsc(GRID);
        let mut v = self.lo + self.step;
        while v < self.hi && self.step > 0 {
            Self::hline(cx, area, y_of(v), &grid);
            v += self.step;
        }
        for q in 1..4 {
            let x = area.x0 + w * q / 4;
            cx.painter()
                .line(Point::new(x, area.y0), Point::new(x, area.y1 - 1), &grid);
        }
        let limit = cx.line_dsc(BAND);
        for l in [self.limits.0, self.limits.1] {
            if l > self.lo && l < self.hi {
                Self::hline(cx, area, y_of(l), &limit);
            }
        }

        // The series: per column, the envelope of its points joined to the previous column.
        let pen = cx.line_dsc(Part::Indicator);
        let fill = cx.rect_dsc(Part::Indicator).base.bg_opa;
        self.trends.with(|trends| {
            let pts = &trends.points;
            let n = pts.len().min(self.span);
            if n == 0 {
                return;
            }
            let first = pts.len() - n;
            // Columns covered by data (right-aligned when the history is shorter than the span).
            let cols = (w as usize * n / self.span).max(1) as i32;
            let x_start = area.x1 - cols;
            let mut prev: Option<(i32, i32)> = None;
            for c in 0..cols {
                let a = first + c as usize * n / cols as usize;
                let b = (first + (c as usize + 1) * n / cols as usize)
                    .max(a + 1)
                    .min(pts.len());
                let (mut lo, mut hi) = (i32::MAX, i32::MIN);
                for p in pts.range(a..b) {
                    let v = (self.series)(p);
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                let (top, bottom) = match prev {
                    Some((plo, phi)) => (y_of(hi.max(plo)), y_of(lo.min(phi))),
                    None => (y_of(hi), y_of(lo)),
                };
                prev = Some((lo, hi));
                let x = x_start + c;
                cx.painter()
                    .fill(Rect::new(x, y_of(hi), x + 1, area.y1), pen.color, fill);
                cx.painter().fill(
                    Rect::new(x, top, x + pen.width.max(1), bottom + pen.width.max(1)),
                    pen.color,
                    pen.opa,
                );
            }
        });
    }
}

/// A trend plot of `series` from `trends` over `range` with grid lines every `step`. Bind its
/// span, the history revision and the limits with the view's modifiers.
pub fn trend_plot(
    trends: StoredValue<Trends>,
    series: Series,
    range: RangeInclusive<i32>,
    step: i32,
) -> TrendPlotView {
    let (lo, hi) = range.into_inner();
    TrendPlotView(widget_view(move || TrendPlot {
        trends,
        series,
        span: 360,
        lo,
        hi: hi.max(lo + 1),
        step,
        limits: (i32::MIN, i32::MAX),
        rev: 0,
    }))
}

/// The view of a [`TrendPlot`].
#[derive(Debug)]
pub struct TrendPlotView(WidgetView<TrendPlot>);

impl TrendPlotView {
    /// The number of most recent points shown.
    #[must_use]
    pub fn span<M>(self, points: impl IntoProp<usize, M>) -> Self {
        Self(self.0.bind(points, TrendPlot::set_span))
    }

    /// The history revision (redraws when it changes).
    #[must_use]
    pub fn revision<M>(self, rev: impl IntoProp<u32, M>) -> Self {
        Self(self.0.bind(rev, TrendPlot::set_rev))
    }

    /// The alarm limits `(low, high)`.
    #[must_use]
    pub fn limits<M>(self, l: impl IntoProp<(i32, i32), M>) -> Self {
        Self(self.0.bind(l, TrendPlot::set_limits))
    }
}

impl View for TrendPlotView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for TrendPlotView {
    type Widget = TrendPlot;

    fn push_op(self, op: twine::view::BuildOp) -> Self {
        Self(self.0.push_op(op))
    }
}
