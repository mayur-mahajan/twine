//! [`Lvgl`]: a faithful port of LVGL's default theme
//! (`src/themes/default/lv_theme_default.c`), light and dark, plus Twine's night and
//! high-contrast modes — as `static` data.
//!
//! - **Styles** ([`styles`]) are LVGL's `style_init()` as `static` [`Style`](twine_style::Style)s.
//! - **Colors, fonts and motion** are [design elements](twine_style::design) in one `'static`
//!   table per mode ([`LIGHT`], [`DARK`], [`NIGHT`], [`HIGH_CONTRAST`], or the application's
//!   own made with [`LvglSet::table`]); the styles are the same in every mode, so
//!   [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode) switches modes by
//!   swapping a pointer.
//! - **Sizes** follow the display like LVGL's: every pixel value is scaled with
//!   [`dpx`](twine_style::dpx) by the display's DPI, and radii and paddings follow the display
//!   size class ([`DisplaySize`]: the larger side ≤ 320 px is small, < 720 px medium, else
//!   large). They are the elements of a small per-display table ([`metrics`]) computed once
//!   when the display is created.
//! - **Per-class looks** are LVGL's `theme_apply()` branches, one
//!   [`ComputedRecipe`] per widget class ([`ALL_RECIPES`], or the ones the application
//!   lists with [`Lvgl::recipes`]: only those are linked),
//!   found along the node's theme-class [lineage](twine_engine::WidgetClass::lineage): a
//!   custom class whose [`base`](twine_engine::WidgetClass::base) is `BUTTON_CLASS` is styled
//!   like a button.
//!
//! The mode tables (LVGL's light and dark colors; Twine's night and high-contrast palettes):
//!
//! | Element | `Light` | `Dark` | `Night` | `HighContrast` | LVGL |
//! |---------|-------|------|-------|--------------|------|
//! | `BACKGROUND` | grey lighten 4 | `#15171A` | `#000000` | `#000000` | `color_scr` |
//! | `SURFACE` | white | `#282B30` | `#100C08` | `#000000` | `color_card` |
//! | `ON_SURFACE` | grey darken 4 | grey lighten 5 | `#BC8644` | `#FFFFFF` | `color_text` |
//! | `ON_SURFACE_MUTED` | grey darken 1 | grey lighten 1 | `#A47644` | grey lighten 3 | — |
//! | `PLACEHOLDER` | grey lighten 1 | grey darken 2 | `#5A462A` | grey lighten 1 | textarea placeholder |
//! | `NEUTRAL` | grey | grey | `#4A3A24` | white | the card's `line_color`, `menu_pressed` |
//! | `SURFACE_VARIANT` | grey lighten 2 | `#2F3237` | `#2A2014` | `#404040` | `color_grey` |
//! | `OUTLINE` | grey lighten 2 | `#2F3237` | `#3A2C1C` | `#FFFFFF` | `color_grey` |
//! | `PRIMARY` / `ON_PRIMARY` (also the switch knob) | the set's primary / white | same | `#B06A10` / black | `#FFD600` / black | `color_primary` / `lv_color_white()` |
//! | `SECONDARY` / `ON_SECONDARY` | the set's secondary / white | same | `#3C1A0A` / `#BC8644` | white / black | `color_secondary` / `lv_color_white()` |
//! | `DANGER`, `WARNING`, `OK` | red, orange, green (palette main) | same | `#C83020`, `#C08010`, `#5E8C24` | `#FF6E6E`, `#FFB300`, `#00E676` | — |
//! | `DISABLED` | grey lighten 2 | grey darken 2 | `#100C08` | `#808080` | disabled recolor |
//! | `FOCUS_RING` | primary | primary | `#C88020` | `#00E5FF` | `outline_primary` |
//! | `SCROLLBAR` | grey | grey darken 2 | `#4A3A24` | white | `sb_color` |
//! | `SHADOW`, `SCRIM` | grey | grey | black | black | button shadow, message box backdrop |
//! | `FOCUS_RING_OPACITY` | 50 % | 50 % | opaque | opaque | outline opacity |
//! | `SHADOW_OPACITY` | 50 % | transparent (no shadow) | transparent | transparent | button shadow |
//! | `DISABLED_OPACITY` | 50 % | 50 % | 30 % | 50 % | disabled recolor opacity |
//! | `SCRIM_OPACITY` | 50 % | 50 % | 70 % | 70 % | backdrop opacity |
//! | `RADIUS_FULL` | circle | same | same | same | `LV_RADIUS_CIRCLE` |
//! | `FONT_SMALL`, `FONT_BODY`, `FONT_LARGE` | the set's fonts | same | same | same | `font_small` / `font_normal` / `font_large` |
//! | `MOTION_*` | [`MOTION`] | same | same | same | transitions, animations |
//!
//! `ACCENT_TINT` (20 % `PRIMARY` on `SURFACE`), `SEPARATOR` (= `OUTLINE`),
//! `GROUPED_BACKGROUND` (= `BACKGROUND`) and `FILL_1..3` (20, 30, 40 % `NEUTRAL` on
//! `SURFACE`) derive from them. Spacings, radii and elevations are in the [`metrics`] table.
//!
//! `Night` keeps every color at most 30 % relative luminance with its blue channel at most
//! half its red channel (warm amber, preserving dark adaptation), and `HighContrast` is white
//! on black with a yellow accent and an opaque cyan focus ring; neither uses the set's
//! primary and secondary colors (a blue accent would break the night limits and may lack
//! contrast). Every mode meets the minimum contrast ratios of
//! [`ThemeMode::min_contrast`] (tested; ratios per pair are listed in the crate
//! documentation).

pub mod colors;
pub mod metrics;
pub mod styles;

