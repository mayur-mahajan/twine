//! The styles of the default theme, built exactly like LVGL's `style_init()` in
//! `src/themes/default/lv_theme_default.c` (every value cites its LVGL line).
//!
//! The styles do not depend on the mode: what LVGL builds differently for dark mode (the
//! screen, card, text and grey colors, the scrollbar, disabled and placeholder colors, the
//! button shadow) and every theme color (primary, secondary) are
//! [design elements](twine_style::design) whose values come from the mode's table.
//! `Light` and `Dark` are LVGL's; `Night` and `HighContrast` are Twine's (LVGL has neither):
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
//! | `PRIMARY` / `ON_PRIMARY` (also the switch knob) | the theme's primary / white | same | `#B06A10` / black | `#FFD600` / black | `color_primary` / `lv_color_white()` |
//! | `SECONDARY` / `ON_SECONDARY` | the theme's secondary / white | same | `#3C1A0A` / `#BC8644` | white / black | `color_secondary` / `lv_color_white()` |
//! | `DANGER`, `WARNING`, `OK` | red, orange, green (palette main) | same | `#C83020`, `#C08010`, `#5E8C24` | `#FF6E6E`, `#FFB300`, `#00E676` | — |
//! | `DISABLED` | grey lighten 2 | grey darken 2 | `#100C08` | `#808080` | disabled recolor |
//! | `FOCUS_RING` | primary | primary | `#C88020` | `#00E5FF` | `outline_primary` |
//! | `SCROLLBAR` | grey | grey darken 2 | `#4A3A24` | white | `sb_color` |
//! | `SHADOW`, `SCRIM` | grey | grey | black | black | button shadow, message box backdrop |
//! | `FOCUS_RING_OPACITY` | 50 % | 50 % | opaque | opaque | outline opacity |
//! | `SHADOW_OPACITY` | 50 % | transparent (no shadow) | transparent | transparent | button shadow |
//! | `DISABLED_OPACITY` | 50 % | 50 % | 30 % | 50 % | disabled recolor opacity |
//! | `SCRIM_OPACITY` | 50 % | 50 % | 70 % | 70 % | backdrop opacity |
//! | `SPACE_XS`, `SPACE_S`, `SPACE_M` | `PAD_TINY`, `PAD_SMALL`, `PAD_DEF` | same | same | same | paddings |
//! | `SPACE_L` | 24 / 28 / 32 dp (small / medium / large display) | same | same | same | — |
//! | `RADIUS_S`, `RADIUS_M`, `RADIUS_L`, `RADIUS_FULL` | `RADIUS_DEFAULT` / 2, `RADIUS_DEFAULT`, `RADIUS_DEFAULT` · 2, circle | same | same | same | radii |
//! | `FONT_SMALL`, `FONT_BODY`, `FONT_LARGE` | the theme's fonts | same | same | same | `font_small` / `font_normal` / `font_large` |
//!
//! `Night` keeps every color at most 30 % relative luminance with its blue channel at most
//! half its red channel (warm amber, preserving dark adaptation), and `HighContrast` is white
//! on black with a yellow accent and an opaque cyan focus ring; neither uses the theme's
//! primary and secondary colors (a blue accent would break the night limits and may lack
//! contrast): override them per mode with
//! [`DefaultThemeBuilder::element_in`](super::DefaultThemeBuilder::element_in). Every mode meets the
//! minimum contrast ratios of [`ThemeMode::min_contrast`] (tested; ratios per pair are listed
//! in the crate documentation).
//!
//! Two fixed colors remain, on purpose: the `pressed` recolor is black (a darkening, the same
//! in every mode), and the `led` style's white and grey are brightness masks the LED replaces
//! with its own color (the LED's color is `design::PRIMARY` by default).

use alloc::rc::Rc;

use twine_core::{Color, Duration, Opa, Scale};
use twine_engine::Easing;
use twine_image::ImageSource;
use twine_style::design::{self, ElementTable};
use twine_style::{
    BorderSide, Length, PropId, Props, Radius, StyleBuf, TextAlign, ThemeMode, Transition, TransitionValue,
};
use twine_text::Font;

use super::{DisplaySize, colors};
use twine_style::{DEFAULT_DPI, dpx};

use crate::{Palette, Tone};

/// `LV_THEME_DEFAULT_TRANSITION_TIME` (80 ms in `lv_conf_template.h`).
pub const TRANSITION_TIME: Duration = Duration::ms(80);

/// `trans_props` of `style_init()`: the properties the default theme's transitions animate
/// (LVGL's exact list, not whole groups: e.g. the checkbox's checked `bg_image` must switch at
/// once, not at the end of a transition).
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

/// What the styles and the element tables depend on (the styles do not use the colors: they
/// are design elements).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub primary: Color,
    pub secondary: Color,
    pub dpi: u16,
    pub size: DisplaySize,
    pub font_small: &'static Font,
    pub font_normal: &'static Font,
    pub font_large: &'static Font,
}

/// The sizes LVGL's `style_init()` derives from the DPI and the display size class.
struct Metrics {
    radius_default: i32,
    pad_def: i32,
    pad_small: i32,
    pad_tiny: i32,
}

