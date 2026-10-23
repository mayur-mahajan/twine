//! Twine Player's look: the palette, the type scale and the theme that installs them.
//!
//! **Design.** A near-black, slightly blue "studio" background with three surface steps
//! (sidebar, panel, raised), warm off-white text in three emphasis levels, and one accent
//! family — coral to amber, with a violet counterpoint used by the visualizer's gradient.
//! Everything sits on a 4 px spacing grid ([`S1`]…[`S5`]); corners are 6 px for controls and
//! 10–14 px for panels and artwork.
//!
//! **How it is wired.** The colors are [design elements](twine::style::design): the standard
//! ones (`BACKGROUND`, `SURFACE`, `ON_SURFACE`, `PRIMARY`, …) are overridden and the app's own
//! ([`SIDEBAR`], [`RAISED`], [`LINE`], …) are added to Twine's `DefaultTheme` in dark mode, so
//! the built-in widgets the app keeps (slider, dropdown, text field, scrollbars) pick the
//! palette up through the theme, and views refer to colors by role, never by value. The
//! dropdown's pop-up list is created by the widget at run time, so it is restyled through a
//! theme class registration ([`style_dropdown_list`]) rather than a view modifier. The custom
//! drawn widgets (artwork, visualizer, response curve, meters) read the plain [`palette`]
//! constants, which are the same values.

use twine::prelude::*;
use twine::style::design::ColorElement;
use twine::theme::ClassCx;
use twine::widgets_ext::dropdown::DROPDOWN_LIST_CLASS;

/// The raw colors, for the custom-drawn widgets (views use the design elements below).
pub mod palette {
    use twine::prelude::Color;

    pub const BG: Color = Color::hex(0x0B_0C_10);
    pub const SIDEBAR: Color = Color::hex(0x11_12_18);
    pub const SURFACE: Color = Color::hex(0x16_17_1F);
    pub const RAISED: Color = Color::hex(0x20_22_2C);
    pub const PRESSED: Color = Color::hex(0x2A_2D_3A);
    pub const LINE: Color = Color::hex(0x26_28_33);
    pub const TEXT: Color = Color::hex(0xF3_F1_EC);
    pub const MUTED: Color = Color::hex(0xA4_A4_B4);
    pub const FAINT: Color = Color::hex(0x66_68_79);
    /// "Twine coral": the accent.
    pub const ACCENT: Color = Color::hex(0xFF_6B_5A);
    /// Amber: the warm end of the accent gradient.
    pub const AMBER: Color = Color::hex(0xFF_B8_4D);
    /// Violet: the cool end of the visualizer gradient.
    pub const VIOLET: Color = Color::hex(0x7D_6B_FF);
    /// The meters' safe zone.
    pub const GREEN: Color = Color::hex(0x3D_DC_97);
    /// Clipping.
    pub const RED: Color = Color::hex(0xFF_40_58);
}

/// The sidebar's background.
pub const SIDEBAR: ColorElement = ColorElement::custom(0);
/// Raised surfaces: the selected sidebar entry, hovered rows, chips, inputs.
pub const RAISED: ColorElement = ColorElement::custom(1);
/// Hairlines and dividers.
pub const LINE: ColorElement = ColorElement::custom(2);
/// Tertiary text: column headers, track numbers, hints.
pub const FAINT: ColorElement = ColorElement::custom(3);
/// The pressed look of flat controls.
pub const PRESSED: ColorElement = ColorElement::custom(4);
/// The warm end of the accent.
pub const AMBER: ColorElement = ColorElement::custom(5);

// The spacing grid.
pub const S1: i32 = 4;
pub const S2: i32 = 8;
pub const S3: i32 = 12;
pub const S4: i32 = 16;
pub const S5: i32 = 24;

// The type scale (Montserrat Medium, the bundled UI font).
/// Page titles.
pub static DISPLAY: &Font = &fonts::MONTSERRAT_28;
/// The now-playing title.
pub static HEADLINE: &Font = &fonts::MONTSERRAT_22;
/// Card titles, sidebar brand.
pub static TITLE: &Font = &fonts::MONTSERRAT_16;
/// Body text, list rows.
pub static BODY: &Font = &fonts::MONTSERRAT_14;
/// Secondary lines, times.
pub static CAPTION: &Font = &fonts::MONTSERRAT_12;
/// Column headers and section labels (upper case, spaced out).
pub static OVERLINE: &Font = &fonts::MONTSERRAT_10;

/// The app's theme: Twine's default theme in dark mode with the palette above.
pub fn theme() -> DefaultTheme {
    use palette::{
        ACCENT, BG, FAINT as FAINT_C, GREEN, LINE as LINE_C, MUTED, RAISED as RAISED_C, RED, SURFACE, TEXT,
        VIOLET,
    };
    DefaultTheme::builder()
        .mode(ThemeMode::Dark)
        .primary(ACCENT)
        .secondary(VIOLET)
        .fonts(FontScale {
            small: CAPTION,
            normal: BODY,
            large: TITLE,
        })
        .element(design::BACKGROUND, BG)
        .element(design::SURFACE, SURFACE)
        .element(design::SURFACE_VARIANT, RAISED_C)
        .element(design::ON_SURFACE, TEXT)
        .element(design::ON_SURFACE_MUTED, MUTED)
        .element(design::PLACEHOLDER, FAINT_C)
        .element(design::OUTLINE, LINE_C)
        .element(design::NEUTRAL, palette::PRESSED)
        .element(design::PRIMARY, ACCENT)
        .element(design::ON_PRIMARY, Color::hex(0x1A_0D_0B))
        .element(design::SCROLLBAR, Color::hex(0x3A_3C_4A))
        .element(design::FOCUS_RING, ACCENT)
        .element(design::OK, GREEN)
        .element(design::DANGER, RED)
        .element(SIDEBAR, palette::SIDEBAR)
        .element(RAISED, palette::RAISED)
        .element(LINE, palette::LINE)
        .element(FAINT, palette::FAINT)
        .element(PRESSED, palette::PRESSED)
        .element(AMBER, palette::AMBER)
        .class(&DROPDOWN_LIST_CLASS, style_dropdown_list)
        .build()
}

/// The dropdown's open list: a raised panel with a soft shadow, the selected row in the
/// accent color.
static LIST_STYLE: Style = style! {
    bg_color: RAISED,
    border_width: 1,
    border_color: LINE,
    radius: 10,
    shadow_width: 24,
    shadow_offset_y: 8,
    shadow_opacity: Opa::P50,
    shadow_color: Color::BLACK,
    text_color: design::ON_SURFACE,
    padding_top: 6,
    padding_bottom: 6,
    padding_left: 12,
    padding_right: 12,
    line_spacing: 14,
};
static LIST_SELECTED_STYLE: Style = style! {
    bg_color: PRESSED,
    bg_opacity: Opa::COVER,
    text_color: design::PRIMARY,
};

/// Theme registration for [`DROPDOWN_LIST_CLASS`].
pub fn style_dropdown_list<S>(cx: &mut ClassCx<'_, '_, S>) {
    cx.add_style(Selector::MAIN, &LIST_STYLE);
    cx.add_style(Selector::part(Part::Selected), &LIST_SELECTED_STYLE);
    cx.add_style(
        Selector::part(Part::Selected).with_state(State::CHECKED),
        &LIST_SELECTED_STYLE,
    );
}
