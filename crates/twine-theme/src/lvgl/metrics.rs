//! The LVGL theme's values that depend on the display: every size LVGL's `style_init()`
//! scales with `LV_DPX_CALC` by the display's DPI or picks by its size class
//! ([`DisplaySize`]). They are design elements — the standard spacings, radii and elevations
//! and the theme-private elements of this module — in one small table per display, computed
//! once when the display is created ([`table`], what
//! [`Lvgl::metrics`](twine_engine::ThemeHook::metrics) returns), so the theme's
//! [styles](super::styles) are `static` and a mode switch never touches them.
//!
//! | Element | Value | LVGL |
//! |---------|-------|------|
//! | `design::SPACE_XS`, `SPACE_S`, `SPACE_M` | `PAD_TINY`, `PAD_SMALL`, `PAD_DEF` (2/6/8, 10/12/14, 16/20/24 dp: small/medium/large display) | paddings |
//! | `design::SPACE_L` | 24 / 28 / 32 dp | — |
//! | `design::RADIUS_S`, `RADIUS_M`, `RADIUS_L` | `RADIUS_DEFAULT` / 2, `RADIUS_DEFAULT` (8 dp, 12 on large displays), · 2 | radii |
//! | `design::ELEVATION_*` | level 1: 3 dp wide, 3 dp down (the button shadow); 2: 8/4 dp; 3: 16/8 dp | — |
//! | [`DP_1`] … [`DP_20`] | `LV_DPX_CALC(n)` | `LV_DPX_CALC` |
//! | [`DP_1_NEG`], [`DP_4_NEG`] | `-LV_DPX_CALC(1)`, `-LV_DPX_CALC(4)` | text area cursor, switch knob |
//! | [`BORDER_WIDTH_X2`], [`BORDER_WIDTH_NEG`] | `BORDER_WIDTH * 2`, `-BORDER_WIDTH` | tab buttons |
//! | [`BUTTON_RADIUS`] | 8 / 12 / 16 dp | button radius |
//! | [`KEY_RADIUS`] | `RADIUS_DEFAULT` / 2 on small displays, else `RADIUS_DEFAULT` | keyboard keys |
//! | [`KEYBOARD_PAD`] | `PAD_SMALL` on large displays, else `PAD_TINY` | keyboard padding |
//! | [`ROTARY_SENSITIVITY`] | `dpi / 4 · 256` (in 1/256) | `rotary_sensitivity` |
//!
//! `dp` here is LVGL's unit: pixels at 160 DPI, rounded like `LV_DPX_CALC`
//! ([`dpx`]).

use twine_core::{Scale, Size};
use twine_style::design::{
    self, ElementTable, Elevation, ElevationLevel, LengthElement, RadiusElement, ScaleElement,
};
use twine_style::{Length, Radius, dpx};

use super::DisplaySize;

/// `LV_DPX_CALC(1)`: card lines, menu and list borders.
pub const DP_1: LengthElement = LengthElement::private(0);
/// `LV_DPX_CALC(2)`: LVGL's `BORDER_WIDTH` (cards, checkbox markers, the text area cursor).
pub const DP_2: LengthElement = LengthElement::private(1);
/// `LV_DPX_CALC(3)`: LVGL's `OUTLINE_WIDTH` (focus rings), the pressed grow, the checkbox
/// marker padding. (The button shadow is `design::ELEVATION_1_*`, the same 3 dp.)
pub const DP_3: LengthElement = LengthElement::private(2);
/// `LV_DPX_CALC(5)`: scrollbar width, button gaps, the LED glow spread.
pub const DP_5: LengthElement = LengthElement::private(3);
/// `LV_DPX_CALC(6)`: the knob padding.
pub const DP_6: LengthElement = LengthElement::private(4);
/// `LV_DPX_CALC(7)`: the scrollbar padding.
pub const DP_7: LengthElement = LengthElement::private(5);
/// `LV_DPX_CALC(10)`: the checkbox gap (`pad_gap`).
pub const DP_10: LengthElement = LengthElement::private(6);
/// `LV_DPX_CALC(15)`: arc width, the LED glow.
pub const DP_15: LengthElement = LengthElement::private(7);
/// `LV_DPX_CALC(20)`: large line spacing (rollers, dropdown lists).
pub const DP_20: LengthElement = LengthElement::private(8);
/// `-LV_DPX_CALC(1)`: the text area cursor's left padding.
pub const DP_1_NEG: LengthElement = LengthElement::private(9);
/// `-LV_DPX_CALC(4)`: the switch knob inset.
pub const DP_4_NEG: LengthElement = LengthElement::private(10);
/// `BORDER_WIDTH * 2`: the selected tab's underline and its top padding.
pub const BORDER_WIDTH_X2: LengthElement = LengthElement::private(11);
/// `-BORDER_WIDTH`: the focus outline of a tab bar, drawn inside.
pub const BORDER_WIDTH_NEG: LengthElement = LengthElement::private(12);
/// The keyboard's padding and gaps: `PAD_SMALL` on large displays, else `PAD_TINY`.
pub const KEYBOARD_PAD: LengthElement = LengthElement::private(13);
/// The button radius: 8, 12 or 16 dp (small, medium, large display).
pub const BUTTON_RADIUS: RadiusElement = RadiusElement::private(14);
/// The radius of keyboard keys: `RADIUS_DEFAULT / 2` on small displays, else
/// `RADIUS_DEFAULT`.
pub const KEY_RADIUS: RadiusElement = RadiusElement::private(15);
/// The rotary (encoder) scroll sensitivity of screens and tab pages: `dpi / 4 · 256`.
pub const ROTARY_SENSITIVITY: ScaleElement = ScaleElement::private(16);