use twine_core::{Color, Duration, Opa, Size};
use twine_engine::{
    AnimSpec, ComputedRecipe, Easing, OBJ_CLASS, RecipeRef, ThemeBase, ThemeCx, ThemeHook, WidgetClass,
};
use twine_style::design::{self, ElementTable, MotionSet};
use twine_style::{Part, Selector, State};
use twine_text::Font;
use twine_widgets::animimg::ANIMIMG_CLASS;
use twine_widgets::arc::ARC_CLASS;
use twine_widgets::bar::BAR_CLASS;
use twine_widgets::button::BUTTON_CLASS;
use twine_widgets::buttonmatrix::{BUTTONMATRIX_CLASS, POPOVER_CLASS};
use twine_widgets::checkbox::CHECKBOX_CLASS;
use twine_widgets::container::CARD_CLASS;
use twine_widgets::image::IMAGE_CLASS;
use twine_widgets::image_button::IMAGE_BUTTON_CLASS;
use twine_widgets::keyboard::KEYBOARD_CLASS;
use twine_widgets::label::LABEL_CLASS;
use twine_widgets::led::LED_CLASS;
use twine_widgets::line::LINE_CLASS;
use twine_widgets::slider::SLIDER_CLASS;
use twine_widgets::spangroup::SPANGROUP_CLASS;
use twine_widgets::spinbox::SPINBOX_CLASS;
use twine_widgets::spinner::SPINNER_CLASS;
use twine_widgets::switch::SWITCH_CLASS;
use twine_widgets::textarea::{PLACEHOLDER, TEXTAREA_CLASS, TEXTAREA_LABEL_CLASS};
use twine_widgets_ext::dropdown::{DROPDOWN_CLASS, DROPDOWN_LIST_CLASS};
use twine_widgets_ext::list::{LIST_BUTTON_CLASS, LIST_CLASS, LIST_TEXT_CLASS};
use twine_widgets_ext::menu::{
    MENU_BACK_BUTTON_CLASS, MENU_CLASS, MENU_CONT_CLASS, MENU_MAIN_CONTAINER_CLASS,
    MENU_MAIN_HEADER_CONTAINER_CLASS, MENU_PAGE_CLASS, MENU_SECTION_CLASS, MENU_SEPARATOR_CLASS,
    MENU_SIDEBAR_CONTAINER_CLASS, MENU_SIDEBAR_HEADER_CONTAINER_CLASS,
};
use twine_widgets_ext::msgbox::{
    MSGBOX_BACKDROP_CLASS, MSGBOX_CLASS, MSGBOX_CONTENT_CLASS, MSGBOX_FOOTER_BUTTON_CLASS,
    MSGBOX_FOOTER_CLASS, MSGBOX_HEADER_BUTTON_CLASS, MSGBOX_HEADER_CLASS,
};
use twine_widgets_ext::roller::ROLLER_CLASS;
use twine_widgets_ext::tabview::{
    TABVIEW_CLASS, TABVIEW_CONTENT_CLASS, TABVIEW_PAGE_CLASS, TABVIEW_TAB_BAR_CLASS, TABVIEW_TAB_BUTTON_CLASS,
};
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CLASS, WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::modes::ModeList;
use crate::{FontScale, Palette, Theme, Tone};
use styles as s;

/// The theme modes: [`ThemeMode::Light`] and [`ThemeMode::Dark`] (LVGL's `dark` flag of the
/// default theme; no button shadows), [`ThemeMode::Night`] and [`ThemeMode::HighContrast`],
/// switched at run time with [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode).
pub use twine_style::ThemeMode;

/// The display size class LVGL's default theme scales radii and paddings with
/// (`disp_size_t`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DisplaySize {
    /// The larger side is at most 320 px (`DISP_SMALL`).
    Small,
    /// The larger side is below 720 px (`DISP_MEDIUM`).
    Medium,
    /// Larger displays (`DISP_LARGE`).
    Large,
}

impl DisplaySize {
    /// The class of a display of `resolution` (LVGL `lv_theme_default_init`).
    ///
    /// ```
    /// use twine_core::Size;
    /// use twine_theme::DisplaySize;
    /// assert_eq!(DisplaySize::of(Size::new(320, 240)), DisplaySize::Small);
    /// assert_eq!(DisplaySize::of(Size::new(480, 320)), DisplaySize::Medium);
    /// assert_eq!(DisplaySize::of(Size::new(800, 480)), DisplaySize::Large);
    /// ```
    #[must_use]
    pub fn of(resolution: Size) -> Self {
        let greater = resolution.w.max(resolution.h);
        if greater <= 320 {
            DisplaySize::Small
        } else if greater < 720 {
            DisplaySize::Medium
        } else {
            DisplaySize::Large
        }
    }
}

/// The motion timings of the LVGL theme (`MOTION_*` design elements): `MOTION_FAST` is
/// LVGL's theme transition ([`TRANSITION_TIME`](styles::TRANSITION_TIME), linear); the
/// longer ones follow LVGL's widget animation lengths (200–300 ms, ease in-out; exits ease
/// out).
pub const MOTION: MotionSet = MotionSet {
    fast: &MOTION_FAST,
    standard: &MOTION_STANDARD,
    emphasized: &MOTION_EMPHASIZED,
    exit: &MOTION_EXIT,
};
static MOTION_FAST: AnimSpec = AnimSpec::new(styles::TRANSITION_TIME);
static MOTION_STANDARD: AnimSpec = AnimSpec::new(Duration::ms(200)).easing(Easing::EaseInOut);
static MOTION_EMPHASIZED: AnimSpec = AnimSpec::new(Duration::ms(300)).easing(Easing::EaseInOut);
static MOTION_EXIT: AnimSpec = AnimSpec::new(Duration::ms(150)).easing(Easing::EaseOut);

/// The LVGL theme's element set: what its mode tables depend on — the primary and secondary
/// colors (LVGL `color_primary` / `color_secondary`) and the fonts — tuned with `const`
/// builders and turned into a mode's `'static` table with [`table`](Self::table)
/// ([`Lvgl::SET`] is LVGL's defaults).
///
/// | Setting | Default |
/// |---------|---------|
/// | [`primary`](Self::primary) / [`secondary`](Self::secondary) | [`Palette::Blue`] / [`Palette::Red`] (LVGL's) |
/// | [`fonts`](Self::fonts) | Montserrat 14 for all three (feature `assets`; without it the empty font: set them) |
///
/// ```
/// use twine_core::Color;
/// use twine_style::ThemeMode;
/// use twine_style::design::{self, ElementTable};
/// use twine_theme::{Lvgl, Palette};
///
/// /// A teal light mode.
/// static TEAL_LIGHT: ElementTable = Lvgl::SET
///     .primary(Palette::Teal.main())
///     .secondary(Color::hex(0xFFC107))
///     .table(ThemeMode::Light);
/// assert_eq!(TEAL_LIGHT.get(design::PRIMARY), Some(Palette::Teal.main()));
/// assert_eq!(TEAL_LIGHT.get(design::FOCUS_RING), Some(Palette::Teal.main()));
/// ```
#[derive(Clone, Copy, Debug)]
pub struct LvglSet {
    /// `design::PRIMARY` and `design::FOCUS_RING` in [`ThemeMode::Light`] and
    /// [`ThemeMode::Dark`]: buttons, sliders, focus rings.
    pub primary: Color,
    /// `design::SECONDARY` in [`ThemeMode::Light`] and [`ThemeMode::Dark`]: checked buttons,
    /// edit outlines.
    pub secondary: Color,
    /// `design::FONT_SMALL` / `FONT_BODY` / `FONT_LARGE` in every mode.
    pub fonts: FontScale,
}

impl LvglSet {
    /// The primary color (a [`Palette`] entry's [`main`](Palette::main) or any [`Color`]).
    #[must_use]
    pub const fn primary(mut self, color: Color) -> Self {
        self.primary = color;
        self
    }

    /// The secondary color.
    #[must_use]
    pub const fn secondary(mut self, color: Color) -> Self {
        self.secondary = color;
        self
    }

    /// The small, normal and large fonts (LVGL `font_small` / `font_normal` /
    /// `font_large`).
    #[must_use]
    pub const fn fonts(mut self, fonts: FontScale) -> Self {
        self.fonts = fonts;
        self
    }

