//! [`SimpleTheme`]: LVGL's simple theme (`src/themes/simple/lv_theme_simple.c`).

use alloc::rc::Rc;

use twine_core::{Color, Opa};
use twine_engine::{ThemeCx, ThemeHook, WidgetClass};
use twine_style::{BorderSide, Part, Selector, State, StyleBuf};
use twine_text::Font;

use crate::{Palette, Theme};

/// `COLOR_SCR` = `lv_palette_lighten(GREY, 4)`.
const COLOR_SCR: Color = Palette::Grey.lighten(4);
/// `COLOR_WHITE` = white.
const COLOR_WHITE: Color = Color::WHITE;
/// `COLOR_LIGHT` = `lv_palette_lighten(GREY, 2)`.
const COLOR_LIGHT: Color = Palette::Grey.lighten(2);
/// `COLOR_DARK` = `lv_palette_main(GREY)`.
const COLOR_DARK: Color = Palette::Grey.main();
/// `COLOR_DIM` = `lv_palette_darken(GREY, 2)`.
const COLOR_DIM: Color = Palette::Grey.darken(2);
/// `SCROLLBAR_WIDTH`.
const SCROLLBAR_WIDTH: i32 = 2;

/// The styles of `lv_theme_simple.c` (`my_theme_styles_t`).
#[derive(Debug)]
#[allow(missing_docs)] // field names are LVGL's style names
pub struct SimpleStyles {
    pub scr: Rc<StyleBuf>,
    pub transp: Rc<StyleBuf>,
    pub white: Rc<StyleBuf>,
    pub light: Rc<StyleBuf>,
    pub dark: Rc<StyleBuf>,
    pub dim: Rc<StyleBuf>,
    pub scrollbar: Rc<StyleBuf>,
    pub arc_line: Rc<StyleBuf>,
    pub arc_knob: Rc<StyleBuf>,
    pub ta_cursor: Rc<StyleBuf>,
}

/// A flat bg + line + arc style of one color (`white`, `light`, `dark`, `dim` in LVGL).
fn tone(c: Color) -> StyleBuf {
    StyleBuf::new()
        .bg_opa(Opa::COVER) // lv_style_set_bg_opa(.., LV_OPA_COVER)
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
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
            .bg_color(COLOR_DARK) // lv_style_set_bg_color(&scrollbar, COLOR_DARK)
            .width(SCROLLBAR_WIDTH); // lv_style_set_width(&scrollbar, SCROLLBAR_WIDTH)
        let scr = StyleBuf::new()
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
            .bg_color(COLOR_SCR) // lv_style_set_bg_color(&scr, COLOR_SCR)
            .text_color(COLOR_DIM) // lv_style_set_text_color(&scr, COLOR_DIM)
            // LVGL takes the default font from `LV_FONT_DEFAULT` instead of a style.
            .text_font(font);
        Self {
            scr: Rc::new(scr),
            // lv_style_set_bg_opa(&transp, LV_OPA_TRANSP)
            transp: Rc::new(StyleBuf::new().bg_opa(Opa::TRANSP)),
            white: Rc::new(tone(COLOR_WHITE)),
            light: Rc::new(tone(COLOR_LIGHT)),
            dark: Rc::new(tone(COLOR_DARK)),
            dim: Rc::new(tone(COLOR_DIM)),
            scrollbar: Rc::new(scrollbar),
            // lv_style_set_arc_width(&arc_line, 6)
            arc_line: Rc::new(StyleBuf::new().arc_width(6)),
            // lv_style_set_pad_all(&arc_knob, 5)
            arc_knob: Rc::new(StyleBuf::new().pad_top(5).pad_bottom(5).pad_left(5).pad_right(5)),
            // #if LV_USE_TEXTAREA (lines 131-138)
            ta_cursor: Rc::new(
                StyleBuf::new()
                    .border_side(BorderSide::LEFT) // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
                    .border_color(COLOR_DIM) // lv_style_set_border_color(&ta_cursor, COLOR_DIM)
                    .border_width(2) // lv_style_set_border_width(&ta_cursor, 2)
                    .bg_opa(Opa::TRANSP) // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
                    .anim_duration(500u32), // lv_style_set_anim_duration(&ta_cursor, 500)
            ),
        }
    }
}

/// LVGL's simple theme: a light grey screen, white containers, grey buttons, no shadows,
/// outlines or transitions. Useful as a cheap base for custom themes (chain it with
/// [`with_parent`](Self::with_parent) of another theme or give it a parent).
///
/// ```
/// use twine_theme::{SimpleTheme, Theme};
/// let t = SimpleTheme::new();
/// assert!(core::ptr::eq(t.font_small(), &raw const twine_assets::fonts::MONTSERRAT_14));
/// ```
pub struct SimpleTheme {
    font: &'static Font,
    styles: SimpleStyles,
    parent: Option<Rc<dyn Theme>>,
}

