//! The styles of the LVGL theme, LVGL's `style_init()` of
//! `src/themes/default/lv_theme_default.c` as `static` data (every value cites its LVGL
//! line): in flash, built while compiling, shared by every node and display.
//!
//! Nothing in them depends on the mode or the display: every color is a
//! [design element](twine_style::design) of the mode's table (values in
//! [`lvgl`](super)'s documentation) and every size LVGL scales by the DPI or the display size
//! class is an element of the display's [metrics](super::metrics) table. Applications reuse
//! them in their own recipes, so custom widgets look native:
//!
//! ```
//! use twine_core::Color;
//! use twine_engine::recipe;
//! use twine_style::{Radius, Selector, State};
//! use twine_theme::lvgl::styles::{BTN, CARD, PRESSED};
//! use twine_widgets::button::BUTTON_CLASS;
//!
//! recipe! {
//!     /// A pill button: the LVGL button with round ends.
//!     pub static PILL for BUTTON_CLASS {
//!         Selector::MAIN => { ..BTN, radius: Radius::Circle },
//!         State::PRESSED => { ..PRESSED },
//!     }
//! }
//! assert_eq!(PILL.entries.len(), 2);
//! # let _ = (&CARD, Color::RED);
//! ```
//!
//! Two fixed colors remain, on purpose: the `PRESSED` recolor is black (a darkening, the same
//! in every mode), and the `LED` style's white and grey are brightness masks the LED replaces
//! with its own color (the LED's color is `design::PRIMARY` by default).

use twine_core::{Color, Duration, Opa};
use twine_engine::Easing;
use twine_image::ImageSource;
use twine_style::design;
use twine_style::{BorderSide, DEFAULT_DPI, PropId, Props, Radius, Style, TextAlign, Transition, style};

use super::metrics::{
    BORDER_WIDTH_NEG, BORDER_WIDTH_X2, BUTTON_RADIUS, DP_1, DP_1_NEG, DP_2, DP_3, DP_4_NEG, DP_5, DP_6, DP_7,
    DP_10, DP_15, DP_20, KEY_RADIUS, KEYBOARD_PAD, ROTARY_SENSITIVITY,
};
use crate::Palette;

/// `LV_THEME_DEFAULT_TRANSITION_TIME` (80 ms in `lv_conf_template.h`).
pub const TRANSITION_TIME: Duration = Duration::ms(80);

/// `trans_props` of `style_init()`: the properties the theme's transitions animate (LVGL's
/// exact list, not whole groups: e.g. the checkbox's checked `bg_image` must switch at once,
/// not at the end of a transition).
pub const TRANS_PROPS: Props = Props::from_ids(&[
    PropId::BgOpacity,
    PropId::BgColor,
    PropId::TransformWidth,
    PropId::TransformHeight,
    PropId::TranslateY,
    PropId::TranslateX,
    PropId::TransformRotation,
    PropId::TransformScaleX,
    PropId::TransformScaleY,
    PropId::RecolorOpacity,
    PropId::Recolor,
]);

/// `lv_style_transition_dsc_init(&theme->trans_delayed, trans_props, lv_anim_path_linear,
/// TRANSITION_TIME, 70, NULL)`: going back to the default state waits 70 ms.
pub static TRANS_DELAYED: Transition = Transition::of(TRANS_PROPS, TRANSITION_TIME)
    .easing(Easing::Linear)
    .delay(Duration::ms(70));

/// `lv_style_transition_dsc_init(&theme->trans_normal, trans_props, lv_anim_path_linear,
/// TRANSITION_TIME, 0, NULL)`.
pub static TRANS_NORMAL: Transition = Transition::of(TRANS_PROPS, TRANSITION_TIME).easing(Easing::Linear);

/// `LV_SYMBOL_OK` as the checkbox marker's background image (`cb_marker_checked`).
pub static CHECK_MARK: ImageSource = ImageSource::symbol(twine_text::Symbol::Ok);

/// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the body font, small gaps, rotary
/// sensitivity.
pub static SCR: Style = style! {
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
    bg_color: design::BACKGROUND, // lv_style_set_bg_color(&scr, theme->color_scr)
    text_color: design::ON_SURFACE, // lv_style_set_text_color(&scr, theme->color_text)
    font: design::FONT_BODY, // lv_style_set_text_font(&scr, font_normal)
    gap: design::SPACE_S, // lv_style_set_pad_row / pad_column(&scr, PAD_SMALL)
    rotary_sensitivity: ROTARY_SENSITIVITY, // dpi / 4 * 256
};