    /// The `'static` element table of `mode` (see the [module documentation](self) for the
    /// values); `const`, so it lives in flash: `static T: ElementTable =
    /// Lvgl::SET.table(ThemeMode::Dark);`. Spacings, radii and elevations depend on the
    /// display and are in its [`metrics`] table instead. Never panics.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::ThemeMode;
    /// use twine_style::design;
    /// use twine_theme::{Lvgl, lvgl};
    ///
    /// assert_eq!(lvgl::DARK.get(design::SURFACE), Some(Color::hex(0x282B30)));
    /// assert_eq!(Lvgl::SET.table(ThemeMode::Night).get(design::ON_PRIMARY), Some(Color::BLACK));
    /// ```
    #[must_use]
    pub const fn table(self, mode: ThemeMode) -> ElementTable {
        let c = ModeColors::new(self.primary, self.secondary, mode);
        ElementTable::new()
            .color(design::BACKGROUND, c.background)
            .color(design::SURFACE, c.surface)
            .color(design::ON_SURFACE, c.on_surface)
            .color(design::ON_SURFACE_MUTED, c.on_surface_muted)
            .color(design::PLACEHOLDER, c.placeholder)
            .color(design::NEUTRAL, c.neutral)
            .color(design::SURFACE_VARIANT, c.surface_variant)
            .color(design::OUTLINE, c.outline)
            .color(design::PRIMARY, c.primary)
            .color(design::ON_PRIMARY, c.on_primary)
            .color(design::SECONDARY, c.secondary)
            .color(design::ON_SECONDARY, c.on_secondary)
            .color(design::DANGER, c.danger)
            .color(design::WARNING, c.warning)
            .color(design::OK, c.ok)
            .color(design::DISABLED, c.disabled)
            .color(design::FOCUS_RING, c.focus_ring)
            .color(design::SCROLLBAR, c.scrollbar)
            .color(design::SHADOW, c.shadow)
            .color(design::SCRIM, c.scrim)
            .color(
                design::ACCENT_TINT,
                Color::mix(c.primary, c.surface, Opa::pct(20)),
            )
            .color(design::SEPARATOR, c.outline)
            .color(design::GROUPED_BACKGROUND, c.background)
            .color(design::FILL_1, Color::mix(c.neutral, c.surface, Opa::pct(20)))
            .color(design::FILL_2, Color::mix(c.neutral, c.surface, Opa::pct(30)))
            .color(design::FILL_3, Color::mix(c.neutral, c.surface, Opa::pct(40)))
            .radius(design::RADIUS_FULL, twine_style::Radius::Circle)
            .opacity(design::FOCUS_RING_OPACITY, c.focus_ring_opacity)
            .opacity(design::SHADOW_OPACITY, c.shadow_opacity)
            .opacity(design::DISABLED_OPACITY, c.disabled_opacity)
            .opacity(design::SCRIM_OPACITY, c.scrim_opacity)
            .font(design::FONT_SMALL, self.fonts.small)
            .font(design::FONT_BODY, self.fonts.normal)
            .font(design::FONT_LARGE, self.fonts.large)
            .motions(MOTION)
    }
}

/// The colors and opacities of one mode of the LVGL theme (see the module documentation).
struct ModeColors {
    background: Color,
    surface: Color,
    on_surface: Color,
    on_surface_muted: Color,
    placeholder: Color,
    neutral: Color,
    surface_variant: Color,
    outline: Color,
    primary: Color,
    on_primary: Color,
    secondary: Color,
    on_secondary: Color,
    danger: Color,
    warning: Color,
    ok: Color,
    disabled: Color,
    focus_ring: Color,
    scrollbar: Color,
    shadow: Color,
    scrim: Color,
    focus_ring_opacity: Opa,
    shadow_opacity: Opa,
    disabled_opacity: Opa,
    scrim_opacity: Opa,
}

impl ModeColors {
    /// LVGL's light and dark colors; the night and high-contrast palettes (not in LVGL).
    const fn new(primary: Color, secondary: Color, mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light | ThemeMode::Dark => {
                let dark = matches!(mode, ThemeMode::Dark);
                let grey = pick(dark, colors::DARK_GREY, colors::LIGHT_GREY);
                Self {
                    background: pick(dark, colors::DARK_SCR, colors::LIGHT_SCR),
                    surface: pick(dark, colors::DARK_CARD, colors::LIGHT_CARD),
                    on_surface: pick(dark, colors::DARK_TEXT, colors::LIGHT_TEXT),
                    on_surface_muted: pick(dark, Palette::Grey.tone(Tone::L1), Palette::Grey.tone(Tone::D1)),
                    // ta_placeholder: dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 1)
                    placeholder: pick(dark, Palette::Grey.tone(Tone::D2), Palette::Grey.tone(Tone::L1)),
                    // lv_palette_main(GREY) in both modes (card lines, menu_pressed).
                    neutral: Palette::Grey.main(),
                    surface_variant: grey,
                    outline: grey,
                    primary,
                    on_primary: Color::WHITE,
                    secondary,
                    on_secondary: Color::WHITE,
                    danger: Palette::Red.main(),
                    warning: Palette::Orange.main(),
                    ok: Palette::Green.main(),
                    // disabled: dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 2)
                    disabled: pick(dark, Palette::Grey.tone(Tone::D2), Palette::Grey.tone(Tone::L2)),
                    focus_ring: primary,
                    // sb_color = dark ? lv_palette_darken(GREY, 2) : lv_palette_main(GREY)
                    scrollbar: pick(dark, Palette::Grey.tone(Tone::D2), Palette::Grey.main()),
                    shadow: Palette::Grey.main(),
                    scrim: Palette::Grey.main(),
                    focus_ring_opacity: Opa::P50,
                    // LVGL's dark mode has no button shadow.
                    shadow_opacity: if dark { Opa::TRANSP } else { Opa::P50 },
                    disabled_opacity: Opa::P50,
                    scrim_opacity: Opa::P50,
                }
            }
            // Warm amber on near-black: every color ≤ 30 % relative luminance, blue ≤ red / 2.
            // The set's primary / secondary colors are not used (a blue accent would break
            // both limits).
            ThemeMode::Night => Self {
                background: Color::BLACK,
                surface: Color::hex(0x0010_0C08),
                on_surface: Color::hex(0x00BC_8644),
                on_surface_muted: Color::hex(0x00A4_7644),
                placeholder: Color::hex(0x005A_462A),
                neutral: Color::hex(0x004A_3A24),
                surface_variant: Color::hex(0x002A_2014),
                outline: Color::hex(0x003A_2C1C),
                primary: Color::hex(0x00B0_6A10),
                on_primary: Color::BLACK,
                secondary: Color::hex(0x003C_1A0A),
                on_secondary: Color::hex(0x00BC_8644),
                danger: Color::hex(0x00C8_3020),
                warning: Color::hex(0x00C0_8010),
                ok: Color::hex(0x005E_8C24),
                disabled: Color::hex(0x0010_0C08),
                focus_ring: Color::hex(0x00C8_8020),
                scrollbar: Color::hex(0x004A_3A24),
                shadow: Color::BLACK,
                scrim: Color::BLACK,
                focus_ring_opacity: Opa::COVER,
                shadow_opacity: Opa::TRANSP,
                disabled_opacity: Opa::P30,
                scrim_opacity: Opa::P70,
            },
            // White on black, yellow accent, inverted (white) checked buttons, an opaque cyan
            // focus ring, no shadows. The set's primary / secondary colors are not used.
            ThemeMode::HighContrast => Self {
                background: Color::BLACK,
                surface: Color::BLACK,
                on_surface: Color::WHITE,
                on_surface_muted: Palette::Grey.tone(Tone::L3),
                placeholder: Palette::Grey.tone(Tone::L1),
                neutral: Color::WHITE,
                surface_variant: Color::hex(0x0040_4040),
                outline: Color::WHITE,
                primary: Color::hex(0x00FF_D600),
                on_primary: Color::BLACK,
                secondary: Color::WHITE,
                on_secondary: Color::BLACK,
                danger: Color::hex(0x00FF_6E6E),
                warning: Color::hex(0x00FF_B300),
                ok: Color::hex(0x0000_E676),
                disabled: Color::GRAY,
                focus_ring: Color::hex(0x0000_E5FF),
                scrollbar: Color::WHITE,
                shadow: Color::BLACK,
                scrim: Color::BLACK,
                focus_ring_opacity: Opa::COVER,
                shadow_opacity: Opa::TRANSP,
                disabled_opacity: Opa::P50,
                scrim_opacity: Opa::P70,
            },
        }
    }
}

