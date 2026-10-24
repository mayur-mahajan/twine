//! [`Mono`]: LVGL's monochrome theme (`src/themes/mono/lv_theme_mono.c`) for 1-bit displays,
//! as `static` data. Every color is pure black or pure white, so `I1` output is exact.
//!
//! The foreground and background colors are [design elements](twine_style::design)
//! (`ON_SURFACE` and `SURFACE` / `BACKGROUND`) of one `'static` table per mode
//! ([`LIGHT`], [`DARK`], [`NIGHT`], [`HIGH_CONTRAST`], or [`MonoSet::table`]), so the theme
//! supports every mode, switched at run time with
//! [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode):
//! [`ThemeMode::Light`] (black on white) and [`ThemeMode::Dark`] (white on black, LVGL's
//! `dark_bg`); a 1-bit panel can only invert, so [`ThemeMode::Night`] is white on black (the
//! least lit pixels; dimming is the backlight's job) and [`ThemeMode::HighContrast`] black on
//! white (every pair is 21:1 in every mode). Its sizes do not depend on the display (no
//! metrics table). The styles are the `static`s of [`styles`], the per-class looks one
//! [`ComputedRecipe`] per widget class ([`ALL_RECIPES`], or the ones the application lists
//! with [`Mono::recipes`]: only those are linked).

use twine_core::{Color, Opa};
use twine_engine::{ComputedRecipe, OBJ_CLASS, RecipeRef, ThemeBase, ThemeCx, ThemeHook, WidgetClass};
use twine_image::ImageSource;
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
use twine_widgets_ext::tabview::{TABVIEW_CLASS, TABVIEW_PAGE_CLASS};
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::Theme;
use crate::modes::ModeList;
use styles as s;

/// `PAD_DEF`.
const PAD_DEF: i32 = 4;

/// The mono theme's element set: its one font (LVGL's mono theme has one), turned into a
/// mode's `'static` table with [`table`](Self::table) ([`Mono::SET`]: Montserrat 14 with the
/// `assets` feature, else the empty font).
///
/// ```
/// use twine_core::Color;
/// use twine_style::ThemeMode;
/// use twine_style::design::{self, ElementTable};
/// use twine_theme::Mono;
///
/// static DARK: ElementTable = Mono::SET.font(&twine_assets::fonts::MONTSERRAT_14).table(ThemeMode::Dark);
/// assert_eq!(DARK.get(design::PRIMARY), Some(Color::WHITE));
/// assert_eq!(DARK.missing_standard(), None);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct MonoSet {
    /// The font of all text: `design::FONT_SMALL`, `FONT_BODY` and `FONT_LARGE`.
    pub font: &'static Font,
}

impl MonoSet {
    /// The font of all text.
    #[must_use]
    pub const fn font(mut self, font: &'static Font) -> Self {
        self.font = font;
        self
    }

