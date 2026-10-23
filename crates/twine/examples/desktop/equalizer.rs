//! The Equalizer page: the frequency-response graph above ten band sliders, with the preset
//! dropdown, an on/off switch and Reset.
//!
//! **Data flow.** Each slider is bound two-way to its band's gain signal; a drag also calls
//! `Equalizer::edited` (preset → "Custom"). A preset from the dropdown starts the eased
//! transition the player clock advances, which writes the same gain signals, so the sliders
//! glide by themselves. The graph is a canvas bound to the response memo
//! (`eq::response_memo`): the memo evaluates the filter math at 97 frequencies whenever a
//! gain changes; the canvas only maps those numbers to pixels.
//!
//! **The graph.** A logarithmic frequency axis (20 Hz–20 kHz, equal distance per octave)
//! against ±15 dB. Boosts are tinted coral and cuts violet, one 1-px column at a time
//! (interpolating between curve points), under the curve itself; a handle marks each band's
//! gain at its centre frequency.

use twine::prelude::*;

use crate::canvas::canvas;
use crate::eq::{self, CURVE_POINTS, Equalizer, LABELS, PRESET_NAMES, RANGE, Response};
use crate::paint::{circle, ring};
use crate::theme::{BODY, CAPTION, DISPLAY, LINE, RAISED, S1, S2, S3, S4, S5, palette};
use crate::ui::{App, caption, overline, pill, slim_slider};

/// Where 20 Hz, 50 Hz, 100 Hz, … sit on the logarithmic axis, in per-mille:
/// `log10(f / 20 Hz) / log10(1000)` (the axis spans three decades).
const HZ_TICKS: [(i32, &str); 10] = [
    (0, "20"),
    (133, "50"),
    (233, "100"),
    (333, "200"),
    (466, "500"),
    (566, "1K"),
    (667, "2K"),
    (799, "5K"),
    (900, "10K"),
    (1000, "20K"),
];
/// The band centres on the same axis (31.25 Hz … 16 kHz, one octave apart = 100.3 ‰ apart).
const BAND_X: [i32; eq::BANDS] = [65, 165, 266, 366, 466, 567, 667, 767, 868, 968];
/// The graph's dB range (each way), in tenths.
const GRAPH_DB10: i32 = 150;

pub fn page(cx: Scope, app: App) -> impl View {
    let eq = app.eq;
    let response = eq::response_memo(cx, eq);

    let header = row((
        column((
            label("Equalizer").font(DISPLAY).text_color(design::ON_SURFACE),
            caption("10 bands, 31 Hz to 16 kHz, 12 dB each way"),
        ))
        .gap(2),
        spacer(),
        row((
            caption("On"),
            switch(eq.enabled)
                .focusable(false)
                .size(40, 22)
                .bg(RAISED)
                .part(Part::Indicator, |s| s.bg(RAISED))
                .on_state(State::CHECKED, |s| {
                    s.part(Part::Indicator, |p| p.bg(design::PRIMARY))
                })
                .part(Part::Knob, |s| s.bg(palette::TEXT).padding(-3))
                .test_id("eq-switch"),
        ))
        .gap(S2)
        .align_items(CrossAlign::Center),
        dropdown(&PRESET_NAMES, eq.preset)
            .focusable(false)
            .disabled(move || !eq.enabled.get())
            .on_state(State::DISABLED, |s| s.opacity(Opa::P40))
            .on_change(move |i| eq.choose(i))
            .width(168)
            .height(36)
            .bg(RAISED)
            .border(1, LINE)
            .radius(10)
            .padding_each(Insets::new(S3, 9, S3, 9))
            .font(BODY)
            .text_color(design::ON_SURFACE)
            .test_id("preset"),
        pill(app, "Reset", false, move || eq.choose(0)),
    ))
    .fill_width()
    .gap(S4)
    .align_items(CrossAlign::Center)
    .scrollable(false);

    let graph = canvas(response, paint_response)
        .fill_width()
        .height(176)
        .test_id("eq-curve");

    let sliders = row((0..eq::BANDS).map(|i| band(eq, i)).collect::<Vec<_>>())
        .fill_width()
        .justify(MainAlign::SpaceBetween)
        .padding_x(14)
        .scrollable(false);

    column((
        header,
        column((overline("FREQUENCY RESPONSE"), graph))
            .fill_width()
            .gap(S2),
        sliders,
    ))
    .fill()
    .padding(S5)
    .gap(S5)
    .scrollable(false)
}

/// `+3.5` / `−12.0` from tenths of a dB.
fn db_text(g: i32) -> String {
    let sign = match g.signum() {
        1 => "+",
        -1 => "-",
        _ => "",
    };
    format!("{sign}{}.{}", g.abs() / 10, g.abs() % 10)
}