/// `d` when `dark`, else `l`.
const fn pick(dark: bool, d: Color, l: Color) -> Color {
    if dark { d } else { l }
}

/// LVGL's light mode with LVGL's colors and fonts (`Lvgl::SET.table(ThemeMode::Light)`).
pub static LIGHT: ElementTable = Lvgl::SET.table(ThemeMode::Light);
/// LVGL's dark mode (`Lvgl::SET.table(ThemeMode::Dark)`).
pub static DARK: ElementTable = Lvgl::SET.table(ThemeMode::Dark);
/// Twine's night mode (`Lvgl::SET.table(ThemeMode::Night)`).
pub static NIGHT: ElementTable = Lvgl::SET.table(ThemeMode::Night);
/// Twine's high-contrast mode (`Lvgl::SET.table(ThemeMode::HighContrast)`).
pub static HIGH_CONTRAST: ElementTable = Lvgl::SET.table(ThemeMode::HighContrast);

/// Every mode with LVGL's colors and fonts, in switching order: what [`Lvgl::new`] lists. An
/// image that only shows some modes lists only their tables ([`Lvgl::modes`]), so the
/// others are not linked.
pub const MODES: &[(ThemeMode, &ElementTable)] = &[
    (ThemeMode::Light, &LIGHT),
    (ThemeMode::Dark, &DARK),
    (ThemeMode::Night, &NIGHT),
    (ThemeMode::HighContrast, &HIGH_CONTRAST),
];

/// The LVGL theme with its defaults ([`Lvgl::new`]: every mode, LVGL's colors and fonts),
/// ready to give to a display: `AppConfig::new().theme(&lvgl::THEME)`. Linked only when
/// named; a product that lists its modes or tunes its sizes declares its own `static`.
pub static THEME: Lvgl = Lvgl::new();

/// LVGL's default theme: material colors, rounded white (or dark) cards with a grey border,
/// primary-colored buttons with a shadow that darken when pressed and turn secondary when
/// checked, 50 % outlines on keyboard focus, and short transitions.
///
/// A `const` value, used as a `static` (one theme per image, fixed at compile time: no heap,
/// nothing built at start-up but the small per-display [`metrics`] table). The configuration
/// is `const` too:
///
/// | Setting | Default |
/// |---------|---------|
/// | [`modes`](Self::modes) | [`MODES`] (all four, LVGL's colors and fonts; the first is the start mode) |
/// | [`recipes`](Self::recipes) | [`ALL_RECIPES`] (every widget class; list the ones the product uses to link only those) |
/// | [`dpi`](Self::dpi) / [`display_size`](Self::display_size) | each display's own |
///
/// Colors and fonts are tuned in the mode tables ([`Lvgl::SET`], [`LvglSet::table`]);
/// per-class looks are changed by a [`DesignSystem`](twine_engine::DesignSystem) that
/// extends this theme (`type Base = Lvgl;`), reusing its [`styles`].
///
/// ```
/// use twine_core::{Color, ColorFormat};
/// use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeHook};
/// use twine_hal::DisplayInfo;
/// use twine_style::{ThemeMode, design};
/// use twine_style::design::ElementTable;
/// use twine_theme::{Lvgl, Palette, lvgl};
///
/// static TEAL_LIGHT: ElementTable = Lvgl::SET.primary(Palette::Teal.main()).table(ThemeMode::Light);
/// /// Teal, light and dark (LVGL's dark colors), sized as on a 160 DPI panel.
/// static THEME: Lvgl = Lvgl::new()
///     .modes(&[(ThemeMode::Light, &TEAL_LIGHT), (ThemeMode::Dark, &lvgl::DARK)])
///     .dpi(160);
///
/// assert_eq!(ThemeHook::modes(&THEME), &[ThemeMode::Light, ThemeMode::Dark]);
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565);
/// let d = e.add_chunked_display(info, 320 * 2 * 8, Some(DisplayTheme::new(&THEME))).unwrap();
/// assert_eq!(e.design_value(d, design::PRIMARY), Some(Palette::Teal.main()));
/// assert_eq!(e.design_value(d, design::SPACE_M), Some(twine_style::Length::Px(16)));
/// e.set_theme_mode(d, ThemeMode::Dark);
/// assert_eq!(e.design_value(d, design::SURFACE), Some(Color::hex(0x282B30)));
/// ```
#[derive(Clone, Copy)]
pub struct Lvgl {
    modes: ModeList,
    recipes: &'static [&'static ComputedRecipe],
    dpi: Option<u16>,
    size: Option<DisplaySize>,
}

impl core::fmt::Debug for Lvgl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lvgl")
            .field("modes", &self.modes.modes())
            .field("recipes", &self.recipes.len())
            .field("dpi", &self.dpi)
            .field("size", &self.size)
            .finish()
    }
}

impl Default for Lvgl {
    fn default() -> Self {
        Self::new()
    }
}

impl Lvgl {
    /// LVGL's element set: [`Palette::Blue`] / [`Palette::Red`], Montserrat 14 (feature
    /// `assets`) for all three fonts (`lv_theme_default_init`'s defaults).
    pub const SET: LvglSet = LvglSet {
        primary: Palette::Blue.main(),
        secondary: Palette::Red.main(),
        fonts: FontScale::uniform(crate::theme::DEFAULT_FONT),
    };