/// Scrollbars (`Part::Scrollbar`): round, `SCROLLBAR` color at 40 %, 5 dp wide, 7 dp from the
/// edges, with the normal transition.
pub static SCROLLBAR: Style = style! {
    bg_color: design::SCROLLBAR, // lv_style_set_bg_color(&scrollbar, sb_color)
    radius: Radius::Circle, // lv_style_set_radius(&scrollbar, LV_RADIUS_CIRCLE)
    padding: DP_7, // lv_style_set_pad_all(&scrollbar, LV_DPX_CALC(dpi, 7))
    width: DP_5, // lv_style_set_width(&scrollbar, LV_DPX_CALC(dpi, 5))
    bg_opacity: Opa::P40, // lv_style_set_bg_opa(&scrollbar, LV_OPA_40)
    transition: &TRANS_NORMAL, // lv_style_set_transition(&scrollbar, &trans_normal)
};

/// Scrollbars while scrolled (`State::SCROLLED`): fully opaque.
pub static SCROLLBAR_SCROLLED: Style = style! {
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&scrollbar_scrolled, LV_OPA_COVER)
};

/// Cards and plain containers: `SURFACE` background, `OUTLINE` border drawn above the
/// children, default radius and padding, small gaps, `NEUTRAL` lines.
pub static CARD: Style = style! {
    radius: design::RADIUS_M, // lv_style_set_radius(&card, RADIUS_DEFAULT)
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&card, LV_OPA_COVER)
    bg_color: design::SURFACE, // lv_style_set_bg_color(&card, theme->color_card)
    border_color: design::OUTLINE, // lv_style_set_border_color(&card, theme->color_grey)
    border_width: DP_2, // lv_style_set_border_width(&card, BORDER_WIDTH)
    border_above_children: true, // lv_style_set_border_post(&card, true)
    text_color: design::ON_SURFACE, // lv_style_set_text_color(&card, theme->color_text)
    padding: design::SPACE_M, // lv_style_set_pad_all(&card, PAD_DEF)
    gap: design::SPACE_S, // lv_style_set_pad_row / pad_column(&card, PAD_SMALL)
    line_color: design::NEUTRAL, // lv_style_set_line_color(&card, lv_palette_main(GREY))
    line_width: DP_1, // lv_style_set_line_width(&card, LV_DPX_CALC(dpi, 1))
};

/// The keyboard focus ring (`State::FOCUS_KEY`): a 3 dp `FOCUS_RING` outline, 3 dp off the
/// node, at `FOCUS_RING_OPACITY`.
pub static OUTLINE_PRIMARY: Style = style! {
    outline_color: design::FOCUS_RING, // lv_style_set_outline_color(&outline_primary, color_primary)
    outline_width: DP_3, // lv_style_set_outline_width(&outline_primary, OUTLINE_WIDTH)
    outline_offset: DP_3, // lv_style_set_outline_pad(&outline_primary, OUTLINE_WIDTH)
    outline_opacity: design::FOCUS_RING_OPACITY, // lv_style_set_outline_opa(&outline_primary, LV_OPA_50)
};

/// The editing outline (`State::EDITED`): a 3 dp `SECONDARY` outline at `FOCUS_RING_OPACITY`.
pub static OUTLINE_SECONDARY: Style = style! {
    outline_color: design::SECONDARY, // lv_style_set_outline_color(&outline_secondary, color_secondary)
    outline_width: DP_3, // lv_style_set_outline_width(&outline_secondary, OUTLINE_WIDTH)
    outline_opacity: design::FOCUS_RING_OPACITY, // lv_style_set_outline_opa(&outline_secondary, LV_OPA_50)
};

