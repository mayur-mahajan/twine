//! The Setup screen: alarm limits, the patient, the display mode and the NIBP interval.
//!
//! - **Alarm limits** are steppers — the spinbox pattern of bedside monitors: `−`, the value,
//!   `+`, repeating while held. Each writes one field of the store's `limits` signal. The
//!   change is live: the tiles' limit captions, the trend plots' bands and the next alarm
//!   evaluation all read the same signal. A low limit can never pass its high limit.
//! - **Patient** opens a modal with the on-screen keyboard (`keyboard` attached to the
//!   focused text field); Save writes the store's patient signals, which the status bar shows.
//! - **Display** switches the theme mode — Day, Dark or Night — with one `set_mode`: every
//!   colour is a design element (`theme.rs`), so no view is rebuilt.
//! - **NIBP auto** is a roller: the interval of automatic cuff measurements.

use twine::prelude::*;
use twine::style::design::ColorElement;

use crate::components::{panel, segmented};
use crate::store::{AlarmLimits, Monitor};
use crate::theme::{self, CAPTION, FLAT_BUTTON, FLAT_BUTTON_PRESSED};

/// Size of a stepper key and width of a whole stepper (key, value, key).
const STEP_KEY: i32 = 26;
const STEPPER_W: i32 = 92;

/// One alarm limit: how to read and write it in [`AlarmLimits`] and its allowed range.
#[derive(Clone, Copy)]
struct Limit {
    get: fn(&AlarmLimits) -> i32,
    set: fn(&mut AlarmLimits, i32),
    min: i32,
    max: i32,
    step: i32,
}

/// A `−`/`+` key of a stepper: steps once on a tap and keeps stepping while held.
fn step_key(symbol: Symbol, f: impl Fn() + Copy + 'static, id: &'static str) -> impl View {
    button(label(symbol).font(&fonts::MONTSERRAT_12))
        .style(&FLAT_BUTTON)
        .on_state(State::PRESSED, |s| s.style(&FLAT_BUTTON_PRESSED))
        .size(STEP_KEY, STEP_KEY)
        .padding(0)
        .on_click(f)
        .on_long_press_repeat(f)
        .test_id(id)
}

/// A stepper for one limit in the parameter's colour. `low`/`high` keep the pair ordered.
fn stepper(m: Monitor, limit: Limit, color: ColorElement, id: &'static str) -> impl View {
    let Limit {
        get,
        set,
        min,
        max,
        step,
    } = limit;
    let nudge = move |d: i32| {
        move || {
            m.limits().update(|l| {
                let v = (get(l) + d).clamp(min, max);
                set(l, v);
            });
        }
    };
    row((
        step_key(Symbol::Minus, nudge(-step), id),
        label(text!("{}", m.limits().with(get)))
            .font(&fonts::MONTSERRAT_16)
            .text_color(color)
            .width(STEPPER_W - 2 * STEP_KEY - 4)
            .text_align(TextAlign::Center)
            .test_id(id),
        step_key(Symbol::Plus, nudge(step), id),
    ))
    .gap(2)
    .align_items(CrossAlign::Center)
}

/// A row of the limits panel: parameter, unit, low and high steppers.
fn limit_row(
    m: Monitor,
    name: &'static str,
    unit: &'static str,
    color: ColorElement,
    low: Option<Limit>,
    high: Option<Limit>,
    id: [&'static str; 2],
) -> impl View {
    let side = move |l: Option<Limit>, id: &'static str| match l {
        Some(l) => stepper(m, l, color, id).into_any(),
        None => label("off")
            .style(&CAPTION)
            .width(STEPPER_W)
            .text_align(TextAlign::Center)
            .into_any(),
    };
    row((
        column((
            label(name).font(&fonts::MONTSERRAT_14).text_color(color),
            label(unit)
                .style(&CAPTION)
                .font(&fonts::MONTSERRAT_10)
                .letter_spacing(0),
        ))
        .flex_grow(1)
        .gap(0),
        side(low, id[0]),
        side(high, id[1]),
    ))
    .fill_width()
    .gap(6)
    .align_items(CrossAlign::Center)
}