/// The metrics table of a display with `dpi` (at least 1) and size class `size`: every
/// element of the [module documentation](self). Built once per display (it allocates the
/// table's private slots); never panics.
///
/// ```
/// use twine_style::design;
/// use twine_style::{Length, Radius};
/// use twine_theme::DisplaySize;
/// use twine_theme::lvgl::metrics::{self, BUTTON_RADIUS, DP_3};
///
/// let t = metrics::table(160, DisplaySize::Small);
/// assert_eq!(t.get(design::SPACE_M), Some(Length::Px(16)));
/// assert_eq!(t.get(DP_3), Some(Length::Px(3)));
/// assert_eq!(t.get(BUTTON_RADIUS), Some(Radius::Px(8)));
/// assert_eq!(metrics::table(320, DisplaySize::Large).get(BUTTON_RADIUS), Some(Radius::Px(32)));
/// ```
#[must_use]
pub fn table(dpi: u16, size: DisplaySize) -> ElementTable {
    let dpi = dpi.max(1);
    let d = |px| dpx(px, dpi);
    let px = |px| Length::Px(d(px));
    let pick = |l, m, s| match size {
        DisplaySize::Large => l,
        DisplaySize::Medium => m,
        DisplaySize::Small => s,
    };
    // #define RADIUS_DEFAULT LV_DPX_CALC(theme->disp_dpi, theme->disp_size == DISP_LARGE ? 12 : 8)
    let radius_default = d(pick(12, 8, 8));
    // #define PAD_DEF / PAD_SMALL / PAD_TINY
    let pad_def = d(pick(24, 20, 16));
    let pad_small = d(pick(14, 12, 10));
    let pad_tiny = d(pick(8, 6, 2));
    // #define BORDER_WIDTH LV_DPX_CALC(theme->disp_dpi, 2)
    let border_width = d(2);
    // lv_style_set_rotary_sensitivity(..., theme->disp_dpi / 4 * 256)
    let rotary =
        Scale::from_raw_256(u16::try_from((u32::from(dpi) / 4).saturating_mul(256)).unwrap_or(u16::MAX));
    let level = |w, y| ElevationLevel {
        width: px(w),
        offset_y: px(y),
        spread: Length::Px(0),
    };
    ElementTable::new()
        .with(design::SPACE_XS, Length::Px(pad_tiny))
        .with(design::SPACE_S, Length::Px(pad_small))
        .with(design::SPACE_M, Length::Px(pad_def))
        .with(design::SPACE_L, px(pick(32, 28, 24)))
        .with(design::RADIUS_S, Radius::Px(radius_default / 2))
        .with(design::RADIUS_M, Radius::Px(radius_default))
        .with(design::RADIUS_L, Radius::Px(radius_default.saturating_mul(2)))
        .elevation(Elevation {
            level_1: level(3, 3),
            level_2: level(8, 4),
            level_3: level(16, 8),
        })
        .with(DP_1, px(1))
        .with(DP_2, Length::Px(border_width))
        .with(DP_3, px(3))
        .with(DP_5, px(5))
        .with(DP_6, px(6))
        .with(DP_7, px(7))
        .with(DP_10, px(10))
        .with(DP_15, px(15))
        .with(DP_20, px(20))
        .with(DP_1_NEG, Length::Px(d(1).saturating_neg()))
        .with(DP_4_NEG, Length::Px(d(4).saturating_neg()))
        .with(BORDER_WIDTH_X2, Length::Px(border_width.saturating_mul(2)))
        .with(BORDER_WIDTH_NEG, Length::Px(border_width.saturating_neg()))
        .with(
            KEYBOARD_PAD,
            Length::Px(if size == DisplaySize::Large {
                pad_small
            } else {
                pad_tiny
            }),
        )
        // lv_style_set_radius(&btn, LV_DPX_CALC(dpi, LARGE ? 16 : MEDIUM ? 12 : 8))
        .with(BUTTON_RADIUS, Radius::Px(d(pick(16, 12, 8))))
        // lv_style_set_radius(&keyboard_button_bg, SMALL ? RADIUS_DEFAULT / 2 : RADIUS_DEFAULT)
        .with(
            KEY_RADIUS,
            Radius::Px(if size == DisplaySize::Small {
                radius_default / 2
            } else {
                radius_default
            }),
        )
        .with(ROTARY_SENSITIVITY, rotary)
}

/// The metrics table for a display of `resolution` at `dpi`, with the theme's overrides
/// (`Lvgl::dpi`, `Lvgl::display_size`).
pub(crate) fn for_display(
    dpi: Option<u16>,
    size: Option<DisplaySize>,
    display_dpi: u16,
    resolution: Size,
) -> ElementTable {
    table(
        dpi.unwrap_or(display_dpi),
        size.unwrap_or_else(|| DisplaySize::of(resolution)),
    )
}