    /// The `'static` table of `mode`: `COLOR_FG` / `COLOR_BG` of LVGL (black and white,
    /// swapped in dark mode) for every color, `PAD_DEF`-based spacings, no shadows, no
    /// motion. A 1-bit panel can only invert: `Night` is white on black like `Dark` (dimming
    /// is the backlight's job), `HighContrast` black on white like `Light` (both already
    /// 21:1). Tints and fills have no intermediate shade: `ACCENT_TINT` is 20 % foreground
    /// on the background, `FILL_1..3` the background. `const`; never panics.
    #[must_use]
    pub const fn table(self, mode: ThemeMode) -> ElementTable {
        // #define COLOR_FG dark_bg ? lv_color_white() : lv_color_black()
        // #define COLOR_BG dark_bg ? lv_color_black() : lv_color_white()
        let (fg, bg) = match mode {
            ThemeMode::Light | ThemeMode::HighContrast => (Color::BLACK, Color::WHITE),
            ThemeMode::Dark | ThemeMode::Night => (Color::WHITE, Color::BLACK),
        };
        ElementTable::new()
            .color(design::BACKGROUND, bg)
            .color(design::SURFACE, bg)
            .color(design::ON_SURFACE, fg)
            .color(design::ON_SURFACE_MUTED, fg)
            .color(design::SURFACE_VARIANT, bg)
            .color(design::OUTLINE, fg)
            .color(design::PRIMARY, fg)
            .color(design::ON_PRIMARY, bg)
            .color(design::SECONDARY, bg)
            .color(design::ON_SECONDARY, fg)
            .color(design::DANGER, fg)
            .color(design::WARNING, fg)
            .color(design::OK, fg)
            .color(design::DISABLED, bg)
            .color(design::FOCUS_RING, fg)
            .color(design::SCROLLBAR, fg)
            .color(design::SHADOW, fg)
            .color(design::SCRIM, bg)
            .color(design::PLACEHOLDER, fg)
            .color(design::NEUTRAL, fg)
            .color(design::ACCENT_TINT, Color::mix(fg, bg, Opa::pct(20)))
            .color(design::SEPARATOR, fg)
            .color(design::GROUPED_BACKGROUND, bg)
            .color(design::FILL_1, bg)
            .color(design::FILL_2, bg)
            .color(design::FILL_3, bg)
            .length(design::SPACE_XS, Length::Px(1))
            .length(design::SPACE_S, Length::Px(2)) // PAD_DEF / 2
            .length(design::SPACE_M, Length::Px(PAD_DEF))
            .length(design::SPACE_L, Length::Px(8)) // PAD_DEF * 2
            .radius(design::RADIUS_S, Radius::Px(1))
            .radius(design::RADIUS_M, Radius::Px(2))
            .radius(design::RADIUS_L, Radius::Px(4))
            .radius(design::RADIUS_FULL, Radius::Circle)
            .opacity(design::FOCUS_RING_OPACITY, Opa::COVER)
            // A 1-bit display shows no partial opacity: no shadows, no disabled overlay, an
            // opaque backdrop.
            .opacity(design::SHADOW_OPACITY, Opa::TRANSP)
            .opacity(design::DISABLED_OPACITY, Opa::TRANSP)
            .opacity(design::SCRIM_OPACITY, Opa::COVER)
            .font(design::FONT_SMALL, self.font)
            .font(design::FONT_BODY, self.font)
            .font(design::FONT_LARGE, self.font)
            .elevation(Elevation::NONE)
            .motions(MotionSet::INSTANT)
    }
}

/// Black on white with the default font (`Mono::SET.table(ThemeMode::Light)`).
pub static LIGHT: ElementTable = Mono::SET.table(ThemeMode::Light);
/// White on black (`Mono::SET.table(ThemeMode::Dark)`; LVGL's `dark_bg`).
pub static DARK: ElementTable = Mono::SET.table(ThemeMode::Dark);
/// White on black (`Mono::SET.table(ThemeMode::Night)`).
pub static NIGHT: ElementTable = Mono::SET.table(ThemeMode::Night);
/// Black on white (`Mono::SET.table(ThemeMode::HighContrast)`).
pub static HIGH_CONTRAST: ElementTable = Mono::SET.table(ThemeMode::HighContrast);

/// Every mode with the default font, in switching order: what [`Mono::new`] lists.
pub const MODES: &[(ThemeMode, &ElementTable)] = &[
    (ThemeMode::Light, &LIGHT),
    (ThemeMode::Dark, &DARK),
    (ThemeMode::Night, &NIGHT),
    (ThemeMode::HighContrast, &HIGH_CONTRAST),
];

/// Not in LVGL: the check mark of a checked checkbox (LVGL's mono theme only inverts the box).
pub static CHECK_MARK: ImageSource = ImageSource::symbol(twine_text::Symbol::Ok);

/// The styles of `lv_theme_mono.c` used by the widgets implemented so far, as `static`s
/// (LVGL's names in capitals), with `COLOR_FG` = the `ON_SURFACE` element and `COLOR_BG` =
/// `SURFACE` (`BACKGROUND` on screens). Reuse them in recipes of custom widgets.
// NOTE(P20.S05): the chart styles come with the chart.
pub mod styles {
    use twine_core::{Duration, Opa};
    use twine_style::design;
    use twine_style::{BorderSide, Radius, Style, TextDecor, style};

    use super::{CHECK_MARK, PAD_DEF};

