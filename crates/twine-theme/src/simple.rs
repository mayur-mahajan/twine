//! [`Simple`]: LVGL's simple theme (`src/themes/simple/lv_theme_simple.c`), as `static` data.
//!
//! Its colors are [design elements](twine_style::design) (`COLOR_WHITE` = `SURFACE`,
//! `COLOR_LIGHT` = `SURFACE_VARIANT`, `COLOR_DARK` = `PRIMARY`, `COLOR_DIM` = `ON_SURFACE`,
//! `COLOR_SCR` = `BACKGROUND`) of one `'static` table per mode ([`LIGHT`], [`DARK`],
//! [`NIGHT`], [`HIGH_CONTRAST`]), so an application recolors it with its own tables
//! ([`SimpleSet::table`] plus `ElementTable`'s const builders).
//!
//! | Mode | Look |
//! |------|------|
//! | [`ThemeMode::Light`] | LVGL's: a light grey screen, white containers, grey buttons and tracks, dark grey text |
//! | [`ThemeMode::Dark`] | Twine's: the same flat tones inverted — `#121212` screen, `#1E1E1E` containers, `#3A3A3A` tracks, mid-grey buttons and indicators, light grey knobs and text |
//! | [`ThemeMode::Night`] | Twine's: dim, warm amber on near-black (every color ≤ 30 % relative luminance, blue ≤ red / 2) |
//! | [`ThemeMode::HighContrast`] | black text on white, black knobs and scrollbars, darker status colors |
//!
//! LVGL's simple theme has no borders or outlines. Its styles put the text color
//! `ON_SURFACE` (not `ON_PRIMARY`) on `PRIMARY` buttons, so the high-contrast `PRIMARY` is a
//! mid grey that black text reads on at 7.8:1, and the dark and night `PRIMARY` a dark tone
//! their light text reads on; for a high-contrast UI with clearly outlined controls prefer
//! [`Lvgl`](crate::Lvgl). Its sizes do not depend on the display (no metrics table).

use twine_core::{Color, Opa};
use twine_engine::{ComputedRecipe, OBJ_CLASS, RecipeRef, ThemeBase, ThemeCx, ThemeHook, WidgetClass};
use twine_style::design::{self, ElementTable, Elevation, MotionSet};
use twine_style::{Length, Part, Radius, Selector, State, ThemeMode};
use twine_text::Font;
use twine_widgets::arc::ARC_CLASS;
use twine_widgets::bar::BAR_CLASS;
use twine_widgets::button::BUTTON_CLASS;
use twine_widgets::buttonmatrix::BUTTONMATRIX_CLASS;
use twine_widgets::checkbox::CHECKBOX_CLASS;
use twine_widgets::container::CARD_CLASS;
use twine_widgets::keyboard::KEYBOARD_CLASS;
use twine_widgets::label::LABEL_CLASS;
use twine_widgets::led::LED_CLASS;
use twine_widgets::slider::SLIDER_CLASS;
use twine_widgets::spinbox::SPINBOX_CLASS;
use twine_widgets::spinner::SPINNER_CLASS;
use twine_widgets::switch::SWITCH_CLASS;
use twine_widgets::textarea::TEXTAREA_CLASS;
use twine_widgets_ext::dropdown::{DROPDOWN_CLASS, DROPDOWN_LIST_CLASS};
use twine_widgets_ext::list::{LIST_BUTTON_CLASS, LIST_CLASS};
use twine_widgets_ext::msgbox::MSGBOX_CLASS;
use twine_widgets_ext::roller::ROLLER_CLASS;
use twine_widgets_ext::tabview::{
    TABVIEW_CLASS, TABVIEW_CONTENT_CLASS, TABVIEW_PAGE_CLASS, TABVIEW_TAB_BAR_CLASS,
};
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::modes::ModeList;
use crate::{Palette, Theme, Tone};
use styles as s;

/// `COLOR_SCR` = `lv_palette_lighten(GREY, 4)`.
const COLOR_SCR: Color = Palette::Grey.tone(Tone::L4);
/// `COLOR_WHITE` = white.
const COLOR_WHITE: Color = Color::WHITE;
/// `COLOR_LIGHT` = `lv_palette_lighten(GREY, 2)`.
const COLOR_LIGHT: Color = Palette::Grey.tone(Tone::L2);
/// `COLOR_DARK` = `lv_palette_main(GREY)`.
const COLOR_DARK: Color = Palette::Grey.main();
/// `COLOR_DIM` = `lv_palette_darken(GREY, 2)`.
const COLOR_DIM: Color = Palette::Grey.tone(Tone::D2);