/// Buttons: `SURFACE_VARIANT` background, a `SHADOW` drop shadow of elevation level 1
/// (`design::ELEVATION_1_*`; transparent in dark mode), radius and padding by display size.
pub static BTN: Style = style! {
    radius: BUTTON_RADIUS, // lv_style_set_radius(&btn, LV_DPX_CALC(dpi, LARGE ? 16 : MEDIUM ? 12 : 8))
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&btn, LV_OPA_COVER)
    bg_color: design::SURFACE_VARIANT, // lv_style_set_bg_color(&btn, theme->color_grey)
    // `if(!theme->dark)`: the shadow is set in every mode; `SHADOW_OPACITY` is transparent in
    // dark mode (no shadow drawn, no extra draw area).
    shadow_color: design::SHADOW, // lv_style_set_shadow_color(&btn, lv_palette_main(GREY))
    // The shadow geometry is elevation level 1 (3 dp wide, 3 dp down in the metrics table),
    // so a mode table that retunes `ELEVATION_1_*` retunes the button. Like LVGL, the style
    // sets no spread (0): one more property here costs ~4 ns on every pressed-button resolve.
    shadow_width: design::ELEVATION_1_WIDTH, // lv_style_set_shadow_width(&btn, LV_DPX_CALC(dpi, 3))
    shadow_opacity: design::SHADOW_OPACITY, // lv_style_set_shadow_opa(&btn, LV_OPA_50)
    shadow_offset_y: design::ELEVATION_1_OFFSET_Y, // lv_style_set_shadow_offset_y(&btn, LV_DPX_CALC(dpi, 3))
    text_color: design::ON_SURFACE, // lv_style_set_text_color(&btn, theme->color_text)
    padding_x: design::SPACE_M, // lv_style_set_pad_hor(&btn, PAD_DEF)
    padding_y: design::SPACE_S, // lv_style_set_pad_ver(&btn, PAD_SMALL)
    column_gap: DP_5, // lv_style_set_pad_column(&btn, LV_DPX_CALC(dpi, 5))
    row_gap: DP_5, // lv_style_set_pad_row(&btn, LV_DPX_CALC(dpi, 5))
};

/// The pressed look (`State::PRESSED`): a 35/255 black recolor.
pub static PRESSED: Style = style! {
    recolor: Color::BLACK, // lv_style_set_recolor(&pressed, lv_color_black())
    recolor_opacity: Opa::from_raw(35), // lv_style_set_recolor_opa(&pressed, 35)
};

/// The disabled look (`State::DISABLED`): a `DISABLED` recolor at `DISABLED_OPACITY`.
pub static DISABLED: Style = style! {
    recolor: design::DISABLED, // dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 2)
    recolor_opacity: design::DISABLED_OPACITY, // lv_style_set_recolor_opa(&disabled, LV_OPA_50)
};

/// Children clipped to the rounded corners, border drawn above them.
pub static CLIP_CORNER: Style = style! {
    clip_corner: true, // lv_style_set_clip_corner(&clip_corner, true)
    border_above_children: true, // lv_style_set_border_post(&clip_corner, true)
};

/// The default padding and gaps (by display size; `PAD_DEF`).
pub static PAD_NORMAL: Style = style! { padding: design::SPACE_M, gap: design::SPACE_M };
/// Small padding and gaps (by display size; `PAD_SMALL`).
pub static PAD_SMALL: Style = style! { padding: design::SPACE_S, gap: design::SPACE_S };
/// Tiny padding and gaps (by display size; `PAD_TINY`).
pub static PAD_TINY: Style = style! { padding: design::SPACE_XS, gap: design::SPACE_XS };
/// No padding and no gaps.
pub static PAD_ZERO: Style = style! { padding: 0, gap: 0 };
/// Row and column gaps of 10 dp (LVGL's `pad_gap`).
pub static PAD_GAP: Style = style! { gap: DP_10 };
/// Text line spacing of 20 dp.
pub static LINE_SPACE_LARGE: Style = style! { line_spacing: DP_20 };
/// Centered text.
pub static TEXT_ALIGN_CENTER: Style = style! { text_align: TextAlign::Center };

