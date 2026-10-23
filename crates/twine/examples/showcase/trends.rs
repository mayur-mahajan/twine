//! The Trends screen: heart rate and SpO2 over the last 1, 4 or 12 hours, as plots or as a
//! table.
//!
//! **Signals.** Two small selection signals (`range`, `mode`) drive everything:
//!
//! - the plots bind their span to a memo of `range` and their redraw to the store's
//!   `trend_rev` (bumped every 10 s), and their limit bands to the alarm limits;
//! - the table is a memo over `range`, `trend_rev` and the clock that picks one row per
//!   5 min / 20 min / 1 h, newest first, keyed by its time — so when a new point arrives only
//!   the rows whose values changed re-format, and a new row slides in at the top;
//! - the axis labels are `&'static str` picked by `range` (no formatting at all).
//!
//! Switching the range redraws the two plots once; nothing on this screen runs between
//! changes.

use core::fmt;

use twine::prelude::*;
use twine::style::design::ColorValue;

use crate::components::{panel, segmented};
use crate::patient::{TREND_STEP_S, TrendPoint};
use crate::store::{Monitor, hhmm};
use crate::theme::{self, BARE, CAPTION};
use crate::trend_plot::{Series, trend_plot};

/// One selectable time range.
struct Range {
    /// Trend points shown.
    points: usize,
    /// Minutes between table rows.
    row_min: u32,
    /// Labels under the plots, oldest first.
    axis: [&'static str; 5],
}

const RANGES: [Range; 3] = [
    Range {
        points: 3600 / TREND_STEP_S as usize,
        row_min: 5,
        axis: ["-60 min", "-45", "-30", "-15", "now"],
    },
    Range {
        points: 4 * 3600 / TREND_STEP_S as usize,
        row_min: 20,
        axis: ["-4 h", "-3 h", "-2 h", "-1 h", "now"],
    },
    Range {
        points: 12 * 3600 / TREND_STEP_S as usize,
        row_min: 60,
        axis: ["-12 h", "-9 h", "-6 h", "-3 h", "now"],
    },
];

static RANGE_NAMES: [&str; 3] = ["1 h", "4 h", "12 h"];
static MODE_NAMES: [&str; 2] = ["Graph", "Table"];

/// One row of the trend table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    /// Clock seconds.
    at: u32,
    point: TrendPoint,
}

/// `hh:mm`.
struct HhMm(u32);

impl fmt::Display for HhMm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m) = hhmm(self.0);
        write!(f, "{h:02}:{m:02}")
    }
}

/// What one plot shows.
struct PlotSpec {
    caption: &'static str,
    color: twine::style::design::ColorElement,
    series: Series,
    range: core::ops::RangeInclusive<i32>,
    /// Grid line spacing, in the series' unit.
    step: i32,
    /// The value axis, top to bottom (at the grid lines).
    labels: &'static [&'static str],
}

/// A plot with its caption and its value axis; `limits` are the alarm limits to shade.
fn plot(
    m: Monitor,
    span: Memo<usize>,
    spec: PlotSpec,
    limits: impl Fn() -> (i32, i32) + 'static,
) -> impl View {
    let PlotSpec {
        caption,
        color,
        series,
        range,
        step,
        labels,
    } = spec;
    row((
        column(
            labels
                .iter()
                .map(|l| label(*l).style(&CAPTION).letter_spacing(0))
                .collect::<Vec<_>>(),
        )
        .width(26)
        .fill_height()
        .justify(MainAlign::SpaceBetween)
        .align_items(CrossAlign::End),
        container((
            trend_plot(m.trends(), series, range, step)
                .span(span)
                .revision(m.trend_rev())
                .limits(limits)
                .part(Part::Indicator, move |s| s.line_color(color))
                .fill(),
            label(caption).style(&CAPTION).text_color(color).pos(6, 2),
        ))
        .style(&BARE)
        .flex_grow(1)
        .fill_height()
        .flag(ObjFlags::SCROLLABLE, false),
    ))
    .fill_width()
    .flex_grow(1)
    .gap(6)
}

/// The table rows for `range`: one every `row_min` minutes, newest first.
fn table_rows(m: Monitor, range: &Range) -> Vec<Row> {
    // The newest point is from the last whole `TREND_STEP_S` of the clock.
    let now = m.clock().get() / TREND_STEP_S * TREND_STEP_S;
    let step = range.row_min * 60;
    let newest = now / step * step;
    m.trends().with(|trends| {
        let pts = &trends.points;
        let Some(last) = pts.len().checked_sub(1) else {
            return Vec::new();
        };
        (0..12)
            .filter_map(|i| {
                let at = newest.checked_sub(i * step)?;
                let back = ((now - at) / TREND_STEP_S) as usize;
                let point = *pts.get(last.checked_sub(back)?)?;
                Some(Row { at, point })
            })
            .collect()
    })
}

