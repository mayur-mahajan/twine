//! The styles of the default theme, built exactly like LVGL's `style_init()` in
//! `src/themes/default/lv_theme_default.c` (every value cites its LVGL line).

use alloc::rc::Rc;

use twine_core::{Color, Duration, Opa};
use twine_engine::Easing;
use twine_image::ImageSource;
use twine_style::{BorderSide, PropId, RADIUS_CIRCLE, StyleBuf, TextAlign, TransitionDsc};
use twine_text::Font;

use super::{DisplaySize, ThemeMode, colors};
use crate::{Palette, dpx};

/// `LV_THEME_DEFAULT_TRANSITION_TIME` (80 ms in `lv_conf_template.h`).
pub const TRANSITION_TIME: Duration = Duration::ms(80);

/// `trans_props` of `style_init()`: the properties the default theme's transitions animate.
static TRANS_PROPS: [PropId; 11] = [
    PropId::BgOpa,
    PropId::BgColor,
    PropId::TransformWidth,
    PropId::TransformHeight,
    PropId::TranslateY,
    PropId::TranslateX,
    PropId::TransformRotation,
    PropId::TransformScaleX,
    PropId::TransformScaleY,
    PropId::RecolorOpa,
    PropId::Recolor,
];

/// `lv_style_transition_dsc_init(&theme->trans_delayed, trans_props, lv_anim_path_linear,
/// TRANSITION_TIME, 70, NULL)`: going back to the default state waits 70 ms.
pub static TRANS_DELAYED: TransitionDsc =
    TransitionDsc::new(&TRANS_PROPS, TRANSITION_TIME, Easing::Linear).delay(Duration::ms(70));

/// `lv_style_transition_dsc_init(&theme->trans_normal, trans_props, lv_anim_path_linear,
/// TRANSITION_TIME, 0, NULL)`.
pub static TRANS_NORMAL: TransitionDsc = TransitionDsc::new(&TRANS_PROPS, TRANSITION_TIME, Easing::Linear);

/// `LV_SYMBOL_OK` as the checkbox marker's background image (`cb_marker_checked`).
pub static CHECK_MARK: ImageSource = ImageSource::Symbol(twine_text::symbols::OK);

/// What the styles depend on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub primary: Color,
    pub secondary: Color,
    pub mode: ThemeMode,
    pub dpi: u16,
    pub size: DisplaySize,
    pub font_small: &'static Font,
    pub font_normal: &'static Font,
}

/// Every style of LVGL's `my_theme_styles_t` that the widgets implemented so far use.
#[derive(Debug)]
#[allow(missing_docs)] // field names are LVGL's style names
pub struct Styles {
    pub scr: Rc<StyleBuf>,
    pub scrollbar: Rc<StyleBuf>,
    pub scrollbar_scrolled: Rc<StyleBuf>,
    pub card: Rc<StyleBuf>,
    pub btn: Rc<StyleBuf>,
    pub bg_color_primary: Rc<StyleBuf>,
    pub bg_color_primary_muted: Rc<StyleBuf>,
    pub bg_color_secondary: Rc<StyleBuf>,
    pub bg_color_secondary_muted: Rc<StyleBuf>,
    pub bg_color_grey: Rc<StyleBuf>,
    pub bg_color_white: Rc<StyleBuf>,
    pub pressed: Rc<StyleBuf>,
    pub disabled: Rc<StyleBuf>,
    pub pad_zero: Rc<StyleBuf>,
    pub pad_tiny: Rc<StyleBuf>,
    pub pad_small: Rc<StyleBuf>,
    pub pad_normal: Rc<StyleBuf>,
    pub pad_gap: Rc<StyleBuf>,
    pub line_space_large: Rc<StyleBuf>,
    pub text_align_center: Rc<StyleBuf>,
    pub outline_primary: Rc<StyleBuf>,
    pub outline_secondary: Rc<StyleBuf>,
    pub circle: Rc<StyleBuf>,
    pub no_radius: Rc<StyleBuf>,
    pub clip_corner: Rc<StyleBuf>,
    pub rotary_scroll: Rc<StyleBuf>,
    pub grow: Rc<StyleBuf>,
    pub transition_delayed: Rc<StyleBuf>,
    pub transition_normal: Rc<StyleBuf>,
    pub anim: Rc<StyleBuf>,
    pub anim_fast: Rc<StyleBuf>,
    pub knob: Rc<StyleBuf>,
    pub arc_indic: Rc<StyleBuf>,
    pub arc_indic_primary: Rc<StyleBuf>,
    pub cb_marker: Rc<StyleBuf>,
    pub cb_marker_checked: Rc<StyleBuf>,
    pub switch_knob: Rc<StyleBuf>,
    pub line: Rc<StyleBuf>,
    pub led: Rc<StyleBuf>,
    pub ta_cursor: Rc<StyleBuf>,
    pub ta_placeholder: Rc<StyleBuf>,
    pub keyboard_button_bg: Rc<StyleBuf>,
}