/// An opaque `PRIMARY` background with `ON_PRIMARY` text (e.g. buttons, checked parts).
pub static BG_COLOR_PRIMARY: Style = style! {
    bg_color: design::PRIMARY, // lv_style_set_bg_color(&bg_color_primary, color_primary)
    text_color: design::ON_PRIMARY, // lv_style_set_text_color(&bg_color_primary, lv_color_white())
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&bg_color_primary, LV_OPA_COVER)
};
/// A 20 % `PRIMARY` background with `PRIMARY` text (a subtle highlight).
pub static BG_COLOR_PRIMARY_MUTED: Style = style! {
    bg_color: design::PRIMARY,
    text_color: design::PRIMARY,
    bg_opacity: Opa::P20,
};
/// An opaque `SECONDARY` background with `ON_SECONDARY` text (e.g. checked buttons).
pub static BG_COLOR_SECONDARY: Style = style! {
    bg_color: design::SECONDARY,
    text_color: design::ON_SECONDARY,
    bg_opacity: Opa::COVER,
};
/// A 20 % `SECONDARY` background with `SECONDARY` text.
pub static BG_COLOR_SECONDARY_MUTED: Style = style! {
    bg_color: design::SECONDARY,
    text_color: design::SECONDARY,
    bg_opacity: Opa::P20,
};
/// An opaque `SURFACE_VARIANT` background with `ON_SURFACE` text (LVGL's grey).
pub static BG_COLOR_GREY: Style = style! {
    bg_color: design::SURFACE_VARIANT,
    bg_opacity: Opa::COVER,
    text_color: design::ON_SURFACE,
};
/// An opaque `SURFACE` background with `ON_SURFACE` text (LVGL's card color).
pub static BG_COLOR_WHITE: Style = style! {
    bg_color: design::SURFACE,
    bg_opacity: Opa::COVER,
    text_color: design::ON_SURFACE,
};

/// Fully round corners (`Radius::Circle`).
pub static CIRCLE: Style = style! { radius: Radius::Circle };
/// Square corners (radius 0).
pub static NO_RADIUS: Style = style! { radius: 0 };
/// The rotary (encoder) scroll sensitivity for the display's DPI.
pub static ROTARY_SCROLL: Style = style! { rotary_sensitivity: ROTARY_SENSITIVITY };
/// Grows the node by 3 dp on each side (`transform_width` / `transform_height`; LVGL
/// `LV_THEME_DEFAULT_GROW`), e.g. while pressed.
pub static GROW: Style = style! { transform_width: DP_3, transform_height: DP_3 };
/// The theme's transition of the background and transform-size properties, after a 70 ms
/// delay (on the default state: the way back from a pressed look).
pub static TRANSITION_DELAYED: Style = style! { transition: &TRANS_DELAYED };
/// The theme's transition of the background and transform-size properties, without delay
/// (on the pressed state: the way into it).
pub static TRANSITION_NORMAL: Style = style! { transition: &TRANS_NORMAL };
/// An animation duration (`anim_duration`) of 200 ms.
pub static ANIM: Style = style! { anim_duration: Duration::ms(200) };
/// An animation duration (`anim_duration`) of 120 ms.
pub static ANIM_FAST: Style = style! { anim_duration: Duration::ms(120) };

/// Slider and switch knobs: a round, opaque `PRIMARY` knob, 6 dp larger than the track on
/// every side.
pub static KNOB: Style = style! {
    bg_color: design::PRIMARY, // lv_style_set_bg_color(&knob, color_primary)
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&knob, LV_OPA_COVER)
    padding: DP_6, // lv_style_set_pad_all(&knob, LV_DPX_CALC(dpi, 6))
    radius: Radius::Circle, // lv_style_set_radius(&knob, LV_RADIUS_CIRCLE)
};

/// Arc tracks: 15 dp wide, rounded ends, `SURFACE_VARIANT` color (`#if LV_USE_ARC`).
pub static ARC_INDIC: Style = style! {
    arc_color: design::SURFACE_VARIANT, // lv_style_set_arc_color(&arc_indic, theme->color_grey)
    arc_width: DP_15, // lv_style_set_arc_width(&arc_indic, LV_DPX_CALC(dpi, 15))
    arc_rounded: true, // lv_style_set_arc_rounded(&arc_indic, true)
};
/// Arc indicators: the `PRIMARY` arc color.
pub static ARC_INDIC_PRIMARY: Style = style! { arc_color: design::PRIMARY };

