//! [`SimpleTheme`]: LVGL's simple theme (`src/themes/simple/lv_theme_simple.c`).
//!
//! Its colors are [design elements](twine_style::design) (`COLOR_WHITE` = `SURFACE`,
//! `COLOR_LIGHT` = `SURFACE_VARIANT`, `COLOR_DARK` = `PRIMARY`, `COLOR_DIM` = `ON_SURFACE`,
//! `COLOR_SCR` = `BACKGROUND`), so an application can recolor it with
//! [`SimpleTheme::element`].
//!
//! It has two modes: [`ThemeMode::Light`] (LVGL's look) and [`ThemeMode::HighContrast`]
//! (black text on white, black knobs and scrollbars, darker status colors). LVGL's simple
//! theme is a light theme without borders or outlines, so it has no dark or night variant:
//! [`ThemeMode::Dark`] and [`ThemeMode::Night`] are refused by
//! [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode) (the display keeps its
//! mode, with a warning). Its styles put the text color `ON_SURFACE` (not `ON_PRIMARY`) on
//! `PRIMARY` buttons, so the high-contrast `PRIMARY` is a mid grey that black text reads on
//! at 7.8:1; for a high-contrast UI with clearly outlined controls prefer
//! [`DefaultTheme`](crate::DefaultTheme).

use alloc::rc::Rc;

use twine_core::{Color, Duration, Opa, Size};
use twine_engine::{OBJ_CLASS, ThemeCx, ThemeHook, WidgetClass};
use twine_style::design::{self, ColorValue, Element, ElementTable, ElementType};
use twine_style::{BorderSide, Length, Part, Radius, Selector, State, StyleBuf, ThemeMode};
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
use twine_widgets_ext::tabview::{TABVIEW_CLASS, TABVIEW_CONTENT_CLASS, TABVIEW_TAB_BAR_CLASS};
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::class::{ClassCx, ClassStyles, common_builder_methods};
use crate::design::DesignTables;
use crate::{Palette, Theme, Tone};

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
/// `SCROLLBAR_WIDTH`.
const SCROLLBAR_WIDTH: i32 = 2;

/// The styles of `lv_theme_simple.c` (`my_theme_styles_t`).
#[derive(Debug)]
pub struct SimpleStyles {
    /// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the theme font.
    pub scr: Rc<StyleBuf>,
    /// A transparent background.
    pub transp: Rc<StyleBuf>,
    /// Opaque `SURFACE` background, 1 px lines and 2 px arcs in the same color (LVGL `COLOR_WHITE`).
    pub white: Rc<StyleBuf>,
    /// Like `white` in `SURFACE_VARIANT` (LVGL `COLOR_LIGHT`: buttons, tracks).
    pub light: Rc<StyleBuf>,
    /// Like `white` in `PRIMARY` (LVGL `COLOR_DARK`: checked states, indicators).
    pub dark: Rc<StyleBuf>,
    /// Like `white` in `ON_SURFACE` (LVGL `COLOR_DIM`).
    pub dim: Rc<StyleBuf>,
    /// Scrollbars: opaque `SCROLLBAR` color, 2 px wide.
    pub scrollbar: Rc<StyleBuf>,
    /// Arcs: 6 px wide.
    pub arc_line: Rc<StyleBuf>,
    /// Arc knobs: 5 px padding (a knob larger than the arc).
    pub arc_knob: Rc<StyleBuf>,
    /// The text area cursor: a 2 px `ON_SURFACE` left border, transparent, blinking every 500 ms.
    pub ta_cursor: Rc<StyleBuf>,
}

/// The modes of the simple theme.
const MODES: [ThemeMode; 2] = [ThemeMode::Light, ThemeMode::HighContrast];