/// The alarm limits panel.
fn limits_panel(m: Monitor) -> impl View {
    // Each low limit stays below its high limit (and the other way round).
    let hr_low = Limit {
        get: |l| l.hr_low,
        set: |l, v| l.hr_low = v.min(l.hr_high - 5),
        min: 30,
        max: 150,
        step: 5,
    };
    let hr_high = Limit {
        get: |l| l.hr_high,
        set: |l, v| l.hr_high = v.max(l.hr_low + 5),
        min: 60,
        max: 250,
        step: 5,
    };
    let spo2_low = Limit {
        get: |l| l.spo2_low,
        set: |l, v| l.spo2_low = v,
        min: 80,
        max: 99,
        step: 1,
    };
    let rr_low = Limit {
        get: |l| l.rr_low,
        set: |l, v| l.rr_low = v.min(l.rr_high - 2),
        min: 4,
        max: 30,
        step: 1,
    };
    let rr_high = Limit {
        get: |l| l.rr_high,
        set: |l, v| l.rr_high = v.max(l.rr_low + 2),
        min: 10,
        max: 60,
        step: 1,
    };
    let sys_low = Limit {
        get: |l| l.sys_low,
        set: |l, v| l.sys_low = v.min(l.sys_high - 10),
        min: 50,
        max: 150,
        step: 5,
    };
    let sys_high = Limit {
        get: |l| l.sys_high,
        set: |l, v| l.sys_high = v.max(l.sys_low + 10),
        min: 90,
        max: 250,
        step: 5,
    };
    panel((
        row((
            label("LIMITS").style(&CAPTION).flex_grow(1),
            label("LOW")
                .style(&CAPTION)
                .width(STEPPER_W)
                .text_align(TextAlign::Center),
            label("HIGH")
                .style(&CAPTION)
                .width(STEPPER_W)
                .text_align(TextAlign::Center),
        ))
        .fill_width()
        .gap(8),
        limit_row(
            m,
            "HR",
            "bpm",
            theme::HR,
            Some(hr_low),
            Some(hr_high),
            ["hr_low", "hr_high"],
        ),
        limit_row(
            m,
            "SpO2",
            "%",
            theme::SPO2,
            Some(spo2_low),
            None,
            ["spo2_low", "spo2_high"],
        ),
        limit_row(
            m,
            "RR",
            "/min",
            theme::RESP,
            Some(rr_low),
            Some(rr_high),
            ["rr_low", "rr_high"],
        ),
        limit_row(
            m,
            "NIBP",
            "sys mmHg",
            theme::NIBP,
            Some(sys_low),
            Some(sys_high),
            ["sys_low", "sys_high"],
        ),
        spacer(),
        row((
            button(label("Adult defaults").font(&fonts::MONTSERRAT_12))
                .style(&FLAT_BUTTON)
                .on_state(State::PRESSED, |s| s.style(&FLAT_BUTTON_PRESSED))
                .padding_each(Insets::hor_ver(10, 5))
                .disabled(move || m.limits().get() == AlarmLimits::ADULT)
                .on_click(move || m.limits().set(AlarmLimits::ADULT))
                .test_id("defaults"),
            label("Simulated patient  \u{2022}  not a medical device")
                .style(&CAPTION)
                .font(&fonts::MONTSERRAT_10)
                .letter_spacing(0)
                .flex_grow(1)
                .text_align(TextAlign::Right),
        ))
        .fill_width()
        .align_items(CrossAlign::Center),
    ))
    .flex_grow(1)
    .fill_height()
    .gap(6)
    .padding(10)
}