fn pad_all(s: StyleBuf, v: i32) -> StyleBuf {
    s.pad_top(v).pad_bottom(v).pad_left(v).pad_right(v)
}

fn pad_gap(s: StyleBuf, v: i32) -> StyleBuf {
    s.pad_row(v).pad_column(v)
}

impl Styles {
    /// LVGL `style_init(theme)`.
    #[allow(clippy::too_many_lines)] // one statement per LVGL line
    pub(crate) fn new(p: &Params) -> Self {
        let dark = p.mode == ThemeMode::Dark;
        let d = |px| dpx(px, p.dpi);
        let large = p.size == DisplaySize::Large;
        let medium = p.size == DisplaySize::Medium;
        // #define RADIUS_DEFAULT LV_DPX_CALC(theme->disp_dpi, theme->disp_size == DISP_LARGE ? 12 : 8)
        let radius_default = d(if large { 12 } else { 8 });
        // #define BORDER_WIDTH LV_DPX_CALC(theme->disp_dpi, 2)
        let border_width = d(2);
        // #define OUTLINE_WIDTH LV_DPX_CALC(theme->disp_dpi, 3)
        let outline_width = d(3);
        // #define PAD_DEF LV_DPX_CALC(dpi, LARGE ? 24 : MEDIUM ? 20 : 16)
        let pad_def = d(if large {
            24
        } else if medium {
            20
        } else {
            16
        });
        // #define PAD_SMALL LV_DPX_CALC(dpi, LARGE ? 14 : MEDIUM ? 12 : 10)
        let pad_small_v = d(if large {
            14
        } else if medium {
            12
        } else {
            10
        });
        // #define PAD_TINY LV_DPX_CALC(dpi, LARGE ? 8 : MEDIUM ? 6 : 2)
        let pad_tiny_v = d(if large {
            8
        } else if medium {
            6
        } else {
            2
        });
        // theme->color_scr / color_text / color_card / color_grey
        let (color_scr, color_text, color_card, color_grey) = if dark {
            (
                colors::DARK_SCR,
                colors::DARK_TEXT,
                colors::DARK_CARD,
                colors::DARK_GREY,
            )
        } else {
            (
                colors::LIGHT_SCR,
                colors::LIGHT_TEXT,
                colors::LIGHT_CARD,
                colors::LIGHT_GREY,
            )
        };
        // lv_style_set_rotary_sensitivity(..., theme->disp_dpi / 4 * 256)
        let rotary = u32::from(p.dpi) / 4 * 256;
        let rc = Rc::new;

        // lv_style_set_transition(&theme->styles.transition_delayed, &theme->trans_delayed)
        let transition_delayed = StyleBuf::new().transition(&TRANS_DELAYED);
        // lv_style_set_transition(&theme->styles.transition_normal, &theme->trans_normal)
        let transition_normal = StyleBuf::new().transition(&TRANS_NORMAL);

        // sb_color = dark ? lv_palette_darken(GREY, 2) : lv_palette_main(GREY)
        let sb_color = if dark {
            Palette::Grey.darken(2)
        } else {
            Palette::Grey.main()
        };
        let scrollbar = pad_all(
            StyleBuf::new()
                .bg_color(sb_color) // lv_style_set_bg_color(&scrollbar, sb_color)
                .radius(RADIUS_CIRCLE), // lv_style_set_radius(&scrollbar, LV_RADIUS_CIRCLE)
            d(7), // lv_style_set_pad_all(&scrollbar, LV_DPX_CALC(dpi, 7))
        )
        .width(d(5)) // lv_style_set_width(&scrollbar, LV_DPX_CALC(dpi, 5))
        .bg_opa(Opa::P40) // lv_style_set_bg_opa(&scrollbar, LV_OPA_40)
        .transition(&TRANS_NORMAL); // lv_style_set_transition(&scrollbar, &trans_normal)

        // lv_style_set_bg_opa(&scrollbar_scrolled, LV_OPA_COVER)
        let scrollbar_scrolled = StyleBuf::new().bg_opa(Opa::COVER);

        let scr = pad_gap(
            StyleBuf::new()
                .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
                .bg_color(color_scr) // lv_style_set_bg_color(&scr, theme->color_scr)
                .text_color(color_text) // lv_style_set_text_color(&scr, theme->color_text)
                .text_font(p.font_normal), // lv_style_set_text_font(&scr, font_normal)
            pad_small_v, // lv_style_set_pad_row / pad_column(&scr, PAD_SMALL)
        )
        .rotary_sensitivity(rotary); // lv_style_set_rotary_sensitivity(&scr, dpi / 4 * 256)

        let card = pad_gap(
            pad_all(
                StyleBuf::new()
                    .radius(radius_default) // lv_style_set_radius(&card, RADIUS_DEFAULT)
                    .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&card, LV_OPA_COVER)
                    .bg_color(color_card) // lv_style_set_bg_color(&card, theme->color_card)
                    .border_color(color_grey) // lv_style_set_border_color(&card, theme->color_grey)
                    .border_width(border_width) // lv_style_set_border_width(&card, BORDER_WIDTH)
                    .border_post(true) // lv_style_set_border_post(&card, true)
                    .text_color(color_text), // lv_style_set_text_color(&card, theme->color_text)
                pad_def, // lv_style_set_pad_all(&card, PAD_DEF)
            ),
            pad_small_v, // lv_style_set_pad_row / pad_column(&card, PAD_SMALL)
        )
        .line_color(Palette::Grey.main()) // lv_style_set_line_color(&card, lv_palette_main(GREY))
        .line_width(d(1)); // lv_style_set_line_width(&card, LV_DPX_CALC(dpi, 1))