    /// `BORDER_W_NORMAL`.
    const BORDER_W_NORMAL: i32 = 1;
    /// `BORDER_W_PR`.
    const BORDER_W_PR: i32 = 3;
    /// `BORDER_W_DIS`.
    const BORDER_W_DIS: i32 = 0;
    /// `BORDER_W_FOCUS`.
    const BORDER_W_FOCUS: i32 = 1;
    /// `BORDER_W_EDIT`.
    const BORDER_W_EDIT: i32 = 2;
    /// `SPINNER_WIDTH`.
    const SPINNER_WIDTH: i32 = 8;

    /// `EXPAND_BORDER(style, state)`: the padding of a border of `w` that keeps the content in
    /// place (`lv_style_set_pad_all(&style, PAD_DEF + BORDER_W_NORMAL - BORDER_W_x)`).
    const fn expand_pad(w: i32) -> i32 {
        PAD_DEF.saturating_add(BORDER_W_NORMAL).saturating_sub(w)
    }

    /// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the theme font, 4 px gaps.
    pub static SCR: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
        bg_color: design::BACKGROUND, // lv_style_set_bg_color(&scr, COLOR_BG)
        text_color: design::ON_SURFACE, // lv_style_set_text_color(&scr, COLOR_FG)
        gap: PAD_DEF, // lv_style_set_pad_row / pad_column(&scr, PAD_DEF)
        font: design::FONT_BODY, // lv_style_set_text_font(&scr, font)
    };
    /// Cards and most widgets: opaque `SURFACE`, a 1 px `ON_SURFACE` border, radius 2, 4 px
    /// padding and gaps, 2 px `ON_SURFACE` lines and arcs, a 300 ms animation duration.
    pub static CARD: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&card, LV_OPA_COVER)
        bg_color: design::SURFACE, // lv_style_set_bg_color(&card, COLOR_BG)
        border_color: design::ON_SURFACE, // lv_style_set_border_color(&card, COLOR_FG)
        radius: 2, // lv_style_set_radius(&card, 2)
        border_width: BORDER_W_NORMAL, // lv_style_set_border_width(&card, BORDER_W_NORMAL)
        padding: PAD_DEF, // lv_style_set_pad_all(&card, PAD_DEF)
        gap: PAD_DEF, // lv_style_set_pad_gap(&card, PAD_DEF)
        text_color: design::ON_SURFACE, // lv_style_set_text_color(&card, COLOR_FG)
        line_width: 2, // lv_style_set_line_width(&card, 2)
        line_color: design::ON_SURFACE, // lv_style_set_line_color(&card, COLOR_FG)
        arc_width: 2, // lv_style_set_arc_width(&card, 2)
        arc_color: design::ON_SURFACE, // lv_style_set_arc_color(&card, COLOR_FG)
        outline_color: design::FOCUS_RING, // lv_style_set_outline_color(&card, COLOR_FG)
        anim_duration: Duration::ms(300), // lv_style_set_anim_duration(&card, 300)
    };
    /// Scrollbars: opaque `SCROLLBAR` color, 4 px wide.
    pub static SCROLLBAR: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
        bg_color: design::SCROLLBAR, // lv_style_set_bg_color(&scrollbar, COLOR_FG)
        width: PAD_DEF, // lv_style_set_width(&scrollbar, PAD_DEF)
    };
    /// The pressed look: a 3 px border, the padding reduced so the content stays in place
    /// (`EXPAND_BORDER(pr, PR)`).
    pub static PR: Style = style! { border_width: BORDER_W_PR, padding: expand_pad(BORDER_W_PR) };
    /// Inverted colors (`PRIMARY` background, `ON_PRIMARY` text, border, lines, arcs and
    /// outline): checked states and indicators.
    pub static INV: Style = style! {
        bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&inv, LV_OPA_COVER)
        bg_color: design::PRIMARY, // lv_style_set_bg_color(&inv, COLOR_FG)
        border_color: design::ON_PRIMARY, // lv_style_set_border_color(&inv, COLOR_BG)
        line_color: design::ON_PRIMARY, // lv_style_set_line_color(&inv, COLOR_BG)
        arc_color: design::ON_PRIMARY, // lv_style_set_arc_color(&inv, COLOR_BG)
        text_color: design::ON_PRIMARY, // lv_style_set_text_color(&inv, COLOR_BG)
        outline_color: design::ON_PRIMARY, // lv_style_set_outline_color(&inv, COLOR_BG)
    };
    /// The disabled look: no border, the padding increased so the content stays in place
    /// (`EXPAND_BORDER(disabled, DIS)`).
    pub static DISABLED: Style = style! { border_width: BORDER_W_DIS, padding: expand_pad(BORDER_W_DIS) };
    /// The keyboard focus look: a 1 px outline, 1 px off the node.
    pub static FOCUS: Style = style! {
        outline_width: 1, // lv_style_set_outline_width(&focus, 1)
        outline_offset: BORDER_W_FOCUS, // lv_style_set_outline_pad(&focus, BORDER_W_FOCUS)
    };
    /// The editing look: a 2 px outline (`lv_style_set_outline_width(&edit, BORDER_W_EDIT)`).
    pub static EDIT: Style = style! { outline_width: BORDER_W_EDIT };
    /// A 2 px border (`lv_style_set_border_width(&large_border, BORDER_W_EDIT)`).
    pub static LARGE_BORDER: Style = style! { border_width: BORDER_W_EDIT };
    /// Gaps of 4 px (`lv_style_set_pad_gap(&pad_gap, PAD_DEF)`).
    pub static PAD_GAP: Style = style! { gap: PAD_DEF };
    /// No padding and no gaps.
    pub static PAD_ZERO: Style = style! { padding: 0, gap: 0 };
    /// Square corners (radius 0).
    pub static NO_RADIUS: Style = style! { radius: 0 };
    /// Fully round corners (`Radius::Circle`).
    pub static RADIUS_CIRCLE: Style = style! { radius: Radius::Circle };
    /// Text line spacing of 6 px.
    pub static LARGE_LINE_SPACE: Style = style! { line_spacing: 6 };
    /// Underlined text.
    pub static UNDERLINE: Style = style! { text_decoration: TextDecor::UNDERLINE };
    /// Spinner indicators: an 8 px rounded `ON_SURFACE` arc.
    pub static SPINNER_INDIC: Style = style! {
        arc_color: design::ON_SURFACE, // lv_style_set_arc_color(&spinner_indic, COLOR_FG)
        arc_width: SPINNER_WIDTH, // lv_style_set_arc_width(&spinner_indic, SPINNER_WIDTH)
        arc_rounded: true, // lv_style_set_arc_rounded(&spinner_indic, true)
    };
    /// The text area cursor: a 2 px `ON_SURFACE` left border, transparent, blinking every
    /// 500 ms (lines 187-194).
    pub static TA_CURSOR: Style = style! {
        border_side: BorderSide::LEFT, // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
        border_color: design::ON_SURFACE, // lv_style_set_border_color(&ta_cursor, COLOR_FG)
        border_width: 2, // lv_style_set_border_width(&ta_cursor, 2)
        bg_opacity: Opa::TRANSP, // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
        anim_duration: Duration::ms(500), // lv_style_set_anim_duration(&ta_cursor, 500)
    };
    /// Not in LVGL: the check mark of a checked checkbox ([`CHECK_MARK`]).
    pub static CB_MARKER_CHECKED: Style = style! { bg_image: &CHECK_MARK };
}

