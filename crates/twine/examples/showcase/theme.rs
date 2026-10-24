//! The monitor's design tokens and theme.
//!
//! **Why tokens.** Every colour in the app is a [design element](twine::style::design): the
//! standard ones (`design::BACKGROUND`, `design::SURFACE`, …) overridden for a clinical look,
//! plus the monitor's own — one colour per parameter, the alarm colours and a few neutrals.
//! Styles and views name the element, never a colour, so switching the display between
//! **Day**, **Dark** and **Night** (`ThemeMode::Light` / `Dark` / `Night`) is a single
//! `set_mode` call: the display swaps its element table (a `static`) and every node
//! re-resolves. No view is rebuilt, no binding runs, nothing is allocated.
//!
//! **The palette** follows bedside-monitor convention so a clinician reads it at a glance: ECG
//! and heart rate green, SpO2 and pleth cyan, blood pressure red, respiration yellow,
//! temperature white. Dark (the default) is near-black for contrast in a ward; Day is a light
//! scheme with deeper hues that stay readable on white; Night keeps the colour coding but dims
//! everything to about a third of its luminance so it does not light up a sleeping room.
//! Alarm colours follow IEC 60601-1-8: red for high priority, yellow for medium.
//!
//! **Typography** is Montserrat throughout: 12 px captions and units, 14 px body, large
//! tabular numerals (24–48 px) for the vitals — the hierarchy is the numbers.

use twine::engine::{DesignSystem, Recipe, Theme};
use twine::prelude::*;
use twine::style::StyleValue;
use twine::style::design::{ColorElement, ElementTable};

use crate::trend_plot::TREND_RECIPE;
use crate::waveform::WAVEFORM_RECIPE;

/// Heart rate and ECG (green).
pub const HR: ColorElement = ColorElement::custom(0);
/// SpO2 and pleth (cyan).
pub const SPO2: ColorElement = ColorElement::custom(1);
/// Non-invasive blood pressure (red).
pub const NIBP: ColorElement = ColorElement::custom(2);
/// Respiration (yellow).
pub const RESP: ColorElement = ColorElement::custom(3);
/// Temperature (white / slate).
pub const TEMP: ColorElement = ColorElement::custom(4);
/// High-priority alarm (red) and the text on it.
pub const ALARM_HIGH: ColorElement = ColorElement::custom(5);
pub const ON_ALARM_HIGH: ColorElement = ColorElement::custom(6);
/// Medium-priority alarm (yellow) and the text on it.
pub const ALARM_MED: ColorElement = ColorElement::custom(7);
pub const ON_ALARM_MED: ColorElement = ColorElement::custom(8);
/// The status and navigation bars.
pub const CHROME: ColorElement = ColorElement::custom(9);
/// Hairlines between waveform strips and grid lines of the trend plots.
pub const HAIRLINE: ColorElement = ColorElement::custom(10);

/// `(element, Day, Dark, Night)`: the whole palette in one table.
const PALETTE: [(ColorElement, u32, u32, u32); 23] = [
    // Standard elements: the clinical neutrals.
    (design::BACKGROUND, 0x00E8_ECF0, 0x0003_0507, 0x0000_0000),
    (design::SURFACE, 0x00FF_FFFF, 0x000C_1117, 0x0005_0708),
    (design::SURFACE_VARIANT, 0x00DD_E3EA, 0x0018_2029, 0x000C_0F12),
    (design::OUTLINE, 0x00CD_D5DE, 0x0022_2C37, 0x0011_1519),
    (design::ON_SURFACE, 0x000F_172A, 0x00E6_ECF2, 0x0078_7F86),
    (design::ON_SURFACE_MUTED, 0x0058_6778, 0x0078_8796, 0x0043_4A51),
    (design::PRIMARY, 0x001D_6BE5, 0x003D_8BFD, 0x0025_4F86),
    (design::ON_PRIMARY, 0x00FF_FFFF, 0x00FF_FFFF, 0x00B8_C0C8),
    (design::SECONDARY, 0x001D_6BE5, 0x003D_8BFD, 0x0025_4F86),
    (design::FOCUS_RING, 0x001D_6BE5, 0x003D_8BFD, 0x0025_4F86),
    (design::SCRIM, 0x000F_172A, 0x0000_0000, 0x0000_0000),
    // The monitor's own.
    (HR, 0x000B_934A, 0x0029_E07E, 0x0014_7A45),
    (SPO2, 0x0006_88AE, 0x0026_D4F0, 0x0013_7584),
    (NIBP, 0x00D3_262B, 0x00FF_5C62, 0x008F_3539),
    (RESP, 0x00A8_6A00, 0x00FA_CC15, 0x0087_6F14),
    (TEMP, 0x0033_4155, 0x00E6_ECF2, 0x0078_7F86),
    (ALARM_HIGH, 0x00D9_1E25, 0x00E3_232A, 0x00A1_1A1F),
    (ON_ALARM_HIGH, 0x00FF_FFFF, 0x00FF_FFFF, 0x00E0_D0D0),
    (ALARM_MED, 0x00F2_B100, 0x00F5_B800, 0x0099_7400),
    (ON_ALARM_MED, 0x0024_1600, 0x0021_1600, 0x0010_0A00),
    (CHROME, 0x00F8_FAFC, 0x0008_0B0F, 0x0002_0303),
    (HAIRLINE, 0x00E1_E6EC, 0x0016_1D25, 0x000A_0D10),
    (design::NEUTRAL, 0x00B8_C2CD, 0x0030_3B47, 0x0018_1D22),
];

