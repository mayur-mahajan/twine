//! [`MonoTheme`]: LVGL's monochrome theme (`src/themes/mono/lv_theme_mono.c`) for 1-bit
//! displays. Every color is pure black or pure white, so `I1` output is exact.

use alloc::rc::Rc;

use twine_core::{Color, Opa};
use twine_engine::{ThemeCx, ThemeHook, WidgetClass};
use twine_style::{BorderSide, Part, RADIUS_CIRCLE, Selector, State, StyleBuf, TextDecor};
use twine_text::Font;

use crate::Theme;

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
#[allow(missing_docs)] // field names are LVGL's style names
pub struct MonoStyles {
    pub scr: Rc<StyleBuf>,
    pub card: Rc<StyleBuf>,
    pub scrollbar: Rc<StyleBuf>,
    pub pr: Rc<StyleBuf>,
    pub inv: Rc<StyleBuf>,
    pub disabled: Rc<StyleBuf>,
    pub focus: Rc<StyleBuf>,
    pub edit: Rc<StyleBuf>,
    pub large_border: Rc<StyleBuf>,
    pub pad_gap: Rc<StyleBuf>,
    pub pad_zero: Rc<StyleBuf>,
    pub no_radius: Rc<StyleBuf>,
    pub radius_circle: Rc<StyleBuf>,
    pub large_line_space: Rc<StyleBuf>,
    pub underline: Rc<StyleBuf>,
    pub spinner_indic: Rc<StyleBuf>,
    pub ta_cursor: Rc<StyleBuf>,
    /// Not in LVGL: the check mark of a checked checkbox (LVGL's mono theme only inverts the
    /// box).
    pub cb_marker_checked: Rc<StyleBuf>,
}

fn pad_all(s: StyleBuf, v: i32) -> StyleBuf {
    s.pad_top(v).pad_bottom(v).pad_left(v).pad_right(v)
}

fn pad_gap(s: StyleBuf, v: i32) -> StyleBuf {
    s.pad_row(v).pad_column(v)
}

/// `EXPAND_BORDER(style, state)`: a thicker border that keeps the content in place.
fn expand_border(w: i32) -> StyleBuf {
    // lv_style_set_border_width(&style, BORDER_W_x)
    // lv_style_set_pad_all(&style, PAD_DEF + BORDER_W_NORMAL - BORDER_W_x)
    pad_all(StyleBuf::new().border_width(w), PAD_DEF + BORDER_W_NORMAL - w)
}

impl MonoStyles {
    /// LVGL `style_init(theme, dark_bg, font)` of the mono theme.
    fn new(dark_bg: bool, font: &'static Font) -> Self {
        // #define COLOR_FG dark_bg ? lv_color_white() : lv_color_black()
        let fg = if dark_bg { Color::WHITE } else { Color::BLACK };
        // #define COLOR_BG dark_bg ? lv_color_black() : lv_color_white()
        let bg = if dark_bg { Color::BLACK } else { Color::WHITE };
        let scrollbar = StyleBuf::new()
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&scrollbar, LV_OPA_COVER)
            .bg_color(fg) // lv_style_set_bg_color(&scrollbar, COLOR_FG)
            .width(PAD_DEF); // lv_style_set_width(&scrollbar, PAD_DEF)
        let scr = pad_gap(
            StyleBuf::new()
                .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
                .bg_color(bg) // lv_style_set_bg_color(&scr, COLOR_BG)
                .text_color(fg), // lv_style_set_text_color(&scr, COLOR_FG)
            PAD_DEF, // lv_style_set_pad_row / pad_column(&scr, PAD_DEF)
        )
        .text_font(font); // lv_style_set_text_font(&scr, font)
        let card = pad_gap(
            pad_all(
                StyleBuf::new()
                    .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&card, LV_OPA_COVER)
                    .bg_color(bg) // lv_style_set_bg_color(&card, COLOR_BG)
                    .border_color(fg) // lv_style_set_border_color(&card, COLOR_FG)
                    .radius(2) // lv_style_set_radius(&card, 2)
                    .border_width(BORDER_W_NORMAL), // lv_style_set_border_width(&card, BORDER_W_NORMAL)
                PAD_DEF, // lv_style_set_pad_all(&card, PAD_DEF)
            ),
            PAD_DEF, // lv_style_set_pad_gap(&card, PAD_DEF)
        )
        .text_color(fg) // lv_style_set_text_color(&card, COLOR_FG)
        .line_width(2) // lv_style_set_line_width(&card, 2)
        .line_color(fg) // lv_style_set_line_color(&card, COLOR_FG)
        .arc_width(2) // lv_style_set_arc_width(&card, 2)
        .arc_color(fg) // lv_style_set_arc_color(&card, COLOR_FG)
        .outline_color(fg) // lv_style_set_outline_color(&card, COLOR_FG)
        .anim_duration(300u32); // lv_style_set_anim_duration(&card, 300)
        let inv = StyleBuf::new()
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&inv, LV_OPA_COVER)
            .bg_color(fg) // lv_style_set_bg_color(&inv, COLOR_FG)
            .border_color(bg) // lv_style_set_border_color(&inv, COLOR_BG)
            .line_color(bg) // lv_style_set_line_color(&inv, COLOR_BG)
            .arc_color(bg) // lv_style_set_arc_color(&inv, COLOR_BG)
            .text_color(bg) // lv_style_set_text_color(&inv, COLOR_BG)
            .outline_color(bg); // lv_style_set_outline_color(&inv, COLOR_BG)
        let focus = StyleBuf::new()
            .outline_width(1) // lv_style_set_outline_width(&focus, 1)
            .outline_pad(BORDER_W_FOCUS); // lv_style_set_outline_pad(&focus, BORDER_W_FOCUS)
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
            pad_gap: Rc::new(pad_gap(StyleBuf::new(), PAD_DEF)),
            // lv_style_set_pad_all / pad_gap(&pad_zero, 0)
            pad_zero: Rc::new(pad_gap(pad_all(StyleBuf::new(), 0), 0)),
            // lv_style_set_radius(&no_radius, 0)
            no_radius: Rc::new(StyleBuf::new().radius(0)),
            // lv_style_set_radius(&radius_circle, LV_RADIUS_CIRCLE)
            radius_circle: Rc::new(StyleBuf::new().radius(RADIUS_CIRCLE)),
            // lv_style_set_text_line_space(&large_line_space, 6)
            large_line_space: Rc::new(StyleBuf::new().text_line_space(6)),
            // lv_style_set_text_decor(&underline, LV_TEXT_DECOR_UNDERLINE)
            underline: Rc::new(StyleBuf::new().text_decor(TextDecor::UNDERLINE)),
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
                    .bg_opa(Opa::TRANSP) // lv_style_set_bg_opa(&ta_cursor, LV_OPA_TRANSP)
                    .anim_duration(500u32), // lv_style_set_anim_duration(&ta_cursor, 500)
            ),
            cb_marker_checked: Rc::new(StyleBuf::new().bg_image_src(&crate::default::styles::CHECK_MARK)),
        }
    }
}