/// The simple theme's element set: its one font, turned into a mode's `'static` table with
/// [`table`](Self::table) ([`Simple::SET`]: Montserrat 14 with the `assets` feature, else the
/// empty font).
///
/// ```
/// use twine_core::Color;
/// use twine_style::ThemeMode;
/// use twine_style::design::{self, ElementTable};
/// use twine_theme::Simple;
///
/// /// LVGL's simple look with a teal accent.
/// static TEAL: ElementTable = Simple::SET.table(ThemeMode::Light).color(design::PRIMARY, Color::hex(0x00897B));
/// assert_eq!(TEAL.get(design::PRIMARY), Some(Color::hex(0x00897B)));
/// assert_eq!(TEAL.missing_standard(), None);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct SimpleSet {
    /// The font of all text: `design::FONT_SMALL`, `FONT_BODY` and `FONT_LARGE`.
    pub font: &'static Font,
}

impl SimpleSet {
    /// The font of all text.
    #[must_use]
    pub const fn font(mut self, font: &'static Font) -> Self {
        self.font = font;
        self
    }

    /// The `'static` table of `mode` (values in the [module documentation](self)): LVGL's
    /// colors in [`ThemeMode::Light`], their high-contrast variant, and Twine's dark and night
    /// variants (the same flat grey tones, inverted; night in dim, warm amber), fixed spacings
    /// and radii, no shadows, no motion. `const`; never panics.
    #[must_use]
    pub const fn table(self, mode: ThemeMode) -> ElementTable {
        let c = SimpleColors::new(mode);
        ElementTable::new()
            .color(design::BACKGROUND, c.background)
            .color(design::SURFACE, c.surface)
            .color(design::ON_SURFACE, c.on_surface)
            .color(design::ON_SURFACE_MUTED, c.on_surface_muted)
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
            .color(design::PLACEHOLDER, c.placeholder)
            .color(design::NEUTRAL, c.neutral)
            .color(
                design::ACCENT_TINT,
                Color::mix(c.primary, c.surface, Opa::pct(20)),
            )
            .color(design::SEPARATOR, c.outline)
            .color(design::GROUPED_BACKGROUND, c.background)
            .color(design::FILL_1, Color::mix(c.neutral, c.surface, Opa::pct(20)))
            .color(design::FILL_2, Color::mix(c.neutral, c.surface, Opa::pct(30)))
            .color(design::FILL_3, Color::mix(c.neutral, c.surface, Opa::pct(40)))
            .length(design::SPACE_XS, Length::Px(2))
            .length(design::SPACE_S, Length::Px(4))
            .length(design::SPACE_M, Length::Px(8))
            .length(design::SPACE_L, Length::Px(12))
            .radius(design::RADIUS_S, Radius::Px(2))
            .radius(design::RADIUS_M, Radius::Px(4))
            .radius(design::RADIUS_L, Radius::Px(8))
            .radius(design::RADIUS_FULL, Radius::Circle)
            .opacity(design::FOCUS_RING_OPACITY, Opa::COVER)
            // No shadows in the simple theme.
            .opacity(design::SHADOW_OPACITY, Opa::TRANSP)
            .opacity(design::DISABLED_OPACITY, c.disabled_opacity)
            .opacity(design::SCRIM_OPACITY, c.scrim_opacity)
            .font(design::FONT_SMALL, self.font)
            .font(design::FONT_BODY, self.font)
            .font(design::FONT_LARGE, self.font)
            .elevation(Elevation::NONE)
            .motions(MotionSet::INSTANT)
    }
}

/// The colors and opacities of one mode of the simple theme (see the module documentation).
struct SimpleColors {
    background: Color,
    surface: Color,
    on_surface: Color,
    on_surface_muted: Color,
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
    placeholder: Color,
    neutral: Color,
    disabled_opacity: Opa,
    scrim_opacity: Opa,
}