/// The simple theme's design elements in `mode` (light or high contrast).
fn elements(mode: ThemeMode, font: &'static Font) -> ElementTable {
    let t = if mode == ThemeMode::HighContrast {
        ElementTable::new()
            .with(design::BACKGROUND, Color::WHITE)
            .with(design::SURFACE, Color::WHITE)
            .with(design::ON_SURFACE, Color::BLACK)
            .with(design::ON_SURFACE_MUTED, Palette::Grey.tone(Tone::D4))
            .with(design::SURFACE_VARIANT, COLOR_LIGHT)
            .with(design::OUTLINE, Color::BLACK)
            // Buttons and indicators: the styles put `ON_SURFACE` text on them (7.8:1).
            .with(design::PRIMARY, COLOR_DARK)
            .with(design::ON_PRIMARY, Color::BLACK)
            .with(design::SECONDARY, Color::BLACK)
            .with(design::ON_SECONDARY, Color::WHITE)
            .with(design::DANGER, Palette::Red.tone(Tone::D4))
            .with(design::WARNING, Color::hex(0x008A_4B00))
            .with(design::OK, Palette::Green.tone(Tone::D4))
            .with(design::DISABLED, COLOR_DARK)
            .with(design::FOCUS_RING, Color::BLACK)
            .with(design::SCROLLBAR, Color::BLACK)
            .with(design::SHADOW, Color::BLACK)
            .with(design::SCRIM, Color::BLACK)
            .with(design::PLACEHOLDER, Palette::Grey.tone(Tone::D3))
            .with(design::NEUTRAL, Color::BLACK)
    } else {
        ElementTable::new()
            .with(design::BACKGROUND, COLOR_SCR)
            .with(design::SURFACE, COLOR_WHITE)
            .with(design::ON_SURFACE, COLOR_DIM)
            // Readable secondary text (4.6:1; the styles do not use it).
            .with(design::ON_SURFACE_MUTED, Palette::Grey.tone(Tone::D1))
            .with(design::SURFACE_VARIANT, COLOR_LIGHT)
            .with(design::OUTLINE, COLOR_LIGHT)
            .with(design::PRIMARY, COLOR_DARK)
            // Black reads on `COLOR_DARK` (7.8:1), white would not (2.7:1).
            .with(design::ON_PRIMARY, Color::BLACK)
            .with(design::SECONDARY, COLOR_DIM)
            .with(design::ON_SECONDARY, COLOR_WHITE)
            .with(design::DANGER, Palette::Red.main())
            .with(design::WARNING, Palette::Orange.main())
            .with(design::OK, Palette::Green.main())
            .with(design::DISABLED, COLOR_LIGHT)
            .with(design::FOCUS_RING, COLOR_DARK)
            .with(design::SCROLLBAR, COLOR_DARK)
            .with(design::SHADOW, COLOR_DARK)
            .with(design::SCRIM, COLOR_DARK)
            .with(design::PLACEHOLDER, COLOR_DARK)
            .with(design::NEUTRAL, COLOR_DARK)
    };
    t.with(design::SPACE_XS, Length::Px(2))
        .with(design::SPACE_S, Length::Px(4))
        .with(design::SPACE_M, Length::Px(8))
        .with(design::SPACE_L, Length::Px(12))
        .with(design::RADIUS_S, Radius::Px(2))
        .with(design::RADIUS_M, Radius::Px(4))
        .with(design::RADIUS_L, Radius::Px(8))
        .with(design::RADIUS_FULL, Radius::Circle)
        .with(design::FOCUS_RING_OPACITY, Opa::COVER)
        // No shadows in the simple theme.
        .with(design::SHADOW_OPACITY, Opa::TRANSP)
        .with(design::DISABLED_OPACITY, Opa::P50)
        .with(design::SCRIM_OPACITY, Opa::P50)
        .with(design::FONT_SMALL, font)
        .with(design::FONT_BODY, font)
        .with(design::FONT_LARGE, font)
}

/// A flat bg + line + arc style of one color (`white`, `light`, `dark`, `dim` in LVGL).
fn tone(c: impl Into<ColorValue> + Copy) -> StyleBuf {
    StyleBuf::new()
        .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(.., LV_OPA_COVER)
        .bg_color(c) // lv_style_set_bg_color(.., COLOR_x)
        .line_width(1) // lv_style_set_line_width(.., 1)
        .line_color(c) // lv_style_set_line_color(.., COLOR_x)
        .arc_width(2) // lv_style_set_arc_width(.., 2)
        .arc_color(c) // lv_style_set_arc_color(.., COLOR_x)
}

