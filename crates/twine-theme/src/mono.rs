//! [`MonoTheme`]: LVGL's monochrome theme (`src/themes/mono/lv_theme_mono.c`) for 1-bit
//! displays. Every color is pure black or pure white, so `I1` output is exact.
//!
//! The foreground and background colors are [design elements](twine_style::design)
//! (`ON_SURFACE` and `SURFACE` / `BACKGROUND`), so the theme supports every mode, switched at
//! run time with [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode):
//! [`ThemeMode::Light`] (black on white) and [`ThemeMode::Dark`] (white on black, LVGL's
//! `dark_bg`); a 1-bit panel can only invert, so [`ThemeMode::Night`] is white on black (the
//! least lit pixels; dimming is the backlight's job) and [`ThemeMode::HighContrast`] black on
//! white (every pair is 21:1 in every mode). [`MonoThemeBuilder::mode`] starts it in any of
//! them.

use alloc::rc::Rc;

use twine_core::{Color, Duration, Opa, Size};
use twine_engine::{OBJ_CLASS, ThemeCx, ThemeHook, WidgetClass};
use twine_style::design::{self, Element, ElementTable, ElementType};
use twine_style::{BorderSide, Length, Part, Radius, Selector, State, StyleBuf, TextDecor, ThemeMode};
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
use twine_widgets_ext::tabview::TABVIEW_CLASS;
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::Theme;
use crate::class::{ClassCx, ClassStyles, common_builder_methods};
use crate::design::DesignTables;

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
/// `PAD_DEF`.
const PAD_DEF: i32 = 4;
/// `SPINNER_WIDTH`.
const SPINNER_WIDTH: i32 = 8;

/// The styles of `lv_theme_mono.c` used by the widgets implemented so far.
// NOTE(P20.S05): the chart styles come with the chart.
#[derive(Debug)]
pub struct MonoStyles {
    /// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the theme font, 4 px gaps.
    pub scr: Rc<StyleBuf>,
    /// Cards and most widgets: opaque `SURFACE`, a 1 px `ON_SURFACE` border, radius 2, 4 px padding
    /// and gaps, 2 px `ON_SURFACE` lines and arcs, a 300 ms animation duration.
    pub card: Rc<StyleBuf>,
    /// Scrollbars: opaque `SCROLLBAR` color, 4 px wide.
    pub scrollbar: Rc<StyleBuf>,
    /// The pressed look: a 3 px border, the padding reduced so the content stays in place.
    pub pr: Rc<StyleBuf>,
    /// Inverted colors (`PRIMARY` background, `ON_PRIMARY` text, border, lines, arcs and outline):
    /// checked states and indicators.
    pub inv: Rc<StyleBuf>,
    /// The disabled look: no border, the padding increased so the content stays in place.
    pub disabled: Rc<StyleBuf>,
    /// The keyboard focus look: a 1 px outline, 1 px off the node.
    pub focus: Rc<StyleBuf>,
    /// The editing look: a 2 px outline.
    pub edit: Rc<StyleBuf>,
    /// A 2 px border.
    pub large_border: Rc<StyleBuf>,
    /// Gaps of 4 px.
    pub pad_gap: Rc<StyleBuf>,
    /// No padding and no gaps.
    pub pad_zero: Rc<StyleBuf>,
    /// Square corners (radius 0).
    pub no_radius: Rc<StyleBuf>,
    /// Fully round corners (`Radius::Circle`).
    pub radius_circle: Rc<StyleBuf>,
    /// Text line spacing of 6 px.
    pub large_line_space: Rc<StyleBuf>,
    /// Underlined text.
    pub underline: Rc<StyleBuf>,
    /// Spinner indicators: an 8 px rounded `ON_SURFACE` arc.
    pub spinner_indic: Rc<StyleBuf>,
    /// The text area cursor: a 2 px `ON_SURFACE` left border, transparent, blinking every 500 ms.
    pub ta_cursor: Rc<StyleBuf>,
    /// Not in LVGL: the check mark of a checked checkbox (LVGL's mono theme only inverts the
    /// box).
    pub cb_marker_checked: Rc<StyleBuf>,
}