impl SimpleColors {
    /// LVGL's light colors, their high-contrast variant, Twine's dark and night variants.
    const fn new(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self {
                background: COLOR_SCR,
                surface: COLOR_WHITE,
                on_surface: COLOR_DIM,
                // Readable secondary text (4.6:1; the styles do not use it).
                on_surface_muted: Palette::Grey.tone(Tone::D1),
                surface_variant: COLOR_LIGHT,
                outline: COLOR_LIGHT,
                // Buttons and indicators: the styles put `ON_SURFACE` text on them.
                primary: COLOR_DARK,
                // Black reads on `COLOR_DARK` (7.8:1), white would not (2.7:1).
                on_primary: Color::BLACK,
                secondary: COLOR_DIM,
                on_secondary: COLOR_WHITE,
                danger: Palette::Red.main(),
                warning: Palette::Orange.main(),
                ok: Palette::Green.main(),
                disabled: COLOR_LIGHT,
                focus_ring: COLOR_DARK,
                scrollbar: COLOR_DARK,
                shadow: COLOR_DARK,
                scrim: COLOR_DARK,
                placeholder: COLOR_DARK,
                neutral: COLOR_DARK,
                disabled_opacity: Opa::P50,
                scrim_opacity: Opa::P50,
            },
            // The flat grey tones inverted: near-black screens, dark grey containers, mid-grey
            // tracks, light grey knobs and text; indicators a lighter grey than the tracks.
            ThemeMode::Dark => Self {
                background: Color::hex(0x0012_1212),
                surface: Color::hex(0x001E_1E1E),
                on_surface: Palette::Grey.tone(Tone::L2),
                on_surface_muted: Palette::Grey.main(),
                surface_variant: Color::hex(0x003A_3A3A),
                outline: Color::hex(0x003A_3A3A),
                primary: Palette::Grey.tone(Tone::D1),
                on_primary: Color::WHITE,
                secondary: Palette::Grey.tone(Tone::L2),
                on_secondary: Color::hex(0x0012_1212),
                danger: Palette::Red.tone(Tone::L1),
                warning: Palette::Orange.tone(Tone::L1),
                ok: Palette::Green.tone(Tone::L1),
                disabled: Color::hex(0x003A_3A3A),
                focus_ring: Palette::Grey.main(),
                scrollbar: Palette::Grey.tone(Tone::D1),
                shadow: Color::BLACK,
                scrim: Color::BLACK,
                placeholder: Palette::Grey.tone(Tone::D1),
                neutral: Palette::Grey.tone(Tone::D1),
                disabled_opacity: Opa::P50,
                scrim_opacity: Opa::P50,
            },
            // Dim, warm amber on near-black: every color at most 30 % relative luminance with
            // blue at most half of red (like the LVGL theme's night mode).
            ThemeMode::Night => Self {
                background: Color::BLACK,
                surface: Color::hex(0x0010_0C08),
                on_surface: Color::hex(0x00BC_8644),
                on_surface_muted: Color::hex(0x00A4_7644),
                surface_variant: Color::hex(0x001C_150D),
                outline: Color::hex(0x002A_2014),
                primary: Color::hex(0x002E_2216),
                on_primary: Color::hex(0x00BC_8644),
                secondary: Color::hex(0x00BC_8644),
                on_secondary: Color::BLACK,
                danger: Color::hex(0x00C8_3020),
                warning: Color::hex(0x00C0_8010),
                ok: Color::hex(0x005E_8C24),
                disabled: Color::hex(0x0010_0C08),
                focus_ring: Color::hex(0x00A4_7644),
                scrollbar: Color::hex(0x004A_3A24),
                shadow: Color::BLACK,
                scrim: Color::BLACK,
                placeholder: Color::hex(0x005A_462A),
                neutral: Color::hex(0x004A_3A24),
                disabled_opacity: Opa::P30,
                scrim_opacity: Opa::P70,
            },
            ThemeMode::HighContrast => Self {
                background: Color::WHITE,
                surface: Color::WHITE,
                on_surface: Color::BLACK,
                on_surface_muted: Palette::Grey.tone(Tone::D4),
                surface_variant: COLOR_LIGHT,
                outline: Color::BLACK,
                primary: COLOR_DARK,
                on_primary: Color::BLACK,
                secondary: Color::BLACK,
                on_secondary: Color::WHITE,
                danger: Palette::Red.tone(Tone::D4),
                warning: Color::hex(0x008A_4B00),
                ok: Palette::Green.tone(Tone::D4),
                disabled: COLOR_DARK,
                focus_ring: Color::BLACK,
                scrollbar: Color::BLACK,
                shadow: Color::BLACK,
                scrim: Color::BLACK,
                placeholder: Palette::Grey.tone(Tone::D3),
                neutral: Color::BLACK,
                disabled_opacity: Opa::P50,
                scrim_opacity: Opa::P50,
            },
        }
    }
}