/// Checkbox markers: a `PRIMARY` bordered `SURFACE` box with half the default radius and the
/// small font (`#if LV_USE_CHECKBOX`).
pub static CB_MARKER: Style = style! {
    padding: DP_3, // lv_style_set_pad_all(&cb_marker, LV_DPX_CALC(dpi, 3))
    border_width: DP_2, // lv_style_set_border_width(&cb_marker, BORDER_WIDTH)
    border_color: design::PRIMARY, // lv_style_set_border_color(&cb_marker, color_primary)
    bg_color: design::SURFACE, // lv_style_set_bg_color(&cb_marker, theme->color_card)
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&cb_marker, LV_OPA_COVER)
    radius: design::RADIUS_S, // lv_style_set_radius(&cb_marker, RADIUS_DEFAULT / 2)
    font: design::FONT_SMALL, // lv_style_set_text_font(&cb_marker, theme->base.font_small)
    text_color: design::ON_PRIMARY, // lv_style_set_text_color(&cb_marker, lv_color_white())
};
/// The checked checkbox marker: the check mark symbol as background image.
pub static CB_MARKER_CHECKED: Style = style! {
    bg_image: &CHECK_MARK, // lv_style_set_bg_image_src(&cb_marker_checked, LV_SYMBOL_OK)
};

/// Switch knobs: an `ON_PRIMARY` knob, 4 dp smaller than the track on every side
/// (`#if LV_USE_SWITCH`; `ON_PRIMARY` is white in light and dark mode).
pub static SWITCH_KNOB: Style = style! {
    padding: DP_4_NEG, // lv_style_set_pad_all(&switch_knob, -LV_DPX_CALC(dpi, 4))
    bg_color: design::ON_PRIMARY, // lv_style_set_bg_color(&switch_knob, lv_color_white())
};

/// Line widgets: 1 px `ON_SURFACE` lines (`#if LV_USE_LINE`).
pub static LINE: Style = style! {
    line_width: 1, // lv_style_set_line_width(&line, 1)
    line_color: design::ON_SURFACE, // lv_style_set_line_color(&line, theme->color_text)
};

/// LEDs: round, a white-to-grey gradient and a white glow (15 dp shadow, 5 dp spread;
/// `#if LV_USE_LED`).
pub static LED: Style = style! {
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&led, LV_OPA_COVER)
    bg_color: Color::WHITE, // lv_style_set_bg_color(&led, lv_color_white())
    bg_gradient_color: Palette::Grey.main(), // lv_style_set_bg_grad_color(&led, lv_palette_main(GREY))
    radius: Radius::Circle, // lv_style_set_radius(&led, LV_RADIUS_CIRCLE)
    shadow_width: DP_15, // lv_style_set_shadow_width(&led, LV_DPX_CALC(dpi, 15))
    shadow_color: Color::WHITE, // lv_style_set_shadow_color(&led, lv_color_white())
    shadow_spread: DP_5, // lv_style_set_shadow_spread(&led, LV_DPX_CALC(dpi, 5))
};

/// The text area cursor: a 2 dp `ON_SURFACE` left border, blinking every 400 ms
/// (`#if LV_USE_TEXTAREA`).
pub static TA_CURSOR: Style = style! {
    border_color: design::ON_SURFACE, // lv_style_set_border_color(&ta_cursor, theme->color_text)
    border_width: DP_2, // lv_style_set_border_width(&ta_cursor, LV_DPX_CALC(dpi, 2))
    padding_left: DP_1_NEG, // lv_style_set_pad_left(&ta_cursor, -LV_DPX_CALC(dpi, 1))
    border_side: BorderSide::LEFT, // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
    anim_duration: Duration::ms(400), // lv_style_set_anim_duration(&ta_cursor, 400)
};
/// The text area placeholder: `PLACEHOLDER` text color.
pub static TA_PLACEHOLDER: Style = style! { text_color: design::PLACEHOLDER };

/// Keyboard keys: no shadow, the default radius (half on small displays;
/// `#if LV_USE_KEYBOARD`).
pub static KEYBOARD_BUTTON_BG: Style = style! {
    shadow_width: 0, // lv_style_set_shadow_width(&keyboard_button_bg, 0)
    radius: KEY_RADIUS, // SMALL ? RADIUS_DEFAULT / 2 : RADIUS_DEFAULT
};
/// The keyboard's padding and gaps: `PAD_SMALL` on large displays, else `PAD_TINY` (LVGL
/// `theme_apply()`'s keyboard branch).
pub static KEYBOARD_PAD_STYLE: Style = style! { padding: KEYBOARD_PAD, gap: KEYBOARD_PAD };

