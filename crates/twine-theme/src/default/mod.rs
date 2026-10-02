//! [`DefaultTheme`]: a faithful port of LVGL's default theme
//! (`src/themes/default/lv_theme_default.c`), light and dark, plus Twine's night and
//! high-contrast modes.
//!
//! The styles are LVGL's `style_init()` (see [`styles`]); [`DefaultTheme`]'s
//! [`ThemeHook::apply`] is LVGL's `theme_apply()` for the widget classes that exist. Like
//! LVGL, sizes depend on the display: every pixel value is scaled with [`dpx`](twine_style::dpx) by
//! the display's DPI, and radii and paddings follow the display size class
//! ([`DisplaySize`]: the larger side ≤ 320 px is small, < 720 px medium, else large).
//!
//! Every color that differs between modes (and every semantic color: primary, secondary,
//! focus ring, …) is a [design element](twine_style::design): the styles are the same in
//! every mode, and the mode's [`ElementTable`] (the values are listed in [`styles`]) gives
//! them, so [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode) switches between
//! [`ThemeMode::Light`], [`ThemeMode::Dark`], [`ThemeMode::Night`] and
//! [`ThemeMode::HighContrast`] without re-applying the theme.
//!
//! Widget classes are recognised by identity (`&'static WidgetClass`, a pointer comparison)
//! along their [`lineage`](WidgetClass::lineage): a custom class whose
//! [`base`](WidgetClass::base) is `BUTTON_CLASS` is styled like a button. Applications style
//! their own classes with [`DefaultThemeBuilder::class`], reusing the theme's [`Styles`]
//! (also public through [`DefaultTheme::styles`]).

pub mod colors;
pub mod styles;

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_core::{Color, Size};
use twine_engine::{OBJ_CLASS, ThemeCx, ThemeHook, WidgetClass};
use twine_style::design::{Element, ElementTable, ElementType};
use twine_style::{Part, Selector, State};
use twine_text::Font;
use twine_widgets::arc::ARC_CLASS;
use twine_widgets::bar::BAR_CLASS;
use twine_widgets::button::BUTTON_CLASS;
use twine_widgets::buttonmatrix::{BUTTONMATRIX_CLASS, POPOVER_CLASS};
use twine_widgets::checkbox::CHECKBOX_CLASS;
use twine_widgets::container::CARD_CLASS;
use twine_widgets::image::IMAGE_CLASS;
use twine_widgets::keyboard::KEYBOARD_CLASS;
use twine_widgets::label::LABEL_CLASS;
use twine_widgets::led::LED_CLASS;
use twine_widgets::line::LINE_CLASS;
use twine_widgets::slider::SLIDER_CLASS;
use twine_widgets::spangroup::SPANGROUP_CLASS;
use twine_widgets::spinbox::SPINBOX_CLASS;
use twine_widgets::spinner::SPINNER_CLASS;
use twine_widgets::switch::SWITCH_CLASS;
use twine_widgets::textarea::{PLACEHOLDER, TEXTAREA_CLASS};
use twine_widgets_ext::dropdown::{DROPDOWN_CLASS, DROPDOWN_LIST_CLASS};
use twine_widgets_ext::list::{LIST_BUTTON_CLASS, LIST_CLASS, LIST_TEXT_CLASS};
use twine_widgets_ext::menu::{
    MENU_CLASS, MENU_CONT_CLASS, MENU_MAIN_CONTAINER_CLASS, MENU_MAIN_HEADER_CONTAINER_CLASS,
    MENU_PAGE_CLASS, MENU_SECTION_CLASS, MENU_SEPARATOR_CLASS, MENU_SIDEBAR_CONTAINER_CLASS,
    MENU_SIDEBAR_HEADER_CONTAINER_CLASS,
};
use twine_widgets_ext::msgbox::{
    MSGBOX_BACKDROP_CLASS, MSGBOX_CLASS, MSGBOX_CONTENT_CLASS, MSGBOX_FOOTER_BUTTON_CLASS,
    MSGBOX_FOOTER_CLASS, MSGBOX_HEADER_BUTTON_CLASS, MSGBOX_HEADER_CLASS,
};
use twine_widgets_ext::roller::ROLLER_CLASS;
use twine_widgets_ext::tabview::{TABVIEW_CLASS, TABVIEW_TAB_BAR_CLASS};
use twine_widgets_ext::tileview::{TILEVIEW_CLASS, TILEVIEW_TILE_CLASS};
use twine_widgets_ext::window::{WIN_CLASS, WIN_CONTENT_CLASS, WIN_HEADER_CLASS};