impl SimpleStyles {
    /// LVGL `style_init()` of the simple theme.
    fn new(font: &'static Font) -> Self {
        let scrollbar = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
            .bg_color(design::SCROLLBAR) // lv_style_set_bg_color(&scrollbar, COLOR_DARK)
            .width(SCROLLBAR_WIDTH); // lv_style_set_width(&scrollbar, SCROLLBAR_WIDTH)
        let scr = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
            .bg_color(design::BACKGROUND) // lv_style_set_bg_color(&scr, COLOR_SCR)
            .text_color(design::ON_SURFACE) // lv_style_set_text_color(&scr, COLOR_DIM)
            // LVGL takes the default font from `LV_FONT_DEFAULT` instead of a style.
            .font(font);
        Self {
            scr: Rc::new(scr),
            // lv_style_set_bg_opa(&transp, LV_OPA_TRANSP)
            transp: Rc::new(StyleBuf::new().bg_opacity(Opa::TRANSP)),
            white: Rc::new(tone(design::SURFACE)),
            light: Rc::new(tone(design::SURFACE_VARIANT)),
            dark: Rc::new(tone(design::PRIMARY)),
            dim: Rc::new(tone(design::ON_SURFACE)),
            scrollbar: Rc::new(scrollbar),
            // lv_style_set_arc_width(&arc_line, 6)
            arc_line: Rc::new(StyleBuf::new().arc_width(6)),
            // lv_style_set_pad_all(&arc_knob, 5)
            arc_knob: Rc::new(StyleBuf::new().padding(5)),
            // #if LV_USE_TEXTAREA (lines 131-138)
            ta_cursor: Rc::new(
                StyleBuf::new()
                    .border_side(BorderSide::LEFT) // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
                    .border_color(design::ON_SURFACE) // lv_style_set_border_color(&ta_cursor, COLOR_DIM)
                    .border_width(2) // lv_style_set_border_width(&ta_cursor, 2)
                    .bg_opacity(Opa::TRANSP) // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
                    .anim_duration(Duration::ms(500)), // lv_style_set_anim_duration(&ta_cursor, 500)
            ),
        }
    }
}

/// LVGL's simple theme: a light grey screen, white containers, grey buttons, no shadows,
/// outlines or transitions. Useful as a cheap base for custom themes (give it to another
/// theme's builder as [`parent`](crate::DefaultThemeBuilder::parent), or give it one with
/// [`SimpleThemeBuilder::parent`]).
///
/// ```
/// use twine_style::ThemeMode;
/// use twine_theme::{SimpleTheme, Theme, ThemeHook};
/// let t = SimpleTheme::new();
/// assert!(core::ptr::eq(t.font_small(), &raw const twine_assets::fonts::MONTSERRAT_14));
/// let hc = SimpleTheme::builder().mode(ThemeMode::HighContrast).build();
/// assert_eq!(hc.mode(), ThemeMode::HighContrast);
/// ```
pub struct SimpleTheme {
    mode: ThemeMode,
    font: &'static Font,
    styles: SimpleStyles,
    design: DesignTables<()>,
    classes: ClassStyles<SimpleStyles>,
    parent: Option<Rc<dyn Theme>>,
}

impl core::fmt::Debug for SimpleTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SimpleTheme")
            .field("mode", &self.mode)
            .field("parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

/// The builder of a [`SimpleTheme`] ([`SimpleTheme::builder`]). Defaults:
/// [`ThemeMode::Light`], Montserrat 14 (feature `assets`; without it, set a font), no parent,
/// no application elements or classes. Of the modes only `Light` and `HighContrast` exist: a
/// [`mode`](Self::mode) of `Dark` or `Night` starts the theme in `Light` (with a warning
/// when built).
///
/// ```
/// use twine_core::{Color, Size};
/// use twine_style::{ThemeMode, design};
/// use twine_theme::{SimpleTheme, ThemeHook};
///
/// let t = SimpleTheme::builder().element(design::PRIMARY, Color::hex(0x00897B)).build();
/// let table = t.design(ThemeMode::Light, 130, Size::new(320, 240)).unwrap();
/// assert_eq!(table.get(design::PRIMARY), Some(Color::hex(0x00897B)));
/// assert!(t.design(ThemeMode::Dark, 130, Size::new(320, 240)).is_none()); // light and high contrast only
/// ```
#[must_use]
pub struct SimpleThemeBuilder(SimpleTheme);

impl core::fmt::Debug for SimpleThemeBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SimpleThemeBuilder").field(&self.0).finish()
    }
}