/// Dropdown lists: at most twice the default DPI (260 px) high (`#if LV_USE_DROPDOWN`;
/// `LV_DPI_DEF * 2`: a constant, not scaled).
pub static DROPDOWN_LIST: Style = style! { max_height: DEFAULT_DPI as i32 * 2 };

/// Menus: no padding, gaps, radius or border; children clipped to the corners
/// (`#if LV_USE_MENU`).
pub static MENU_BG: Style = style! {
    padding: 0,
    gap: 0, // lv_style_set_pad_all / pad_gap(&menu_bg, 0)
    radius: 0, // lv_style_set_radius(&menu_bg, 0)
    clip_corner: true, // lv_style_set_clip_corner(&menu_bg, true)
    border_side: BorderSide::NONE, // lv_style_set_border_side(&menu_bg, LV_BORDER_SIDE_NONE)
};
/// Menu sections: opaque `SURFACE` background with the default radius, children clipped.
pub static MENU_SECTION: Style = style! {
    radius: design::RADIUS_M, // lv_style_set_radius(&menu_section, RADIUS_DEFAULT)
    clip_corner: true, // lv_style_set_clip_corner(&menu_section, true)
    bg_opacity: Opa::COVER, // lv_style_set_bg_opa(&menu_section, LV_OPA_COVER)
    bg_color: design::SURFACE, // lv_style_set_bg_color(&menu_section, theme->color_card)
    text_color: design::ON_SURFACE, // lv_style_set_text_color(&menu_section, theme->color_text)
};
/// Menu containers (rows): small padding and gaps, a faint `ON_SURFACE` border (10 %, not
/// drawn by default).
pub static MENU_CONT: Style = style! {
    padding: design::SPACE_S, // lv_style_set_pad_hor / pad_ver(&menu_cont, PAD_SMALL)
    gap: design::SPACE_S, // lv_style_set_pad_gap(&menu_cont, PAD_SMALL)
    border_width: DP_1, // lv_style_set_border_width(&menu_cont, LV_DPX_CALC(dpi, 1))
    border_opacity: Opa::P10, // lv_style_set_border_opa(&menu_cont, LV_OPA_10)
    border_color: design::ON_SURFACE, // lv_style_set_border_color(&menu_cont, theme->color_text)
    border_side: BorderSide::NONE, // lv_style_set_border_side(&menu_cont, LV_BORDER_SIDE_NONE)
};
/// The menu sidebar: no padding or gaps, a faint right border.
pub static MENU_SIDEBAR_CONT: Style = style! {
    padding: 0,
    gap: 0, // lv_style_set_pad_all / pad_gap(&menu_sidebar_cont, 0)
    border_width: DP_1, // lv_style_set_border_width(&menu_sidebar_cont, LV_DPX_CALC(dpi, 1))
    border_opacity: Opa::P10, // lv_style_set_border_opa(&menu_sidebar_cont, LV_OPA_10)
    border_color: design::ON_SURFACE, // lv_style_set_border_color(&menu_sidebar_cont, theme->color_text)
    border_side: BorderSide::RIGHT, // lv_style_set_border_side(&menu_sidebar_cont, LV_BORDER_SIDE_RIGHT)
};
/// The menu's main area: no padding or gaps.
pub static MENU_MAIN_CONT: Style = style! { padding: 0, gap: 0 };
/// Menu headers: small horizontal padding and gaps, tiny vertical padding.
pub static MENU_HEADER_CONT: Style = style! {
    gap: design::SPACE_S, // lv_style_set_pad_gap(&menu_header_cont, PAD_SMALL)
    padding_x: design::SPACE_S, // lv_style_set_pad_hor(&menu_header_cont, PAD_SMALL)
    padding_y: design::SPACE_XS, // lv_style_set_pad_ver(&menu_header_cont, PAD_TINY)
};
/// Menu header buttons (back): tiny padding, transparent background and shadow,
/// `ON_SURFACE` text.
pub static MENU_HEADER_BTN: Style = style! {
    padding: design::SPACE_XS, // lv_style_set_pad_hor / pad_ver(&menu_header_btn, PAD_TINY)
    shadow_opacity: Opa::TRANSP, // lv_style_set_shadow_opa(&menu_header_btn, LV_OPA_TRANSP)
    bg_opacity: Opa::TRANSP, // lv_style_set_bg_opa(&menu_header_btn, LV_OPA_TRANSP)
    text_color: design::ON_SURFACE, // lv_style_set_text_color(&menu_header_btn, theme->color_text)
};
/// Menu pages: no horizontal padding, no gaps.
pub static MENU_PAGE: Style = style! { gap: 0, padding_x: 0 };
/// Pressed menu items: a 20 % `NEUTRAL` background.
pub static MENU_PRESSED: Style = style! {
    bg_opacity: Opa::P20, // lv_style_set_bg_opa(&menu_pressed, LV_OPA_20)
    bg_color: design::NEUTRAL, // lv_style_set_bg_color(&menu_pressed, lv_palette_main(GREY))
};
/// Menu separators: transparent, tiny vertical padding.
pub static MENU_SEPARATOR: Style = style! {
    bg_opacity: Opa::TRANSP, // lv_style_set_bg_opa(&menu_separator, LV_OPA_TRANSP)
    padding_y: design::SPACE_XS, // lv_style_set_pad_ver(&menu_separator, PAD_TINY)
};