/// One band: its gain, a vertical slider, its frequency.
fn band(eq: Equalizer, i: usize) -> Flex {
    let g = eq.gains[i];
    column((
        label(text!("{}", db_text(g.get())))
            .font(CAPTION)
            .text_color(move || {
                if g.get() == 0 {
                    design::ON_SURFACE_MUTED
                } else {
                    design::ON_SURFACE
                }
            }),
        slim_slider(g, 4)
            .orientation(Orientation::Vertical)
            .mode(SliderMode::Symmetrical)
            .range(-RANGE..=RANGE)
            .size(4, 132)
            .margin_y(S2)
            .disabled(move || !eq.enabled.get())
            .on_change(move |v| eq.edited(i, v)),
        label(LABELS[i])
            .font(CAPTION)
            .text_color(design::ON_SURFACE_MUTED),
        label("Hz")
            .font(&fonts::MONTSERRAT_8)
            .text_color(crate::theme::FAINT),
    ))
    .width(44)
    .gap(S1)
    // Bypassed: the bands keep their settings but cannot be moved, and say so by fading.
    .disabled(move || !eq.enabled.get())
    .on_state(State::DISABLED, |s| s.opacity(Opa::P40))
    .align_items(CrossAlign::Center)
    .scrollable(false)
}

fn paint_response(r: Response, cx: &mut DrawCx<'_, '_>) {
    let a = cx.coords();
    // The plot area, leaving room for the dB labels (left) and the Hz labels (below).
    let plot = Rect::new(a.x0 + 38, a.y0 + 10, a.x1 - 22, a.y1 - 22);
    let (pw, ph) = (plot.width(), plot.height());
    let mid = plot.y0 + ph / 2;
    let y_of = |db10: i32| mid - db10.clamp(-GRAPH_DB10, GRAPH_DB10) * (ph / 2) / GRAPH_DB10;
    let x_of = |permille: i32| plot.x0 + permille * (pw - 1) / 1000;
    let mut label_dsc = cx.text_dsc(Part::Main);
    label_dsc.font = &fonts::MONTSERRAT_10;
    label_dsc.color = palette::FAINT;

    // Panel and grid.
    crate::paint::rrect(cx.painter(), a, 12, palette::SURFACE, Opa::COVER);
    for (db10, text) in [(120, "+12"), (60, "+6"), (0, "0"), (-60, "-6"), (-120, "-12")] {
        let y = y_of(db10);
        let opa = if db10 == 0 { Opa::P50 } else { Opa::P20 };
        cx.painter()
            .fill(Rect::from_xywh(plot.x0, y, pw, 1), palette::FAINT, opa);
        label_dsc.align = TextAlign::Right;
        cx.draw_text(Rect::new(a.x0, y - 6, plot.x0 - 8, y + 6), text, &label_dsc);
    }
    label_dsc.align = TextAlign::Center;
    for (pos, text) in HZ_TICKS {
        let x = x_of(pos);
        cx.painter()
            .fill(Rect::from_xywh(x, plot.y0, 1, ph), palette::FAINT, Opa::P20);
        cx.draw_text(Rect::new(x - 20, plot.y1 + 6, x + 20, a.y1), text, &label_dsc);
    }

    // The curve's points: equal x steps, because the curve is sampled at log-spaced
    // frequencies and the axis is logarithmic too.
    let mut pts = [Point::new(0, 0); CURVE_POINTS];
    for (i, pt) in pts.iter_mut().enumerate() {
        *pt = Point::new(
            plot.x0 + i as i32 * (pw - 1) / (CURVE_POINTS as i32 - 1),
            y_of(i32::from(r.curve[i])),
        );
    }
    let (boost, cut, stroke) = if r.enabled {
        (palette::ACCENT, palette::VIOLET, palette::ACCENT)
    } else {
        (palette::FAINT, palette::FAINT, palette::FAINT)
    };
    // The area between the curve and 0 dB, one column at a time.
    let p = cx.painter();
    for w in pts.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let span = (p1.x - p0.x).max(1);
        for x in p0.x..p1.x {
            let y = p0.y + (p1.y - p0.y) * (x - p0.x) / span;
            let (top, bottom, col) = if y < mid { (y, mid, boost) } else { (mid, y, cut) };
            if bottom > top {
                p.fill(Rect::new(x, top, x + 1, bottom), col, Opa::from_raw(46));
            }
        }
    }
    p.polyline(
        &pts,
        &twine::render::LineDsc {
            color: stroke,
            opa: Opa::COVER,
            width: 3,
            round_start: true,
            round_end: true,
            ..twine::render::LineDsc::default()
        },
    );
    // A handle per band at its slider's gain.
    for (i, &x) in BAND_X.iter().enumerate() {
        let c = Point::new(x_of(x), y_of(i32::from(r.gains[i])));
        circle(p, c, 5, palette::SURFACE, Opa::COVER);
        ring(p, c, 5, 2, stroke, Opa::COVER);
    }
}