/// LVGL's look with the default font (`Simple::SET.table(ThemeMode::Light)`).
pub static LIGHT: ElementTable = Simple::SET.table(ThemeMode::Light);
/// The flat grey tones inverted (`Simple::SET.table(ThemeMode::Dark)`).
pub static DARK: ElementTable = Simple::SET.table(ThemeMode::Dark);
/// Dim, warm amber on near-black (`Simple::SET.table(ThemeMode::Night)`).
pub static NIGHT: ElementTable = Simple::SET.table(ThemeMode::Night);
/// Black text on white (`Simple::SET.table(ThemeMode::HighContrast)`).
pub static HIGH_CONTRAST: ElementTable = Simple::SET.table(ThemeMode::HighContrast);

/// Every mode with the default font, in switching order: what [`Simple::new`] lists.
pub const MODES: &[(ThemeMode, &ElementTable)] = &[
    (ThemeMode::Light, &LIGHT),
    (ThemeMode::Dark, &DARK),
    (ThemeMode::Night, &NIGHT),
    (ThemeMode::HighContrast, &HIGH_CONTRAST),
];

/// The styles of `lv_theme_simple.c` (`my_theme_styles_t`) as `static`s (LVGL's names in
/// capitals). Reuse them in recipes of custom widgets.
pub mod styles {
    use twine_core::{Duration, Opa};
    use twine_style::design;
    use twine_style::{BorderSide, Style, style};

    /// `SCROLLBAR_WIDTH`.
    const SCROLLBAR_WIDTH: i32 = 2;

    /// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the theme font (LVGL takes the
    /// default font from `LV_FONT_DEFAULT` instead of a style).
    pub static SCR: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
        bg_color: design::BACKGROUND, // lv_style_set_bg_color(&scr, COLOR_SCR)
        text_color: design::ON_SURFACE, // lv_style_set_text_color(&scr, COLOR_DIM)
        font: design::FONT_BODY,
    };
    /// A transparent background (`lv_style_set_bg_opa(&transp, LV_OPA_TRANSP)`).
    pub static TRANSP: Style = style! { bg_opacity: Opa::TRANSP };
    /// Opaque `SURFACE` background, 1 px lines and 2 px arcs in the same color (LVGL
    /// `COLOR_WHITE`).
    pub static WHITE: Style = style! {
        bg_opacity: Opa::COVER,
        bg_color: design::SURFACE,
        line_width: 1,
        line_color: design::SURFACE,
        arc_width: 2,
        arc_color: design::SURFACE,
    };
    /// Like `WHITE` in `SURFACE_VARIANT` (LVGL `COLOR_LIGHT`: buttons, tracks).
    pub static LIGHT: Style = style! {
        bg_opacity: Opa::COVER,
        bg_color: design::SURFACE_VARIANT,
        line_width: 1,
        line_color: design::SURFACE_VARIANT,
        arc_width: 2,
        arc_color: design::SURFACE_VARIANT,
    };
    /// Like `WHITE` in `PRIMARY` (LVGL `COLOR_DARK`: checked states, indicators).
    pub static DARK: Style = style! {
        bg_opacity: Opa::COVER,
        bg_color: design::PRIMARY,
        line_width: 1,
        line_color: design::PRIMARY,
        arc_width: 2,
        arc_color: design::PRIMARY,
    };
    /// Like `WHITE` in `ON_SURFACE` (LVGL `COLOR_DIM`).
    pub static DIM: Style = style! {
        bg_opacity: Opa::COVER,
        bg_color: design::ON_SURFACE,
        line_width: 1,
        line_color: design::ON_SURFACE,
        arc_width: 2,
        arc_color: design::ON_SURFACE,
    };
    /// Scrollbars: opaque `SCROLLBAR` color, 2 px wide.
    pub static SCROLLBAR: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
        bg_color: design::SCROLLBAR, // lv_style_set_bg_color(&scrollbar, COLOR_DARK)
        width: SCROLLBAR_WIDTH, // lv_style_set_width(&scrollbar, SCROLLBAR_WIDTH)
    };
    /// Arcs: 6 px wide (`lv_style_set_arc_width(&arc_line, 6)`).
    pub static ARC_LINE: Style = style! { arc_width: 6 };
    /// Arc knobs: 5 px padding, a knob larger than the arc
    /// (`lv_style_set_pad_all(&arc_knob, 5)`).
    pub static ARC_KNOB: Style = style! { padding: 5 };
    /// The text area cursor: a 2 px `ON_SURFACE` left border, transparent, blinking every
    /// 500 ms (lines 131-138).
    pub static TA_CURSOR: Style = style! {
        border_side: BorderSide::LEFT, // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
        border_color: design::ON_SURFACE, // lv_style_set_border_color(&ta_cursor, COLOR_DIM)
        border_width: 2, // lv_style_set_border_width(&ta_cursor, 2)
        bg_opacity: Opa::TRANSP, // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
        anim_duration: Duration::ms(500), // lv_style_set_anim_duration(&ta_cursor, 500)
    };
}