/// The mono theme with its defaults ([`Mono::new`]: every mode, the default font), ready to
/// give to a display: `AppConfig::new().theme(&mono::THEME)`. Linked only when named.
pub static THEME: Mono = Mono::new();

/// LVGL's monochrome theme: black on white (or white on black), 1-px borders, pressed
/// buttons get a thicker border, checked ones are inverted, focus and edit show outlines.
/// Supports every [`ThemeMode`], switchable at run time: a 1-bit panel can only invert, so
/// `Night` is white on black like `Dark` and `HighContrast` black on white like `Light`.
///
/// A `const` value used as a `static`; [`modes`](Self::modes) lists the modes (with their
/// tables, e.g. made with another font: [`MonoSet::table`]); the first is the start mode.
/// Per-class looks are changed by a [`DesignSystem`](twine_engine::DesignSystem) that
/// extends it (`type Base = Mono;`), reusing its [`styles`].
///
/// ```
/// use twine_core::Color;
/// use twine_style::{ThemeMode, design};
/// use twine_style::design::ElementTable;
/// use twine_theme::{Mono, ThemeHook};
///
/// static DARK: ElementTable = Mono::SET.font(&twine_assets::fonts::MONTSERRAT_14).table(ThemeMode::Dark);
/// /// An OLED in dark mode only.
/// static OLED: Mono = Mono::new().modes(&[(ThemeMode::Dark, &DARK)]);
/// assert_eq!(ThemeHook::modes(&OLED), &[ThemeMode::Dark]);
/// assert_eq!(OLED.design(ThemeMode::Dark).and_then(|t| t.get(design::PRIMARY)), Some(Color::WHITE));
/// assert!(OLED.design(ThemeMode::Light).is_none());
/// ```
#[derive(Clone, Copy)]
pub struct Mono {
    modes: ModeList,
    recipes: &'static [&'static ComputedRecipe],
}

