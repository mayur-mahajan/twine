//! [`DefaultTheme`]: a faithful port of LVGL's default theme
//! (`src/themes/default/lv_theme_default.c`), light and dark.
//!
//! The styles are LVGL's `style_init()` (see [`styles`]); [`DefaultTheme`]'s
//! [`ThemeHook::apply`] is LVGL's `theme_apply()` for the widget classes that exist. Like
//! LVGL, sizes depend on the display: every pixel value is scaled with [`dpx`](twine_style::dpx) by
//! the display's DPI, and radii and paddings follow the display size class
//! ([`DisplaySize`]: the larger side ≤ 320 px is small, < 720 px medium, else large).

pub mod colors;
pub mod styles;

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_core::{Color, Size};
use twine_engine::{ThemeCx, ThemeHook, WidgetClass};
use twine_style::{Part, Selector, State};
use twine_text::Font;

use crate::{Palette, Theme};
use styles::{Params, Styles};

/// Light or dark variant of the [`DefaultTheme`] (LVGL's `dark` flag).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThemeMode {
    /// Light screens and cards, dark text.
    #[default]
    Light,
    /// Dark screens and cards, light text, no button shadows.
    Dark,
}

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

/// LVGL's default theme: material colors, rounded white (or dark) cards with a grey border,
/// primary-colored buttons with a shadow that darken when pressed and turn secondary when
/// checked, 50 % outlines on keyboard focus, and short transitions.
///
/// Style sets are built on first use for each (DPI, display size) combination (usually one)
/// and shared by every node through `Rc`.
///
/// ```
/// use twine_core::Color;
/// use twine_theme::{DefaultTheme, Palette, ThemeHook, ThemeMode};
///
/// let dark = DefaultTheme::dark().with_dpi(160);
/// assert_eq!(dark.mode(), ThemeMode::Dark);
/// assert_eq!(dark.color_primary(), Palette::Blue.main());
/// ```
pub struct DefaultTheme {
    primary: Color,
    secondary: Color,
    mode: ThemeMode,
    font_small: &'static Font,
    font_normal: &'static Font,
    font_large: &'static Font,
    dpi: Option<u16>,
    size: Option<DisplaySize>,
    styles: RefCell<Vec<(u16, DisplaySize, Rc<Styles>)>>,
    parent: Option<Rc<dyn Theme>>,
}