use crate::class::{ClassCx, ClassStyles, common_builder_methods};
use crate::design::DesignTables;
use crate::{FontScale, Palette, Theme};
use styles::{Params, Styles};

/// The theme modes: [`ThemeMode::Light`] and [`ThemeMode::Dark`] (LVGL's `dark` flag of the
/// default theme; no button shadows), [`ThemeMode::Night`] and [`ThemeMode::HighContrast`],
/// switched at run time with [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode).
pub use twine_style::ThemeMode;

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
/// Built with [`DefaultTheme::builder`] (or the [`light`](Self::light) /
/// [`dark`](Self::dark) shortcuts). Style sets are built on first use for each (DPI, display
/// size) combination (usually one) and shared by every node through `Rc`; they are the same in
/// every mode. The [design element](twine_style::design) tables (one per mode and
/// combination, built on first use; values listed in [`styles`]) define every standard
/// element; the application adds its own with [`DefaultThemeBuilder::element`] /
/// [`DefaultThemeBuilder::element_in`], which also override standard ones.
///
/// ```
/// use twine_core::{Color, Size};
/// use twine_style::design;
/// use twine_theme::{DefaultTheme, Palette, ThemeHook, ThemeMode};
///
/// let dark = DefaultTheme::builder().mode(ThemeMode::Dark).dpi(160).build();
/// assert_eq!(dark.mode(), ThemeMode::Dark);
/// assert_eq!(dark.primary(), Palette::Blue.main());
/// let table = dark.design(ThemeMode::Dark, 160, Size::new(320, 240)).unwrap();
/// assert_eq!(table.get(design::SURFACE), Some(Color::hex(0x282B30)));
/// assert_eq!(table.get(design::PRIMARY), Some(Palette::Blue.main()));
/// assert_eq!(table.missing_standard(), None);
/// ```
pub struct DefaultTheme {
    primary: Color,
    secondary: Color,
    mode: ThemeMode,
    fonts: FontScale,
    dpi: Option<u16>,
    size: Option<DisplaySize>,
    styles: RefCell<Vec<(u16, DisplaySize, Rc<Styles>)>>,
    design: DesignTables<(u16, DisplaySize)>,
    classes: ClassStyles<Styles>,
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

/// The builder of a [`DefaultTheme`] ([`DefaultTheme::builder`]): every setting has LVGL's
/// default, so only what differs is set.
///
/// | Setting | Default |
/// |---------|---------|
/// | [`primary`](Self::primary) / [`secondary`](Self::secondary) | [`Palette::Blue`] / [`Palette::Red`] (LVGL's) |
/// | [`mode`](Self::mode) | [`ThemeMode::Light`] |
/// | [`fonts`](Self::fonts) | Montserrat 14 for all three (feature `assets`; without it, set them) |
/// | [`dpi`](Self::dpi) / [`display_size`](Self::display_size) | each display's own |
/// | [`parent`](Self::parent) | none |
/// | [`element`](Self::element) / [`element_in`](Self::element_in) / [`class`](Self::class) | none |
///
/// ```
/// use twine_core::Color;
/// use twine_theme::{DefaultTheme, FontScale, Palette, ThemeMode};
///
/// let f = &twine_assets::fonts::MONTSERRAT_14;
/// let theme = DefaultTheme::builder()
///     .primary(Palette::Teal)
///     .secondary(Color::hex(0xFFC107))
///     .mode(ThemeMode::Night)
///     .fonts(FontScale { small: f, normal: f, large: f })
///     .build();
/// assert_eq!(theme.primary(), Palette::Teal.main());
/// ```
#[must_use]
pub struct DefaultThemeBuilder(DefaultTheme);

impl core::fmt::Debug for DefaultThemeBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("DefaultThemeBuilder").field(&self.0).finish()
    }
}

