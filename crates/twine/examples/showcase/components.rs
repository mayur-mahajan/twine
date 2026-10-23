//! Small building blocks shared by the screens: the vital-sign tile, the heart symbol, the
//! segmented control and the waveform strip.
//!
//! Each is a plain function returning a view (a *component*): it composes built-in widgets and
//! styles them with the design tokens of `theme.rs`. The only custom widget here is the heart
//! symbol — a filled polygon, because the fonts carry no heart glyph.
//!
//! **Alarm look of a tile.** A tile whose parameter is in alarm gets a 2 px border in the
//! alarm colour; while the alarm's flash phase is on, its background turns that colour and the
//! numbers switch to the matching on-colour (white on red, near-black on yellow) — the
//! convention of most monitors. All of it is closures over the store's signals: the flash
//! phase toggles at 2 Hz (high) or 0.5 Hz (medium), which re-runs exactly these few bindings,
//! and the engine redraws exactly that tile.

use twine::core::Angle;
use twine::core::math::{TRIG_SHIFT, cos, sin};
use twine::prelude::*;
use twine::render::PolygonDsc;
use twine::style::design::ColorElement;
use twine::style::design::{ColorValue, LengthValue};

use crate::store::{Monitor, Param, Priority};
use crate::theme::{self, CAPTION, FLAT_BUTTON, FLAT_BUTTON_CHECKED, FLAT_BUTTON_PRESSED, PANEL};
use crate::waveform::{Trace, waveform};

// ---- Panels ------------------------------------------------------------------------------------

/// A panel: the theme's card restyled with [`PANEL`], its children in a column (give it
/// `.layout(Layout::row())` for a row). Fixed content: never scrolls.
pub fn panel(children: impl ViewSeq) -> Flex<Card> {
    card(children).style(&PANEL).flag(ObjFlags::SCROLLABLE, false)
}

// ---- Vital-sign tiles ---------------------------------------------------------------------

/// The colours of a tile (background, numbers) for its parameter's alarm state now.
fn tile_colors(m: Monitor, param: Param, normal: ColorElement) -> (ColorElement, ColorElement) {
    match m.alarm_of(param) {
        Some(Priority::High) if m.flash_high().get() => (theme::ALARM_HIGH, theme::ON_ALARM_HIGH),
        Some(Priority::Medium) if m.flash_med().get() => (theme::ALARM_MED, theme::ON_ALARM_MED),
        _ => (design::SURFACE, normal),
    }
}

/// The border colour of a tile: the alarm's colour while one is active.
fn tile_border(m: Monitor, param: Param) -> Option<ColorElement> {
    m.alarm_of(param).map(|p| match p {
        Priority::High => theme::ALARM_HIGH,
        Priority::Medium => theme::ALARM_MED,
    })
}

/// A tile for `param`: `header` across the top (caption, limits, symbols), `body` below.
/// `color` is the parameter's colour; `fg` gives each child the current number colour.
pub fn tile(
    m: Monitor,
    param: Param,
    color: ColorElement,
    header: impl ViewSeq,
    body: impl ViewSeq,
) -> impl ViewExt {
    panel((
        row(header)
            .fill_width()
            .gap(4)
            .align_items(CrossAlign::Center)
            .clickable(false),
        row(body)
            .fill_width()
            .flex_grow(1)
            .gap(6)
            .justify(MainAlign::End)
            .align_items(CrossAlign::End)
            .clickable(false),
    ))
    .gap(0)
    .padding_each(Insets::new(8, 5, 8, 2))
    .bg_color(move || ColorValue::from(tile_colors(m, param, color).0))
    .border_width(move || LengthValue::from(Length::Px(if tile_border(m, param).is_some() { 2 } else { 0 })))
    .border_color(move || ColorValue::from(tile_border(m, param).unwrap_or(design::OUTLINE)))
}

/// The colour of a tile's text now (its parameter colour, or the alarm on-colour).
pub fn fg(m: Monitor, param: Param, color: ColorElement) -> impl Fn() -> ColorValue + 'static {
    move || ColorValue::from(tile_colors(m, param, color).1)
}

/// A tile caption ("HR") in the parameter's colour.
pub fn tile_caption(m: Monitor, param: Param, color: ColorElement, text: &'static str) -> impl ViewExt {
    label(text)
        .style(&CAPTION)
        .font(&fonts::MONTSERRAT_14)
        .text_color(fg(m, param, color))
}

/// The alarm limits beside a caption ("50-120"), small and muted, pushed to the right.
pub fn limits_text<MT>(text: impl IntoText<MT>) -> WidgetView<Label> {
    label(text)
        .style(&CAPTION)
        .flex_grow(1)
        .text_align(TextAlign::Right)
}