/// The simple theme with its defaults ([`Simple::new`]: light and high contrast, the default
/// font), ready to give to a display: `AppConfig::new().theme(&simple::THEME)`. Linked only
/// when named.
pub static THEME: Simple = Simple::new();

/// LVGL's simple theme: a light grey screen, white containers, grey buttons, no shadows,
/// outlines or transitions. A cheap base for design systems
/// ([`DesignSystem`](twine_engine::DesignSystem) with `type Base = Simple;`).
///
/// A `const` value used as a `static`; [`modes`](Self::modes) lists the modes (with their
/// tables); the first is the start mode.
///
/// ```
/// use twine_style::ThemeMode;
/// use twine_theme::{Simple, Theme, ThemeHook};
///
/// static THEME: Simple = Simple::new();
/// assert!(core::ptr::eq(THEME.font_small(), &raw const twine_assets::fonts::MONTSERRAT_14));
/// assert_eq!(ThemeHook::modes(&THEME).len(), 4);
/// assert!(THEME.design(ThemeMode::Dark).is_some());
/// ```
#[derive(Clone, Copy)]
pub struct Simple {
    modes: ModeList,
    recipes: &'static [&'static ComputedRecipe],
}

impl core::fmt::Debug for Simple {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Simple")
            .field("modes", &self.modes.modes())
            .field("recipes", &self.recipes.len())
            .finish()
    }
}

impl Default for Simple {
    fn default() -> Self {
        Self::new()
    }
}

impl Simple {
    /// The default element set (Montserrat 14 with the `assets` feature).
    pub const SET: SimpleSet = SimpleSet {
        font: crate::theme::DEFAULT_FONT,
    };

    /// The theme in every mode with the default font ([`MODES`]); LVGL
    /// `lv_theme_simple_init`. `const`: `static THEME: Simple = Simple::new();`.
    #[doc(alias = "lv_theme_simple_init")]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modes: ModeList::new(MODES),
            recipes: ALL_RECIPES,
        }
    }

    /// Lists the modes the theme supports, each with its `'static` table, in switching
    /// order (the first is the start mode). Only the listed tables are linked; a mode listed
    /// twice counts once. Never panics.
    ///
    /// ```
    /// use twine_style::ThemeMode;
    /// use twine_theme::{Simple, ThemeHook, simple};
    ///
    /// static HC: Simple = Simple::new().modes(&[(ThemeMode::HighContrast, &simple::HIGH_CONTRAST)]);
    /// assert_eq!(ThemeHook::modes(&HC), &[ThemeMode::HighContrast]);
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
    /// use twine_theme::{Simple, simple};
    /// use twine_widgets::button::BUTTON_CLASS;
    /// use twine_widgets::slider::SLIDER_CLASS;
    ///
    /// /// A product showing labels and buttons on screens and cards only.
    /// static THEME: Simple = Simple::new().recipes(&[&simple::OBJ, &simple::LABEL, &simple::BUTTON]);
    /// assert_eq!(THEME.recipe(&BUTTON_CLASS).map(|r| r.name()), Some("BUTTON"));
    /// assert!(THEME.recipe(&SLIDER_CLASS).is_none()); // its recipe is not linked
    /// ```
    #[must_use]
    pub const fn recipes(mut self, recipes: &'static [&'static ComputedRecipe]) -> Self {
        self.recipes = recipes;
        self
    }
}