impl core::fmt::Debug for SimpleTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SimpleTheme")
            .field("parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

impl SimpleTheme {
    /// The simple theme with the Montserrat 14 font.
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn new() -> Self {
        Self::with_font(&twine_assets::fonts::MONTSERRAT_14)
    }

    /// The simple theme with `font`.
    #[must_use]
    pub fn with_font(font: &'static Font) -> Self {
        Self {
            font,
            styles: SimpleStyles::new(font),
            parent: None,
        }
    }

    /// Applies `parent` first on every node (LVGL `lv_theme_set_parent`).
    #[must_use]
    pub fn with_parent(mut self, parent: Rc<dyn Theme>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// The theme's styles.
    #[must_use]
    pub fn styles(&self) -> &SimpleStyles {
        &self.styles
    }
}

#[cfg(feature = "assets")]
impl Default for SimpleTheme {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeHook for SimpleTheme {
    /// LVGL `theme_apply()` of `lv_theme_simple.c`.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if let Some(p) = &self.parent {
            p.apply(cx, class);
        }
        let s = &self.styles;
        if cx.parent().is_none() {
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
            return;
        }
        match class.name {
            "obj" => {
                // NOTE(P19.S04): tabview and window children branch here, as in LVGL.
                cx.add_style(Selector::MAIN, s.white.clone());
                cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
            }
            "button" => cx.add_style(Selector::MAIN, s.dark.clone()),
            // Lines 290-295.
            "bar" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
            }
            // Lines 297-303.
            "slider" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
                cx.add_style(Selector::part(Part::Knob), s.dim.clone());
            }
            // Lines 312-317.
            "checkbox" => {
                let ind = Selector::part(Part::Indicator);
                cx.add_style(ind, s.light.clone());
                cx.add_style(ind.with_state(State::CHECKED), s.dark.clone());
            }
            // Lines 319-324.
            "switch" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::part(Part::Knob), s.dim.clone());
            }
            // Lines 354-364.
            "arc" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::MAIN, s.transp.clone());
                cx.add_style(Selector::MAIN, s.arc_line.clone());
                cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_line.clone());
                cx.add_style(Selector::part(Part::Knob), s.dim.clone());
                cx.add_style(Selector::part(Part::Knob), s.arc_knob.clone());
            }
            // Lines 366-374.
            "spinner" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::MAIN, s.transp.clone());
                cx.add_style(Selector::MAIN, s.arc_line.clone());
                cx.add_style(Selector::part(Part::Indicator), s.dark.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_line.clone());
            }
            // Lines 271-288.
            "buttonmatrix" => {
                // NOTE(P19.S04): message box and tabview button matrices branch here.
                cx.add_style(Selector::MAIN, s.white.clone());
                cx.add_style(Selector::part(Part::Items), s.light.clone());
            }
            // Lines 376-382.
            "textarea" => {
                cx.add_style(Selector::MAIN, s.white.clone());
                cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
                cx.add_style(
                    Selector::part(Part::Cursor).with_state(State::FOCUSED),
                    s.ta_cursor.clone(),
                );
            }
            // Lines 390-396.
            "keyboard" => {
                cx.add_style(Selector::MAIN, s.scr.clone());
                cx.add_style(Selector::part(Part::Items), s.white.clone());
                cx.add_style(
                    Selector::part(Part::Items).with_state(State::CHECKED),
                    s.light.clone(),
                );
            }
            // Lines 418-423.
            "spinbox" => {
                cx.add_style(Selector::MAIN, s.light.clone());
                cx.add_style(Selector::part(Part::Cursor), s.dark.clone());
            }
            // Lines 434-437.
            "led" => cx.add_style(Selector::MAIN, s.light.clone()),
            // LVGL's simple theme has no line, image button or animated image branch.
            "label" | "image" | "line" | "imagebutton" | "animimg" | "spangroup" | "buttonmatrix_popover" => {
            }
            other => {
                twine_core::trace!(target: "twine::style", "simple theme: no styles for class {}", other);
            }
        }
    }

    fn font_normal(&self) -> &'static Font {
        self.font
    }

    fn name(&self) -> &'static str {
        "simple"
    }
}

impl Theme for SimpleTheme {
    fn font_small(&self) -> &'static Font {
        self.font
    }
    fn font_large(&self) -> &'static Font {
        self.font
    }
    fn color_primary(&self) -> Color {
        COLOR_DARK
    }
    fn color_secondary(&self) -> Color {
        COLOR_DIM
    }
}