/// The modal backdrop of message boxes: `SCRIM` at `SCRIM_OPACITY` (`#if LV_USE_MSGBOX`).
pub static MSGBOX_BACKDROP_BG: Style = style! {
    bg_color: design::SCRIM, // lv_style_set_bg_color(&msgbox_backdrop_bg, lv_palette_main(GREY))
    bg_opacity: design::SCRIM_OPACITY, // lv_style_set_bg_opa(&msgbox_backdrop_bg, LV_OPA_50)
};

/// Tab buttons: a bottom `PRIMARY` border twice the default border width (the selected-tab
/// underline; `#if LV_USE_TABVIEW`).
pub static TAB_BTN: Style = style! {
    border_color: design::PRIMARY, // lv_style_set_border_color(&tab_btn, color_primary)
    border_width: BORDER_WIDTH_X2, // lv_style_set_border_width(&tab_btn, BORDER_WIDTH * 2)
    border_side: BorderSide::BOTTOM, // lv_style_set_border_side(&tab_btn, LV_BORDER_SIDE_BOTTOM)
    padding_top: BORDER_WIDTH_X2, // lv_style_set_pad_top(&tab_btn, BORDER_WIDTH * 2)
};
/// Focused tab bars: the outline drawn inside (a negative outline offset).
pub static TAB_BG_FOCUS: Style = style! {
    outline_offset: BORDER_WIDTH_NEG, // lv_style_set_outline_pad(&tab_bg_focus, -BORDER_WIDTH)
};

/// Lists: default horizontal padding, no vertical padding or gaps, children clipped to the
/// corners (`#if LV_USE_LIST`).
pub static LIST_BG: Style = style! {
    gap: 0, // lv_style_set_pad_gap(&list_bg, 0)
    padding_x: design::SPACE_M, // lv_style_set_pad_hor(&list_bg, PAD_DEF)
    padding_y: 0, // lv_style_set_pad_ver(&list_bg, 0)
    clip_corner: true, // lv_style_set_clip_corner(&list_bg, true)
};
/// List buttons: small padding, a 1 dp bottom `OUTLINE` border between items.
pub static LIST_BTN: Style = style! {
    padding: design::SPACE_S, // lv_style_set_pad_all(&list_btn, PAD_SMALL)
    border_width: DP_1, // lv_style_set_border_width(&list_btn, LV_DPX_CALC(dpi, 1))
    border_color: design::OUTLINE, // lv_style_set_border_color(&list_btn, theme->color_grey)
    border_side: BorderSide::BOTTOM, // lv_style_set_border_side(&list_btn, LV_BORDER_SIDE_BOTTOM)
    column_gap: design::SPACE_S, // lv_style_set_pad_column(&list_btn, PAD_SMALL)
};
/// List items: wider by the default padding (`transform_width`; applied while pressed or
/// focused).
pub static LIST_ITEM_GROW: Style = style! {
    transform_width: design::SPACE_M, // lv_style_set_transform_width(&list_item_grow, PAD_DEF)
};