impl core::fmt::Debug for Mono {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mono")
            .field("modes", &self.modes.modes())
            .field("recipes", &self.recipes.len())
            .finish()
    }
}

impl Default for Mono {
    fn default() -> Self {
        Self::new()
    }
}

impl Mono {
    /// The default element set (Montserrat 14 with the `assets` feature).
    pub const SET: MonoSet = MonoSet {
        font: crate::theme::DEFAULT_FONT,
    };

    /// The theme in every mode with the default font ([`MODES`]); LVGL
    /// `lv_theme_mono_init`. `const`: `static THEME: Mono = Mono::new();`.
    #[doc(alias = "lv_theme_mono_init")]
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
    /// use twine_theme::{Mono, mono};
    /// use twine_widgets::button::BUTTON_CLASS;
    /// use twine_widgets::slider::SLIDER_CLASS;
    ///
    /// /// A product showing labels and buttons on screens and cards only.
    /// static THEME: Mono = Mono::new().recipes(&[&mono::OBJ, &mono::LABEL, &mono::BUTTON]);
    /// assert_eq!(THEME.recipe(&BUTTON_CLASS).map(|r| r.name()), Some("BUTTON"));
    /// assert!(THEME.recipe(&SLIDER_CLASS).is_none()); // its recipe is not linked
    /// ```
    #[must_use]
    pub const fn recipes(mut self, recipes: &'static [&'static ComputedRecipe]) -> Self {
        self.recipes = recipes;
        self
    }
}

impl ThemeHook for Mono {
    /// LVGL `theme_apply()` of `lv_theme_mono.c`: a screen gets the screen styles; any other
    /// node the recipe of the nearest class of its theme-class lineage the theme knows.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if cx.parent().is_none() {
            screen(cx);
        } else if let Some(r) = self.recipe(class) {
            cx.add_recipe(r);
        } else {
            twine_core::trace!(target: "twine::style", "mono theme: no styles for class {}", class.name);
        }
    }

    fn font_normal(&self) -> &'static Font {
        crate::theme::first_font(self.modes.tables(), design::FONT_BODY)
    }

    fn name(&self) -> &'static str {
        "mono"
    }

    fn modes(&self) -> &[ThemeMode] {
        self.modes.modes()
    }

    fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        self.modes.design(mode)
    }

    /// The recipe of the nearest class of `class`'s lineage among the theme's
    /// [`recipes`](Mono::recipes) ([`ALL_RECIPES`] by default).
    fn recipe(&self, class: &'static WidgetClass) -> Option<RecipeRef> {
        crate::recipes::find(self.recipes, class)
    }
}

impl ThemeBase for Mono {
    const THEME: &'static Self = &THEME;
}