impl core::fmt::Debug for DefaultTheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DefaultTheme")
            .field("primary", &self.primary)
            .field("secondary", &self.secondary)
            .field("mode", &self.mode)
            .field("dpi", &self.dpi)
            .field("size", &self.size)
            .field("parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

impl DefaultTheme {
    /// A default theme with `primary` / `secondary` palette colors, `mode` and `font` as the
    /// small, normal and large font (LVGL `lv_theme_default_init`).
    #[must_use]
    pub fn new(primary: Palette, secondary: Palette, mode: ThemeMode, font: &'static Font) -> Self {
        Self::with_colors(primary.main(), secondary.main(), mode, font)
    }

    /// Like [`new`](Self::new) with any colors.
    #[must_use]
    pub fn with_colors(primary: Color, secondary: Color, mode: ThemeMode, font: &'static Font) -> Self {
        Self {
            primary,
            secondary,
            mode,
            font_small: font,
            font_normal: font,
            font_large: font,
            dpi: None,
            size: None,
            styles: RefCell::new(Vec::new()),
            parent: None,
        }
    }

    /// Blue / red, light, Montserrat 14 (LVGL's defaults: `LV_THEME_DEFAULT_DARK 0`,
    /// `lv_palette_main(LV_PALETTE_BLUE)`, `lv_palette_main(LV_PALETTE_RED)`,
    /// `LV_FONT_DEFAULT`).
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn light() -> Self {
        Self::new(
            Palette::Blue,
            Palette::Red,
            ThemeMode::Light,
            &twine_assets::fonts::MONTSERRAT_14,
        )
    }

    /// Blue / red, dark, Montserrat 14.
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn dark() -> Self {
        Self::new(
            Palette::Blue,
            Palette::Red,
            ThemeMode::Dark,
            &twine_assets::fonts::MONTSERRAT_14,
        )
    }

    /// With separate small, normal and large fonts.
    #[must_use]
    pub fn with_fonts(mut self, small: &'static Font, normal: &'static Font, large: &'static Font) -> Self {
        self.font_small = small;
        self.font_normal = normal;
        self.font_large = large;
        self.styles.get_mut().clear();
        self
    }

    /// Scales for `dpi` instead of each display's own DPI (LVGL uses the display's,
    /// [`DEFAULT_DPI`](twine_style::DEFAULT_DPI) = 130 by default).
    #[must_use]
    pub fn with_dpi(mut self, dpi: u16) -> Self {
        self.dpi = Some(dpi.max(1));
        self.styles.get_mut().clear();
        self
    }

    /// Uses the size class `size` instead of deriving it from each display's resolution.
    #[must_use]
    pub fn with_display_size(mut self, size: DisplaySize) -> Self {
        self.size = Some(size);
        self.styles.get_mut().clear();
        self
    }

    /// Applies `parent` before this theme on every node (LVGL `lv_theme_set_parent`): the
    /// parent's styles come first, so this theme's styles win.
    #[must_use]
    pub fn with_parent(mut self, parent: Rc<dyn Theme>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Light or dark.
    #[must_use]
    pub fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// The display size class the styles of `cx`'s display use.
    fn size_of(&self, cx: &ThemeCx<'_>) -> DisplaySize {
        self.size.unwrap_or_else(|| DisplaySize::of(cx.resolution()))
    }

    /// The styles for a display with `dpi` and `resolution` (built on first use).
    #[must_use]
    pub fn styles(&self, dpi: u16, resolution: Size) -> Rc<Styles> {
        let dpi = self.dpi.unwrap_or(dpi);
        let size = self.size.unwrap_or_else(|| DisplaySize::of(resolution));
        let mut cache = self.styles.borrow_mut();
        if let Some((_, _, s)) = cache.iter().find(|(d, z, _)| *d == dpi && *z == size) {
            return s.clone();
        }
        let s = Rc::new(Styles::new(&Params {
            primary: self.primary,
            secondary: self.secondary,
            mode: self.mode,
            dpi,
            size,
            font_small: self.font_small,
            font_normal: self.font_normal,
        }));
        twine_core::debug!(target: "twine::style", "default theme: styles for {} dpi, {:?}", dpi, size);
        cache.push((dpi, size, s.clone()));
        s
    }
}

impl ThemeHook for DefaultTheme {
    /// LVGL `theme_apply()` of `lv_theme_default.c`.
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if let Some(p) = &self.parent {
            p.apply(cx, class);
        }
        let s = self.styles(cx.dpi(), cx.resolution());
        let scrollbar = Selector::part(Part::Scrollbar);
        let scrolled = Selector::part(Part::Scrollbar).with_state(State::SCROLLED);
        if cx.parent().is_none() {
            // Screens.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            return;
        }
        match class.name {
            // Lines 758-808: tabview pages (the content and the bar have their own classes).
            "obj" if cx.grandparent_class().is_some_and(|c| c.name == "tabview") => {
                cx.add_style(Selector::MAIN, s.pad_normal.clone());
                cx.add_style(Selector::MAIN, s.rotary_scroll.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "obj" => {
                // NOTE(P20.S03): calendar children branch before the card styles here, as in
                // LVGL.
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            // Lines 815-826: a button in a tabview's bar.
            "button" if cx.parent_class().is_some_and(|c| c.name == "tabview_tab_bar") => {
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
                cx.add_style(Selector::state(State::CHECKED), s.bg_color_primary_muted.clone());
                cx.add_style(Selector::state(State::CHECKED), s.tab_btn.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.tab_bg_focus.clone());
            }
            "button" => {
                cx.add_style(Selector::MAIN, s.btn.clone());
                cx.add_style(Selector::MAIN, s.bg_color_primary.clone());
                cx.add_style(Selector::MAIN, s.transition_delayed.clone());
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
                cx.add_style(Selector::state(State::PRESSED), s.transition_normal.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                // LV_THEME_DEFAULT_GROW
                cx.add_style(Selector::state(State::PRESSED), s.grow.clone());
                cx.add_style(Selector::state(State::CHECKED), s.bg_color_secondary.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
                // Lines 840-845: the back buttons of a menu's headers.
                let menu_header = cx.parent_class().is_some_and(|c| {
                    matches!(
                        c.name,
                        "menu_sidebar_header_container" | "menu_main_header_container"
                    )
                });
                if menu_header {
                    cx.add_style(Selector::MAIN, s.menu_header_btn.clone());
                    cx.add_style(Selector::state(State::PRESSED), s.menu_pressed.clone());
                }
            }
            // Lines 1092-1111.
            "list" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.list_bg.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "list_text" => {
                cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
                cx.add_style(Selector::MAIN, s.list_item_grow.clone());
            }
            "list_button" => {
                cx.add_style(Selector::MAIN, s.bg_color_white.clone());
                cx.add_style(Selector::MAIN, s.list_btn.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.bg_color_primary.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.list_item_grow.clone());
                cx.add_style(Selector::state(State::PRESSED), s.list_item_grow.clone());
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
            }
            // Lines 1113-1149.
            "menu" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.menu_bg.clone());
            }
            "menu_sidebar_container" => {
                cx.add_style(Selector::MAIN, s.menu_sidebar_cont.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "menu_main_container" => {
                cx.add_style(Selector::MAIN, s.menu_main_cont.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "menu_cont" => {
                cx.add_style(Selector::MAIN, s.menu_cont.clone());
                cx.add_style(Selector::state(State::PRESSED), s.menu_pressed.clone());
                cx.add_style(
                    Selector::state(State::PRESSED | State::CHECKED),
                    s.bg_color_primary_muted.clone(),
                );
                cx.add_style(Selector::state(State::CHECKED), s.bg_color_primary_muted.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.bg_color_primary.clone());
            }
            "menu_sidebar_header_container" | "menu_main_header_container" => {
                cx.add_style(Selector::MAIN, s.menu_header_cont.clone());
            }
            "menu_page" => {
                cx.add_style(Selector::MAIN, s.menu_page.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "menu_section" => cx.add_style(Selector::MAIN, s.menu_section.clone()),
            "menu_separator" => cx.add_style(Selector::MAIN, s.menu_separator.clone()),
            // Lines 1151-1189.
            "msgbox" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_zero.clone());
                cx.add_style(Selector::MAIN, s.clip_corner.clone());
            }
            "msgbox_backdrop" => cx.add_style(Selector::MAIN, s.msgbox_backdrop_bg.clone()),
            "msgbox_header" => {
                cx.add_style(Selector::MAIN, s.pad_small.clone());
                cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
            }
            "msgbox_footer" => cx.add_style(Selector::MAIN, s.pad_small.clone()),
            "msgbox_content" => {
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
                cx.add_style(Selector::MAIN, s.pad_small.clone());
            }
            "msgbox_header_button" | "msgbox_footer_button" => {
                cx.add_style(Selector::MAIN, s.btn.clone());
                cx.add_style(Selector::MAIN, s.bg_color_primary.clone());
                cx.add_style(Selector::MAIN, s.transition_delayed.clone());
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
                cx.add_style(Selector::state(State::PRESSED), s.transition_normal.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::CHECKED), s.bg_color_secondary.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
            }
            // Lines 1200-1210.
            "tileview" => {
                cx.add_style(Selector::MAIN, s.scr.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            "tileview_tile" => {
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            // Lines 1212-1217 and 760-771: the tabview, its bar (the content has no style).
            "tabview" => {
                cx.add_style(Selector::MAIN, s.scr.clone());
                cx.add_style(Selector::MAIN, s.pad_zero.clone());
            }
            "tabview_tab_bar" => {
                cx.add_style(Selector::MAIN, s.bg_color_white.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.tab_bg_focus.clone());
            }
            // Lines 783-797 and 1219-1223: the window, its header and content.
            "win" => cx.add_style(Selector::MAIN, s.clip_corner.clone()),
            "win_header" => {
                cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
                cx.add_style(Selector::MAIN, s.pad_tiny.clone());
            }
            "win_content" => {
                cx.add_style(Selector::MAIN, s.scr.clone());
                cx.add_style(Selector::MAIN, s.pad_normal.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            }
            // Lines 850-854.
            "line" => cx.add_style(Selector::MAIN, s.line.clone()),
            // Lines 884-894.
            "bar" => {
                cx.add_style(Selector::MAIN, s.bg_color_primary_muted.clone());
                cx.add_style(Selector::MAIN, s.circle.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::part(Part::Indicator), s.bg_color_primary.clone());
                cx.add_style(Selector::part(Part::Indicator), s.circle.clone());
            }
            // Lines 895-911.
            "slider" => {
                let knob = Selector::part(Part::Knob);
                cx.add_style(Selector::MAIN, s.bg_color_primary_muted.clone());
                cx.add_style(Selector::MAIN, s.circle.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::part(Part::Indicator), s.bg_color_primary.clone());
                cx.add_style(Selector::part(Part::Indicator), s.circle.clone());
                cx.add_style(knob, s.knob.clone());
                // LV_THEME_DEFAULT_GROW
                cx.add_style(knob.with_state(State::PRESSED), s.grow.clone());
                cx.add_style(knob, s.transition_delayed.clone());
                cx.add_style(knob.with_state(State::PRESSED), s.transition_normal.clone());
            }
            // Lines 930-947.
            "checkbox" => {
                let ind = Selector::part(Part::Indicator);
                cx.add_style(Selector::MAIN, s.pad_gap.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(ind.with_state(State::DISABLED), s.disabled.clone());
                cx.add_style(ind, s.cb_marker.clone());
                cx.add_style(ind.with_state(State::CHECKED), s.bg_color_primary.clone());
                cx.add_style(ind.with_state(State::CHECKED), s.cb_marker_checked.clone());
                cx.add_style(ind.with_state(State::PRESSED), s.pressed.clone());
                // LV_THEME_DEFAULT_GROW
                cx.add_style(ind.with_state(State::PRESSED), s.grow.clone());
                cx.add_style(ind.with_state(State::PRESSED), s.transition_normal.clone());
                cx.add_style(ind, s.transition_delayed.clone());
            }
            // Lines 947-964.
            "switch" => {
                let ind = Selector::part(Part::Indicator);
                let knob = Selector::part(Part::Knob);
                cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
                cx.add_style(Selector::MAIN, s.circle.clone());
                cx.add_style(Selector::MAIN, s.anim_fast.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(ind.with_state(State::CHECKED), s.bg_color_primary.clone());
                cx.add_style(ind, s.circle.clone());
                cx.add_style(knob, s.knob.clone());
                cx.add_style(knob, s.bg_color_white.clone());
                cx.add_style(knob, s.switch_knob.clone());
                cx.add_style(ind.with_state(State::CHECKED), s.transition_normal.clone());
                cx.add_style(ind, s.transition_normal.clone());
            }
            // Lines 1015-1025.
            "arc" => {
                cx.add_style(Selector::MAIN, s.arc_indic.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_indic.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_indic_primary.clone());
                cx.add_style(Selector::part(Part::Knob), s.knob.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
            }
            // Lines 1026-1032.
            "spinner" => {
                cx.add_style(Selector::MAIN, s.arc_indic.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_indic.clone());
                cx.add_style(Selector::part(Part::Indicator), s.arc_indic_primary.clone());
            }
            // Lines 1225-1229.
            "led" => cx.add_style(Selector::MAIN, s.led.clone()),
            // Lines 856-883.
            "buttonmatrix" => {
                // NOTE(P20.S03): calendar button matrices branch here, as in LVGL.
                let items = Selector::part(Part::Items);
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(items, s.btn.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pressed.clone());
                cx.add_style(items.with_state(State::CHECKED), s.bg_color_primary.clone());
                cx.add_style(items.with_state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(items.with_state(State::EDITED), s.outline_secondary.clone());
            }
            // Lines 1034-1046.
            "textarea" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_small.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
                cx.add_style(
                    Selector::part(Part::Cursor).with_state(State::FOCUSED),
                    s.ta_cursor.clone(),
                );
                // LV_PART_TEXTAREA_PLACEHOLDER = LV_PART_CUSTOM_FIRST
                cx.add_style(Selector::part(Part::CustomFirst), s.ta_placeholder.clone());
            }
            // Lines 1069-1084.
            "keyboard" => {
                let items = Selector::part(Part::Items);
                cx.add_style(Selector::MAIN, s.scr.clone());
                let pad = if self.size_of(cx) == DisplaySize::Large {
                    s.pad_small.clone()
                } else {
                    s.pad_tiny.clone()
                };
                cx.add_style(Selector::MAIN, pad);
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(items, s.btn.clone());
                cx.add_style(items.with_state(State::DISABLED), s.disabled.clone());
                cx.add_style(items, s.bg_color_white.clone());
                cx.add_style(items, s.keyboard_button_bg.clone());
                cx.add_style(items.with_state(State::PRESSED), s.pressed.clone());
                cx.add_style(items.with_state(State::CHECKED), s.bg_color_grey.clone());
                cx.add_style(
                    items.with_state(State::FOCUS_KEY),
                    s.bg_color_primary_muted.clone(),
                );
                cx.add_style(
                    items.with_state(State::EDITED),
                    s.bg_color_secondary_muted.clone(),
                );
            }
            // Lines 1191-1198.
            "spinbox" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_small.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::part(Part::Cursor), s.bg_color_primary.clone());
            }
            // Lines 978-988.
            "roller" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.anim.clone());
                cx.add_style(Selector::MAIN, s.line_space_large.clone());
                cx.add_style(Selector::MAIN, s.text_align_center.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::part(Part::Selected), s.bg_color_primary.clone());
            }
            // Lines 990-1004.
            "dropdown" => {
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.pad_small.clone());
                cx.add_style(Selector::MAIN, s.transition_delayed.clone());
                cx.add_style(Selector::state(State::PRESSED), s.transition_normal.clone());
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::part(Part::Indicator), s.transition_normal.clone());
                cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
            }
            // Lines 1001-1012.
            "dropdown_list" => {
                let sel = Selector::part(Part::Selected);
                cx.add_style(Selector::MAIN, s.card.clone());
                cx.add_style(Selector::MAIN, s.clip_corner.clone());
                cx.add_style(Selector::MAIN, s.line_space_large.clone());
                cx.add_style(Selector::MAIN, s.dropdown_list.clone());
                cx.add_style(scrollbar, s.scrollbar.clone());
                cx.add_style(scrolled, s.scrollbar_scrolled.clone());
                cx.add_style(sel, s.bg_color_white.clone());
                cx.add_style(sel.with_state(State::CHECKED), s.bg_color_primary.clone());
                cx.add_style(sel.with_state(State::PRESSED), s.pressed.clone());
            }
            // Lines 1086-1090: a label inside a textarea (an exact type check: not a spinbox's).
            "label" if cx.parent_class().is_some_and(|c| c.name == "textarea") => {
                cx.add_style(Selector::part(Part::Selected), s.bg_color_primary.clone());
            }
            // LVGL styles neither image buttons, animated images nor span groups.
            "label"
            | "image"
            | "imagebutton"
            | "animimg"
            | "spangroup"
            | "buttonmatrix_popover"
            | "tabview_content" => {}
            other => {
                twine_core::trace!(target: "twine::style", "default theme: no styles for class {}", other);
            }
        }
    }

    fn font_normal(&self) -> &'static Font {
        self.font_normal
    }

    fn name(&self) -> &'static str {
        match self.mode {
            ThemeMode::Light => "default-light",
            ThemeMode::Dark => "default-dark",
        }
    }
    fn color_primary(&self) -> Color {
        self.primary
    }
    fn color_secondary(&self) -> Color {
        self.secondary
    }
}

impl Theme for DefaultTheme {
    fn font_small(&self) -> &'static Font {
        self.font_small
    }
    fn font_large(&self) -> &'static Font {
        self.font_large
    }
}