impl DefaultThemeBuilder {
    /// The primary color: buttons, sliders, focus rings (`design::PRIMARY` and
    /// `design::FOCUS_RING` in [`ThemeMode::Light`] and [`ThemeMode::Dark`]; LVGL
    /// `color_primary`). A [`Palette`] entry (its main color) or any [`Color`].
    pub fn primary(mut self, color: impl Into<Color>) -> Self {
        self.0.primary = color.into();
        self
    }

    /// The secondary color: checked buttons, edit outlines (`design::SECONDARY` in
    /// [`ThemeMode::Light`] and [`ThemeMode::Dark`]; LVGL `color_secondary`).
    /// ```
    /// use twine_core::Color;
    /// use twine_theme::{DefaultTheme, Palette};
    ///
    /// let t = DefaultTheme::builder().primary(Palette::Teal).secondary(Color::hex(0xFF9800)).build();
    /// assert_eq!(t.secondary(), Color::hex(0xFF9800));
    /// ```
    pub fn secondary(mut self, color: impl Into<Color>) -> Self {
        self.0.secondary = color.into();
        self
    }

    /// The small, normal and large fonts (LVGL `font_small` / `font_normal` /
    /// `font_large`).
    /// ```
    /// use twine_theme::{DefaultTheme, FontScale};
    ///
    /// let body = &twine_assets::fonts::MONTSERRAT_14;
    /// let t = DefaultTheme::builder().fonts(FontScale::uniform(body)).build();
    /// assert!(core::ptr::eq(t.fonts().normal, body));
    /// ```
    pub fn fonts(mut self, fonts: FontScale) -> Self {
        self.0.fonts = fonts;
        self
    }

    /// Scales for `dpi` (at least 1) instead of each display's own DPI (LVGL uses the
    /// display's, [`DEFAULT_DPI`](twine_style::DEFAULT_DPI) = 130 by default).
    /// Never panics: `0` is taken as `1`.
    ///
    /// ```
    /// use twine_theme::DefaultTheme;
    /// // Sizes as on a 160 DPI panel, whatever the display reports.
    /// let _t = DefaultTheme::builder().dpi(160).build();
    /// ```
    pub fn dpi(mut self, dpi: u16) -> Self {
        self.0.dpi = Some(dpi.max(1));
        self
    }

    /// Uses the size class `size` instead of deriving it from each display's resolution.
    /// ```
    /// use twine_theme::{DefaultTheme, DisplaySize};
    /// // The compact layout of small displays, also on a larger one.
    /// let _t = DefaultTheme::builder().display_size(DisplaySize::Small).build();
    /// ```
    pub fn display_size(mut self, size: DisplaySize) -> Self {
        self.0.size = Some(size);
        self
    }

    common_builder_methods!(Styles, DefaultTheme);

    /// The theme. Never panics.
    #[must_use]
    pub fn build(self) -> DefaultTheme {
        crate::theme::check_font("default", self.0.fonts.normal);
        self.0
    }
}

impl DefaultTheme {
    /// A builder with LVGL's defaults (see [`DefaultThemeBuilder`]); LVGL
    /// `lv_theme_default_init`.
    ///
    /// ```
    /// use twine_theme::{DefaultTheme, Palette, ThemeHook, ThemeMode};
    /// let t = DefaultTheme::builder().primary(Palette::Teal).mode(ThemeMode::Dark).build();
    /// assert_eq!(t.mode(), ThemeMode::Dark);
    /// ```
    #[doc(alias = "lv_theme_default_init")]
    pub fn builder() -> DefaultThemeBuilder {
        DefaultThemeBuilder(DefaultTheme {
            primary: Palette::Blue.main(),
            secondary: Palette::Red.main(),
            mode: ThemeMode::Light,
            fonts: FontScale::uniform(crate::theme::default_font()),
            dpi: None,
            size: None,
            styles: RefCell::new(Vec::new()),
            design: DesignTables::new(),
            classes: ClassStyles::new(),
            parent: None,
        })
    }