/// The patient editor: a modal with two fields and the on-screen keyboard.
fn edit_patient(cx: Scope, m: Monitor) {
    let _ = cx.show_modal(move |cx, modal: ModalHandle| {
        let name = cx.signal(m.patient_name().get_untracked());
        let id = cx.signal(m.patient_id().get_untracked());
        let fields: [NodeRef<Textarea>; 2] = [cx.node_ref(), cx.node_ref()];
        let editing = cx.signal(0usize);
        let save = move || {
            m.patient_name().set(name.get_untracked());
            m.patient_id().set(id.get_untracked());
            modal.close();
        };
        let field = |text: Signal<String>, i: usize, hint: &'static str, test: &'static str| {
            textarea(text)
                .one_line(true)
                .placeholder(hint)
                .max_length(24)
                .flex_grow(1)
                .node_ref(fields[i])
                .on_focus(move || editing.set(i))
                .on_click(move || editing.set(i))
                .test_id(test)
        };
        panel((
            row((
                label("Patient").font(&fonts::MONTSERRAT_18).flex_grow(1),
                button(label("Cancel"))
                    .style(&FLAT_BUTTON)
                    .on_state(State::PRESSED, |s| s.style(&FLAT_BUTTON_PRESSED))
                    .on_click(move || modal.close())
                    .test_id("cancel"),
                button(label("Save")).on_click(save).test_id("save"),
            ))
            .fill_width()
            .gap(8)
            .align_items(CrossAlign::Center),
            row((
                field(name, 0, "LAST, First", "name_field"),
                field(id, 1, "MRN", "id_field").width(120),
            ))
            .fill_width()
            .gap(8),
            keyboard(move || Some(fields[editing.get()]))
                .on_ready(save)
                .on_cancel(move || modal.close())
                .fill_width()
                .flex_grow(1)
                .test_id("keyboard"),
        ))
        .size(468, 300)
        .padding(10)
        .gap(8)
    });
}

/// The display modes offered, in segment order (Day, Dark, Night).
const MODES: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Night];
/// The NIBP auto intervals offered by the roller, and their minutes.
static NIBP_EVERY: [&str; 5] = ["Off", "5 min", "10 min", "15 min", "30 min"];
static NIBP_MIN: [u32; 5] = [0, 5, 10, 15, 30];

/// The Setup screen.
pub fn page(cx: Scope) -> impl ViewExt {
    let m = cx.expect_context::<Monitor>();
    let theme = use_theme(cx);
    // The display mode as a segment index, written back to the theme when a segment is tapped.
    let display = cx.signal(MODES.iter().position(|&md| md == theme.mode()).unwrap_or(1));
    cx.effect(move || theme.set_mode(MODES[display.get()]));
    let every = cx.signal(
        NIBP_MIN
            .iter()
            .position(|&v| v == m.nibp_every().get_untracked())
            .unwrap_or(3),
    );
    cx.effect(move || m.nibp_every().set(NIBP_MIN[every.get()]));

    let right = column((
        panel((
            column((
                label("PATIENT").style(&CAPTION),
                label(m.patient_name())
                    .font(&fonts::MONTSERRAT_16)
                    .long_mode(LongMode::Dots)
                    .max_lines(1)
                    .fill_width(),
                row((
                    label("MRN").style(&CAPTION),
                    label(m.patient_id()).style(&CAPTION).letter_spacing(0),
                ))
                .gap(4),
            ))
            .flex_grow(1)
            .gap(3),
            button(label(Symbol::Edit).font(&fonts::MONTSERRAT_14))
                .style(&FLAT_BUTTON)
                .on_state(State::PRESSED, |s| s.style(&FLAT_BUTTON_PRESSED))
                .size(32, 32)
                .padding(0)
                .on_click(move || edit_patient(cx, m))
                .test_id("edit_patient"),
        ))
        .layout(Layout::row())
        .fill_width()
        .gap(6)
        .padding(10)
        .align_items(CrossAlign::Center),
        panel((
            label("DISPLAY").style(&CAPTION),
            segmented(&["Day", "Dark", "Night"], display, "display"),
        ))
        .fill_width()
        .gap(6)
        .padding(10),
        panel((
            label("NIBP AUTO").style(&CAPTION).flex_grow(1),
            roller(&NIBP_EVERY, every)
                .visible_rows(3)
                .width(84)
                .font(&fonts::MONTSERRAT_12)
                .test_id("nibp_every"),
        ))
        .layout(Layout::row())
        .fill_width()
        .flex_grow(1)
        .padding_each(Insets::new(10, 4, 6, 4))
        .align_items(CrossAlign::Center),
    ))
    .width(184)
    .fill_height()
    .gap(6);

    row((limits_panel(m), right))
        .fill()
        .padding(8)
        .gap(6)
        .flag(ObjFlags::SCROLLABLE, false)
}