/// `EXPAND_BORDER(style, state)`: a thicker border that keeps the content in place.
fn expand_border(w: i32) -> StyleBuf {
    // lv_style_set_border_width(&style, BORDER_W_x)
    // lv_style_set_pad_all(&style, PAD_DEF + BORDER_W_NORMAL - BORDER_W_x)
    StyleBuf::new()
        .border_width(w)
        .padding(PAD_DEF + BORDER_W_NORMAL - w)
}

/// The mono theme's design elements in `mode`: `COLOR_FG` / `COLOR_BG` of LVGL (black and
/// white, swapped in dark mode) for every color, `PAD_DEF`-based spacings, no shadows.
/// A 1-bit panel can only invert: `Night` is white on black like `Dark` (dimming is the
/// backlight's job), `HighContrast` black on white like `Light` (both already 21:1).
fn elements(mode: ThemeMode, font: &'static Font) -> ElementTable {
    // #define COLOR_FG dark_bg ? lv_color_white() : lv_color_black()
    // #define COLOR_BG dark_bg ? lv_color_black() : lv_color_white()
    let (fg, bg) = match mode {
        ThemeMode::Light | ThemeMode::HighContrast => (Color::BLACK, Color::WHITE),
        ThemeMode::Dark | ThemeMode::Night => (Color::WHITE, Color::BLACK),
    };
    ElementTable::new()
        .with(design::BACKGROUND, bg)
        .with(design::SURFACE, bg)
        .with(design::ON_SURFACE, fg)
        .with(design::ON_SURFACE_MUTED, fg)
        .with(design::SURFACE_VARIANT, bg)
        .with(design::OUTLINE, fg)
        .with(design::PRIMARY, fg)
        .with(design::ON_PRIMARY, bg)
        .with(design::SECONDARY, bg)
        .with(design::ON_SECONDARY, fg)
        .with(design::DANGER, fg)
        .with(design::WARNING, fg)
        .with(design::OK, fg)
        .with(design::DISABLED, bg)
        .with(design::FOCUS_RING, fg)
        .with(design::SCROLLBAR, fg)
        .with(design::SHADOW, fg)
        .with(design::SCRIM, bg)
        .with(design::PLACEHOLDER, fg)
        .with(design::NEUTRAL, fg)
        .with(design::SPACE_XS, Length::Px(1))
        .with(design::SPACE_S, Length::Px(PAD_DEF / 2))
        .with(design::SPACE_M, Length::Px(PAD_DEF))
        .with(design::SPACE_L, Length::Px(PAD_DEF * 2))
        .with(design::RADIUS_S, Radius::Px(1))
        .with(design::RADIUS_M, Radius::Px(2))
        .with(design::RADIUS_L, Radius::Px(4))
        .with(design::RADIUS_FULL, Radius::Circle)
        .with(design::FOCUS_RING_OPACITY, Opa::COVER)
        // A 1-bit display shows no partial opacity: no shadows, no disabled overlay, an
        // opaque backdrop.
        .with(design::SHADOW_OPACITY, Opa::TRANSP)
        .with(design::DISABLED_OPACITY, Opa::TRANSP)
        .with(design::SCRIM_OPACITY, Opa::COVER)
        .with(design::FONT_SMALL, font)
        .with(design::FONT_BODY, font)
        .with(design::FONT_LARGE, font)
}