impl ThemeHook for Simple {
    /// LVGL `theme_apply()` of `lv_theme_simple.c`: a screen gets the screen styles; any
    /// other node the recipe of the nearest class of its theme-class lineage the theme knows.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if cx.parent().is_none() {
            screen(cx);
        } else if let Some(r) = self.recipe(class) {
            cx.add_recipe(r);
        } else {
            twine_core::trace!(target: "twine::style", "simple theme: no styles for class {}", class.name);
        }
    }

    fn font_normal(&self) -> &'static Font {
        crate::theme::first_font(self.modes.tables(), design::FONT_BODY)
    }

    fn name(&self) -> &'static str {
        "simple"
    }

    fn modes(&self) -> &[ThemeMode] {
        self.modes.modes()
    }

    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        self.modes.design(mode)
    }

    /// The recipe of the nearest class of `class`'s lineage among the theme's
    /// [`recipes`](Simple::recipes) ([`ALL_RECIPES`] by default).
    fn recipe(&self, class: &'static WidgetClass) -> Option<RecipeRef> {
        crate::recipes::find(self.recipes, class)
    }
}

impl ThemeBase for Simple {
    const THEME: &'static Self = &THEME;
}

impl Theme for Simple {
    fn font_small(&self) -> &'static Font {
        self.font_normal()
    }
    fn font_large(&self) -> &'static Font {
        self.font_normal()
    }
}

// ---- Per-class looks (LVGL `theme_apply()`) ------------------------------------------------

/// `Part::Scrollbar`.
const SCROLLBAR: Selector = Selector::part(Part::Scrollbar);

/// Screens.
fn screen(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

/// Declares one `ComputedRecipe` per `(NAME, CLASS)` with the function `apply`.
macro_rules! recipes {
    ($($(#[$doc:meta])* $name:ident for $class:path => $apply:path;)*) => {$(
        $(#[$doc])*
        pub static $name: ComputedRecipe = ComputedRecipe::new(stringify!($name), &$class, $apply);
    )*};
}

recipes! {
    /// Labels: nothing (LVGL's simple theme gives labels nothing).
    LABEL for LABEL_CLASS => nothing;
    /// Plain objects: tabview pages (lines 234-247), else white containers.
    OBJ for OBJ_CLASS => obj;
    /// Tabview pages (lines 234-247).
    TABVIEW_PAGE for TABVIEW_PAGE_CLASS => tabview_page;
    /// Tab bars: the screen look.
    TABVIEW_TAB_BAR for TABVIEW_TAB_BAR_CLASS => tab_part;
    /// Tab contents: the screen look.
    TABVIEW_CONTENT for TABVIEW_CONTENT_CLASS => tab_part;
    /// Cards: white containers.
    CARD for CARD_CLASS => white;
    /// Buttons: `COLOR_DARK`.
    BUTTON for BUTTON_CLASS => dark;
    /// List buttons (lines 398-409): `COLOR_DARK`.
    LIST_BUTTON for LIST_BUTTON_CLASS => dark;
    /// Bars (lines 290-295).
    BAR for BAR_CLASS => bar;
    /// Sliders (lines 297-303).
    SLIDER for SLIDER_CLASS => slider;
    /// Checkboxes (lines 312-317).
    CHECKBOX for CHECKBOX_CLASS => checkbox;
    /// Switches (lines 319-324).
    SWITCH for SWITCH_CLASS => switch;
    /// Arcs (lines 354-364).
    ARC for ARC_CLASS => arc;
    /// Spinners (lines 366-374).
    SPINNER for SPINNER_CLASS => spinner;
    /// Button matrices (lines 271-288).
    BUTTONMATRIX for BUTTONMATRIX_CLASS => buttonmatrix;
    /// Text areas (lines 376-382).
    TEXTAREA for TEXTAREA_CLASS => textarea;
    /// Keyboards (lines 390-396).
    KEYBOARD for KEYBOARD_CLASS => keyboard;
    /// Spin boxes (lines 418-423).
    SPINBOX for SPINBOX_CLASS => spinbox;
    /// Rollers (lines 335-340).
    ROLLER for ROLLER_CLASS => roller;
    /// Dropdowns (lines 342-352).
    DROPDOWN for DROPDOWN_CLASS => plain_white;
    /// Dropdown lists.
    DROPDOWN_LIST for DROPDOWN_LIST_CLASS => dropdown_list;
    /// Lists (lines 398-409).
    LIST for LIST_CLASS => light_scrolled;
    /// Window contents (lines 249-260).
    WIN_CONTENT for WIN_CONTENT_CLASS => light_scrolled;
    /// Message boxes (lines 411-416).
    MSGBOX for MSGBOX_CLASS => light;
    /// Window headers (lines 249-260).
    WIN_HEADER for WIN_HEADER_CLASS => light;
    /// LEDs (lines 434-437).
    LED for LED_CLASS => light;
    /// Tile views (lines 424-432).
    TILEVIEW for TILEVIEW_CLASS => tileview;
    /// Tiles.
    TILEVIEW_TILE for TILEVIEW_TILE_CLASS => tile;
}

/// Every per-class recipe of the simple theme (the order the lineage lookup scans).
pub const ALL_RECIPES: &[&ComputedRecipe] = &[
    &LABEL,
    &OBJ,
    &TABVIEW_PAGE,
    &TABVIEW_TAB_BAR,
    &TABVIEW_CONTENT,
    &CARD,
    &BUTTON,
    &LIST_BUTTON,
    &BAR,
    &SLIDER,
    &CHECKBOX,
    &SWITCH,
    &ARC,
    &SPINNER,
    &BUTTONMATRIX,
    &TEXTAREA,
    &KEYBOARD,
    &SPINBOX,
    &ROLLER,
    &DROPDOWN,
    &DROPDOWN_LIST,
    &LIST,
    &WIN_CONTENT,
    &MSGBOX,
    &WIN_HEADER,
    &LED,
    &TILEVIEW,
    &TILEVIEW_TILE,
];

fn nothing(_: &mut ThemeCx<'_>) {}

/// Lines 234-247: tabview pages (`TABVIEW_PAGE_CLASS`; the bar and the content have their own
/// classes).
fn tabview_page(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn obj(cx: &mut ThemeCx<'_>) {
    if cx.parent().is_none() {
        screen(cx);
    } else {
        white(cx);
    }
}

fn tab_part(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
}

fn white(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::WHITE);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn plain_white(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::WHITE);
}

fn dark(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::DARK);
}

fn light(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
}

fn light_scrolled(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn bar(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::part(Part::Indicator), &s::DARK);
}

fn slider(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::part(Part::Indicator), &s::DARK);
    cx.add_style(Selector::part(Part::Knob), &s::DIM);
}

fn checkbox(cx: &mut ThemeCx<'_>) {
    let ind = Selector::part(Part::Indicator);
    cx.add_style(ind, &s::LIGHT);
    cx.add_style(ind.with_state(State::CHECKED), &s::DARK);
}

fn switch(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::part(Part::Knob), &s::DIM);
}

fn arc(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::MAIN, &s::TRANSP);
    cx.add_style(Selector::MAIN, &s::ARC_LINE);
    cx.add_style(Selector::part(Part::Indicator), &s::DARK);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_LINE);
    cx.add_style(Selector::part(Part::Knob), &s::DIM);
    cx.add_style(Selector::part(Part::Knob), &s::ARC_KNOB);
}