/// One cell of the table.
fn cell<MT>(text: impl IntoText<MT>, width: i32) -> impl ViewExt {
    label(text).width(width).text_align(TextAlign::Right)
}

/// The header and the rows of the table.
fn table(m: Monitor, rows: Memo<Vec<Row>>) -> impl ViewExt {
    const W: [i32; 6] = [56, 56, 56, 56, 90, 60];
    let head = |t: &'static str, w: i32| label(t).style(&CAPTION).width(w).text_align(TextAlign::Right);
    panel((
        row((
            head("TIME", W[0]),
            head("HR", W[1]).text_color(theme::HR),
            head("SpO2", W[2]).text_color(theme::SPO2),
            head("RR", W[3]).text_color(theme::RESP),
            head("NIBP", W[4]).text_color(theme::NIBP),
            head("TEMP", W[5]).text_color(theme::TEMP),
        ))
        .gap(8)
        .padding_each(Insets::new(0, 4, 0, 4)),
        scroll_view(
            Axis::Vertical,
            for_each(
                rows,
                |r| r.at,
                move |_cx, r| {
                    let p = move || r.with(|r| r.point);
                    // A value beyond its alarm limit is shown in the alarm colour.
                    let spo2_color = move || {
                        if i32::from(p().spo2) < m.limits().with(|l| l.spo2_low) {
                            theme::ALARM_HIGH
                        } else {
                            theme::SPO2
                        }
                    };
                    row((
                        cell(text!("{}", HhMm(r.with(|r| r.at))), W[0]).text_color(design::ON_SURFACE_MUTED),
                        cell(text!("{}", p().hr), W[1]).text_color(theme::HR),
                        cell(text!("{}", p().spo2), W[2]).text_color(move || ColorValue::from(spo2_color())),
                        cell(text!("{}", p().rr), W[3]).text_color(theme::RESP),
                        cell(text!("{}/{}", p().nibp.sys, p().nibp.dia), W[4]).text_color(theme::NIBP),
                        cell(text!("{}.{}", p().temp / 10, p().temp % 10), W[5]).text_color(theme::TEMP),
                    ))
                    .gap(8)
                    .padding_each(Insets::hor_ver(4, 3))
                },
            ),
        )
        .fill_width()
        .flex_grow(1),
    ))
    .fill()
    .gap(4)
    .padding(6)
}

/// The Trends screen.
pub fn page(cx: Scope) -> impl ViewExt {
    let m = cx.expect_context::<Monitor>();
    let range = cx.signal(0usize);
    let mode = cx.signal(0usize);
    let span = cx.memo(move || RANGES[range.get()].points);
    let rows = cx.memo(move || {
        m.trend_rev().get(); // a new point also changes the rows
        table_rows(m, &RANGES[range.get()])
    });
    let axis = move |i: usize| move || RANGES[range.get()].axis[i];

    let graph = panel((
        plot(
            m,
            span,
            PlotSpec {
                caption: "HR  bpm",
                color: theme::HR,
                series: |p| p.hr.into(),
                range: 40..=160,
                step: 40,
                labels: &["160", "120", "80", "40"],
            },
            move || m.limits().with(|l| (l.hr_low, l.hr_high)),
        ),
        plot(
            m,
            span,
            PlotSpec {
                caption: "SpO2  %",
                color: theme::SPO2,
                series: |p| p.spo2.into(),
                range: 80..=100,
                step: 5,
                labels: &["100", "95", "90", "85", "80"],
            },
            move || m.limits().with(|l| (l.spo2_low, i32::MAX)),
        ),
        row((0..5)
            .map(|i| label(axis(i)).style(&CAPTION).letter_spacing(0))
            .collect::<Vec<_>>())
        .fill_width()
        .padding_left(32)
        .justify(MainAlign::SpaceBetween),
    ))
    .fill()
    .gap(6)
    .padding_each(Insets::new(4, 8, 8, 4))
    .hidden(move || mode.get() != 0);

    column((
        row((
            label("TRENDS").style(&CAPTION).flex_grow(1),
            segmented(&MODE_NAMES, mode, "trend_mode"),
            segmented(&RANGE_NAMES, range, "trend_range"),
        ))
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        container((graph, table(m, rows).hidden(move || mode.get() != 1)))
            .style(&BARE)
            .fill_width()
            .flex_grow(1),
    ))
    .fill()
    .padding(8)
    .gap(6)
    .flag(ObjFlags::SCROLLABLE, false)
}