impl MonoStyles {
    /// LVGL `style_init(theme, dark_bg, font)` of the mono theme, with `COLOR_FG` = the
    /// `ON_SURFACE` element and `COLOR_BG` = `SURFACE` (`BACKGROUND` on screens).
    fn new(font: &'static Font) -> Self {
        let fg = design::ON_SURFACE;
        let bg = design::SURFACE;
        let scrollbar = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
            .bg_color(design::SCROLLBAR) // lv_style_set_bg_color(&scrollbar, COLOR_FG)
            .width(PAD_DEF); // lv_style_set_width(&scrollbar, PAD_DEF)
        let scr = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
            .bg_color(design::BACKGROUND) // lv_style_set_bg_color(&scr, COLOR_BG)
            .text_color(fg) // lv_style_set_text_color(&scr, COLOR_FG)
            .gap(PAD_DEF) // lv_style_set_pad_row / pad_column(&scr, PAD_DEF)
            .font(font); // lv_style_set_text_font(&scr, font)
        let card = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&card, LV_OPA_COVER)
            .bg_color(bg) // lv_style_set_bg_color(&card, COLOR_BG)
            .border_color(fg) // lv_style_set_border_color(&card, COLOR_FG)
            .radius(2) // lv_style_set_radius(&card, 2)
            .border_width(BORDER_W_NORMAL) // lv_style_set_border_width(&card, BORDER_W_NORMAL)
            .padding(PAD_DEF) // lv_style_set_pad_all(&card, PAD_DEF)
            .gap(PAD_DEF) // lv_style_set_pad_gap(&card, PAD_DEF)
            .text_color(fg) // lv_style_set_text_color(&card, COLOR_FG)
            .line_width(2) // lv_style_set_line_width(&card, 2)
            .line_color(fg) // lv_style_set_line_color(&card, COLOR_FG)
            .arc_width(2) // lv_style_set_arc_width(&card, 2)
            .arc_color(fg) // lv_style_set_arc_color(&card, COLOR_FG)
            .outline_color(design::FOCUS_RING) // lv_style_set_outline_color(&card, COLOR_FG)
            .anim_duration(Duration::ms(300)); // lv_style_set_anim_duration(&card, 300)
        let inv = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&inv, LV_OPA_COVER)
            .bg_color(design::PRIMARY) // lv_style_set_bg_color(&inv, COLOR_FG)
            .border_color(design::ON_PRIMARY) // lv_style_set_border_color(&inv, COLOR_BG)
            .line_color(design::ON_PRIMARY) // lv_style_set_line_color(&inv, COLOR_BG)
            .arc_color(design::ON_PRIMARY) // lv_style_set_arc_color(&inv, COLOR_BG)
            .text_color(design::ON_PRIMARY) // lv_style_set_text_color(&inv, COLOR_BG)
            .outline_color(design::ON_PRIMARY); // lv_style_set_outline_color(&inv, COLOR_BG)
        let focus = StyleBuf::new()
            .outline_width(1) // lv_style_set_outline_width(&focus, 1)
            .outline_offset(BORDER_W_FOCUS); // lv_style_set_outline_pad(&focus, BORDER_W_FOCUS)
        Self {
            scr: Rc::new(scr),
            card: Rc::new(card),
            scrollbar: Rc::new(scrollbar),
            pr: Rc::new(expand_border(BORDER_W_PR)), // EXPAND_BORDER(pr, PR)
            inv: Rc::new(inv),
            disabled: Rc::new(expand_border(BORDER_W_DIS)), // EXPAND_BORDER(disabled, DIS)
            focus: Rc::new(focus),
            // lv_style_set_outline_width(&edit, BORDER_W_EDIT)
            edit: Rc::new(StyleBuf::new().outline_width(BORDER_W_EDIT)),
            // lv_style_set_border_width(&large_border, BORDER_W_EDIT)
            large_border: Rc::new(StyleBuf::new().border_width(BORDER_W_EDIT)),
            // lv_style_set_pad_gap(&pad_gap, PAD_DEF)
            pad_gap: Rc::new(StyleBuf::new().gap(PAD_DEF)),
            // lv_style_set_pad_all / pad_gap(&pad_zero, 0)
            pad_zero: Rc::new(StyleBuf::new().padding(0).gap(0)),
            // lv_style_set_radius(&no_radius, 0)
            no_radius: Rc::new(StyleBuf::new().radius(0)),
            // lv_style_set_radius(&radius_circle, LV_RADIUS_CIRCLE)
            radius_circle: Rc::new(StyleBuf::new().radius(Radius::Circle)),
            // lv_style_set_text_line_space(&large_line_space, 6)
            large_line_space: Rc::new(StyleBuf::new().line_spacing(6)),
            // lv_style_set_text_decor(&underline, LV_TEXT_DECOR_UNDERLINE)
            underline: Rc::new(StyleBuf::new().text_decoration(TextDecor::UNDERLINE)),
            spinner_indic: Rc::new(
                StyleBuf::new()
                    .arc_color(fg) // lv_style_set_arc_color(&spinner_indic, COLOR_FG)
                    .arc_width(SPINNER_WIDTH) // lv_style_set_arc_width(&spinner_indic, SPINNER_WIDTH)
                    .arc_rounded(true), // lv_style_set_arc_rounded(&spinner_indic, true)
            ),
            // #if LV_USE_TEXTAREA (lines 187-194)
            ta_cursor: Rc::new(
                StyleBuf::new()
                    .border_side(BorderSide::LEFT) // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
                    .border_color(fg) // lv_style_set_border_color(&ta_cursor, COLOR_FG)
                    .border_width(2) // lv_style_set_border_width(&ta_cursor, 2)
                    .bg_opacity(Opa::TRANSP) // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
                    .anim_duration(Duration::ms(500)), // lv_style_set_anim_duration(&ta_cursor, 500)
            ),
            cb_marker_checked: Rc::new(StyleBuf::new().bg_image(&crate::default::styles::CHECK_MARK)),
        }
    }
}