// ---- Waveform strip -------------------------------------------------------------------------

/// A waveform strip: a panel with the sweep trace and its label ("II", "Pleth", "Resp").
pub fn strip(
    trace: Trace,
    color: ColorElement,
    name: &'static str,
    range: core::ops::RangeInclusive<i32>,
    filled: bool,
) -> impl ViewExt {
    container((
        waveform(trace)
            .range(range)
            .filled(filled)
            .part(Part::Indicator, move |s| s.line_color(color))
            .fill(),
        label(name).style(&CAPTION).text_color(color).pos(4, 2),
    ))
    .style(&PANEL)
    // Keep the trace off the rounded corners (cheaper than clipping every column to them).
    .padding_each(Insets::new(4, 2, 4, 2))
    .flag(ObjFlags::SCROLLABLE, false)
}

// ---- Heart symbol -----------------------------------------------------------------------------

/// The class of [`Heart`]: `Main` is drawn as a heart in its `bg_color`.
static HEART_CLASS: WidgetClass = WidgetClass::new("heart");

/// Points on the outline of the heart curve `x = 16 sin³t`,
/// `y = 13 cos t − 5 cos 2t − 2 cos 3t − cos 4t` (x in −16..=16, y in −17..=12), in integer
/// trigonometry; scaled into the widget when drawn.
const HEART_POINTS: usize = 32;

/// A heart symbol drawn as a polygon (no font has one). Flashes with each beat through its
/// opacity, bound by the view.
#[derive(Debug, Default)]
pub struct Heart;

impl Widget for Heart {
    fn class(&self) -> &'static WidgetClass {
        &HEART_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let area = cx.content_area();
        let (w, h) = (area.width(), area.height());
        if w < 4 || h < 4 {
            return;
        }
        let rect = cx.rect_dsc(Part::Main).base;
        let c = Point::new(area.x0 + w / 2, area.y0 + h * 17 / 30);
        let mut pts = [Point::new(0, 0); HEART_POINTS];
        for (i, p) in pts.iter_mut().enumerate() {
            let t = Angle::deci_deg(3600 * i as i32 / HEART_POINTS as i32);
            let k = |n: i32| Angle::deci_deg(t.as_deci_deg() * n);
            let s = i64::from(sin(t));
            // 16 sin³t, with sin in Q15.
            let x = (16 * s * s * s) >> (3 * TRIG_SHIFT);
            let y = (13 * cos(t) - 5 * cos(k(2)) - 2 * cos(k(3)) - cos(k(4))) >> TRIG_SHIFT;
            *p = Point::new(c.x + x as i32 * w / 34, c.y - y * h / 30);
        }
        let dsc = PolygonDsc {
            color: rect.bg_color,
            opa: rect.bg_opa,
            ..PolygonDsc::default()
        };
        cx.painter().polygon(&pts, &dsc);
    }
}

/// The heart-beat indicator: a heart in `color` that lights up with each R wave and fades.
pub fn heart(cx: Scope, m: Monitor, color: ColorElement) -> impl ViewExt {
    // The fade is a tween of the opacity: a 60 Hz animation for 200 ms after each beat, idle
    // in between (the engine runs no code for it then).
    let glow = cx.tween(
        move || if m.pulse().get() { 255u8 } else { 60 },
        AnimSpec::new(Duration::ms(200)).ease_out(),
    );
    widget_view(|| Heart)
        .size(15, 13)
        .bg_color(color)
        .bg_opacity(move || Opa::from_raw(glow.get()))
}

// ---- Segmented control -------------------------------------------------------------------------

/// A segmented control: one flat button per option, the selected one highlighted. `selected`
/// is written when a segment is tapped.
pub fn segmented(
    options: &'static [&'static str],
    selected: Signal<usize>,
    id: &'static str,
) -> impl ViewExt {
    row(options
        .iter()
        .enumerate()
        .map(|(i, text)| {
            button(label(*text).font(&fonts::MONTSERRAT_12))
                .style(&FLAT_BUTTON)
                .radius(5)
                .padding_each(Insets::hor_ver(10, 5))
                .on_state(State::PRESSED, |s| s.style(&FLAT_BUTTON_PRESSED))
                .on_state(State::CHECKED, |s| s.style(&FLAT_BUTTON_CHECKED))
                .state(State::CHECKED, move || selected.get() == i)
                .on_click(move || selected.set(i))
                .test_id(id)
        })
        .collect::<Vec<_>>())
    .style(&FLAT_BUTTON)
    .padding(3)
    .gap(2)
    .radius(8)
}