/// The number of the monitor's own elements (`custom(0)` … `custom(10)`).
const OWN: usize = 11;

/// The column of `mode` in [`PALETTE`] (Day, Dark, Night).
const fn column(entry: &(ColorElement, u32, u32, u32), mode: usize) -> Color {
    Color::hex(match mode {
        0 => entry.1,
        1 => entry.2,
        _ => entry.3,
    })
}

/// The monitor's own elements of `mode` (a column of [`PALETTE`]) as table slots.
const fn own(mode: usize) -> [Option<StyleValue>; OWN] {
    let mut slots = [None; OWN];
    let mut rest: &[(ColorElement, u32, u32, u32)] = &PALETTE;
    while let [entry, tail @ ..] = rest {
        if entry.0.is_custom() {
            // Evaluated while compiling: an index out of range does not compile.
            slots[entry.0.index() as usize] = Some(StyleValue::Color(column(entry, mode)));
        }
        rest = tail;
    }
    slots
}

/// The LVGL theme's table of `mode` with the standard elements of [`PALETTE`]'s column `col`,
/// Montserrat 12/14/20 as small/body/large fonts, and the monitor's elements `own`.
const fn table(mode: ThemeMode, col: usize, own: &'static [Option<StyleValue>]) -> ElementTable {
    let mut t = Lvgl::SET
        .fonts(FontScale {
            small: &fonts::MONTSERRAT_12,
            normal: &fonts::MONTSERRAT_14,
            large: &fonts::MONTSERRAT_20,
        })
        .table(mode);
    let mut rest: &[(ColorElement, u32, u32, u32)] = &PALETTE;
    while let [entry, tail @ ..] = rest {
        // The monitor's own elements are not standard: `color` leaves them to `custom`.
        t = t.color(entry.0, column(entry, col));
        rest = tail;
    }
    t.custom(own)
}

static DAY_OWN: [Option<StyleValue>; OWN] = own(0);
static DARK_OWN: [Option<StyleValue>; OWN] = own(1);
static NIGHT_OWN: [Option<StyleValue>; OWN] = own(2);
/// Day ([`PALETTE`]'s first column).
pub static DAY: ElementTable = table(ThemeMode::Light, 0, &DAY_OWN);
/// Dark, the default ([`PALETTE`]'s second column).
pub static DARK: ElementTable = table(ThemeMode::Dark, 1, &DARK_OWN);
/// Night ([`PALETTE`]'s third column).
pub static NIGHT: ElementTable = table(ThemeMode::Night, 2, &NIGHT_OWN);

/// The monitor's design system: the LVGL theme's widgets, re-coloured by [`PALETTE`] in Day,
/// Dark and Night (in that order, Dark first: the start mode), with the custom widgets'
/// recipes, so they theme like built-in ones.
pub struct Monitor;

impl DesignSystem for Monitor {
    type Base = Lvgl;
    const NAME: &'static str = "monitor";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[
        (ThemeMode::Dark, &DARK),
        (ThemeMode::Light, &DAY),
        (ThemeMode::Night, &NIGHT),
    ];
    const RECIPES: &'static [&'static Recipe] = &[&WAVEFORM_RECIPE, &TREND_RECIPE];
}

/// The application theme.
pub static MONITOR_THEME: Theme<Monitor> = Theme::new();

// ---- Shared styles ------------------------------------------------------------------------
// `static`s (in flash on a device) built only from design elements, so they follow the mode.

/// A panel: the tiles of the monitor screen and the cards of the other screens.
pub static PANEL: Style = style! {
    bg_color: design::SURFACE,
    bg_opacity: Opa::COVER,
    radius: 8,
    border_width: 0,
    shadow_width: 0,
    padding: 8,
};

/// A plain container used only for layout: no background, border, shadow or padding (the
/// theme's container look removed).
pub static BARE: Style = style! {
    bg_opacity: Opa::TRANSP,
    border_width: 0,
    shadow_width: 0,
    padding: 0,
    radius: 0,
};

/// Section headings and parameter labels: small, muted, spaced capitals.
pub static CAPTION: Style = style! {
    text_color: design::ON_SURFACE_MUTED,
    font: &fonts::MONTSERRAT_12,
    letter_spacing: 1,
};

/// A flat button on a panel (Setup's −/+ keys, the trend range selector).
pub static FLAT_BUTTON: Style = style! {
    bg_color: design::SURFACE_VARIANT,
    bg_opacity: Opa::COVER,
    text_color: design::ON_SURFACE,
    shadow_width: 0,
    radius: 6,
};
/// … pressed.
pub static FLAT_BUTTON_PRESSED: Style = style! { bg_color: design::NEUTRAL };
/// … selected (a segment of a segmented control).
pub static FLAT_BUTTON_CHECKED: Style = style! {
    bg_color: design::PRIMARY,
    text_color: design::ON_PRIMARY,
};