    /// Blue / red, light, Montserrat 14 (LVGL's defaults: `LV_THEME_DEFAULT_DARK 0`,
    /// `lv_palette_main(LV_PALETTE_BLUE)`, `lv_palette_main(LV_PALETTE_RED)`,
    /// `LV_FONT_DEFAULT`): `DefaultTheme::builder().build()`.
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn light() -> Self {
        Self::builder().build()
    }

    /// Blue / red, dark, Montserrat 14: `DefaultTheme::builder().mode(ThemeMode::Dark).build()`.
    #[cfg(feature = "assets")]
    #[must_use]
    pub fn dark() -> Self {
        Self::builder().mode(ThemeMode::Dark).build()
    }

    /// The mode the theme starts in.
    #[must_use]
    pub fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// The primary color (`design::PRIMARY` in light and dark mode).
    #[must_use]
    pub fn primary(&self) -> Color {
        self.primary
    }

    /// The secondary color (`design::SECONDARY` in light and dark mode).
    #[must_use]
    pub fn secondary(&self) -> Color {
        self.secondary
    }

    /// The fonts.
    #[must_use]
    pub fn fonts(&self) -> FontScale {
        self.fonts
    }

    /// The parameters of the styles and element tables for a display with `dpi` and
    /// `resolution`.
    fn params(&self, dpi: u16, resolution: Size) -> Params {
        Params {
            primary: self.primary,
            secondary: self.secondary,
            dpi: self.dpi.unwrap_or(dpi),
            size: self.size.unwrap_or_else(|| DisplaySize::of(resolution)),
            font_small: self.fonts.small,
            font_normal: self.fonts.normal,
            font_large: self.fonts.large,
        }
    }

    /// The display size class the styles of `cx`'s display use.
    fn size_of(&self, cx: &ThemeCx<'_>) -> DisplaySize {
        self.size.unwrap_or_else(|| DisplaySize::of(cx.resolution()))
    }

    /// The theme's styles for a display with `dpi` and logical `resolution` (built on first
    /// use for each DPI and size class, then shared): its card, button, pressed, focus
    /// outline, … styles, for custom widgets that should look native (see also
    /// [`DefaultThemeBuilder::class`], whose [`ClassCx::styles`] gives the same set).
    ///
    /// ```
    /// use twine_core::Size;
    /// use twine_style::{PropId, StyleValue, design};
    /// use twine_theme::DefaultTheme;
    ///
    /// let t = DefaultTheme::light();
    /// let s = t.styles(130, Size::new(320, 240));
    /// assert!(matches!(s.card.get(PropId::BgColor), Some(StyleValue::Element(_))));
    /// assert!(std::rc::Rc::ptr_eq(&s, &t.styles(130, Size::new(320, 240))));
    /// ```
    #[must_use]
    pub fn styles(&self, dpi: u16, resolution: Size) -> Rc<Styles> {
        let p = self.params(dpi, resolution);
        let (dpi, size) = (p.dpi, p.size);
        let mut cache = self.styles.borrow_mut();
        if let Some((_, _, s)) = cache.iter().find(|(d, z, _)| *d == dpi && *z == size) {
            return s.clone();
        }
        let s = Rc::new(Styles::new(&p));
        twine_core::debug!(target: "twine::style", "default theme: styles for {} dpi, {:?}", dpi, size);
        cache.push((dpi, size, s.clone()));
        s
    }