/// LVGL's monochrome theme: black on white (or white on black), 1-px borders, pressed
/// buttons get a thicker border, checked ones are inverted, focus and edit show outlines.
///
/// ```
/// use twine_theme::{MonoTheme, ThemeHook};
/// let t = MonoTheme::new(true, &twine_assets::fonts::MONTSERRAT_14);
/// assert_eq!(t.color_primary(), twine_core::Color::WHITE);
/// ```
pub struct MonoTheme {
    dark: bool,
    font: &'static Font,
    styles: MonoStyles,
    parent: Option<Rc<dyn Theme>>,
}

impl core::fmt::Debug for MonoTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MonoTheme")
            .field("dark", &self.dark)
            .field("parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

impl MonoTheme {
    /// The mono theme, white on black when `dark` (LVGL `lv_theme_mono_init(disp, dark_bg,
    /// font)`).
    #[must_use]
    pub fn new(dark: bool, font: &'static Font) -> Self {
        Self {
            dark,
            font,
            styles: MonoStyles::new(dark, font),
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
    pub fn styles(&self) -> &MonoStyles {
        &self.styles
    }
}

impl ThemeHook for MonoTheme {
    /// LVGL `theme_apply()` of `lv_theme_mono.c`.
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
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
            }
            "button" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::state(State::PRESSED), s.pr.clone());
                cx.add_style(Selector::state(State::CHECKED), s.inv.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            }
            // Lines 381-389.
            "bar" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_zero.clone());
                cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
            }
            // Lines 390-401.
            "slider" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_zero.clone());
                cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
                cx.add_style(Selector::part(Part::Knob), s.card.clone());
                cx.add_style(Selector::part(Part::Knob), s.radius_circle.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            }
            // Lines 414-424.
            "checkbox" => {
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
            }
            // Lines 426-440.
            "switch" => {
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
            }
            // Lines 480-490.
            "arc" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::part(Part::Indicator), s.inv.clone());
                cx.add_style(Selector::part(Part::Indicator), s.pad_zero.clone());
                cx.add_style(Selector::part(Part::Knob), s.card.clone());
                cx.add_style(Selector::part(Part::Knob), s.radius_circle.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            }
            // Lines 492-496.
            "spinner" => cx.add_style(Selector::part(Part::Indicator), s.spinner_indic.clone()),
            // Lines 344-375.
            "buttonmatrix" => {
                // NOTE(P19.S04): message box and tabview button matrices branch here.
                let items = Selector::part(Part::Items);
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(items, s.card.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
                cx.add_style(items.with_state(State::CHECKED), s.inv.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
                cx.add_style(items.with_state(State::FOCUS_KEY), s.underline.clone());
                cx.add_style(items.with_state(State::FOCUS_KEY), s.large_border.clone());
            }
            // Lines 498-506.
            "textarea" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
                cx.add_style(
                    Selector::part(Part::Cursor).with_state(State::FOCUSED),
                    s.ta_cursor.clone(),
                );
                cx.add_style(Selector::state(State::FOCUSED), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            }
            // Lines 520-530.
            "keyboard" => {
                let items = Selector::part(Part::Items);
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(items, s.card.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pr.clone());
                cx.add_style(items.with_state(State::CHECKED), s.inv.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
                cx.add_style(items.with_state(State::EDITED), s.large_border.clone());
            }
            // Lines 555-562.
            "spinbox" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::part(Part::Cursor), s.inv.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.focus.clone());
                cx.add_style(Selector::state(State::EDITED), s.edit.clone());
            }
            // Lines 573-577.
            "led" => cx.add_style(Selector::MAIN, s.card.clone()),
            // LVGL's mono theme has no line, image button or animated image branch.
            "label" | "image" | "line" | "imagebutton" | "animimg" | "spangroup" | "buttonmatrix_popover" => {
            }
            other => {
                twine_core::trace!(target: "twine::style", "mono theme: no styles for class {}", other);
            }
        }
    }

    fn font_normal(&self) -> &'static Font {
        self.font
    }

    fn name(&self) -> &'static str {
        if self.dark { "mono-dark" } else { "mono-light" }
    }
    fn color_primary(&self) -> Color {
        if self.dark { Color::WHITE } else { Color::BLACK }
    }
    fn color_secondary(&self) -> Color {
        if self.dark { Color::BLACK } else { Color::WHITE }
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