impl SimpleThemeBuilder {
    /// The font of all text.
    pub fn font(mut self, font: &'static Font) -> Self {
        self.0.font = font;
        self.0.styles = SimpleStyles::new(font);
        self
    }

    common_builder_methods!(SimpleStyles, SimpleTheme);

    /// The theme; an unsupported starting mode becomes [`ThemeMode::Light`] (logged). Never
    /// panics.
    #[must_use]
    pub fn build(mut self) -> SimpleTheme {
        crate::theme::check_font("simple", self.0.font);
        if !MODES.contains(&self.0.mode) {
            twine_core::warn!(target: "twine::style", "simple theme has no {:?} mode, starting in Light", self.0.mode);
            self.0.mode = ThemeMode::Light;
        }
        self.0
    }
}

impl SimpleTheme {
    /// The simple theme with the Montserrat 14 font, light: `SimpleTheme::builder().build()`.
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// A builder (see [`SimpleThemeBuilder`]; LVGL `lv_theme_simple_init`).
    #[doc(alias = "lv_theme_simple_init")]
    pub fn builder() -> SimpleThemeBuilder {
        let font = crate::theme::default_font();
        SimpleThemeBuilder(SimpleTheme {
            mode: ThemeMode::Light,
            font,
            styles: SimpleStyles::new(font),
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

    /// The theme's styles (LVGL's names: `white`, `light`, `dark`, `dim`, `scrollbar`, …),
    /// for custom widgets that should look native (see also [`SimpleThemeBuilder::class`]).
    ///
    /// ```
    /// use twine_style::{PropId, StyleValue};
    /// use twine_theme::SimpleTheme;
    /// let t = SimpleTheme::new();
    /// assert!(matches!(t.styles().white.get(PropId::BgColor), Some(StyleValue::Element(_))));
    /// ```
    #[must_use]
    pub fn styles(&self) -> &SimpleStyles {
        &self.styles
    }

    /// Adds the styles of `class` if the theme knows it (LVGL `theme_apply()`'s branch);
    /// `false` for a class it does not know.
    #[allow(clippy::too_many_lines)] // one branch per LVGL class
    fn style_class(cx: &mut ThemeCx<'_>, class: &'static WidgetClass, s: &SimpleStyles) -> bool {
        let scrollbar = Selector::part(Part::Scrollbar);
        if class.is(&LABEL_CLASS) {
            // LVGL's simple theme gives labels nothing.
        } else if class.is(&OBJ_CLASS) && cx.grandparent_class().is_some_and(|c| c.is_a(&TABVIEW_CLASS)) {
            // Lines 234-247: tabview pages (the bar and the content have their own classes).
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&TABVIEW_TAB_BAR_CLASS) || class.is(&TABVIEW_CONTENT_CLASS) {
            cx.add_style(Selector::MAIN, s.scr.clone());
        } else if class.is(&OBJ_CLASS) || class.is(&CARD_CLASS) {
            cx.add_style(Selector::MAIN, s.white.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&BUTTON_CLASS) || class.is(&LIST_BUTTON_CLASS) {
            // Lines 398-409 (list buttons).
            cx.add_style(Selector::MAIN, s.dark.clone());
        } else if class.is(&BAR_CLASS) {
            // Lines 290-295.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
        } else if class.is(&SLIDER_CLASS) {
            // Lines 297-303.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
            cx.add_style(Selector::part(Part::Knob), s.dim.clone());
        } else if class.is(&CHECKBOX_CLASS) {
            // Lines 312-317.
            let ind = Selector::part(Part::Indicator);
            cx.add_style(ind, s.light.clone());
            cx.add_style(ind.with_state(State::CHECKED), s.dark.clone());
        } else if class.is(&SWITCH_CLASS) {
            // Lines 319-324.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::part(Part::Knob), s.dim.clone());
        } else if class.is(&ARC_CLASS) {
            // Lines 354-364.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::MAIN, s.transp.clone());
            cx.add_style(Selector::MAIN, s.arc_line.clone());
            cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_line.clone());
            cx.add_style(Selector::part(Part::Knob), s.dim.clone());
            cx.add_style(Selector::part(Part::Knob), s.arc_knob.clone());
        } else if class.is(&SPINNER_CLASS) {
            // Lines 366-374.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::MAIN, s.transp.clone());
            cx.add_style(Selector::MAIN, s.arc_line.clone());
            cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_line.clone());
        } else if class.is(&BUTTONMATRIX_CLASS) {
            // Lines 271-288.
            let in_box = cx
                .parent_class()
                .is_some_and(|c| c.is_a(&MSGBOX_CLASS) || c.is_a(&TABVIEW_CLASS));
            if !in_box {
                cx.add_style(Selector::MAIN, s.white.clone());
            }
            cx.add_style(Selector::part(Part::Items), s.light.clone());
        } else if class.is(&TEXTAREA_CLASS) {
            // Lines 376-382.
            cx.add_style(Selector::MAIN, s.white.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(
                Selector::part(Part::Cursor).with_state(State::FOCUSED),
                s.ta_cursor.clone(),
            );
        } else if class.is(&KEYBOARD_CLASS) {
            // Lines 390-396.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::part(Part::Items), s.white.clone());
            cx.add_style(
                Selector::part(Part::Items).with_state(State::CHECKED),
                s.light.clone(),
            );
        } else if class.is(&SPINBOX_CLASS) {
            // Lines 418-423.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::part(Part::Cursor), s.dark.clone());
        } else if class.is(&ROLLER_CLASS) {
            // Lines 335-340.
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(Selector::part(Part::Selected), s.dark.clone());
        } else if class.is(&DROPDOWN_CLASS) {
            // Lines 342-352.
            cx.add_style(Selector::MAIN, s.white.clone());
        } else if class.is(&DROPDOWN_LIST_CLASS) {
            cx.add_style(Selector::MAIN, s.white.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(Selector::part(Part::Selected), s.light.clone());
            cx.add_style(
                Selector::part(Part::Selected).with_state(State::CHECKED),
                s.dark.clone(),
            );
        } else if class.is(&LIST_CLASS) || class.is(&WIN_CONTENT_CLASS) {
            // Lines 398-409 (list) and 249-260 (a window's content).
            cx.add_style(Selector::MAIN, s.light.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&MSGBOX_CLASS) || class.is(&WIN_HEADER_CLASS) || class.is(&LED_CLASS) {
            // Lines 411-416 (msgbox), 249-260 (a window's header) and 434-437 (led).
            cx.add_style(Selector::MAIN, s.light.clone());
        } else if class.is(&TILEVIEW_CLASS) {
            // Lines 424-432.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else if class.is(&TILEVIEW_TILE_CLASS) {
            cx.add_style(scrollbar, s.scrollbar.clone());
        } else {
            // LVGL's simple theme has no line, image, image button, animated image, span group
            // or list text branch.
            return false;
        }
        true
    }
}

#[cfg(feature = "assets")]
impl Default for SimpleTheme {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeHook for SimpleTheme {
    /// LVGL `theme_apply()` of `lv_theme_simple.c`: the parent theme first, then the styles of
    /// the nearest class of `class`'s [lineage](WidgetClass::lineage) the theme knows, then
    /// the application's [class registrations](SimpleThemeBuilder::class).
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if let Some(p) = &self.parent {
            p.apply(cx, class);
        }
        let s = &self.styles;
        if cx.parent().is_none() {
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
        } else if !class.lineage().any(|c| Self::style_class(cx, c, s)) {
            twine_core::trace!(target: "twine::style", "simple theme: no styles for class {}", class.name);
        }
        self.classes.apply(cx, class, s);
    }

    fn font_normal(&self) -> &'static Font {
        self.font
    }

    fn name(&self) -> &'static str {
        "simple"
    }

    fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// [`ThemeMode::Light`] and [`ThemeMode::HighContrast`].
    fn modes(&self) -> &'static [ThemeMode] {
        &MODES
    }

    /// Light and high contrast (`None` for dark and night).
    fn design(&self, mode: ThemeMode, dpi: u16, resolution: Size) -> Option<Rc<ElementTable>> {
        MODES.contains(&mode).then(|| {
            self.design.get((), mode, || {
                let mut t = self
                    .parent
                    .as_ref()
                    .and_then(|parent| parent.design(mode, dpi, resolution))
                    .map_or_else(ElementTable::new, |t| (*t).clone());
                t.overlay(&elements(mode, self.font));
                t
            })
        })
    }
}

impl Theme for SimpleTheme {
    fn font_small(&self) -> &'static Font {
        self.font
    }
    fn font_large(&self) -> &'static Font {
        self.font
    }
}