    /// Adds the styles of `class` if the theme knows it (LVGL `theme_apply()`'s branch for the
    /// class); `false` for a class it does not know. One pointer comparison per known class,
    /// the most common classes first.
    #[allow(clippy::too_many_lines)] // one branch per LVGL class
    fn style_class(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass, s: &Styles) -> bool {
        let scrollbar = Selector::part(Part::Scrollbar);
        let scrolled = Selector::part(Part::Scrollbar).with_state(State::SCROLLED);
        if class.is(&LABEL_CLASS) {
            // Lines 1086-1090: a label inside a textarea (not a spinbox's).
            if cx.parent_class().is_some_and(|c| c.is_a(&TEXTAREA_CLASS)) {
                cx.add_style(Selector::part(Part::Selected), s.bg_color_primary.clone());
            }
        } else if class.is(&OBJ_CLASS) {
            if cx.grandparent_class().is_some_and(|c| c.is_a(&TABVIEW_CLASS)) {
                // Lines 758-808: tabview pages (the content and the bar have their own classes).
                cx.add_style(Selector::MAIN, s.pad_normal.clone());
                cx.add_style(Selector::MAIN, s.rotary_scroll.clone());
            } else {
                // NOTE(P20.S03): calendar children branch before the card styles here, as in
                // LVGL.
                cx.add_style(Selector::MAIN, s.card.clone());
            }
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&CARD_CLASS) {
            // Twine's card: LVGL's look of a plain object, explicitly.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&BUTTON_CLASS) {
            if cx.parent_class().is_some_and(|c| c.is_a(&TABVIEW_TAB_BAR_CLASS)) {
                // Lines 815-826: a button in a tabview's bar.
                cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
                cx.add_style(Selector::state(State::CHECKED), s.bg_color_primary_muted.clone());
                cx.add_style(Selector::state(State::CHECKED), s.tab_btn.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
                cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
                cx.add_style(Selector::state(State::FOCUS_KEY), s.tab_bg_focus.clone());
                return true;
            }
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
                c.is_a(&MENU_SIDEBAR_HEADER_CONTAINER_CLASS) || c.is_a(&MENU_MAIN_HEADER_CONTAINER_CLASS)
            });
            if menu_header {
                cx.add_style(Selector::MAIN, s.menu_header_btn.clone());
                cx.add_style(Selector::state(State::PRESSED), s.menu_pressed.clone());
            }
        } else if class.is(&IMAGE_CLASS) || class.is(&SPANGROUP_CLASS) || class.is(&POPOVER_CLASS) {
            // LVGL styles neither images nor span groups (nor image buttons, animated images,
            // a tabview's content: unknown classes get nothing either).
        } else if class.is(&LIST_CLASS) {
            // Lines 1092-1111.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.list_bg.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&LIST_TEXT_CLASS) {
            cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
            cx.add_style(Selector::MAIN, s.list_item_grow.clone());
        } else if class.is(&LIST_BUTTON_CLASS) {
            cx.add_style(Selector::MAIN, s.bg_color_white.clone());
            cx.add_style(Selector::MAIN, s.list_btn.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.bg_color_primary.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.list_item_grow.clone());
            cx.add_style(Selector::state(State::PRESSED), s.list_item_grow.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
        } else if class.is(&MENU_CLASS) {
            // Lines 1113-1149.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.menu_bg.clone());
        } else if class.is(&MENU_SIDEBAR_CONTAINER_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_sidebar_cont.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&MENU_MAIN_CONTAINER_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_main_cont.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&MENU_CONT_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_cont.clone());
            cx.add_style(Selector::state(State::PRESSED), s.menu_pressed.clone());
            cx.add_style(
                Selector::state(State::PRESSED | State::CHECKED),
                s.bg_color_primary_muted.clone(),
            );
            cx.add_style(Selector::state(State::CHECKED), s.bg_color_primary_muted.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.bg_color_primary.clone());
        } else if class.is(&MENU_SIDEBAR_HEADER_CONTAINER_CLASS)
            || class.is(&MENU_MAIN_HEADER_CONTAINER_CLASS)
        {
            cx.add_style(Selector::MAIN, s.menu_header_cont.clone());
        } else if class.is(&MENU_PAGE_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_page.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&MENU_SECTION_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_section.clone());
        } else if class.is(&MENU_SEPARATOR_CLASS) {
            cx.add_style(Selector::MAIN, s.menu_separator.clone());
        } else if class.is(&MSGBOX_CLASS) {
            // Lines 1151-1189.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.pad_zero.clone());
            cx.add_style(Selector::MAIN, s.clip_corner.clone());
        } else if class.is(&MSGBOX_BACKDROP_CLASS) {
            cx.add_style(Selector::MAIN, s.msgbox_backdrop_bg.clone());
        } else if class.is(&MSGBOX_HEADER_CLASS) {
            cx.add_style(Selector::MAIN, s.pad_small.clone());
            cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
        } else if class.is(&MSGBOX_FOOTER_CLASS) {
            cx.add_style(Selector::MAIN, s.pad_small.clone());
        } else if class.is(&MSGBOX_CONTENT_CLASS) {
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
            cx.add_style(Selector::MAIN, s.pad_small.clone());
        } else if class.is(&MSGBOX_HEADER_BUTTON_CLASS) || class.is(&MSGBOX_FOOTER_BUTTON_CLASS) {
            cx.add_style(Selector::MAIN, s.btn.clone());
            cx.add_style(Selector::MAIN, s.bg_color_primary.clone());
            cx.add_style(Selector::MAIN, s.transition_delayed.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
            cx.add_style(Selector::state(State::PRESSED), s.transition_normal.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::CHECKED), s.bg_color_secondary.clone());
            cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
        } else if class.is(&TILEVIEW_CLASS) {
            // Lines 1200-1210.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&TILEVIEW_TILE_CLASS) {
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&TABVIEW_CLASS) {
            // Lines 1212-1217 and 760-771: the tabview, its bar (the content has no style).
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::MAIN, s.pad_zero.clone());
        } else if class.is(&TABVIEW_TAB_BAR_CLASS) {
            cx.add_style(Selector::MAIN, s.bg_color_white.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.tab_bg_focus.clone());
        } else if class.is(&WIN_CLASS) {
            // Lines 783-797 and 1219-1223: the window, its header and content.
            cx.add_style(Selector::MAIN, s.clip_corner.clone());
        } else if class.is(&WIN_HEADER_CLASS) {
            cx.add_style(Selector::MAIN, s.bg_color_grey.clone());
            cx.add_style(Selector::MAIN, s.pad_tiny.clone());
        } else if class.is(&WIN_CONTENT_CLASS) {
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::MAIN, s.pad_normal.clone());
            cx.add_style(scrollbar, s.scrollbar.clone());
            cx.add_style(scrolled, s.scrollbar_scrolled.clone());
        } else if class.is(&LINE_CLASS) {
            // Lines 850-854.
            cx.add_style(Selector::MAIN, s.line.clone());
        } else if class.is(&BAR_CLASS) {
            // Lines 884-894.
            cx.add_style(Selector::MAIN, s.bg_color_primary_muted.clone());
            cx.add_style(Selector::MAIN, s.circle.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
            cx.add_style(Selector::part(Part::Indicator), s.bg_color_primary.clone());
            cx.add_style(Selector::part(Part::Indicator), s.circle.clone());
        } else if class.is(&SLIDER_CLASS) {
            // Lines 895-911.
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
        } else if class.is(&CHECKBOX_CLASS) {
            // Lines 930-947.
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
        } else if class.is(&SWITCH_CLASS) {
            // Lines 947-964.
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
        } else if class.is(&ARC_CLASS) {
            // Lines 1015-1025.
            cx.add_style(Selector::MAIN, s.arc_indic.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_indic.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_indic_primary.clone());
            cx.add_style(Selector::part(Part::Knob), s.knob.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
        } else if class.is(&SPINNER_CLASS) {
            // Lines 1026-1032.
            cx.add_style(Selector::MAIN, s.arc_indic.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_indic.clone());
            cx.add_style(Selector::part(Part::Indicator), s.arc_indic_primary.clone());
        } else if class.is(&LED_CLASS) {
            // Lines 1225-1229.
            cx.add_style(Selector::MAIN, s.led.clone());
        } else if class.is(&BUTTONMATRIX_CLASS) {
            // Lines 856-883.
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
        } else if class.is(&TEXTAREA_CLASS) {
            // Lines 1034-1046.
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
            // LV_PART_TEXTAREA_PLACEHOLDER (= LV_PART_CUSTOM_FIRST)
            cx.add_style(Selector::part(PLACEHOLDER), s.ta_placeholder.clone());
        } else if class.is(&KEYBOARD_CLASS) {
            // Lines 1069-1084.
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
        } else if class.is(&SPINBOX_CLASS) {
            // Lines 1191-1198.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.pad_small.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
            cx.add_style(Selector::part(Part::Cursor), s.bg_color_primary.clone());
        } else if class.is(&ROLLER_CLASS) {
            // Lines 978-988.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.anim.clone());
            cx.add_style(Selector::MAIN, s.line_space_large.clone());
            cx.add_style(Selector::MAIN, s.text_align_center.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
            cx.add_style(Selector::part(Part::Selected), s.bg_color_primary.clone());
        } else if class.is(&DROPDOWN_CLASS) {
            // Lines 990-1004.
            cx.add_style(Selector::MAIN, s.card.clone());
            cx.add_style(Selector::MAIN, s.pad_small.clone());
            cx.add_style(Selector::MAIN, s.transition_delayed.clone());
            cx.add_style(Selector::state(State::PRESSED), s.transition_normal.clone());
            cx.add_style(Selector::state(State::PRESSED), s.pressed.clone());
            cx.add_style(Selector::state(State::FOCUS_KEY), s.outline_primary.clone());
            cx.add_style(Selector::state(State::EDITED), s.outline_secondary.clone());
            cx.add_style(Selector::part(Part::Indicator), s.transition_normal.clone());
            cx.add_style(Selector::state(State::DISABLED), s.disabled.clone());
        } else if class.is(&DROPDOWN_LIST_CLASS) {
            // Lines 1001-1012.
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
        } else {
            return false;
        }
        true
    }
}

impl ThemeHook for DefaultTheme {
    /// LVGL `theme_apply()` of `lv_theme_default.c`: the parent theme first, then the styles
    /// of the nearest class of `class`'s [lineage](WidgetClass::lineage) the theme knows,
    /// then the application's [class registrations](DefaultThemeBuilder::class).
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        if let Some(p) = &self.parent {
            p.apply(cx, class);
        }
        let s = self.styles(cx.dpi(), cx.resolution());
        if cx.parent().is_none() {
            // Screens.
            cx.add_style(Selector::MAIN, s.scr.clone());
            cx.add_style(Selector::part(Part::Scrollbar), s.scrollbar.clone());
            cx.add_style(
                Selector::part(Part::Scrollbar).with_state(State::SCROLLED),
                s.scrollbar_scrolled.clone(),
            );
        } else if !class.lineage().any(|c| self.style_class(cx, c, &s)) {
            twine_core::trace!(target: "twine::style", "default theme: no styles for class {}", class.name);
        }
        self.classes.apply(cx, class, &s);
    }

    fn font_normal(&self) -> &'static Font {
        self.fonts.normal
    }

    fn name(&self) -> &'static str {
        "default"
    }

    fn mode(&self) -> ThemeMode {
        self.mode
    }

    /// Every mode.
    fn modes(&self) -> &'static [ThemeMode] {
        &ThemeMode::ALL
    }

    /// The tables of [`styles`] for the display's (DPI, size class) and mode, built once
    /// each, with the parent's elements below and the application's on top.
    fn design(&self, mode: ThemeMode, dpi: u16, resolution: Size) -> Option<Rc<ElementTable>> {
        let p = self.params(dpi, resolution);
        Some(self.design.get((p.dpi, p.size), mode, || {
            let mut t = self
                .parent
                .as_ref()
                .and_then(|parent| parent.design(mode, dpi, resolution))
                .map_or_else(ElementTable::new, |t| (*t).clone());
            t.overlay(&styles::elements(&p, mode));
            t
        }))
    }
}

impl Theme for DefaultTheme {
    fn font_small(&self) -> &'static Font {
        self.fonts.small
    }
    fn font_large(&self) -> &'static Font {
        self.fonts.large
    }
}
