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
//! ([`SIDEBAR`], [`RAISED`], [`LINE`], …) are added to the LVGL theme's dark table ([`DARK`]),
//! so the built-in widgets the app keeps (slider, dropdown, text field, scrollbars) pick the
//! palette up through the theme, and views refer to colors by role, never by value. The
//! dropdown's pop-up list is created by the widget at run time, so it is restyled through a
//! recipe of the app's design system ([`Player`], extending the LVGL theme) rather than a view
//! modifier. The custom
//! drawn widgets (artwork, visualizer, response curve, meters) read the plain [`palette`]
//! constants, which are the same values.

use twine::engine::{ComputedRecipe, DesignSystem, RecipeRef, Theme, ThemeCx};
use twine::prelude::*;
use twine::style::StyleValue;
use twine::style::design::{ColorElement, ElementTable};
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

// The type scale (Montserrat Medium, the bundled UI font). Sharp at every display scale: each
// size is the bundled bitmap font at 1× (the look of the embedded build, pixel for pixel) and
// Montserrat rasterized at `size × scale` elsewhere (Retina, 1.25×, 1.5× …), with Font Awesome
// for the symbols, made the first time a scale is drawn (`TtfScaledFont`).

/// Montserrat Medium, the face of the bundled bitmap fonts.
static MONTSERRAT: &[u8] = include_bytes!("../../../../assets/fonts/Montserrat-Medium.ttf");
/// Font Awesome (solid), the face of the symbols merged into the bundled bitmap fonts.
static FONT_AWESOME: &[u8] = include_bytes!("../../../../assets/fonts/fa-solid-900.ttf");

twine::text::scaled_font! {
    static DISPLAY_FONT =
        ttf(fonts::MONTSERRAT_28, MONTSERRAT, 28).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static HEADLINE_FONT =
        ttf(fonts::MONTSERRAT_22, MONTSERRAT, 22).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static CONTROL_FONT =
        ttf(fonts::MONTSERRAT_18, MONTSERRAT, 18).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static TITLE_FONT =
        ttf(fonts::MONTSERRAT_16, MONTSERRAT, 16).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static BODY_FONT =
        ttf(fonts::MONTSERRAT_14, MONTSERRAT, 14).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static CAPTION_FONT =
        ttf(fonts::MONTSERRAT_12, MONTSERRAT, 12).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static OVERLINE_FONT =
        ttf(fonts::MONTSERRAT_10, MONTSERRAT, 10).fallback(FONT_AWESOME);
}
twine::text::scaled_font! {
    static MICRO_FONT =
        ttf(fonts::MONTSERRAT_8, MONTSERRAT, 8).fallback(FONT_AWESOME);
}

/// Page titles.
pub static DISPLAY: &Font = &DISPLAY_FONT;
/// The now-playing title.
pub static HEADLINE: &Font = &HEADLINE_FONT;
/// The play / pause glyph.
pub static CONTROL: &Font = &CONTROL_FONT;
/// Card titles, sidebar brand, icon glyphs.
pub static TITLE: &Font = &TITLE_FONT;
/// Body text, list rows.
pub static BODY: &Font = &BODY_FONT;
/// Secondary lines, times.
pub static CAPTION: &Font = &CAPTION_FONT;
/// Column headers and section labels (upper case, spaced out), chart labels.
pub static OVERLINE: &Font = &OVERLINE_FONT;
/// Tiny marks: sort arrows, slider ticks.
pub static MICRO: &Font = &MICRO_FONT;

/// The app's only mode: the LVGL theme's dark table with the palette above, the type scale
/// and the app's own elements (`const`, in flash).
pub static DARK: ElementTable = Lvgl::SET
    .primary(palette::ACCENT)
    .secondary(palette::VIOLET)
    .fonts(FontScale {
        small: &CAPTION_FONT,
        normal: &BODY_FONT,
        large: &TITLE_FONT,
    })
    .table(ThemeMode::Dark)
    .color(design::BACKGROUND, palette::BG)
    .color(design::SURFACE, palette::SURFACE)
    .color(design::SURFACE_VARIANT, palette::RAISED)
    .color(design::ON_SURFACE, palette::TEXT)
    .color(design::ON_SURFACE_MUTED, palette::MUTED)
    .color(design::PLACEHOLDER, palette::FAINT)
    .color(design::OUTLINE, palette::LINE)
    .color(design::NEUTRAL, palette::PRESSED)
    .color(design::ON_PRIMARY, Color::hex(0x1A_0D_0B))
    .color(design::SCROLLBAR, Color::hex(0x3A_3C_4A))
    .color(design::OK, palette::GREEN)
    .color(design::DANGER, palette::RED)
    .custom(&[
        Some(StyleValue::Color(palette::SIDEBAR)),
        Some(StyleValue::Color(palette::RAISED)),
        Some(StyleValue::Color(palette::LINE)),
        Some(StyleValue::Color(palette::FAINT)),
        Some(StyleValue::Color(palette::PRESSED)),
        Some(StyleValue::Color(palette::AMBER)),
    ]);

/// Twine Player's design system: the LVGL theme ([`DARK`] only) with the dropdown list
/// restyled ([`style_dropdown_list`]).
pub struct Player;

impl DesignSystem for Player {
    type Base = Lvgl;
    const NAME: &'static str = "player";
    const MODES: &'static [(ThemeMode, &'static ElementTable)] = &[(ThemeMode::Dark, &DARK)];
    fn recipe(class: &'static WidgetClass) -> Option<RecipeRef> {
        DROPDOWN_LIST.for_class(class)
    }
}

/// The app's theme.
pub static THEME: Theme<Player> = Theme::new();

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

/// The recipe of [`DROPDOWN_LIST_CLASS`]: the LVGL look, then the app's.
pub fn style_dropdown_list(cx: &mut ThemeCx<'_>) {
    lvgl::THEME.apply(cx, &DROPDOWN_LIST_CLASS);
    cx.add_style(Selector::MAIN, &LIST_STYLE);
    cx.add_style(Selector::part(Part::Selected), &LIST_SELECTED_STYLE);
    cx.add_style(
        Selector::part(Part::Selected).with_state(State::CHECKED),
        &LIST_SELECTED_STYLE,
    );
}

static DROPDOWN_LIST: ComputedRecipe =
    ComputedRecipe::new("DROPDOWN_LIST", &DROPDOWN_LIST_CLASS, style_dropdown_list);