/// LVGL's monochrome theme: black on white (or white on black), 1-px borders, pressed
/// buttons get a thicker border, checked ones are inverted, focus and edit show outlines.
/// Supports every [`ThemeMode`], switchable at run time and selectable as the starting mode
/// ([`MonoThemeBuilder::mode`]): a 1-bit panel can only invert, so `Night` is white on black
/// like `Dark` and `HighContrast` black on white like `Light`. The application adds or
/// overrides [design elements](twine_style::design) with [`MonoThemeBuilder::element`] /
/// [`MonoThemeBuilder::element_in`] and styles its own classes with
/// [`MonoThemeBuilder::class`].
///
/// ```
/// use twine_core::{Color, Size};
/// use twine_style::{ThemeMode, design};
/// use twine_theme::{MonoTheme, ThemeHook};
/// let t = MonoTheme::builder().mode(ThemeMode::Dark).font(&twine_assets::fonts::MONTSERRAT_14).build();
/// assert_eq!(t.mode(), ThemeMode::Dark);
/// let dark = t.design(ThemeMode::Dark, 130, Size::new(128, 64)).unwrap();
/// assert_eq!(dark.get(design::PRIMARY), Some(Color::WHITE));
/// let light = t.design(ThemeMode::Light, 130, Size::new(128, 64)).unwrap();
/// assert_eq!(light.get(design::ON_SURFACE), Some(Color::BLACK));
/// assert_eq!(light.missing_standard(), None);
/// ```
pub struct MonoTheme {
    mode: ThemeMode,
    font: &'static Font,
    styles: MonoStyles,
    design: DesignTables<()>,
    classes: ClassStyles<MonoStyles>,
    parent: Option<Rc<dyn Theme>>,
}

impl core::fmt::Debug for MonoTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MonoTheme")
            .field("mode", &self.mode)
            .field("parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

/// The builder of a [`MonoTheme`] ([`MonoTheme::builder`]). Defaults: [`ThemeMode::Light`]
/// (black on white), Montserrat 14 (feature `assets`; without it, set a font), no parent, no
/// application elements or classes.
///
/// ```
/// use twine_style::ThemeMode;
/// use twine_theme::{MonoTheme, ThemeHook};
/// for mode in ThemeMode::ALL {
///     assert_eq!(MonoTheme::builder().mode(mode).build().mode(), mode);
/// }
/// ```
#[must_use]
pub struct MonoThemeBuilder(MonoTheme);

impl core::fmt::Debug for MonoThemeBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("MonoThemeBuilder").field(&self.0).finish()
    }
}

impl MonoThemeBuilder {
    /// The font of all text (LVGL's mono theme has one font).
    pub fn font(mut self, font: &'static Font) -> Self {
        self.0.font = font;
        self.0.styles = MonoStyles::new(font);
        self
    }

    common_builder_methods!(MonoStyles, MonoTheme);

    /// The theme. Never panics.
    #[must_use]
    pub fn build(self) -> MonoTheme {
        crate::theme::check_font("mono", self.0.font);
        self.0
    }
}

impl MonoTheme {
    /// A builder (see [`MonoThemeBuilder`]; LVGL `lv_theme_mono_init(disp, dark_bg, font)`
    /// with `dark_bg` = [`mode`](MonoThemeBuilder::mode) `Dark`).
    #[doc(alias = "lv_theme_mono_init")]
    pub fn builder() -> MonoThemeBuilder {
        let font = crate::theme::default_font();
        MonoThemeBuilder(MonoTheme {
            mode: ThemeMode::Light,
            font,
            styles: MonoStyles::new(font),
            design: DesignTables::new(),
            classes: ClassStyles::new(),
            parent: None,
        })
    }