impl Metrics {
    fn new(p: &Params) -> Self {
        let d = |px| dpx(px, p.dpi);
        let large = p.size == DisplaySize::Large;
        let medium = p.size == DisplaySize::Medium;
        let pick = |l, m, s| {
            if large {
                l
            } else if medium {
                m
            } else {
                s
            }
        };
        Self {
            // #define RADIUS_DEFAULT LV_DPX_CALC(theme->disp_dpi, theme->disp_size == DISP_LARGE ? 12 : 8)
            radius_default: d(if large { 12 } else { 8 }),
            // #define PAD_DEF LV_DPX_CALC(dpi, LARGE ? 24 : MEDIUM ? 20 : 16)
            pad_def: d(pick(24, 20, 16)),
            // #define PAD_SMALL LV_DPX_CALC(dpi, LARGE ? 14 : MEDIUM ? 12 : 10)
            pad_small: d(pick(14, 12, 10)),
            // #define PAD_TINY LV_DPX_CALC(dpi, LARGE ? 8 : MEDIUM ? 6 : 2)
            pad_tiny: d(pick(8, 6, 2)),
        }
    }
}

/// The colors and opacities of one mode of the default theme (see the module documentation).
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
    fn new(p: &Params, mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light | ThemeMode::Dark => {
                let dark = mode == ThemeMode::Dark;
                let dark_or = |d: Color, l: Color| if dark { d } else { l };
                let grey = dark_or(colors::DARK_GREY, colors::LIGHT_GREY);
                Self {
                    background: dark_or(colors::DARK_SCR, colors::LIGHT_SCR),
                    surface: dark_or(colors::DARK_CARD, colors::LIGHT_CARD),
                    on_surface: dark_or(colors::DARK_TEXT, colors::LIGHT_TEXT),
                    on_surface_muted: dark_or(Palette::Grey.tone(Tone::L1), Palette::Grey.tone(Tone::D1)),
                    // ta_placeholder: dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 1)
                    placeholder: dark_or(Palette::Grey.tone(Tone::D2), Palette::Grey.tone(Tone::L1)),
                    // lv_palette_main(GREY) in both modes (card lines, menu_pressed).
                    neutral: Palette::Grey.main(),
                    surface_variant: grey,
                    outline: grey,
                    primary: p.primary,
                    on_primary: Color::WHITE,
                    secondary: p.secondary,
                    on_secondary: Color::WHITE,
                    danger: Palette::Red.main(),
                    warning: Palette::Orange.main(),
                    ok: Palette::Green.main(),
                    // disabled: dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 2)
                    disabled: dark_or(Palette::Grey.tone(Tone::D2), Palette::Grey.tone(Tone::L2)),
                    focus_ring: p.primary,
                    // sb_color = dark ? lv_palette_darken(GREY, 2) : lv_palette_main(GREY)
                    scrollbar: dark_or(Palette::Grey.tone(Tone::D2), Palette::Grey.main()),
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
            // The theme's primary / secondary colors are not used (a blue accent would break
            // both limits); override them per mode with `element_in(ThemeMode::Night, ..)`.
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
            // focus ring, no shadows. The theme's primary / secondary colors are not used.
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

/// The design element table of the default theme in `mode` (see the module documentation
/// for the values): every standard element, LVGL's values where LVGL has one.
pub(crate) fn elements(p: &Params, mode: ThemeMode) -> ElementTable {
    let m = Metrics::new(p);
    let pick = |l, md, s| match p.size {
        DisplaySize::Large => l,
        DisplaySize::Medium => md,
        DisplaySize::Small => s,
    };
    let c = ModeColors::new(p, mode);
    ElementTable::new()
        .with(design::BACKGROUND, c.background)
        .with(design::SURFACE, c.surface)
        .with(design::ON_SURFACE, c.on_surface)
        .with(design::ON_SURFACE_MUTED, c.on_surface_muted)
        .with(design::PLACEHOLDER, c.placeholder)
        .with(design::NEUTRAL, c.neutral)
        .with(design::SURFACE_VARIANT, c.surface_variant)
        .with(design::OUTLINE, c.outline)
        .with(design::PRIMARY, c.primary)
        .with(design::ON_PRIMARY, c.on_primary)
        .with(design::SECONDARY, c.secondary)
        .with(design::ON_SECONDARY, c.on_secondary)
        .with(design::DANGER, c.danger)
        .with(design::WARNING, c.warning)
        .with(design::OK, c.ok)
        .with(design::DISABLED, c.disabled)
        .with(design::FOCUS_RING, c.focus_ring)
        .with(design::SCROLLBAR, c.scrollbar)
        .with(design::SHADOW, c.shadow)
        .with(design::SCRIM, c.scrim)
        .with(design::SPACE_XS, Length::Px(m.pad_tiny))
        .with(design::SPACE_S, Length::Px(m.pad_small))
        .with(design::SPACE_M, Length::Px(m.pad_def))
        .with(design::SPACE_L, Length::Px(dpx(pick(32, 28, 24), p.dpi)))
        .with(design::RADIUS_S, Radius::Px(m.radius_default / 2))
        .with(design::RADIUS_M, Radius::Px(m.radius_default))
        .with(design::RADIUS_L, Radius::Px(m.radius_default * 2))
        .with(design::RADIUS_FULL, Radius::Circle)
        .with(design::FOCUS_RING_OPACITY, c.focus_ring_opacity)
        .with(design::SHADOW_OPACITY, c.shadow_opacity)
        .with(design::DISABLED_OPACITY, c.disabled_opacity)
        .with(design::SCRIM_OPACITY, c.scrim_opacity)
        .with(design::FONT_SMALL, p.font_small)
        .with(design::FONT_BODY, p.font_normal)
        .with(design::FONT_LARGE, p.font_large)
}

/// Every style of LVGL's `my_theme_styles_t` that the widgets implemented so far use, under
/// LVGL's names: [`DefaultTheme::styles`](super::DefaultTheme::styles) and
/// [`ClassCx::styles`](crate::ClassCx::styles) give them to custom widgets that should look
/// native. The most useful ones: `card` (containers), `btn` + `bg_color_primary` (buttons),
/// `pressed`, `disabled`, `outline_primary` (keyboard focus), `outline_secondary` (editing),
/// `scrollbar`, `knob`, `circle`, `transition_normal` / `transition_delayed`. Each is built
/// from [design elements](twine_style::design), so it follows the theme mode.
#[derive(Debug)]
pub struct Styles {
    /// Screens: opaque `BACKGROUND`, `ON_SURFACE` text, the normal font, small gaps, rotary
    /// sensitivity.
    pub scr: Rc<StyleBuf>,
    /// Scrollbars (`Part::Scrollbar`): round, `SCROLLBAR` color at 40 %, 5 dp wide, 7 dp from the
    /// edges, with the normal transition.
    pub scrollbar: Rc<StyleBuf>,
    /// Scrollbars while scrolled (`State::SCROLLED`): fully opaque.
    pub scrollbar_scrolled: Rc<StyleBuf>,
    /// Cards and plain containers: `SURFACE` background, `OUTLINE` border drawn above the children,
    /// default radius and padding, small gaps, `NEUTRAL` lines.
    pub card: Rc<StyleBuf>,
    /// Buttons: `SURFACE_VARIANT` background, a `SHADOW` drop shadow (transparent in dark mode),
    /// radius and padding by display size.
    pub btn: Rc<StyleBuf>,
    /// An opaque `PRIMARY` background with `ON_PRIMARY` text (e.g. buttons, checked parts).
    pub bg_color_primary: Rc<StyleBuf>,
    /// A 20 % `PRIMARY` background with `PRIMARY` text (a subtle highlight).
    pub bg_color_primary_muted: Rc<StyleBuf>,
    /// An opaque `SECONDARY` background with `ON_SECONDARY` text (e.g. checked buttons).
    pub bg_color_secondary: Rc<StyleBuf>,
    /// A 20 % `SECONDARY` background with `SECONDARY` text.
    pub bg_color_secondary_muted: Rc<StyleBuf>,
    /// An opaque `SURFACE_VARIANT` background with `ON_SURFACE` text (LVGL's grey).
    pub bg_color_grey: Rc<StyleBuf>,
    /// An opaque `SURFACE` background with `ON_SURFACE` text (LVGL's card color).
    pub bg_color_white: Rc<StyleBuf>,
    /// The pressed look (`State::PRESSED`): a 35/255 black recolor.
    pub pressed: Rc<StyleBuf>,
    /// The disabled look (`State::DISABLED`): a `DISABLED` recolor at `DISABLED_OPACITY`.
    pub disabled: Rc<StyleBuf>,
    /// No padding and no gaps.
    pub pad_zero: Rc<StyleBuf>,
    /// Tiny padding and gaps (by display size).
    pub pad_tiny: Rc<StyleBuf>,
    /// Small padding and gaps (by display size).
    pub pad_small: Rc<StyleBuf>,
    /// The default padding and gaps (by display size).
    pub pad_normal: Rc<StyleBuf>,
    /// Row and column gaps of 10 dp (LVGL's `pad_gap`).
    pub pad_gap: Rc<StyleBuf>,
    /// Text line spacing of 20 dp.
    pub line_space_large: Rc<StyleBuf>,
    /// Centered text.
    pub text_align_center: Rc<StyleBuf>,
    /// The keyboard focus ring (`State::FOCUS_KEY`): a 3 dp `FOCUS_RING` outline, 3 dp off the
    /// node, at `FOCUS_RING_OPACITY`.
    pub outline_primary: Rc<StyleBuf>,
    /// The editing outline (`State::EDITED`): a 3 dp `SECONDARY` outline at `FOCUS_RING_OPACITY`.
    pub outline_secondary: Rc<StyleBuf>,
    /// Fully round corners (`Radius::Circle`).
    pub circle: Rc<StyleBuf>,
    /// Square corners (radius 0).
    pub no_radius: Rc<StyleBuf>,
    /// Children clipped to the rounded corners, border drawn above them.
    pub clip_corner: Rc<StyleBuf>,
    /// The rotary (encoder) scroll sensitivity for the display's DPI.
    pub rotary_scroll: Rc<StyleBuf>,
    /// Grows the node by 3 dp on each side (`transform_width` / `transform_height`; LVGL
    /// `LV_THEME_DEFAULT_GROW`), e.g. while pressed.
    pub grow: Rc<StyleBuf>,
    /// The theme's transition of the background and transform-size properties, after a 70 ms
    /// delay (on the default state: the way back from a pressed look).
    pub transition_delayed: Rc<StyleBuf>,
    /// The theme's transition of the background and transform-size properties, without delay
    /// (on the pressed state: the way into it).
    pub transition_normal: Rc<StyleBuf>,
    /// An animation duration (`anim_duration`) of 200 ms.
    pub anim: Rc<StyleBuf>,
    /// An animation duration (`anim_duration`) of 120 ms.
    pub anim_fast: Rc<StyleBuf>,
    /// Slider and switch knobs: a round, opaque `PRIMARY` knob, 6 dp larger than the track on every
    /// side.
    pub knob: Rc<StyleBuf>,
    /// Arc tracks: 15 dp wide, rounded ends, `SURFACE_VARIANT` color.
    pub arc_indic: Rc<StyleBuf>,
    /// Arc indicators: the `PRIMARY` arc color.
    pub arc_indic_primary: Rc<StyleBuf>,
    /// Checkbox markers: a `PRIMARY` bordered `SURFACE` box with half the default radius and the
    /// small font.
    pub cb_marker: Rc<StyleBuf>,
    /// The checked checkbox marker: the check mark symbol as background image.
    pub cb_marker_checked: Rc<StyleBuf>,
    /// Switch knobs: an `ON_PRIMARY` knob, 4 dp smaller than the track on every side.
    pub switch_knob: Rc<StyleBuf>,
    /// Line widgets: 1 px `ON_SURFACE` lines.
    pub line: Rc<StyleBuf>,
    /// LEDs: round, a white-to-grey gradient and a white glow (15 dp shadow, 5 dp spread).
    pub led: Rc<StyleBuf>,
    /// The text area cursor: a 2 dp `ON_SURFACE` left border, blinking every 400 ms.
    pub ta_cursor: Rc<StyleBuf>,
    /// The text area placeholder: `PLACEHOLDER` text color.
    pub ta_placeholder: Rc<StyleBuf>,
    /// Keyboard keys: no shadow, the default radius (half on small displays).
    pub keyboard_button_bg: Rc<StyleBuf>,
    /// Dropdown lists: at most twice the default DPI (260 px) high.
    pub dropdown_list: Rc<StyleBuf>,
    /// Menus: no padding, gaps, radius or border; children clipped to the corners.
    pub menu_bg: Rc<StyleBuf>,
    /// Menu sections: opaque `SURFACE` background with the default radius, children clipped.
    pub menu_section: Rc<StyleBuf>,
    /// Menu containers (rows): small padding and gaps, a faint `ON_SURFACE` border (10 %, not drawn
    /// by default).
    pub menu_cont: Rc<StyleBuf>,
    /// The menu sidebar: no padding or gaps, a faint right border.
    pub menu_sidebar_cont: Rc<StyleBuf>,
    /// The menu's main area: no padding or gaps.
    pub menu_main_cont: Rc<StyleBuf>,
    /// Menu headers: small horizontal padding and gaps, tiny vertical padding.
    pub menu_header_cont: Rc<StyleBuf>,
    /// Menu header buttons (back): tiny padding, transparent background and shadow, `ON_SURFACE`
    /// text.
    pub menu_header_btn: Rc<StyleBuf>,
    /// Menu pages: no horizontal padding, no gaps.
    pub menu_page: Rc<StyleBuf>,
    /// Pressed menu items: a 20 % `NEUTRAL` background.
    pub menu_pressed: Rc<StyleBuf>,
    /// Menu separators: transparent, tiny vertical padding.
    pub menu_separator: Rc<StyleBuf>,
    /// The modal backdrop of message boxes: `SCRIM` at `SCRIM_OPACITY`.
    pub msgbox_backdrop_bg: Rc<StyleBuf>,
    /// Tab buttons: a bottom `PRIMARY` border twice the default border width (the selected-tab
    /// underline).
    pub tab_btn: Rc<StyleBuf>,
    /// Focused tab bars: the outline drawn inside (a negative outline offset).
    pub tab_bg_focus: Rc<StyleBuf>,
    /// Lists: default horizontal padding, no vertical padding or gaps, children clipped to the
    /// corners.
    pub list_bg: Rc<StyleBuf>,
    /// List buttons: small padding, a 1 dp bottom `OUTLINE` border between items.
    pub list_btn: Rc<StyleBuf>,
    /// List items: wider by the default padding (`transform_width`; applied while pressed or
    /// focused).
    pub list_item_grow: Rc<StyleBuf>,
}

impl Styles {
    /// LVGL `style_init(theme)`.
    #[allow(clippy::too_many_lines)] // one statement per LVGL line
    pub(crate) fn new(p: &Params) -> Self {
        let d = |px| dpx(px, p.dpi);
        let large = p.size == DisplaySize::Large;
        let medium = p.size == DisplaySize::Medium;
        let Metrics {
            radius_default,
            pad_def,
            pad_small: pad_small_v,
            pad_tiny: pad_tiny_v,
        } = Metrics::new(p);
        // #define BORDER_WIDTH LV_DPX_CALC(theme->disp_dpi, 2)
        let border_width = d(2);
        // #define OUTLINE_WIDTH LV_DPX_CALC(theme->disp_dpi, 3)
        let outline_width = d(3);
        // lv_style_set_rotary_sensitivity(..., theme->disp_dpi / 4 * 256)
        let rotary = Scale::from_raw_256(u16::try_from(u32::from(p.dpi) / 4 * 256).unwrap_or(u16::MAX));
        let rc = Rc::new;

        // lv_style_set_transition(&theme->styles.transition_delayed, &theme->trans_delayed)
        let transition_delayed = StyleBuf::new().transition(TransitionValue::Static(&TRANS_DELAYED));
        // lv_style_set_transition(&theme->styles.transition_normal, &theme->trans_normal)
        let transition_normal = StyleBuf::new().transition(TransitionValue::Static(&TRANS_NORMAL));

        let scrollbar = StyleBuf::new()
            .bg_color(design::SCROLLBAR) // lv_style_set_bg_color(&scrollbar, sb_color)
            .radius(Radius::Circle) // lv_style_set_radius(&scrollbar, LV_RADIUS_CIRCLE)
            .padding(d(7)) // lv_style_set_pad_all(&scrollbar, LV_DPX_CALC(dpi, 7))
            .width(d(5)) // lv_style_set_width(&scrollbar, LV_DPX_CALC(dpi, 5))
            .bg_opacity(Opa::P40) // lv_style_set_bg_opa(&scrollbar, LV_OPA_40)
            .transition(TransitionValue::Static(&TRANS_NORMAL)); // lv_style_set_transition(&scrollbar, &trans_normal)

        // lv_style_set_bg_opa(&scrollbar_scrolled, LV_OPA_COVER)
        let scrollbar_scrolled = StyleBuf::new().bg_opacity(Opa::COVER);

        let scr = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&scr, LV_OPA_COVER)
            .bg_color(design::BACKGROUND) // lv_style_set_bg_color(&scr, theme->color_scr)
            .text_color(design::ON_SURFACE) // lv_style_set_text_color(&scr, theme->color_text)
            .font(p.font_normal) // lv_style_set_text_font(&scr, font_normal)
            .gap(pad_small_v) // lv_style_set_pad_row / pad_column(&scr, PAD_SMALL)
            .rotary_sensitivity(rotary); // lv_style_set_rotary_sensitivity(&scr, dpi / 4 * 256)

        let card = StyleBuf::new()
            .radius(radius_default) // lv_style_set_radius(&card, RADIUS_DEFAULT)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&card, LV_OPA_COVER)
            .bg_color(design::SURFACE) // lv_style_set_bg_color(&card, theme->color_card)
            .border_color(design::OUTLINE) // lv_style_set_border_color(&card, theme->color_grey)
            .border_width(border_width) // lv_style_set_border_width(&card, BORDER_WIDTH)
            .border_above_children(true) // lv_style_set_border_post(&card, true)
            .text_color(design::ON_SURFACE) // lv_style_set_text_color(&card, theme->color_text)
            .padding(pad_def) // lv_style_set_pad_all(&card, PAD_DEF)
            .gap(pad_small_v) // lv_style_set_pad_row / pad_column(&card, PAD_SMALL)
            .line_color(design::NEUTRAL) // lv_style_set_line_color(&card, lv_palette_main(GREY))
            .line_width(d(1)); // lv_style_set_line_width(&card, LV_DPX_CALC(dpi, 1))

        let outline_primary = StyleBuf::new()
            .outline_color(design::FOCUS_RING) // lv_style_set_outline_color(&outline_primary, color_primary)
            .outline_width(outline_width) // lv_style_set_outline_width(&outline_primary, OUTLINE_WIDTH)
            .outline_offset(outline_width) // lv_style_set_outline_pad(&outline_primary, OUTLINE_WIDTH)
            .outline_opacity(design::FOCUS_RING_OPACITY); // lv_style_set_outline_opa(&outline_primary, LV_OPA_50)

        let outline_secondary = StyleBuf::new()
            .outline_color(design::SECONDARY) // lv_style_set_outline_color(&outline_secondary, color_secondary)
            .outline_width(outline_width) // lv_style_set_outline_width(&outline_secondary, OUTLINE_WIDTH)
            .outline_opacity(design::FOCUS_RING_OPACITY); // lv_style_set_outline_opa(&outline_secondary, LV_OPA_50)

        // lv_style_set_radius(&btn, LV_DPX_CALC(dpi, LARGE ? 16 : MEDIUM ? 12 : 8))
        let btn_radius = d(if large {
            16
        } else if medium {
            12
        } else {
            8
        });
        let btn = StyleBuf::new()
            .radius(btn_radius)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&btn, LV_OPA_COVER)
            .bg_color(design::SURFACE_VARIANT) // lv_style_set_bg_color(&btn, theme->color_grey)
            // `if(!theme->dark)`: the shadow is set in both modes; `SHADOW_OPACITY` is
            // transparent in dark mode (no shadow drawn, no extra draw area).
            .shadow_color(design::SHADOW) // lv_style_set_shadow_color(&btn, lv_palette_main(GREY))
            .shadow_width(d(3)) // lv_style_set_shadow_width(&btn, LV_DPX_CALC(dpi, 3))
            .shadow_opacity(design::SHADOW_OPACITY) // lv_style_set_shadow_opa(&btn, LV_OPA_50)
            .shadow_offset_y(d(3)) // lv_style_set_shadow_offset_y(&btn, LV_DPX_CALC(dpi, 3))
            .text_color(design::ON_SURFACE) // lv_style_set_text_color(&btn, theme->color_text)
            .padding_x(pad_def) // lv_style_set_pad_hor(&btn, PAD_DEF)
            .padding_y(pad_small_v) // lv_style_set_pad_ver(&btn, PAD_SMALL)
            .column_gap(d(5)) // lv_style_set_pad_column(&btn, LV_DPX_CALC(dpi, 5))
            .row_gap(d(5)); // lv_style_set_pad_row(&btn, LV_DPX_CALC(dpi, 5))

        let pressed = StyleBuf::new()
            .recolor(Color::BLACK) // lv_style_set_recolor(&pressed, lv_color_black())
            .recolor_opacity(Opa::from_raw(35)); // lv_style_set_recolor_opa(&pressed, 35)

        let disabled = StyleBuf::new()
            // dark ? lv_palette_darken(GREY, 2) : lv_palette_lighten(GREY, 2)
            .recolor(design::DISABLED)
            .recolor_opacity(design::DISABLED_OPACITY); // lv_style_set_recolor_opa(&disabled, LV_OPA_50)

        let clip_corner = StyleBuf::new()
            .clip_corner(true) // lv_style_set_clip_corner(&clip_corner, true)
            .border_above_children(true); // lv_style_set_border_post(&clip_corner, true)

        // lv_style_set_pad_all / pad_row / pad_column(&pad_normal, PAD_DEF)
        let pad_normal = StyleBuf::new().padding(pad_def).gap(pad_def);
        // lv_style_set_pad_all / pad_gap(&pad_small, PAD_SMALL)
        let pad_small = StyleBuf::new().padding(pad_small_v).gap(pad_small_v);
        // lv_style_set_pad_row / pad_column(&pad_gap, LV_DPX_CALC(dpi, 10))
        let pad_gap_s = StyleBuf::new().gap(d(10));
        // lv_style_set_text_line_space(&line_space_large, LV_DPX_CALC(dpi, 20))
        let line_space_large = StyleBuf::new().line_spacing(d(20));
        // lv_style_set_text_align(&text_align_center, LV_TEXT_ALIGN_CENTER)
        let text_align_center = StyleBuf::new().text_align(TextAlign::Center);
        // lv_style_set_pad_all / pad_row / pad_column(&pad_zero, 0)
        let pad_zero = StyleBuf::new().padding(0).gap(0);
        // lv_style_set_pad_all / pad_row / pad_column(&pad_tiny, PAD_TINY)
        let pad_tiny = StyleBuf::new().padding(pad_tiny_v).gap(pad_tiny_v);

        let bg_color_primary = StyleBuf::new()
            .bg_color(design::PRIMARY) // lv_style_set_bg_color(&bg_color_primary, color_primary)
            .text_color(design::ON_PRIMARY) // lv_style_set_text_color(&bg_color_primary, lv_color_white())
            .bg_opacity(Opa::COVER); // lv_style_set_bg_opa(&bg_color_primary, LV_OPA_COVER)
        let bg_color_primary_muted = StyleBuf::new()
            .bg_color(design::PRIMARY) // lv_style_set_bg_color(&bg_color_primary_muted, color_primary)
            .text_color(design::PRIMARY) // lv_style_set_text_color(&bg_color_primary_muted, color_primary)
            .bg_opacity(Opa::P20); // lv_style_set_bg_opa(&bg_color_primary_muted, LV_OPA_20)
        let bg_color_secondary = StyleBuf::new()
            .bg_color(design::SECONDARY) // lv_style_set_bg_color(&bg_color_secondary, color_secondary)
            .text_color(design::ON_SECONDARY) // lv_style_set_text_color(&bg_color_secondary, lv_color_white())
            .bg_opacity(Opa::COVER); // lv_style_set_bg_opa(&bg_color_secondary, LV_OPA_COVER)
        let bg_color_secondary_muted = StyleBuf::new()
            .bg_color(design::SECONDARY) // lv_style_set_bg_color(&bg_color_secondary_muted, color_secondary)
            .text_color(design::SECONDARY) // lv_style_set_text_color(&bg_color_secondary_muted, color_secondary)
            .bg_opacity(Opa::P20); // lv_style_set_bg_opa(&bg_color_secondary_muted, LV_OPA_20)
        let bg_color_grey = StyleBuf::new()
            .bg_color(design::SURFACE_VARIANT) // lv_style_set_bg_color(&bg_color_grey, theme->color_grey)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&bg_color_grey, LV_OPA_COVER)
            .text_color(design::ON_SURFACE); // lv_style_set_text_color(&bg_color_grey, theme->color_text)
        let bg_color_white = StyleBuf::new()
            .bg_color(design::SURFACE) // lv_style_set_bg_color(&bg_color_white, theme->color_card)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&bg_color_white, LV_OPA_COVER)
            .text_color(design::ON_SURFACE); // lv_style_set_text_color(&bg_color_white, theme->color_text)

        // lv_style_set_radius(&circle, LV_RADIUS_CIRCLE)
        let circle = StyleBuf::new().radius(Radius::Circle);
        // lv_style_set_radius(&no_radius, 0)
        let no_radius = StyleBuf::new().radius(0);
        // lv_style_set_rotary_sensitivity(&rotary_scroll, theme->disp_dpi / 4 * 256)
        let rotary_scroll = StyleBuf::new().rotary_sensitivity(rotary);
        // LV_THEME_DEFAULT_GROW: lv_style_set_transform_width / height(&grow, LV_DPX_CALC(dpi, 3))
        let grow = StyleBuf::new().transform_width(d(3)).transform_height(d(3));

        let knob = StyleBuf::new()
            .bg_color(design::PRIMARY) // lv_style_set_bg_color(&knob, color_primary)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&knob, LV_OPA_COVER)
            .padding(d(6)) // lv_style_set_pad_all(&knob, LV_DPX_CALC(dpi, 6))
            .radius(Radius::Circle); // lv_style_set_radius(&knob, LV_RADIUS_CIRCLE)

        // lv_style_set_anim_duration(&anim, 200)
        let anim = StyleBuf::new().anim_duration(Duration::ms(200));
        // lv_style_set_anim_duration(&anim_fast, 120)
        let anim_fast = StyleBuf::new().anim_duration(Duration::ms(120));

        // #if LV_USE_ARC (lines 398-406)
        let arc_indic = StyleBuf::new()
            .arc_color(design::SURFACE_VARIANT) // lv_style_set_arc_color(&arc_indic, theme->color_grey)
            .arc_width(d(15)) // lv_style_set_arc_width(&arc_indic, LV_DPX_CALC(dpi, 15))
            .arc_rounded(true); // lv_style_set_arc_rounded(&arc_indic, true)
        // lv_style_set_arc_color(&arc_indic_primary, theme->base.color_primary)
        let arc_indic_primary = StyleBuf::new().arc_color(design::PRIMARY);

        // #if LV_USE_CHECKBOX (lines 412-425)
        let cb_marker = StyleBuf::new()
            .padding(d(3)) // lv_style_set_pad_all(&cb_marker, LV_DPX_CALC(dpi, 3))
            .border_width(border_width) // lv_style_set_border_width(&cb_marker, BORDER_WIDTH)
            .border_color(design::PRIMARY) // lv_style_set_border_color(&cb_marker, color_primary)
            .bg_color(design::SURFACE) // lv_style_set_bg_color(&cb_marker, theme->color_card)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&cb_marker, LV_OPA_COVER)
            .radius(radius_default / 2) // lv_style_set_radius(&cb_marker, RADIUS_DEFAULT / 2)
            .font(p.font_small) // lv_style_set_text_font(&cb_marker, theme->base.font_small)
            .text_color(design::ON_PRIMARY); // lv_style_set_text_color(&cb_marker, lv_color_white())
        // lv_style_set_bg_image_src(&cb_marker_checked, LV_SYMBOL_OK)
        let cb_marker_checked = StyleBuf::new().bg_image(&CHECK_MARK);

        // #if LV_USE_SWITCH (lines 427-431)
        let switch_knob = StyleBuf::new()
            .padding(-d(4)) // lv_style_set_pad_all(&switch_knob, -LV_DPX_CALC(dpi, 4))
            // lv_style_set_bg_color(&switch_knob, lv_color_white()): `ON_PRIMARY` is white in
            // light and dark mode and follows the night and high-contrast palettes.
            .bg_color(design::ON_PRIMARY);

        // #if LV_USE_LINE (lines 433-437)
        let line = StyleBuf::new()
            .line_width(1) // lv_style_set_line_width(&line, 1)
            .line_color(design::ON_SURFACE); // lv_style_set_line_color(&line, theme->color_text)

        // #if LV_USE_LED (lines 601-610)
        let led = StyleBuf::new()
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&led, LV_OPA_COVER)
            .bg_color(Color::WHITE) // lv_style_set_bg_color(&led, lv_color_white())
            .bg_gradient_color(Palette::Grey.main()) // lv_style_set_bg_grad_color(&led, lv_palette_main(GREY))
            .radius(Radius::Circle) // lv_style_set_radius(&led, LV_RADIUS_CIRCLE)
            .shadow_width(d(15)) // lv_style_set_shadow_width(&led, LV_DPX_CALC(dpi, 15))
            .shadow_color(Color::WHITE) // lv_style_set_shadow_color(&led, lv_color_white())
            .shadow_spread(d(5)); // lv_style_set_shadow_spread(&led, LV_DPX_CALC(dpi, 5))

        // #if LV_USE_TEXTAREA (lines 528-540)
        let ta_cursor = StyleBuf::new()
            .border_color(design::ON_SURFACE) // lv_style_set_border_color(&ta_cursor, theme->color_text)
            .border_width(d(2)) // lv_style_set_border_width(&ta_cursor, LV_DPX_CALC(dpi, 2))
            .padding_left(-d(1)) // lv_style_set_pad_left(&ta_cursor, -LV_DPX_CALC(dpi, 1))
            .border_side(BorderSide::LEFT) // lv_style_set_border_side(&ta_cursor, LV_BORDER_SIDE_LEFT)
            .anim_duration(Duration::ms(400)); // lv_style_set_anim_duration(&ta_cursor, 400)
        // lv_style_set_text_color(&ta_placeholder, dark ? darken(GREY, 2) : lighten(GREY, 1))
        let ta_placeholder = StyleBuf::new().text_color(design::PLACEHOLDER);

        // #if LV_USE_KEYBOARD (lines 565-569)
        let keyboard_button_bg = StyleBuf::new()
            .shadow_width(0) // lv_style_set_shadow_width(&keyboard_button_bg, 0)
            // lv_style_set_radius(&keyboard_button_bg, SMALL ? RADIUS_DEFAULT / 2 : RADIUS_DEFAULT)
            .radius(if p.size == DisplaySize::Small {
                radius_default / 2
            } else {
                radius_default
            });

        // #if LV_USE_DROPDOWN (lines 408-411)
        // lv_style_set_max_height(&dropdown_list, LV_DPI_DEF * 2): a constant, not scaled
        let dropdown_list = StyleBuf::new().max_height(i32::from(DEFAULT_DPI) * 2);

        // #if LV_USE_MENU (lines 460-519)
        let menu_bg = StyleBuf::new()
            .padding(0)
            .gap(0) // lv_style_set_pad_all / &menu_bg.gap(0)
            .radius(0) // lv_style_set_radius(&menu_bg, 0)
            .clip_corner(true) // lv_style_set_clip_corner(&menu_bg, true)
            .border_side(BorderSide::NONE); // lv_style_set_border_side(&menu_bg, LV_BORDER_SIDE_NONE)
        let menu_section = StyleBuf::new()
            .radius(radius_default) // lv_style_set_radius(&menu_section, RADIUS_DEFAULT)
            .clip_corner(true) // lv_style_set_clip_corner(&menu_section, true)
            .bg_opacity(Opa::COVER) // lv_style_set_bg_opa(&menu_section, LV_OPA_COVER)
            .bg_color(design::SURFACE) // lv_style_set_bg_color(&menu_section, theme->color_card)
            .text_color(design::ON_SURFACE); // lv_style_set_text_color(&menu_section, theme->color_text)
        // lv_style_set_pad_hor / pad_ver / pad_gap(&menu_cont, PAD_SMALL)
        let menu_cont = StyleBuf::new()
            .padding(pad_small_v)
            .gap(pad_small_v)
            .border_width(d(1)) // lv_style_set_border_width(&menu_cont, LV_DPX_CALC(dpi, 1))
            .border_opacity(Opa::P10) // lv_style_set_border_opa(&menu_cont, LV_OPA_10)
            .border_color(design::ON_SURFACE) // lv_style_set_border_color(&menu_cont, theme->color_text)
            .border_side(BorderSide::NONE); // lv_style_set_border_side(&menu_cont, LV_BORDER_SIDE_NONE)
        let menu_sidebar_cont = StyleBuf::new()
            .padding(0)
            .gap(0) // lv_style_set_pad_all / pad_gap(&menu_sidebar_cont, 0)
            .border_width(d(1)) // lv_style_set_border_width(&menu_sidebar_cont, LV_DPX_CALC(dpi, 1))
            .border_opacity(Opa::P10) // lv_style_set_border_opa(&menu_sidebar_cont, LV_OPA_10)
            .border_color(design::ON_SURFACE) // lv_style_set_border_color(&menu_sidebar_cont, theme->color_text)
            .border_side(BorderSide::RIGHT); // lv_style_set_border_side(&menu_sidebar_cont, LV_BORDER_SIDE_RIGHT)
        // lv_style_set_pad_all / pad_gap(&menu_main_cont, 0)
        let menu_main_cont = StyleBuf::new().padding(0).gap(0);
        // lv_style_set_pad_hor(&menu_header_cont, PAD_SMALL), pad_ver(PAD_TINY), pad_gap(PAD_SMALL)
        let menu_header_cont = StyleBuf::new()
            .gap(pad_small_v)
            .padding_x(pad_small_v)
            .padding_y(pad_tiny_v);
        // lv_style_set_pad_hor / pad_ver(&menu_header_btn, PAD_TINY)
        let menu_header_btn = StyleBuf::new()
            .padding(pad_tiny_v)
            .shadow_opacity(Opa::TRANSP) // lv_style_set_shadow_opa(&menu_header_btn, LV_OPA_TRANSP)
            .bg_opacity(Opa::TRANSP) // lv_style_set_bg_opa(&menu_header_btn, LV_OPA_TRANSP)
            .text_color(design::ON_SURFACE); // lv_style_set_text_color(&menu_header_btn, theme->color_text)
        // lv_style_set_pad_hor(&menu_page, 0), pad_gap(0)
        let menu_page = StyleBuf::new().gap(0).padding_x(0);
        let menu_pressed = StyleBuf::new()
            .bg_opacity(Opa::P20) // lv_style_set_bg_opa(&menu_pressed, LV_OPA_20)
            .bg_color(design::NEUTRAL); // lv_style_set_bg_color(&menu_pressed, lv_palette_main(GREY))
        let menu_separator = StyleBuf::new()
            .bg_opacity(Opa::TRANSP) // lv_style_set_bg_opa(&menu_separator, LV_OPA_TRANSP)
            .padding_y(pad_tiny_v); // lv_style_set_pad_ver(&menu_separator, PAD_TINY)

        // #if LV_USE_MSGBOX (lines 560-564)
        let msgbox_backdrop_bg = StyleBuf::new()
            .bg_color(design::SCRIM) // lv_style_set_bg_color(&msgbox_backdrop_bg, lv_palette_main(GREY))
            .bg_opacity(design::SCRIM_OPACITY); // lv_style_set_bg_opa(&msgbox_backdrop_bg, LV_OPA_50)

        // #if LV_USE_TABVIEW (lines 572-581)
        let tab_btn = StyleBuf::new()
            .border_color(design::PRIMARY) // lv_style_set_border_color(&tab_btn, color_primary)
            .border_width(border_width * 2) // lv_style_set_border_width(&tab_btn, BORDER_WIDTH * 2)
            .border_side(BorderSide::BOTTOM) // lv_style_set_border_side(&tab_btn, LV_BORDER_SIDE_BOTTOM)
            .padding_top(border_width * 2); // lv_style_set_pad_top(&tab_btn, BORDER_WIDTH * 2)
        // lv_style_set_outline_pad(&tab_bg_focus, -BORDER_WIDTH)
        let tab_bg_focus = StyleBuf::new().outline_offset(-border_width);

        // #if LV_USE_LIST (lines 583-598)
        let list_bg = StyleBuf::new()
            .gap(0) // lv_style_set_pad_gap(&list_bg, 0)
            .padding_x(pad_def) // lv_style_set_pad_hor(&list_bg, PAD_DEF)
            .padding_y(0) // lv_style_set_pad_ver(&list_bg, 0)
            .clip_corner(true); // lv_style_set_clip_corner(&list_bg, true)
        let list_btn = StyleBuf::new()
            .padding(pad_small_v) // lv_style_set_pad_all(&list_btn, PAD_SMALL)
            .border_width(d(1)) // lv_style_set_border_width(&list_btn, LV_DPX_CALC(dpi, 1))
            .border_color(design::OUTLINE) // lv_style_set_border_color(&list_btn, theme->color_grey)
            .border_side(BorderSide::BOTTOM) // lv_style_set_border_side(&list_btn, LV_BORDER_SIDE_BOTTOM)
            .column_gap(pad_small_v); // lv_style_set_pad_column(&list_btn, PAD_SMALL)
        // lv_style_set_transform_width(&list_item_grow, PAD_DEF)
        let list_item_grow = StyleBuf::new().transform_width(pad_def);

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
            dropdown_list: rc(dropdown_list),
            menu_bg: rc(menu_bg),
            menu_section: rc(menu_section),
            menu_cont: rc(menu_cont),
            menu_sidebar_cont: rc(menu_sidebar_cont),
            menu_main_cont: rc(menu_main_cont),
            menu_header_cont: rc(menu_header_cont),
            menu_header_btn: rc(menu_header_btn),
            menu_page: rc(menu_page),
            menu_pressed: rc(menu_pressed),
            menu_separator: rc(menu_separator),
            msgbox_backdrop_bg: rc(msgbox_backdrop_bg),
            tab_btn: rc(tab_btn),
            tab_bg_focus: rc(tab_bg_focus),
            list_bg: rc(list_bg),
            list_btn: rc(list_btn),
            list_item_grow: rc(list_item_grow),
        }
    }
}