impl Theme for Mono {
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
    /// Labels: nothing (LVGL's mono theme gives labels nothing).
    LABEL for LABEL_CLASS => nothing;
    /// Plain objects: tabview pages (lines 300-313), else cards.
    OBJ for OBJ_CLASS => obj;
    /// Tabview pages: square cards.
    TABVIEW_PAGE for TABVIEW_PAGE_CLASS => tab_page;
    /// Window contents (lines 315-329).
    WIN_CONTENT for WIN_CONTENT_CLASS => tab_page;
    /// Cards.
    CARD for CARD_CLASS => card;
    /// Window headers (LVGL compares them with `0`, never true: the plain object look).
    WIN_HEADER for WIN_HEADER_CLASS => card;
    /// Lists (the plain object look).
    LIST for LIST_CLASS => card;
    /// Buttons.
    BUTTON for BUTTON_CLASS => button;
    /// Bars (lines 381-389).
    BAR for BAR_CLASS => bar;
    /// Sliders (lines 390-401).
    SLIDER for SLIDER_CLASS => slider;
    /// Checkboxes (lines 414-424).
    CHECKBOX for CHECKBOX_CLASS => checkbox;
    /// Switches (lines 426-440).
    SWITCH for SWITCH_CLASS => switch;
    /// Arcs (lines 480-490).
    ARC for ARC_CLASS => arc;
    /// Spinners (lines 492-496).
    SPINNER for SPINNER_CLASS => spinner;
    /// Button matrices (lines 344-375).
    BUTTONMATRIX for BUTTONMATRIX_CLASS => buttonmatrix;
    /// Text areas (lines 498-506).
    TEXTAREA for TEXTAREA_CLASS => textarea;
    /// Keyboards (lines 520-530).
    KEYBOARD for KEYBOARD_CLASS => keyboard;
    /// Spin boxes (lines 555-562).
    SPINBOX for SPINBOX_CLASS => spinbox;
    /// Rollers (lines 452-460).
    ROLLER for ROLLER_CLASS => roller;
    /// Dropdowns (lines 462-477).
    DROPDOWN for DROPDOWN_CLASS => dropdown;
    /// Dropdown lists.
    DROPDOWN_LIST for DROPDOWN_LIST_CLASS => dropdown_list;
    /// List buttons (lines 532-546).
    LIST_BUTTON for LIST_BUTTON_CLASS => list_button;
    /// Tile views (lines 563-571).
    TILEVIEW for TILEVIEW_CLASS => tileview;
    /// Tiles.
    TILEVIEW_TILE for TILEVIEW_TILE_CLASS => tile;
    /// Message boxes (lines 548-553).
    MSGBOX for MSGBOX_CLASS => plain_card;
    /// LEDs (lines 573-577).
    LED for LED_CLASS => plain_card;
}

/// Every per-class recipe of the mono theme (the order the lineage lookup scans).
pub const ALL_RECIPES: &[&ComputedRecipe] = &[
    &LABEL,
    &OBJ,
    &TABVIEW_PAGE,
    &WIN_CONTENT,
    &CARD,
    &WIN_HEADER,
    &LIST,
    &BUTTON,
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
    &LIST_BUTTON,
    &TILEVIEW,
    &TILEVIEW_TILE,
    &MSGBOX,
    &LED,
];

fn nothing(_: &mut ThemeCx<'_>) {}

fn obj(cx: &mut ThemeCx<'_>) {
    if cx.parent().is_none() {
        screen(cx);
    } else {
        card(cx);
    }
}

/// Tabview pages and window contents: a square card.
fn tab_page(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::NO_RADIUS);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn card(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn plain_card(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
}

fn button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::state(State::PRESSED), &s::PR);
    cx.add_style(Selector::state(State::CHECKED), &s::INV);
    cx.add_style(Selector::state(State::DISABLED), &s::DISABLED);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn bar(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_ZERO);
    cx.add_style(Selector::part(Part::Indicator), &s::INV);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
}