    /// The theme with LVGL's defaults (see the table of [`Lvgl`]); LVGL
    /// `lv_theme_default_init`. `const`: `static THEME: Lvgl = Lvgl::new();`.
    #[doc(alias = "lv_theme_default_init")]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modes: ModeList::new(MODES),
            recipes: ALL_RECIPES,
            dpi: None,
            size: None,
        }
    }

    /// Lists the modes the theme supports, each with its `'static` table, in the order a
    /// mode switch cycles through them; the first is the start mode unless the display names
    /// another. Only the listed tables are linked; a switch to an unlisted mode is refused (the
    /// display keeps its mode). A mode listed twice counts once (its first table). Never
    /// panics.
    ///
    /// ```
    /// use twine_style::ThemeMode;
    /// use twine_theme::{Lvgl, ThemeHook, lvgl};
    ///
    /// static DARK_ONLY: Lvgl = Lvgl::new().modes(&[(ThemeMode::Dark, &lvgl::DARK)]);
    /// assert_eq!(ThemeHook::modes(&DARK_ONLY), &[ThemeMode::Dark]);
    /// assert!(DARK_ONLY.design(ThemeMode::Light).is_none());
    /// ```
    #[must_use]
    pub const fn modes(mut self, modes: &'static [(ThemeMode, &'static ElementTable)]) -> Self {
        self.modes = ModeList::new(modes);
        self
    }

    /// Lists the per-class recipes the theme uses — **pay for what you use**: only the
    /// listed recipes (and the styles they add) are linked, so an image does not carry the
    /// looks of widget classes it never creates. Default: [`ALL_RECIPES`] (every class this
    /// theme styles).
    ///
    /// A node is styled by the recipe of the nearest class of its theme-class
    /// [lineage](twine_engine::WidgetClass::lineage) in the list (a variant or a custom widget
    /// based on `BUTTON_CLASS` uses [`BUTTON`]); a class with no listed recipe in its lineage
    /// gets no theme styles (screens are always styled). The list is checked, not guessed:
    /// `twine_testing::assert_theme_covers` fails a test whose UI creates a class without a
    /// recipe, and `cargo xtask theme-recipes -- <test args>` prints the minimal list of a
    /// UI. The first recipe listed for a class wins. Never panics.
    ///
    /// ```
    /// use twine_engine::ThemeHook;
    /// use twine_theme::{Lvgl, lvgl};
    /// use twine_widgets::button::BUTTON_CLASS;
    /// use twine_widgets::slider::SLIDER_CLASS;
    ///
    /// /// A product showing labels and buttons on screens and cards only.
    /// static THEME: Lvgl = Lvgl::new().recipes(&[&lvgl::OBJ, &lvgl::LABEL, &lvgl::BUTTON]);
    /// assert_eq!(THEME.recipe(&BUTTON_CLASS).map(|r| r.name()), Some("BUTTON"));
    /// assert!(THEME.recipe(&SLIDER_CLASS).is_none()); // its recipe is not linked
    /// ```
    #[must_use]
    pub const fn recipes(mut self, recipes: &'static [&'static ComputedRecipe]) -> Self {
        self.recipes = recipes;
        self
    }

    /// Scales for `dpi` (at least 1) instead of each display's own DPI (LVGL uses the
    /// display's, [`DEFAULT_DPI`](twine_style::DEFAULT_DPI) = 130 by default).
    /// Never panics: `0` is taken as `1`.
    ///
    /// ```
    /// use twine_theme::Lvgl;
    /// // Sizes as on a 160 DPI panel, whatever the display reports.
    /// static THEME: Lvgl = Lvgl::new().dpi(160);
    /// ```
    #[must_use]
    pub const fn dpi(mut self, dpi: u16) -> Self {
        self.dpi = Some(if dpi == 0 { 1 } else { dpi });
        self
    }

    /// Uses the size class `size` instead of deriving it from each display's resolution.
    ///
    /// ```
    /// use twine_theme::{DisplaySize, Lvgl};
    /// // The compact layout of small displays, also on a larger one.
    /// static THEME: Lvgl = Lvgl::new().display_size(DisplaySize::Small);
    /// ```
    #[must_use]
    pub const fn display_size(mut self, size: DisplaySize) -> Self {
        self.size = Some(size);
        self
    }
}

impl ThemeHook for Lvgl {
    /// LVGL `theme_apply()` of `lv_theme_default.c`: a screen gets the screen styles; any
    /// other node the recipe of the nearest class of its theme-class
    /// [lineage](WidgetClass::lineage) the theme knows ([`recipe`](ThemeHook::recipe)).
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if cx.parent().is_none() {
            screen(cx);
        } else if let Some(r) = self.recipe(class) {
            cx.add_recipe(r);
        } else {
            twine_core::trace!(target: "twine::style", "lvgl theme: no styles for class {}", class.name);
        }
    }

    fn font_normal(&self) -> &'static Font {
        crate::theme::first_font(self.modes.tables(), design::FONT_BODY)
    }

    fn name(&self) -> &'static str {
        "lvgl"
    }

    fn modes(&self) -> &[ThemeMode] {
        self.modes.modes()
    }

    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        self.modes.design(mode)
    }

    /// The [`metrics`] table for the display's DPI and size class (or the theme's
    /// [`dpi`](Lvgl::dpi) / [`display_size`](Lvgl::display_size)).
    fn metrics(&self, dpi: u16, resolution: Size) -> Option<ElementTable> {
        Some(metrics::for_display(self.dpi, self.size, dpi, resolution))
    }

    /// The recipe of the nearest class of `class`'s lineage among the theme's
    /// [`recipes`](Lvgl::recipes) ([`ALL_RECIPES`] by default).
    fn recipe(&self, class: &'static WidgetClass) -> Option<RecipeRef> {
        crate::recipes::find(self.recipes, class)
    }
}

impl ThemeBase for Lvgl {
    const THEME: &'static Self = &THEME;
}

impl Theme for Lvgl {
    fn font_small(&self) -> &'static Font {
        crate::theme::first_font(self.modes.tables(), design::FONT_SMALL)
    }
    fn font_large(&self) -> &'static Font {
        crate::theme::first_font(self.modes.tables(), design::FONT_LARGE)
    }
}

// ---- Per-class looks (LVGL `theme_apply()`) ------------------------------------------------

/// `Part::Scrollbar`.
const SCROLLBAR: Selector = Selector::part(Part::Scrollbar);
/// `Part::Scrollbar` while scrolled.
const SCROLLED: Selector = Selector::part(Part::Scrollbar).with_state(State::SCROLLED);

/// Screens (any node without a parent).
fn screen(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(SCROLLED, &s::SCROLLBAR_SCROLLED);
}

/// The scrollbar styles of scrollable containers.
fn scrollbars(cx: &mut ThemeCx<'_>) {
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(SCROLLED, &s::SCROLLBAR_SCROLLED);
}

