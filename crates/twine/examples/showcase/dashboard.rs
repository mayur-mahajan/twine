//! The dashboard tab: a simulated sensor sampled by a timer drives a gauge, a chart and a few
//! derived values. Each sample changes a handful of properties; nothing else is redrawn.

use twine::prelude::*;

/// Samples kept by the chart.
const SAMPLES: usize = 24;
/// The chart's size in pixels.
const CHART_W: i32 = 200;
const CHART_H: i32 = 70;

/// A stand-in for a sensor: a triangle wave between 18 and 30 °C with a little jitter.
fn sample(t: u32) -> i32 {
    let phase = (t % 24) as i32;
    let tri = if phase < 12 { phase } else { 24 - phase };
    let jitter = ((t.wrapping_mul(2_654_435_761) >> 28) as i32) - 8; // -8..=7
    180 + tri * 10 + jitter // tenths of a degree
}

/// Maps the history (tenths of a degree, 150..=330) into chart coordinates.
fn chart_points(history: &[i32]) -> Vec<Point> {
    let step = CHART_W / (SAMPLES as i32 - 1);
    history
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let y = CHART_H - (v - 150) * CHART_H / 180;
            Point::new(i as i32 * step, y.clamp(0, CHART_H))
        })
        .collect()
}

pub fn page(cx: Scope) -> impl View {
    let tick = cx.signal(SAMPLES as u32);
    let history = cx.signal((0..SAMPLES as u32).map(sample).collect::<Vec<_>>());
    cx.interval(Duration::ms(500), move || {
        tick.update(|t| *t += 1);
        let v = sample(tick.get_untracked());
        history.update(|h| {
            h.remove(0);
            h.push(v);
        });
    });

    let current = cx.memo(move || history.with(|h| h.last().copied().unwrap_or(0)));
    let peak = cx.memo(move || history.with(|h| h.iter().copied().max().unwrap_or(0)));
    // The gauge glides to each new sample instead of jumping.
    let shown = cx.tween(move || current.get(), AnimSpec::new(Duration::ms(400)).ease_out());
    // The arc is an editable widget; this one only displays, so it gets its own signal
    // (following the tween) and ignores touches.
    let needle = cx.signal(shown.get_untracked());
    cx.effect(move || needle.set(shown.get()));
    let alarm = cx.memo(move || current.get() >= 280);
    let points = cx.memo(move || history.with(|h| chart_points(h)));

    let gauge = card((
        label("Temperature").text_color(design::ON_SURFACE_MUTED),
        stack((
            arc(needle)
                .range(150..=330)
                .size(110, 110)
                .flag(ObjFlags::CLICKABLE, false)
                .test_id("gauge"),
            label(text!("{}.{} °C", shown.get() / 10, shown.get() % 10))
                .font(&fonts::MONTSERRAT_20)
                .test_id("temperature"),
        ))
        .size(120, 116)
        .bg_opacity(Opa::TRANSP)
        .border_width(0)
        .padding(0),
    ))
    .padding(10)
    .gap(4)
    .align_items(CrossAlign::Center);

    let trend = card((
        label("Last 12 seconds").text_color(design::ON_SURFACE_MUTED),
        line(points)
            .width(3)
            .rounded(true)
            .line_color(design::PRIMARY)
            .size(CHART_W, CHART_H),
        row((
            label(text!("Peak {}.{} °C", peak.get() / 10, peak.get() % 10)),
            label(text!("{}", if alarm.get() { "Too warm" } else { "Normal" }))
                .text_color(move || if alarm.get() { design::DANGER } else { design::OK })
                .test_id("status"),
        ))
        .fill_width()
        .justify(MainAlign::SpaceBetween),
    ))
    .padding(10)
    .gap(8)
    .width(CHART_W + 24);

    row((gauge, trend))
        .gap(10)
        .padding(10)
        .justify(MainAlign::Center)
        .align_items(CrossAlign::Center)
        .fill()
}