    /// The mode the theme starts in.
    #[must_use]
    pub fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// The theme's styles (LVGL's names: `card`, `pr` (pressed), `inv` (checked), `focus`,
    /// `edit`, `scrollbar`, …), for custom widgets that should look native (see also
    /// [`MonoThemeBuilder::class`]).
    ///
    /// ```
    /// use twine_style::{PropId, StyleValue};
    /// use twine_theme::MonoTheme;
    /// let t = MonoTheme::builder().build();
    /// assert!(matches!(t.styles().card.get(PropId::BorderWidth), Some(StyleValue::Length(_))));
    /// ```
    #[must_use]
    pub fn styles(&self) -> &MonoStyles {
        &self.styles
    }

    /// Adds the styles of `class` if the theme knows it (LVGL `theme_apply()`'s branch);
    /// `false` for a class it does not know.
    #[allow(clippy::too_many_lines)] // one branch per LVGL class
    fn style_class(cx: &mut ThemeCx<'_>, class: &'static WidgetClass, s: &MonoStyles) -> bool {
        let scrollbar = Selector::part(Part::Scrollbar);
        if class.is(&LABEL_CLASS) {
            // LVGL's mono theme gives labels nothing.
        } else if class.is(&OBJ_CLASS) && cx.grandparent_class().is_some_and(|c| c.is_a(&TABVIEW_CLASS)) {
            // Lines 300-313: tabview pages (the bar and the content have no style).
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.no_radius.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&WIN_CONTENT_CLASS) {
            // Lines 315-329 (a window's header: LVGL compares it with `0`, never true, so it
            // gets the plain object styles below).
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.no_radius.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&OBJ_CLASS)
            || class.is(&CARD_CLASS)
            || class.is(&WIN_HEADER_CLASS)
            || class.is(&LIST_CLASS)
        {
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&BUTTON_CLASS) {
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pr.clone());
            cx.add_style(Selector::state(State::CHECKED), s.inv.clone());
            cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&BAR_CLASS) {
            // Lines 381-389.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.pad_zero.clone());
            cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
        } else if class.is(&SLIDER_CLASS) {
            // Lines 390-401.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.pad_zero.clone());
            cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
            cx.add_style(Selector::part(Part::Knob), s.card.clone());
            cx.add_style(Selector::part(Part::Knob), s.radius_circle.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&CHECKBOX_CLASS) {
            // Lines 414-424.
            let ind = Selector::part(Part::Indicator);
            cx.add_style(Selector::MAIN, s.pad_gap.clone());
            cx.add_style(ind, s.card.clone());
            cx.add_style(ind.with_state(State::DISABLED), s.disabled.clone());
            cx.add_style(ind.with_state(State::CHECKED), s.inv.clone());
            // Twine addition: a check mark in the inverted box (crisp on 1-bit panels).
            cx.add_style(ind.with_state(State::CHECKED), s.cb_marker_checked.clone());
            cx.add_style(ind.with_state(State::PRESSED), s.pr.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&SWITCH_CLASS) {
            // Lines 426-440.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.radius_circle.clone());
            cx.add_style(Selector::MAIN, s.pad_zero.clone());
            cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
            cx.add_style(Selector::part(Part::Indicator), s.radius_circle.clone());
            cx.add_style(Selector::part(Part::Knob), s.card.clone());
            cx.add_style(Selector::part(Part::Knob), s.radius_circle.clone());
            cx.add_style(Selector::part(Part::Knob), s.pad_zero.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&ARC_CLASS) {
            // Lines 480-490.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
            cx.add_style(Selector::part(Part::Indicator), s.pad_zero.clone());
            cx.add_style(Selector::part(Part::Knob), s.card.clone());
            cx.add_style(Selector::part(Part::Knob), s.radius_circle.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&SPINNER_CLASS) {
            // Lines 492-496.
            cx.add_style(Selector::part(Part::Indicator), s.spinner_indic.clone());
        } else if class.is(&BUTTONMATRIX_CLASS) {
            // Lines 344-375.
            let items = Selector::part(Part::Items);
            if cx.parent_class().is_some_and(|c| c.is_a(&MSGBOX_CLASS)) {
                // Lines 346-356.
                cx.add_style(Selector::MAIN, s.pad_gap.clone());
                cx.add_style(items, s.card.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
            } else if cx.parent_class().is_some_and(|c| c.is_a(&TABVIEW_CLASS)) {
                // Lines 357-368.
                cx.add_style(Selector::MAIN, s.pad_gap.clone());
                cx.add_style(items, s.card.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
                cx.add_style(items.with_state(State::CHECKED), s.inv.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            } else {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(items, s.card.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
                cx.add_style(items.with_state(State::CHECKED), s.inv.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
            }
            cx.add_style(items.with_state(State::FOCUS_KEY), s.underline.clone());
            cx.add_style(items.with_state(State::FOCUS_KEY), s.large_border.clone());
        } else if class.is(&TEXTAREA_CLASS) {
            // Lines 498-506.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(
                Selector::part(Part::Cursor).with_state(State::FOCUSED),
                s.ta_cursor.clone(),
            );
            cx.add_style(Selector::state(State::FOCUSED), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&KEYBOARD_CLASS) {
            // Lines 520-530.
            let items = Selector::part(Part::Items);
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(items, s.card.clone());
            cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
            cx.add_style(items.with_state(State::CHECKED), s.inv.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            cx.add_style(items.with_state(State::EDITED), s.large_border.clone());
        } else if class.is(&SPINBOX_CLASS) {
            // Lines 555-562.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::part(Part::Cursor), s.inv.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&ROLLER_CLASS) {
            // Lines 452-460.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.large_line_space.clone());
            cx.add_style(Selector::part(Part::Selected), s.inv.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&DROPDOWN_CLASS) {
            // Lines 462-477.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pr.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&DROPDOWN_LIST_CLASS) {
            let sel = Selector::part(Part::Selected);
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.large_line_space.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(sel.with_state(State::CHECKED), s.inv.clone());
            cx.add_style(sel.with_state(State::PRESSED), s.pr.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.edit.clone());
        } else if class.is(&LIST_BUTTON_CLASS) {
            // Lines 532-546 (the list itself is a plain object above).
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pr.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            cx.add_style(Selector::state(State::EDITED), s.large_border.clone());
        } else if class.is(&TILEVIEW_CLASS) {
            // Lines 563-571.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&TILEVIEW_TILE_CLASS) {
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&MSGBOX_CLASS) || class.is(&LED_CLASS) {
            // Lines 548-553 (msgbox) and 573-577 (led).
            cx.add_style(Selector::MAIN, s.card.clone());
        } else {
            // LVGL's mono theme has no line, image, image button, animated image, span group,
            // list text or tabview bar / content branch.
            return false;
        }
        true
    }
}

impl ThemeHook for MonoTheme {
    /// LVGL `theme_apply()` of `lv_theme_mono.c`: the parent theme first, then the styles of
    /// the nearest class of `class`'s [lineage](WidgetClass::lineage) the theme knows, then
    /// the application's [class registrations](MonoThemeBuilder::class).
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if let Some(p) = &self.parent {
            p.apply(cx, class);
        }
        let s = &self.styles;
        if cx.parent().is_none() {
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
        } else if !class.lineage().any(|c| Self::style_class(cx, c, s)) {
            twine_core::trace!(target: "twine::style", "mono theme: no styles for class {}", class.name);
        }
        self.classes.apply(cx, class, s);
    }

    fn font_normal(&self) -> &'static Font {
        self.font
    }

    fn name(&self) -> &'static str {
        "mono"
    }

    fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// Every mode.
    fn modes(&self) -> &'static [ThemeMode] {
        &ThemeMode::ALL
    }

    /// Black on white or white on black (built once per mode).
    fn design(&self, mode: ThemeMode, dpi: u16, resolution: Size) -> Option<Rc<ElementTable>> {
        Some(self.design.get((), mode, || {
            let mut t = self
                .parent
                .as_ref()
                .and_then(|parent| parent.design(mode, dpi, resolution))
                .map_or_else(ElementTable::new, |t| (*t).clone());
            t.overlay(&elements(mode, self.font));
            t
        }))
    }
}

impl Theme for MonoTheme {
    fn font_small(&self) -> &'static Font {
        self.font
    }
    fn font_large(&self) -> &'static Font {
        self.font
    }
}