/// Declares one `ComputedRecipe` per `(NAME, CLASS)` with the function `apply`.
macro_rules! recipes {
    ($($(#[$doc:meta])* $name:ident for $class:path => $apply:path;)*) => {$(
        $(#[$doc])*
        pub static $name: ComputedRecipe = ComputedRecipe::new(stringify!($name), &$class, $apply);
    )*};
}

recipes! {
    /// Labels: no styles (they inherit their text style).
    LABEL for LABEL_CLASS => nothing;
    /// A textarea's text: a primary selection (lines 1086-1090).
    TEXTAREA_LABEL for TEXTAREA_LABEL_CLASS => textarea_label;
    /// Plain objects: screens, else cards (lines 758-808).
    OBJ for OBJ_CLASS => obj;
    /// Tabview pages: padded, not cards (lines 758-808).
    TABVIEW_PAGE for TABVIEW_PAGE_CLASS => tabview_page;
    /// Twine's card: LVGL's look of a plain object, explicitly.
    CARD for CARD_CLASS => card;
    /// Buttons (lines 827-839).
    BUTTON for BUTTON_CLASS => button;
    /// Tab buttons of tabviews (lines 815-826).
    TABVIEW_TAB_BUTTON for TABVIEW_TAB_BUTTON_CLASS => tabview_tab_button;
    /// The back buttons of menus (lines 840-845).
    MENU_BACK_BUTTON for MENU_BACK_BUTTON_CLASS => menu_back_button;
    /// Images: no styles (LVGL styles neither images nor span groups).
    IMAGE for IMAGE_CLASS => nothing;
    /// Span groups: no styles.
    SPANGROUP for SPANGROUP_CLASS => nothing;
    /// Button-matrix popovers: no styles.
    POPOVER for POPOVER_CLASS => nothing;
    /// Image buttons: no styles (their images are their look).
    IMAGE_BUTTON for IMAGE_BUTTON_CLASS => nothing;
    /// Animated images: no styles.
    ANIMIMG for ANIMIMG_CLASS => nothing;
    /// The page container of tab views: no styles (its pages are styled).
    TABVIEW_CONTENT for TABVIEW_CONTENT_CLASS => nothing;
    /// Lists (lines 1092-1111).
    LIST for LIST_CLASS => list;
    /// List texts.
    LIST_TEXT for LIST_TEXT_CLASS => list_text;
    /// List buttons.
    LIST_BUTTON for LIST_BUTTON_CLASS => list_button;
    /// Menus (lines 1113-1149).
    MENU for MENU_CLASS => menu;
    /// The menu sidebar.
    MENU_SIDEBAR_CONTAINER for MENU_SIDEBAR_CONTAINER_CLASS => menu_sidebar_cont;
    /// The menu's main area.
    MENU_MAIN_CONTAINER for MENU_MAIN_CONTAINER_CLASS => menu_main_cont;
    /// Menu rows.
    MENU_CONT for MENU_CONT_CLASS => menu_cont;
    /// The sidebar header.
    MENU_SIDEBAR_HEADER_CONTAINER for MENU_SIDEBAR_HEADER_CONTAINER_CLASS => menu_header_cont;
    /// The main header.
    MENU_MAIN_HEADER_CONTAINER for MENU_MAIN_HEADER_CONTAINER_CLASS => menu_header_cont;
    /// Menu pages.
    MENU_PAGE for MENU_PAGE_CLASS => menu_page;
    /// Menu sections.
    MENU_SECTION for MENU_SECTION_CLASS => menu_section;
    /// Menu separators.
    MENU_SEPARATOR for MENU_SEPARATOR_CLASS => menu_separator;
    /// Message boxes (lines 1151-1189).
    MSGBOX for MSGBOX_CLASS => msgbox;
    /// The modal backdrop of message boxes.
    MSGBOX_BACKDROP for MSGBOX_BACKDROP_CLASS => msgbox_backdrop;
    /// Message box headers.
    MSGBOX_HEADER for MSGBOX_HEADER_CLASS => msgbox_header;
    /// Message box footers.
    MSGBOX_FOOTER for MSGBOX_FOOTER_CLASS => msgbox_footer;
    /// Message box contents.
    MSGBOX_CONTENT for MSGBOX_CONTENT_CLASS => msgbox_content;
    /// Message box header buttons.
    MSGBOX_HEADER_BUTTON for MSGBOX_HEADER_BUTTON_CLASS => msgbox_button;
    /// Message box footer buttons.
    MSGBOX_FOOTER_BUTTON for MSGBOX_FOOTER_BUTTON_CLASS => msgbox_button;
    /// Tile views (lines 1200-1210).
    TILEVIEW for TILEVIEW_CLASS => tileview;
    /// Tiles.
    TILEVIEW_TILE for TILEVIEW_TILE_CLASS => scrollbars;
    /// Tab views (lines 1212-1217).
    TABVIEW for TABVIEW_CLASS => tabview;
    /// Tab bars (lines 760-771).
    TABVIEW_TAB_BAR for TABVIEW_TAB_BAR_CLASS => tabview_tab_bar;
    /// Windows (lines 783-797, 1219-1223).
    WIN for WIN_CLASS => win;
    /// Window headers.
    WIN_HEADER for WIN_HEADER_CLASS => win_header;
    /// Window contents.
    WIN_CONTENT for WIN_CONTENT_CLASS => win_content;
    /// Lines (lines 850-854).
    LINE for LINE_CLASS => line;
    /// Bars (lines 884-894).
    BAR for BAR_CLASS => bar;
    /// Sliders (lines 895-911).
    SLIDER for SLIDER_CLASS => slider;
    /// Checkboxes (lines 930-947).
    CHECKBOX for CHECKBOX_CLASS => checkbox;
    /// Switches (lines 947-964).
    SWITCH for SWITCH_CLASS => switch;
    /// Arcs (lines 1015-1025).
    ARC for ARC_CLASS => arc;
    /// Spinners (lines 1026-1032).
    SPINNER for SPINNER_CLASS => spinner;
    /// LEDs (lines 1225-1229).
    LED for LED_CLASS => led;
    /// Button matrices (lines 856-883).
    BUTTONMATRIX for BUTTONMATRIX_CLASS => buttonmatrix;
    /// Text areas (lines 1034-1046).
    TEXTAREA for TEXTAREA_CLASS => textarea;
    /// Keyboards (lines 1069-1084).
    KEYBOARD for KEYBOARD_CLASS => keyboard;
    /// Spin boxes (lines 1191-1198).
    SPINBOX for SPINBOX_CLASS => spinbox;
    /// Rollers (lines 978-988).
    ROLLER for ROLLER_CLASS => roller;
    /// Dropdowns (lines 990-1004).
    DROPDOWN for DROPDOWN_CLASS => dropdown;
    /// Dropdown lists (lines 1001-1012).
    DROPDOWN_LIST for DROPDOWN_LIST_CLASS => dropdown_list;
}

/// Every per-class recipe of the LVGL theme, the most common classes first (the order the
/// lineage lookup scans).
pub const ALL_RECIPES: &[&ComputedRecipe] = &[
    &LABEL,
    &OBJ,
    &CARD,
    &BUTTON,
    &TEXTAREA_LABEL,
    &TABVIEW_PAGE,
    &TABVIEW_TAB_BUTTON,
    &MENU_BACK_BUTTON,
    &IMAGE,
    &SPANGROUP,
    &POPOVER,
    &IMAGE_BUTTON,
    &ANIMIMG,
    &TABVIEW_CONTENT,
    &LIST,
    &LIST_TEXT,
    &LIST_BUTTON,
    &MENU,
    &MENU_SIDEBAR_CONTAINER,
    &MENU_MAIN_CONTAINER,
    &MENU_CONT,
    &MENU_SIDEBAR_HEADER_CONTAINER,
    &MENU_MAIN_HEADER_CONTAINER,
    &MENU_PAGE,
    &MENU_SECTION,
    &MENU_SEPARATOR,
    &MSGBOX,
    &MSGBOX_BACKDROP,
    &MSGBOX_HEADER,
    &MSGBOX_FOOTER,
    &MSGBOX_CONTENT,
    &MSGBOX_HEADER_BUTTON,
    &MSGBOX_FOOTER_BUTTON,
    &TILEVIEW,
    &TILEVIEW_TILE,
    &TABVIEW,
    &TABVIEW_TAB_BAR,
    &WIN,
    &WIN_HEADER,
    &WIN_CONTENT,
    &LINE,
    &BAR,
    &SLIDER,
    &CHECKBOX,
    &SWITCH,
    &ARC,
    &SPINNER,
    &LED,
    &BUTTONMATRIX,
    &TEXTAREA,
    &KEYBOARD,
    &SPINBOX,
    &ROLLER,
    &DROPDOWN,
    &DROPDOWN_LIST,
];

fn nothing(_: &mut ThemeCx<'_>) {}

/// Lines 1086-1090: a textarea's text (`TEXTAREA_LABEL_CLASS`) selects in the primary color.
fn textarea_label(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::part(Part::Selected), &s::BG_COLOR_PRIMARY);
}

fn obj(cx: &mut ThemeCx<'_>) {
    if cx.parent().is_none() {
        screen(cx);
        return;
    }
    // NOTE(P20.S03): calendar children branch before the card styles here, as in LVGL.
    cx.add_style(Selector::MAIN, &s::CARD);
    scrollbars(cx);
}

/// Lines 758-808: tabview pages (`TABVIEW_PAGE_CLASS`) are padded, not cards.
fn tabview_page(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::PAD_NORMAL);
    cx.add_style(Selector::MAIN, &s::ROTARY_SCROLL);
    scrollbars(cx);
}

fn card(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    scrollbars(cx);
}

/// Lines 815-826: a tab's button in a tabview's bar (`TABVIEW_TAB_BUTTON_CLASS`).
fn tabview_tab_button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::state(State::PRESSED), &s::PRESSED);
    cx.add_style(Selector::state(State::CHECKED), &s::BG_COLOR_PRIMARY_MUTED);
    cx.add_style(Selector::state(State::CHECKED), &s::TAB_BTN);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::TAB_BG_FOCUS);
}