fn spinner(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::MAIN, &s::TRANSP);
    cx.add_style(Selector::MAIN, &s::ARC_LINE);
    cx.add_style(Selector::part(Part::Indicator), &s::DARK);
    cx.add_style(Selector::part(Part::Indicator), &s::ARC_LINE);
}

fn buttonmatrix(cx: &mut ThemeCx<'_>) {
    let in_box = cx
        .parent_class()
        .is_some_and(|c| c.is_a(&MSGBOX_CLASS) || c.is_a(&TABVIEW_CLASS));
    if !in_box {
        cx.add_style(Selector::MAIN, &s::WHITE);
    }
    cx.add_style(Selector::part(Part::Items), &s::LIGHT);
}

fn textarea(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::WHITE);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(
        Selector::part(Part::Cursor).with_state(State::FOCUSED),
        &s::TA_CURSOR,
    );
}

fn keyboard(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(Selector::part(Part::Items), &s::WHITE);
    cx.add_style(Selector::part(Part::Items).with_state(State::CHECKED), &s::LIGHT);
}

fn spinbox(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::part(Part::Cursor), &s::DARK);
}

fn roller(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::LIGHT);
    cx.add_style(Selector::part(Part::Selected), &s::DARK);
}

fn dropdown_list(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::WHITE);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(Selector::part(Part::Selected), &s::LIGHT);
    cx.add_style(
        Selector::part(Part::Selected).with_state(State::CHECKED),
        &s::DARK,
    );
}

fn tileview(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn tile(cx: &mut ThemeCx<'_>) {
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}