        let outline_primary = StyleBuf::new()
            .outline_color(p.primary) // lv_style_set_outline_color(&outline_primary, color_primary)
            .outline_width(outline_width) // lv_style_set_outline_width(&outline_primary, OUTLINE_WIDTH)
            .outline_pad(outline_width) // lv_style_set_outline_pad(&outline_primary, OUTLINE_WIDTH)
            .outline_opa(Opa::P50); // lv_style_set_outline_opa(&outline_primary, LV_OPA_50)

        let outline_secondary = StyleBuf::new()
            .outline_color(p.secondary) // lv_style_set_outline_color(&outline_secondary, color_secondary)
            .outline_width(outline_width) // lv_style_set_outline_width(&outline_secondary, OUTLINE_WIDTH)
            .outline_opa(Opa::P50); // lv_style_set_outline_opa(&outline_secondary, LV_OPA_50)

        // lv_style_set_radius(&btn, LV_DPX_CALC(dpi, LARGE ? 16 : MEDIUM ? 12 : 8))
        let btn_radius = d(if large {
            16
        } else if medium {
            12
        } else {
            8
        });
        let mut btn = StyleBuf::new()
            .radius(btn_radius)
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&btn, LV_OPA_COVER)
            .bg_color(color_grey); // lv_style_set_bg_color(&btn, theme->color_grey)
        if !dark {
            btn = btn
                .shadow_color(Palette::Grey.main()) // lv_style_set_shadow_color(&btn, lv_palette_main(GREY))
                .shadow_width(d(3)) // lv_style_set_shadow_width(&btn, LV_DPX_CALC(dpi, 3))
                .shadow_opa(Opa::P50) // lv_style_set_shadow_opa(&btn, LV_OPA_50)
                .shadow_offset_y(d(3)); // lv_style_set_shadow_offset_y(&btn, LV_DPX_CALC(dpi, 3))
        }
        let btn = btn
            .text_color(color_text) // lv_style_set_text_color(&btn, theme->color_text)
            .pad_left(pad_def) // lv_style_set_pad_hor(&btn, PAD_DEF)
            .pad_right(pad_def)
            .pad_top(pad_small_v) // lv_style_set_pad_ver(&btn, PAD_SMALL)
            .pad_bottom(pad_small_v)
            .pad_column(d(5)) // lv_style_set_pad_column(&btn, LV_DPX_CALC(dpi, 5))
            .pad_row(d(5)); // lv_style_set_pad_row(&btn, LV_DPX_CALC(dpi, 5))

        let pressed = StyleBuf::new()
            .recolor(Color::BLACK) // lv_style_set_recolor(&pressed, lv_color_black())
            .recolor_opa(Opa(35)); // lv_style_set_recolor_opa(&pressed, 35)

        let disabled = StyleBuf::new()
            // dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 2)
            .recolor(if dark {
                Palette::Grey.darken(2)
            } else {
                Palette::Grey.lighten(2)
            })
            .recolor_opa(Opa::P50); // lv_style_set_recolor_opa(&disabled, LV_OPA_50)

        let clip_corner = StyleBuf::new()
            .clip_corner(true) // lv_style_set_clip_corner(&clip_corner, true)
            .border_post(true); // lv_style_set_border_post(&clip_corner, true)

        // lv_style_set_pad_all / pad_row / pad_column(&pad_normal, PAD_DEF)
        let pad_normal = pad_gap(pad_all(StyleBuf::new(), pad_def), pad_def);
        // lv_style_set_pad_all / pad_gap(&pad_small, PAD_SMALL)
        let pad_small = pad_gap(pad_all(StyleBuf::new(), pad_small_v), pad_small_v);
        // lv_style_set_pad_row / pad_column(&pad_gap, LV_DPX_CALC(dpi, 10))
        let pad_gap_s = pad_gap(StyleBuf::new(), d(10));
        // lv_style_set_text_line_space(&line_space_large, LV_DPX_CALC(dpi, 20))
        let line_space_large = StyleBuf::new().text_line_space(d(20));
        // lv_style_set_text_align(&text_align_center, LV_TEXT_ALIGN_CENTER)
        let text_align_center = StyleBuf::new().text_align(TextAlign::Center);
        // lv_style_set_pad_all / pad_row / pad_column(&pad_zero, 0)
        let pad_zero = pad_gap(pad_all(StyleBuf::new(), 0), 0);
        // lv_style_set_pad_all / pad_row / pad_column(&pad_tiny, PAD_TINY)
        let pad_tiny = pad_gap(pad_all(StyleBuf::new(), pad_tiny_v), pad_tiny_v);

        let bg_color_primary = StyleBuf::new()
            .bg_color(p.primary) // lv_style_set_bg_color(&bg_color_primary, color_primary)
            .text_color(Color::WHITE) // lv_style_set_text_color(&bg_color_primary, lv_color_white())
            .bg_opa(Opa::COVER); // lv_style_set_bg_opa(&bg_color_primary, LV_OPA_COVER)
        let bg_color_primary_muted = StyleBuf::new()
            .bg_color(p.primary) // lv_style_set_bg_color(&bg_color_primary_muted, color_primary)
            .text_color(p.primary) // lv_style_set_text_color(&bg_color_primary_muted, color_primary)
            .bg_opa(Opa::P20); // lv_style_set_bg_opa(&bg_color_primary_muted, LV_OPA_20)
        let bg_color_secondary = StyleBuf::new()
            .bg_color(p.secondary) // lv_style_set_bg_color(&bg_color_secondary, color_secondary)
            .text_color(Color::WHITE) // lv_style_set_text_color(&bg_color_secondary, lv_color_white())
            .bg_opa(Opa::COVER); // lv_style_set_bg_opa(&bg_color_secondary, LV_OPA_COVER)
        let bg_color_secondary_muted = StyleBuf::new()
            .bg_color(p.secondary) // lv_style_set_bg_color(&bg_color_secondary_muted, color_secondary)
            .text_color(p.secondary) // lv_style_set_text_color(&bg_color_secondary_muted, color_secondary)
            .bg_opa(Opa::P20); // lv_style_set_bg_opa(&bg_color_secondary_muted, LV_OPA_20)
        let bg_color_grey = StyleBuf::new()
            .bg_color(color_grey) // lv_style_set_bg_color(&bg_color_grey, theme->color_grey)
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&bg_color_grey, LV_OPA_COVER)
            .text_color(color_text); // lv_style_set_text_color(&bg_color_grey, theme->color_text)
        let bg_color_white = StyleBuf::new()
            .bg_color(color_card) // lv_style_set_bg_color(&bg_color_white, theme->color_card)
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&bg_color_white, LV_OPA_COVER)
            .text_color(color_text); // lv_style_set_text_color(&bg_color_white, theme->color_text)

        // lv_style_set_radius(&circle, LV_RADIUS_CIRCLE)
        let circle = StyleBuf::new().radius(RADIUS_CIRCLE);
        // lv_style_set_radius(&no_radius, 0)
        let no_radius = StyleBuf::new().radius(0);
        // lv_style_set_rotary_sensitivity(&rotary_scroll, theme->disp_dpi / 4 * 256)
        let rotary_scroll = StyleBuf::new().rotary_sensitivity(rotary);
        // LV_THEME_DEFAULT_GROW: lv_style_set_transform_width / height(&grow, LV_DPX_CALC(dpi, 3))
        let grow = StyleBuf::new().transform_width(d(3)).transform_height(d(3));

        let knob = pad_all(
            StyleBuf::new()
                .bg_color(p.primary) // lv_style_set_bg_color(&knob, color_primary)
                .bg_opa(Opa::COVER), // lv_style_set_bg_opa(&knob, LV_OPA_COVER)
            d(6), // lv_style_set_pad_all(&knob, LV_DPX_CALC(dpi, 6))
        )
        .radius(RADIUS_CIRCLE); // lv_style_set_radius(&knob, LV_RADIUS_CIRCLE)

        // lv_style_set_anim_duration(&anim, 200)
        let anim = StyleBuf::new().anim_duration(200u32);
        // lv_style_set_anim_duration(&anim_fast, 120)
        let anim_fast = StyleBuf::new().anim_duration(120u32);

        // #if LV_USE_ARC (lines 398-406)
        let arc_indic = StyleBuf::new()
            .arc_color(color_grey) // lv_style_set_arc_color(&arc_indic, theme->color_grey)
            .arc_width(d(15)) // lv_style_set_arc_width(&arc_indic, LV_DPX_CALC(dpi, 15))
            .arc_rounded(true); // lv_style_set_arc_rounded(&arc_indic, true)
        // lv_style_set_arc_color(&arc_indic_primary, theme->base.color_primary)
        let arc_indic_primary = StyleBuf::new().arc_color(p.primary);

        // #if LV_USE_CHECKBOX (lines 412-425)
        let cb_marker = pad_all(StyleBuf::new(), d(3)) // lv_style_set_pad_all(&cb_marker, LV_DPX_CALC(dpi, 3))
            .border_width(border_width) // lv_style_set_border_width(&cb_marker, BORDER_WIDTH)
            .border_color(p.primary) // lv_style_set_border_color(&cb_marker, color_primary)
            .bg_color(color_card) // lv_style_set_bg_color(&cb_marker, theme->color_card)
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&cb_marker, LV_OPA_COVER)
            .radius(radius_default / 2) // lv_style_set_radius(&cb_marker, RADIUS_DEFAULT / 2)
            .text_font(p.font_small) // lv_style_set_text_font(&cb_marker, theme->base.font_small)
            .text_color(Color::WHITE); // lv_style_set_text_color(&cb_marker, lv_color_white())
        // lv_style_set_bg_image_src(&cb_marker_checked, LV_SYMBOL_OK)
        let cb_marker_checked = StyleBuf::new().bg_image_src(&CHECK_MARK);

        // #if LV_USE_SWITCH (lines 427-431)
        let switch_knob = pad_all(StyleBuf::new(), -d(4)) // lv_style_set_pad_all(&switch_knob, -LV_DPX_CALC(dpi, 4))
            .bg_color(Color::WHITE); // lv_style_set_bg_color(&switch_knob, lv_color_white())

        // #if LV_USE_LINE (lines 433-437)
        let line = StyleBuf::new()
            .line_width(1) // lv_style_set_line_width(&line, 1)
            .line_color(color_text); // lv_style_set_line_color(&line, theme->color_text)

        // #if LV_USE_LED (lines 601-610)
        let led = StyleBuf::new()
            .bg_opa(Opa::COVER) // lv_style_set_bg_opa(&led, LV_OPA_COVER)
            .bg_color(Color::WHITE) // lv_style_set_bg_color(&led, lv_color_white())
            .bg_grad_color(Palette::Grey.main()) // lv_style_set_bg_grad_color(&led, lv_palette_main(GREY))
            .radius(RADIUS_CIRCLE) // lv_style_set_radius(&led, LV_RADIUS_CIRCLE)
            .shadow_width(d(15)) // lv_style_set_shadow_width(&led, LV_DPX_CALC(dpi, 15))
            .shadow_color(Color::WHITE) // lv_style_set_shadow_color(&led, lv_color_white())
            .shadow_spread(d(5)); // lv_style_set_shadow_spread(&led, LV_DPX_CALC(dpi, 5))

        // #if LV_USE_TEXTAREA (lines 528-540)
        let ta_cursor = StyleBuf::new()
            .border_color(color_text) // lv_style_set_border_color(&ta_cursor, theme->color_text)
            .border_width(d(2)) // lv_style_set_border_width(&ta_cursor, LV_DPX_CALC(dpi, 2))
            .pad_left(-d(1)) // lv_style_set_pad_left(&ta_cursor, -LV_DPX_CALC(dpi, 1))
            .border_side(BorderSide::LEFT) // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
            .anim_duration(400u32); // lv_style_set_anim_duration(&ta_cursor, 400)
        // lv_style_set_text_color(&ta_placeholder, dark ? darken(GREY, 2) : lighten(GREY, 1))
        let ta_placeholder = StyleBuf::new().text_color(if dark {
            Palette::Grey.darken(2)
        } else {
            Palette::Grey.lighten(1)
        });

        // #if LV_USE_KEYBOARD (lines 565-569)
        let keyboard_button_bg = StyleBuf::new()
            .shadow_width(0) // lv_style_set_shadow_width(&keyboard_button_bg, 0)
            // lv_style_set_radius(&keyboard_button_bg, SMALL ? RADIUS_DEFAULT / 2 : RADIUS_DEFAULT)
            .radius(if p.size == DisplaySize::Small {
                radius_default / 2
            } else {
                radius_default
            });

        Self {
            scr: rc(scr),
            scrollbar: rc(scrollbar),
            scrollbar_scrolled: rc(scrollbar_scrolled),
            card: rc(card),
            btn: rc(btn),
            bg_color_primary: rc(bg_color_primary),
            bg_color_primary_muted: rc(bg_color_primary_muted),
            bg_color_secondary: rc(bg_color_secondary),
            bg_color_secondary_muted: rc(bg_color_secondary_muted),
            bg_color_grey: rc(bg_color_grey),
            bg_color_white: rc(bg_color_white),
            pressed: rc(pressed),
            disabled: rc(disabled),
            pad_zero: rc(pad_zero),
            pad_tiny: rc(pad_tiny),
            pad_small: rc(pad_small),
            pad_normal: rc(pad_normal),
            pad_gap: rc(pad_gap_s),
            line_space_large: rc(line_space_large),
            text_align_center: rc(text_align_center),
            outline_primary: rc(outline_primary),
            outline_secondary: rc(outline_secondary),
            circle: rc(circle),
            no_radius: rc(no_radius),
            clip_corner: rc(clip_corner),
            rotary_scroll: rc(rotary_scroll),
            grow: rc(grow),
            transition_delayed: rc(transition_delayed),
            transition_normal: rc(transition_normal),
            anim: rc(anim),
            anim_fast: rc(anim_fast),
            knob: rc(knob),
            arc_indic: rc(arc_indic),
            arc_indic_primary: rc(arc_indic_primary),
            cb_marker: rc(cb_marker),
            cb_marker_checked: rc(cb_marker_checked),
            switch_knob: rc(switch_knob),
            line: rc(line),
            led: rc(led),
            ta_cursor: rc(ta_cursor),
            ta_placeholder: rc(ta_placeholder),
            keyboard_button_bg: rc(keyboard_button_bg),
        }
    }
}
