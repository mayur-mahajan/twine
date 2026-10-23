//! The Monitor screen (home): three sweep waveforms with their numerics beside them, and the
//! blood pressure and temperature below — the layout of a bedside monitor.
//!
//! ```text
//! ┌ II ─────────────────────────────┐ ┌ HR ♥   50-120 ┐
//! │  ECG sweep                      │ │            72 │
//! ├ Pleth ──────────────────────────┤ ├ SpO2      >90 ┤
//! │  pleth sweep (filled)           │ │          97 % │
//! ├ Resp ───────────────────────────┤ ├ RR  8-24   16 ┤
//! ├ NIBP mmHg 14:25    118/76 (90)  ┤ ├ TEMP °C  36.8 ┤
//! ```
//!
//! **What redraws when.** At 60 Hz only the three waveform strips change, each by a few
//! pixel columns (see `waveform.rs`). The heart symbol fades for 200 ms per beat. The numbers
//! are `text!` labels over the store's 1 Hz signals: a label re-formats in place (no
//! allocation) and redraws only when its text actually changed. A tile in alarm flashes at
//! the alarm's rate (see `components.rs`). Everything else is idle.
//!
//! **Safety-critical numerics.** The HR and SpO2 numbers are marked `safety_critical`
//! (feature `display-integrity`, part of the safety profile): the engine always draws them
//! whole and uncached and lists them in its critical report, so a product could cross-check
//! the pixels on the panel against the value with a frame observer or a read-back.

use core::fmt;

use twine::prelude::*;

use crate::components::{fg, heart, limits_text, panel, strip, tile, tile_caption};
use crate::patient::Nibp;
use crate::store::{Monitor, Param, hhmm};
use crate::theme::{self, CAPTION};

/// The NIBP readout: the last result, or the cuff pressure while measuring.
struct NibpText(Option<i32>, Nibp);

impl fmt::Display for NibpText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(cuff) => write!(f, "{cuff}"),
            None => write!(f, "{}/{}", self.1.sys, self.1.dia),
        }
    }
}

/// The Monitor screen.
pub fn page(cx: Scope) -> impl ViewExt {
    let m = cx.expect_context::<Monitor>();
    let lim = move || m.limits().get();

    // ECG with heart rate.
    let ecg = row((
        strip(m.ecg(), theme::HR, "II", -450..=1250, false)
            .flex_grow(1)
            .fill_height(),
        tile(
            m,
            Param::Hr,
            theme::HR,
            (
                tile_caption(m, Param::Hr, theme::HR, "HR"),
                heart(cx, m, theme::HR),
                limits_text(text!("{}-{}", lim().hr_low, lim().hr_high)),
            ),
            (label(text!("{}", m.hr().get()))
                .font(&fonts::MONTSERRAT_44)
                .text_color(fg(m, Param::Hr, theme::HR))
                .safety_critical(true)
                .test_id("hr"),),
        )
        .width(TILE_W)
        .fill_height(),
    ));

    // Pleth with SpO2.
    let spo2 = row((
        strip(m.pleth(), theme::SPO2, "Pleth", -60..=1150, true)
            .flex_grow(1)
            .fill_height(),
        tile(
            m,
            Param::Spo2,
            theme::SPO2,
            // A single row: caption and limit stacked on the left, the number on the right.
            (
                column((
                    row((
                        tile_caption(m, Param::Spo2, theme::SPO2, "SpO2"),
                        label("%").style(&CAPTION),
                    ))
                    .gap(4)
                    .align_items(CrossAlign::Center),
                    label(text!("> {}", lim().spo2_low)).style(&CAPTION),
                ))
                .flex_grow(1)
                .gap(4)
                .clickable(false),
                label(text!("{}", m.spo2().get()))
                    .font(&fonts::MONTSERRAT_40)
                    .text_color(fg(m, Param::Spo2, theme::SPO2))
                    .safety_critical(true)
                    .test_id("spo2"),
            ),
            (),
        )
        .width(TILE_W)
        .fill_height(),
    ));

    // Respiration with its rate: a single-line tile.
    let resp = row((
        strip(m.resp(), theme::RESP, "Resp", -650..=650, false)
            .flex_grow(1)
            .fill_height(),
        tile(
            m,
            Param::Rr,
            theme::RESP,
            (
                tile_caption(m, Param::Rr, theme::RESP, "RR"),
                limits_text(text!("{}-{}", lim().rr_low, lim().rr_high)),
                label(text!("{}", m.rr().get()))
                    .font(&fonts::MONTSERRAT_28)
                    .text_color(fg(m, Param::Rr, theme::RESP))
                    .width(44)
                    .text_align(TextAlign::Right)
                    .test_id("rr"),
            ),
            (),
        )
        .width(TILE_W)
        .fill_height(),
    ));

    // Blood pressure (tap to measure now) and temperature.
    let measuring = move || m.cuff().get().is_some();
    let nibp = tile(
        m,
        Param::Nibp,
        theme::NIBP,
        (
            column((
                tile_caption(m, Param::Nibp, theme::NIBP, "NIBP"),
                label(move || {
                    if measuring() {
                        "Measuring..."
                    } else {
                        "mmHg  tap to start"
                    }
                })
                .style(&CAPTION)
                .letter_spacing(0),
            ))
            .gap(1),
            label(text!(
                "{:02}:{:02}",
                hhmm(m.nibp_at().get()).0,
                hhmm(m.nibp_at().get()).1
            ))
            .style(&CAPTION)
            .flex_grow(1)
            .text_align(TextAlign::Center),
            label(text!("{}", NibpText(m.cuff().get(), m.nibp().get())))
                .font(&fonts::MONTSERRAT_28)
                .text_color(fg(m, Param::Nibp, theme::NIBP))
                .test_id("nibp"),
            label(text!("({})", m.nibp().get().map()))
                .font(&fonts::MONTSERRAT_18)
                .text_color(fg(m, Param::Nibp, theme::NIBP))
                .hidden(measuring),
        ),
        (),
    )
    .flex_grow(1)
    .fill_height()
    .on_click(move || m.start_nibp())
    .test_id("nibp_tile");

    let temp = panel((row((
        label("TEMP")
            .style(&CAPTION)
            .font(&fonts::MONTSERRAT_14)
            .text_color(theme::TEMP),
        label("°C").style(&CAPTION).flex_grow(1),
        label(text!("{}.{}", m.temp().get() / 10, m.temp().get() % 10))
            .font(&fonts::MONTSERRAT_28)
            .text_color(theme::TEMP)
            .test_id("temp"),
    ))
    .fill_width()
    .gap(6)
    .align_items(CrossAlign::Center),))
    .padding_each(Insets::new(8, 4, 8, 2))
    .width(TILE_W)
    .fill_height();

    column((
        ecg.fill_width().height(82).gap(GAP),
        spo2.fill_width().height(64).gap(GAP),
        resp.fill_width().height(46).gap(GAP),
        row((nibp, temp)).fill_width().flex_grow(1).gap(GAP),
    ))
    .fill()
    .padding(GAP)
    .gap(GAP)
    .flag(ObjFlags::SCROLLABLE, false)
}

/// Width of the numeric tiles.
const TILE_W: i32 = 150;
/// The spacing grid: 4 px between panels.
const GAP: i32 = 4;