fn slider(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::PAD_ZERO);
    cx.add_style(Selector::part(Part::Indicator), &s::INV);
    cx.add_style(Selector::part(Part::Knob), &s::CARD);
    cx.add_style(Selector::part(Part::Knob), &s::RADIUS_CIRCLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn checkbox(cx: &mut ThemeCx<'_>) {
    let ind = Selector::part(Part::Indicator);
    cx.add_style(Selector::MAIN, &s::PAD_GAP);
    cx.add_style(ind, &s::CARD);
    cx.add_style(ind.with_state(State::DISABLED), &s::DISABLED);
    cx.add_style(ind.with_state(State::CHECKED), &s::INV);
    // Twine addition: a check mark in the inverted box (crisp on 1-bit panels).
    cx.add_style(ind.with_state(State::CHECKED), &s::CB_MARKER_CHECKED);
    cx.add_style(ind.with_state(State::PRESSED), &s::PR);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn switch(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::RADIUS_CIRCLE);
    cx.add_style(Selector::MAIN, &s::PAD_ZERO);
    cx.add_style(Selector::part(Part::Indicator), &s::INV);
    cx.add_style(Selector::part(Part::Indicator), &s::RADIUS_CIRCLE);
    cx.add_style(Selector::part(Part::Knob), &s::CARD);
    cx.add_style(Selector::part(Part::Knob), &s::RADIUS_CIRCLE);
    cx.add_style(Selector::part(Part::Knob), &s::PAD_ZERO);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn arc(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::part(Part::Indicator), &s::INV);
    cx.add_style(Selector::part(Part::Indicator), &s::PAD_ZERO);
    cx.add_style(Selector::part(Part::Knob), &s::CARD);
    cx.add_style(Selector::part(Part::Knob), &s::RADIUS_CIRCLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn spinner(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::part(Part::Indicator), &s::SPINNER_INDIC);
}

fn buttonmatrix(cx: &mut ThemeCx<'_>) {
    let items = Selector::part(Part::Items);
    if cx.parent_class().is_some_and(|c| c.is_a(&MSGBOX_CLASS)) {
        // Lines 346-356.
        cx.add_style(Selector::MAIN, &s::PAD_GAP);
        cx.add_style(items, &s::CARD);
        cx.add_style(items.with_state(State::PRESSED), &s::PR);
        cx.add_style(items.with_state(State::DISABLED), &s::DISABLED);
    } else if cx.parent_class().is_some_and(|c| c.is_a(&TABVIEW_CLASS)) {
        // Lines 357-368.
        cx.add_style(Selector::MAIN, &s::PAD_GAP);
        cx.add_style(items, &s::CARD);
        cx.add_style(items.with_state(State::PRESSED), &s::PR);
        cx.add_style(items.with_state(State::CHECKED), &s::INV);
        cx.add_style(items.with_state(State::DISABLED), &s::DISABLED);
        cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    } else {
        cx.add_style(Selector::MAIN, &s::CARD);
        cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
        cx.add_style(items, &s::CARD);
        cx.add_style(items.with_state(State::PRESSED), &s::PR);
        cx.add_style(items.with_state(State::CHECKED), &s::INV);
        cx.add_style(items.with_state(State::DISABLED), &s::DISABLED);
    }
    cx.add_style(items.with_state(State::FOCUS_KEY), &s::UNDERLINE);
    cx.add_style(items.with_state(State::FOCUS_KEY), &s::LARGE_BORDER);
}

fn textarea(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(
        Selector::part(Part::Cursor).with_state(State::FOCUSED),
        &s::TA_CURSOR,
    );
    cx.add_style(Selector::state(State::FOCUSED), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn keyboard(cx: &mut ThemeCx<'_>) {
    let items = Selector::part(Part::Items);
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(items, &s::CARD);
    cx.add_style(items.with_state(State::PRESSED), &s::PR);
    cx.add_style(items.with_state(State::CHECKED), &s::INV);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
    cx.add_style(items.with_state(State::EDITED), &s::LARGE_BORDER);
}

fn spinbox(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::part(Part::Cursor), &s::INV);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn roller(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::LARGE_LINE_SPACE);
    cx.add_style(Selector::part(Part::Selected), &s::INV);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn dropdown(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::state(State::PRESSED), &s::PR);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn dropdown_list(cx: &mut ThemeCx<'_>) {
    let sel = Selector::part(Part::Selected);
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::MAIN, &s::LARGE_LINE_SPACE);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
    cx.add_style(sel.with_state(State::CHECKED), &s::INV);
    cx.add_style(sel.with_state(State::PRESSED), &s::PR);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::EDIT);
}

fn list_button(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::CARD);
    cx.add_style(Selector::state(State::PRESSED), &s::PR);
    cx.add_style(Selector::state(State::FOCUS_KEY), &s::FOCUS);
    cx.add_style(Selector::state(State::EDITED), &s::LARGE_BORDER);
}

fn tileview(cx: &mut ThemeCx<'_>) {
    cx.add_style(Selector::MAIN, &s::SCR);
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}

fn tile(cx: &mut ThemeCx<'_>) {
    cx.add_style(SCROLLBAR, &s::SCROLLBAR);
}