fn button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BTN);
    cx.add_style(Selector::MAIN, &s::BG_COLOR_PRIMARY);
    cx.add_style(Selector::MAIN, &s::TRANSITION_DELAYED);
    cx.add_style(Selector::state(State::PRESSED), &s::PRESSED);
    cx.add_style(Selector::state(State::PRESSED), &s::TRANSITION_NORMAL);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    // LV_THEME_DEFAULT_GROW
    cx.add_style(Selector::state(State::PRESSED), &s::GROW);
    cx.add_style(Selector::state(State::CHECKED), &s::BG_COLOR_SECONDARY);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
}

/// Lines 840-845: the back button of a menu's headers (`MENU_BACK_BUTTON_CLASS`): a button,
/// flat.
fn menu_back_button(cx: &mut ThemeCx<'_>) {
    button(cx);
    cx.add_style(Selector::MAIN, &s::MENU_HEADER_BTN);
    cx.add_style(Selector::state(State::PRESSED), &s::MENU_PRESSED);
}

fn list(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::LIST_BG);
    scrollbars(cx);
}

fn list_text(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BG_COLOR_GREY);
    cx.add_style(Selector::MAIN, &s::LIST_ITEM_GROW);
}

fn list_button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BG_COLOR_WHITE);
    cx.add_style(Selector::MAIN, &s::LIST_BTN);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::BG_COLOR_PRIMARY);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::LIST_ITEM_GROW);
    cx.add_style(Selector::state(State::PRESSED), &s::LIST_ITEM_GROW);
    cx.add_style(Selector::state(State::PRESSED), &s::PRESSED);
}

fn menu(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::MENU_BG);
}

fn menu_sidebar_cont(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_SIDEBAR_CONT);
    scrollbars(cx);
}

fn menu_main_cont(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_MAIN_CONT);
    scrollbars(cx);
}

fn menu_cont(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_CONT);
    cx.add_style(Selector::state(State::PRESSED), &s::MENU_PRESSED);
    cx.add_style(
        Selector::state(State::PRESSED | State::CHECKED),
        &s::BG_COLOR_PRIMARY_MUTED,
    );
    cx.add_style(Selector::state(State::CHECKED), &s::BG_COLOR_PRIMARY_MUTED);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::BG_COLOR_PRIMARY);
}

fn menu_header_cont(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_HEADER_CONT);
}

fn menu_page(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_PAGE);
    scrollbars(cx);
}

fn menu_section(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_SECTION);
}

fn menu_separator(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MENU_SEPARATOR);
}

fn msgbox(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_ZERO);
    cx.add_style(Selector::MAIN, &s::CLIP_CORNER);
}

fn msgbox_backdrop(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::MSGBOX_BACKDROP_BG);
}

fn msgbox_header(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
    cx.add_style(Selector::MAIN, &s::BG_COLOR_GREY);
}

fn msgbox_footer(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
}

fn msgbox_content(cx: &mut ThemeCx<'_>) {
    scrollbars(cx);
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
}

fn msgbox_button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BTN);
    cx.add_style(Selector::MAIN, &s::BG_COLOR_PRIMARY);
    cx.add_style(Selector::MAIN, &s::TRANSITION_DELAYED);
    cx.add_style(Selector::state(State::PRESSED), &s::PRESSED);
    cx.add_style(Selector::state(State::PRESSED), &s::TRANSITION_NORMAL);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::CHECKED), &s::BG_COLOR_SECONDARY);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
}

fn tileview(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    scrollbars(cx);
}

fn tabview(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(Selector::MAIN, &s::PAD_ZERO);
}

fn tabview_tab_bar(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BG_COLOR_WHITE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::TAB_BG_FOCUS);
}

fn win(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CLIP_CORNER);
}

fn win_header(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BG_COLOR_GREY);
    cx.add_style(Selector::MAIN, &s::PAD_TINY);
}

fn win_content(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(Selector::MAIN, &s::PAD_NORMAL);
    scrollbars(cx);
}

fn line(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LINE);
}

fn bar(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::BG_COLOR_PRIMARY_MUTED);
    cx.add_style(Selector::MAIN, &s::CIRCLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::part(Part::Indicator), &s::BG_COLOR_PRIMARY);
    cx.add_style(Selector::part(Part::Indicator), &s::CIRCLE);
}

fn slider(cx: &mut ThemeCx<'_>) {
    let knob = Selector::part(Part::Knob);
    cx.add_style(Selector::MAIN, &s::BG_COLOR_PRIMARY_MUTED);
    cx.add_style(Selector::MAIN, &s::CIRCLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::part(Part::Indicator), &s::BG_COLOR_PRIMARY);
    cx.add_style(Selector::part(Part::Indicator), &s::CIRCLE);
    cx.add_style(knob, &s::KNOB);
    // LV_THEME_DEFAULT_GROW
    cx.add_style(knob.with_state(State::PRESSED), &s::GROW);
    cx.add_style(knob, &s::TRANSITION_DELAYED);
    cx.add_style(knob.with_state(State::PRESSED), &s::TRANSITION_NORMAL);
}

fn checkbox(cx: &mut ThemeCx<'_>) {
    let ind = Selector::part(Part::Indicator);
    cx.add_style(Selector::MAIN, &s::PAD_GAP);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(ind.with_state(State::DISABLED), &s::DISABLED);
    cx.add_style(ind, &s::CB_MARKER);
    cx.add_style(ind.with_state(State::CHECKED), &s::BG_COLOR_PRIMARY);
    cx.add_style(ind.with_state(State::CHECKED), &s::CB_MARKER_CHECKED);
    cx.add_style(ind.with_state(State::PRESSED), &s::PRESSED);
    // LV_THEME_DEFAULT_GROW
    cx.add_style(ind.with_state(State::PRESSED), &s::GROW);
    cx.add_style(ind.with_state(State::PRESSED), &s::TRANSITION_NORMAL);
    cx.add_style(ind, &s::TRANSITION_DELAYED);
}

fn switch(cx: &mut ThemeCx<'_>) {
    let ind = Selector::part(Part::Indicator);
    let knob = Selector::part(Part::Knob);
    cx.add_style(Selector::MAIN, &s::BG_COLOR_GREY);
    cx.add_style(Selector::MAIN, &s::CIRCLE);
    cx.add_style(Selector::MAIN, &s::ANIM_FAST);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(ind.with_state(State::CHECKED), &s::BG_COLOR_PRIMARY);
    cx.add_style(ind, &s::CIRCLE);
    cx.add_style(knob, &s::KNOB);
    cx.add_style(knob, &s::BG_COLOR_WHITE);
    cx.add_style(knob, &s::SWITCH_KNOB);
    cx.add_style(ind.with_state(State::CHECKED), &s::TRANSITION_NORMAL);
    cx.add_style(ind, &s::TRANSITION_NORMAL);
}

fn arc(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::ARC_INDIC);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_INDIC);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_INDIC_PRIMARY);
    cx.add_style(Selector::part(Part::Knob), &s::KNOB);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
}

fn spinner(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::ARC_INDIC);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_INDIC);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_INDIC_PRIMARY);
}

fn led(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LED);
}

fn buttonmatrix(cx: &mut ThemeCx<'_>) {
    // NOTE(P20.S03): calendar button matrices branch here, as in LVGL.
    let items = Selector::part(Part::Items);
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(items, &s::BTN);
    cx.add_style(items.with_state(State::DISABLED), &s::DISABLED);
    cx.add_style(items.with_state(State::PRESSED), &s::PRESSED);
    cx.add_style(items.with_state(State::CHECKED), &s::BG_COLOR_PRIMARY);
    cx.add_style(items.with_state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(items.with_state(State::EDITED), &s::OUTLINE_SECONDARY);
}

fn textarea(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    scrollbars(cx);
    cx.add_style(
        Selector::part(Part::Cursor).with_state(State::FOCUSED),
        &s::TA_CURSOR,
    );
    // LV_PART_TEXTAREA_PLACEHOLDER (= LV_PART_CUSTOM_FIRST)
    cx.add_style(Selector::part(PLACEHOLDER), &s::TA_PLACEHOLDER);
}

fn keyboard(cx: &mut ThemeCx<'_>) {
    let items = Selector::part(Part::Items);
    cx.add_style(Selector::MAIN, &s::SCR);
    // `PAD_SMALL` on large displays, else `PAD_TINY` (the `KEYBOARD_PAD` metric).
    cx.add_style(Selector::MAIN, &s::KEYBOARD_PAD_STYLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(items, &s::BTN);
    cx.add_style(items.with_state(State::DISABLED), &s::DISABLED);
    cx.add_style(items, &s::BG_COLOR_WHITE);
    cx.add_style(items, &s::KEYBOARD_BUTTON_BG);
    cx.add_style(items.with_state(State::PRESSED), &s::PRESSED);
    cx.add_style(items.with_state(State::CHECKED), &s::BG_COLOR_GREY);
    cx.add_style(items.with_state(State::FOCUS_KEY), &s::BG_COLOR_PRIMARY_MUTED);
    cx.add_style(items.with_state(State::EDITED), &s::BG_COLOR_SECONDARY_MUTED);
}

fn spinbox(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::part(Part::Cursor), &s::BG_COLOR_PRIMARY);
}

fn roller(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::ANIM);
    cx.add_style(Selector::MAIN, &s::LINE_SPACE_LARGE);
    cx.add_style(Selector::MAIN, &s::TEXT_ALIGN_CENTER);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::part(Part::Selected), &s::BG_COLOR_PRIMARY);
}

fn dropdown(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_SMALL);
    cx.add_style(Selector::MAIN, &s::TRANSITION_DELAYED);
    cx.add_style(Selector::state(State::PRESSED), &s::TRANSITION_NORMAL);
    cx.add_style(Selector::state(State::PRESSED), &s::PRESSED);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::OUTLINE_PRIMARY);
    cx.add_style(Selector::state(State::EDITED), &s::OUTLINE_SECONDARY);
    cx.add_style(Selector::part(Part::Indicator), &s::TRANSITION_NORMAL);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
}

fn dropdown_list(cx: &mut ThemeCx<'_>) {
    let sel = Selector::part(Part::Selected);
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::CLIP_CORNER);
    cx.add_style(Selector::MAIN, &s::LINE_SPACE_LARGE);
    cx.add_style(Selector::MAIN, &s::DROPDOWN_LIST);
    scrollbars(cx);
    cx.add_style(sel, &s::BG_COLOR_WHITE);
    cx.add_style(sel.with_state(State::CHECKED), &s::BG_COLOR_PRIMARY);
    cx.add_style(sel.with_state(State::PRESSED), &s::PRESSED);
}
