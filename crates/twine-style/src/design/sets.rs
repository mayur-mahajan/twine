//! Typed element sets: every standard element of a group as a named field, so a theme that
//! leaves one out does not compile, with `const fn to_table` to build a mode's
//! [`ElementTable`] in flash.
//!
//! | Set | Elements | Field type |
//! |-----|----------|------------|
//! | [`SystemColors`] | every standard color (`BACKGROUND` … `FILL_3`) and the four standard opacities | `Color`, `Opa` |
//! | [`Spacing`] | `SPACE_XS` … `SPACE_L` | `Length` |
//! | [`Shape`] | `RADIUS_S` … `RADIUS_FULL` | `Radius` |
//! | [`Elevation`] | `ELEVATION_1_*` … `ELEVATION_3_*` (shadow width, offset, spread) | [`ElevationLevel`] |
//! | [`MotionSet`] | `MOTION_FAST` … `MOTION_EXIT` | `&'static AnimSpec` |
//! | [`IconSet`] | `ICON_CHECK` … `ICON_CLEAR` (optional: only the icons listed are linked) | `Option<&'static ImageSource>` |
//! | [`FontSet`] | `FONT_SMALL`, `FONT_BODY`, `FONT_LARGE` | `&'static Font` |
//! | [`DesignSet`] | all of the above: a whole mode | the sets |

use twine_anim::AnimSpec;
use twine_core::{Color, Duration, Opa};
use twine_image::ImageSource;
use twine_text::Font;

#[cfg(doc)]
use super::*;
use super::{
    ELEVATION_1_OFFSET_Y, ELEVATION_1_SPREAD, ELEVATION_1_WIDTH, ELEVATION_2_OFFSET_Y, ELEVATION_2_SPREAD,
    ELEVATION_2_WIDTH, ELEVATION_3_OFFSET_Y, ELEVATION_3_SPREAD, ELEVATION_3_WIDTH, ElementTable, FONT_BODY,
    FONT_LARGE, FONT_SMALL, MOTION_EMPHASIZED, MOTION_EXIT, MOTION_FAST, MOTION_STANDARD, RADIUS_FULL,
    RADIUS_L, RADIUS_M, RADIUS_S, SPACE_L, SPACE_M, SPACE_S, SPACE_XS,
};
use crate::value_types::{Length, Radius};

/// Every standard color element and the four standard opacity elements (they qualify
/// colors: focus ring, shadow, disabled overlay, scrim), one field each. See the element
/// constants for what each one is for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]

pub struct SystemColors {
    /// [`BACKGROUND`].
    pub background: Color,
    /// [`SURFACE`].
    pub surface: Color,
    /// [`ON_SURFACE`].
    pub on_surface: Color,
    /// [`ON_SURFACE_MUTED`].
    pub on_surface_muted: Color,
    /// [`SURFACE_VARIANT`].
    pub surface_variant: Color,
    /// [`OUTLINE`].
    pub outline: Color,
    /// [`PRIMARY`].
    pub primary: Color,
    /// [`ON_PRIMARY`].
    pub on_primary: Color,
    /// [`SECONDARY`].
    pub secondary: Color,
    /// [`ON_SECONDARY`].
    pub on_secondary: Color,
    /// [`DANGER`].
    pub danger: Color,
    /// [`WARNING`].
    pub warning: Color,
    /// [`OK`].
    pub ok: Color,
    /// [`DISABLED`].
    pub disabled: Color,
    /// [`FOCUS_RING`].
    pub focus_ring: Color,
    /// [`SCROLLBAR`].
    pub scrollbar: Color,
    /// [`SHADOW`].
    pub shadow: Color,
    /// [`SCRIM`].
    pub scrim: Color,
    /// [`PLACEHOLDER`].
    pub placeholder: Color,
    /// [`NEUTRAL`].
    pub neutral: Color,
    /// [`ACCENT_TINT`].
    pub accent_tint: Color,
    /// [`SEPARATOR`].
    pub separator: Color,
    /// [`GROUPED_BACKGROUND`].
    pub grouped_background: Color,
    /// [`FILL_1`].
    pub fill_1: Color,
    /// [`FILL_2`].
    pub fill_2: Color,
    /// [`FILL_3`].
    pub fill_3: Color,
    /// [`FOCUS_RING_OPACITY`].
    pub focus_ring_opacity: Opa,
    /// [`SHADOW_OPACITY`].
    pub shadow_opacity: Opa,
    /// [`DISABLED_OPACITY`].
    pub disabled_opacity: Opa,
    /// [`SCRIM_OPACITY`].
    pub scrim_opacity: Opa,
}

impl SystemColors {
    /// Two colors for everything — `fg` for text, accents and lines, `bg` for backgrounds and
    /// surfaces, with opaque focus rings, no shadows and no disabled overlay: a monochrome
    /// display's colors (and a compact starting point to adjust with struct update syntax).
    /// `const`; never panics.
    ///
    /// ```
    /// use twine_core::{Color, Opa};
    /// use twine_style::design::SystemColors;
    ///
    /// const NIGHT: SystemColors = SystemColors {
    ///     primary: Color::hex(0xFFB300),
    ///     ..SystemColors::monochrome(Color::WHITE, Color::BLACK)
    /// };
    /// assert_eq!(NIGHT.surface, Color::BLACK);
    /// assert_eq!(NIGHT.shadow_opacity, Opa::TRANSP);
    /// ```
    #[must_use]
    pub const fn monochrome(fg: Color, bg: Color) -> Self {
        Self {
            background: bg,
            surface: bg,
            on_surface: fg,
            on_surface_muted: fg,
            surface_variant: bg,
            outline: fg,
            primary: fg,
            on_primary: bg,
            secondary: bg,
            on_secondary: fg,
            danger: fg,
            warning: fg,
            ok: fg,
            disabled: bg,
            focus_ring: fg,
            scrollbar: fg,
            shadow: fg,
            scrim: bg,
            placeholder: fg,
            neutral: fg,
            accent_tint: bg,
            separator: fg,
            grouped_background: bg,
            fill_1: bg,
            fill_2: bg,
            fill_3: bg,
            focus_ring_opacity: Opa::COVER,
            shadow_opacity: Opa::TRANSP,
            disabled_opacity: Opa::TRANSP,
            scrim_opacity: Opa::COVER,
        }
    }

    /// A table with these elements only (`const`). Never panics.
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().colors(self)
    }
}

/// The spacing scale: [`SPACE_XS`], [`SPACE_S`], [`SPACE_M`], [`SPACE_L`] (ascending). `Dp`
/// lengths make a table independent of the display's DPI (the resolver scales them).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Spacing {
    /// [`SPACE_XS`].
    pub xs: Length,
    /// [`SPACE_S`].
    pub s: Length,
    /// [`SPACE_M`].
    pub m: Length,
    /// [`SPACE_L`].
    pub l: Length,
}

impl Spacing {
    /// A table with these elements only (`const`). Never panics.
    ///
    /// ```
    /// use twine_style::design::{self, Spacing};
    /// use twine_style::Length;
    ///
    /// static T: twine_style::design::ElementTable =
    ///     Spacing { xs: Length::dp(2), s: Length::dp(4), m: Length::dp(8), l: Length::dp(16) }.to_table();
    /// assert_eq!(T.get(design::SPACE_L), Some(Length::dp(16)));
    /// ```
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().spacing(self)
    }
}

/// The corner radius scale: [`RADIUS_S`], [`RADIUS_M`], [`RADIUS_L`], [`RADIUS_FULL`].
// NOTE(R7.S08): the set gains the corner shape (`CornerShape`) with continuous corners.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    /// [`RADIUS_S`].
    pub s: Radius,
    /// [`RADIUS_M`].
    pub m: Radius,
    /// [`RADIUS_L`].
    pub l: Radius,
    /// [`RADIUS_FULL`].
    pub full: Radius,
}

impl Shape {
    /// A table with these elements only (`const`). Never panics.
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().shape(self)
    }
}

/// One elevation level: the shadow a surface at that height casts (with the [`SHADOW`] color
/// at [`SHADOW_OPACITY`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ElevationLevel {
    /// The shadow's width (blur radius).
    pub width: Length,
    /// The shadow's downward offset.
    pub offset_y: Length,
    /// The shadow's spread.
    pub spread: Length,
}

impl ElevationLevel {
    /// No shadow (all zero).
    pub const FLAT: ElevationLevel = ElevationLevel {
        width: Length::Px(0),
        offset_y: Length::Px(0),
        spread: Length::Px(0),
    };
}

/// The elevation scale: three shadow levels ([`ELEVATION_1_WIDTH`] …
/// [`ELEVATION_3_SPREAD`]) for raised controls, cards and menus, dialogs and sheets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Elevation {
    /// Raised controls (buttons).
    pub level_1: ElevationLevel,
    /// Cards and menus above the content.
    pub level_2: ElevationLevel,
    /// Dialogs, sheets and popovers.
    pub level_3: ElevationLevel,
}

impl Elevation {
    /// No shadows at any level (flat designs, monochrome displays).
    pub const NONE: Elevation = Elevation {
        level_1: ElevationLevel::FLAT,
        level_2: ElevationLevel::FLAT,
        level_3: ElevationLevel::FLAT,
    };

    /// A table with these elements only (`const`). Never panics.
    ///
    /// ```
    /// use twine_style::design::{self, Elevation};
    /// use twine_style::Length;
    ///
    /// static T: twine_style::design::ElementTable = Elevation::NONE.to_table();
    /// assert_eq!(T.get(design::ELEVATION_2_WIDTH), Some(Length::Px(0)));
    /// ```
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().elevation(self)
    }
}

/// The motion set: the timing of the theme's animations and transitions by purpose
/// ([`MOTION_FAST`], [`MOTION_STANDARD`], [`MOTION_EMPHASIZED`], [`MOTION_EXIT`]). Each is a
/// `&'static AnimSpec`, so a curve is linked only when a motion set in use names it.
// NOTE(R7.S06): spring motion elements and the screen transitions join the set.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MotionSet {
    /// [`MOTION_FAST`].
    pub fast: &'static AnimSpec,
    /// [`MOTION_STANDARD`].
    pub standard: &'static AnimSpec,
    /// [`MOTION_EMPHASIZED`].
    pub emphasized: &'static AnimSpec,
    /// [`MOTION_EXIT`].
    pub exit: &'static AnimSpec,
}

/// The zero-duration timing of [`MotionSet::INSTANT`].
static INSTANT: AnimSpec = AnimSpec::new(Duration::ZERO);

impl MotionSet {
    /// No motion: every element is a zero-duration, linear timing (changes apply at once).
    /// For designs without decorative motion; links no curve.
    pub const INSTANT: MotionSet = MotionSet {
        fast: &INSTANT,
        standard: &INSTANT,
        emphasized: &INSTANT,
        exit: &INSTANT,
    };

    /// A table with these elements only (`const`). Never panics.
    ///
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::design::{self, MotionSet};
    ///
    /// static T: twine_style::design::ElementTable = MotionSet::INSTANT.to_table();
    /// assert_eq!(T.get(design::MOTION_STANDARD).map(|s| s.duration), Some(Duration::ZERO));
    /// ```
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().motions(self)
    }
}

/// The icon set: the image of each standard icon element ([`ICON_CHECK`] …
/// [`ICON_CLEAR`]), or `None` for icons the application does not use — an icon not listed is
/// not linked (icons are the one optional kind: [`ElementTable::missing_standard`] does not
/// require them).
// NOTE(R7.S07): the semantic `Icon` enum maps onto these elements.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IconSet {
    /// [`ICON_CHECK`].
    pub check: Option<&'static ImageSource>,
    /// [`ICON_CHEVRON_RIGHT`].
    pub chevron_right: Option<&'static ImageSource>,
    /// [`ICON_BACK`].
    pub back: Option<&'static ImageSource>,
    /// [`ICON_CLOSE`].
    pub close: Option<&'static ImageSource>,
    /// [`ICON_SEARCH`].
    pub search: Option<&'static ImageSource>,
    /// [`ICON_PLUS`].
    pub plus: Option<&'static ImageSource>,
    /// [`ICON_MINUS`].
    pub minus: Option<&'static ImageSource>,
    /// [`ICON_CLEAR`].
    pub clear: Option<&'static ImageSource>,
}

impl IconSet {
    /// No icons.
    pub const NONE: IconSet = IconSet {
        check: None,
        chevron_right: None,
        back: None,
        close: None,
        search: None,
        plus: None,
        minus: None,
        clear: None,
    };

    /// A table with these elements only (`const`). Never panics.
    ///
    /// ```
    /// use twine_image::{ImageSource, Symbol};
    /// use twine_style::design::{self, IconSet};
    ///
    /// static CHECK: ImageSource = ImageSource::symbol(Symbol::Ok);
    /// static T: twine_style::design::ElementTable = IconSet { check: Some(&CHECK), ..IconSet::NONE }.to_table();
    /// assert!(T.get(design::ICON_CHECK).is_some());
    /// assert!(T.get(design::ICON_BACK).is_none());
    /// ```
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().icons(self)
    }
}

/// The font scale: [`FONT_SMALL`], [`FONT_BODY`], [`FONT_LARGE`].
// NOTE(R7.S07): replaced by the `TypeScale` of text styles.
#[derive(Copy, Clone)]
pub struct FontSet {
    /// [`FONT_SMALL`].
    pub small: &'static Font,
    /// [`FONT_BODY`].
    pub body: &'static Font,
    /// [`FONT_LARGE`].
    pub large: &'static Font,
}

impl core::fmt::Debug for FontSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FontSet").finish_non_exhaustive()
    }
}

impl FontSet {
    /// `font` for all three (`const`).
    #[must_use]
    pub const fn uniform(font: &'static Font) -> Self {
        Self {
            small: font,
            body: font,
            large: font,
        }
    }

    /// A table with these elements only (`const`). Never panics.
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new().fonts(self)
    }
}

/// Every typed set: the standard elements of one mode. [`to_table`](Self::to_table) gives a
/// table that defines every required standard element:
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_style::design::{self, DesignSet, ElementTable, Elevation, FontSet, IconSet, MotionSet,
///     Shape, Spacing, SystemColors};
/// use twine_style::{Length, Radius};
///
/// const COLORS: SystemColors = SystemColors::monochrome(Color::BLACK, Color::WHITE);
/// const LIGHT: DesignSet = DesignSet {
///     colors: COLORS,
///     spacing: Spacing { xs: Length::dp(2), s: Length::dp(4), m: Length::dp(8), l: Length::dp(16) },
///     shape: Shape { s: Radius::Px(2), m: Radius::Px(6), l: Radius::Px(12), full: Radius::Circle },
///     elevation: Elevation::NONE,
///     motion: MotionSet::INSTANT,
///     icons: IconSet::NONE,
///     fonts: FontSet::uniform(&twine_text::EMPTY_FONT),
/// };
/// static LIGHT_TABLE: ElementTable = LIGHT.to_table();
/// assert_eq!(LIGHT_TABLE.get(design::ON_SURFACE), Some(Color::BLACK));
/// assert_eq!(LIGHT_TABLE.get(design::SPACE_M), Some(Length::dp(8)));
/// assert_eq!(LIGHT_TABLE.missing_standard(), None); // complete by construction
/// ```
#[derive(Copy, Clone, Debug)]
pub struct DesignSet {
    /// Colors and opacities.
    pub colors: SystemColors,
    /// The spacing scale.
    pub spacing: Spacing,
    /// The radius scale.
    pub shape: Shape,
    /// The shadow levels.
    pub elevation: Elevation,
    /// The motion timings.
    pub motion: MotionSet,
    /// The icons (optional ones).
    pub icons: IconSet,
    /// The font scale.
    pub fonts: FontSet,
}

impl DesignSet {
    /// The mode's table (`const`: `static LIGHT: ElementTable = SET.to_table();` lives in
    /// flash). Add application elements with [`ElementTable::custom`] and theme-private ones
    /// with [`ElementTable::private`]. Never panics.
    #[must_use]
    pub const fn to_table(self) -> ElementTable {
        ElementTable::new()
            .colors(self.colors)
            .spacing(self.spacing)
            .shape(self.shape)
            .elevation(self.elevation)
            .motions(self.motion)
            .icons(self.icons)
            .fonts(self.fonts)
    }
}

impl ElementTable {
    /// Sets every element of `c` (`const` builder; see [`SystemColors`]). Never panics.
    #[must_use]
    pub const fn colors(mut self, c: SystemColors) -> Self {
        // In element id order (`BACKGROUND` = 0 … `FILL_3` = 25; checked by a test).
        self.colors = [
            Some(c.background),
            Some(c.surface),
            Some(c.on_surface),
            Some(c.on_surface_muted),
            Some(c.surface_variant),
            Some(c.outline),
            Some(c.primary),
            Some(c.on_primary),
            Some(c.secondary),
            Some(c.on_secondary),
            Some(c.danger),
            Some(c.warning),
            Some(c.ok),
            Some(c.disabled),
            Some(c.focus_ring),
            Some(c.scrollbar),
            Some(c.shadow),
            Some(c.scrim),
            Some(c.placeholder),
            Some(c.neutral),
            Some(c.accent_tint),
            Some(c.separator),
            Some(c.grouped_background),
            Some(c.fill_1),
            Some(c.fill_2),
            Some(c.fill_3),
        ];
        self.opacities = [
            Some(c.focus_ring_opacity),
            Some(c.shadow_opacity),
            Some(c.disabled_opacity),
            Some(c.scrim_opacity),
        ];
        self
    }

    /// Sets every element of `s` (`const` builder; see [`Spacing`]). Never panics.
    #[must_use]
    pub const fn spacing(self, s: Spacing) -> Self {
        self.length(SPACE_XS, s.xs)
            .length(SPACE_S, s.s)
            .length(SPACE_M, s.m)
            .length(SPACE_L, s.l)
    }

    /// Sets every element of `s` (`const` builder; see [`Shape`]). Never panics.
    #[must_use]
    pub const fn shape(self, s: Shape) -> Self {
        self.radius(RADIUS_S, s.s)
            .radius(RADIUS_M, s.m)
            .radius(RADIUS_L, s.l)
            .radius(RADIUS_FULL, s.full)
    }

    /// Sets every element of `e` (`const` builder; see [`Elevation`]). Never panics.
    #[must_use]
    pub const fn elevation(self, e: Elevation) -> Self {
        self.length(ELEVATION_1_WIDTH, e.level_1.width)
            .length(ELEVATION_1_OFFSET_Y, e.level_1.offset_y)
            .length(ELEVATION_1_SPREAD, e.level_1.spread)
            .length(ELEVATION_2_WIDTH, e.level_2.width)
            .length(ELEVATION_2_OFFSET_Y, e.level_2.offset_y)
            .length(ELEVATION_2_SPREAD, e.level_2.spread)
            .length(ELEVATION_3_WIDTH, e.level_3.width)
            .length(ELEVATION_3_OFFSET_Y, e.level_3.offset_y)
            .length(ELEVATION_3_SPREAD, e.level_3.spread)
    }

    /// Sets every element of `m` (`const` builder; see [`MotionSet`]). Never panics.
    #[must_use]
    pub const fn motions(self, m: MotionSet) -> Self {
        self.motion(MOTION_FAST, m.fast)
            .motion(MOTION_STANDARD, m.standard)
            .motion(MOTION_EMPHASIZED, m.emphasized)
            .motion(MOTION_EXIT, m.exit)
    }

    /// Sets the icons `i` lists, clearing the others (`const` builder; see [`IconSet`]).
    /// Never panics.
    #[must_use]
    pub const fn icons(mut self, i: IconSet) -> Self {
        // In element id order (`ICON_CHECK` = 0 … `ICON_CLEAR` = 7; checked by a test).
        self.icons = [
            i.check,
            i.chevron_right,
            i.back,
            i.close,
            i.search,
            i.plus,
            i.minus,
            i.clear,
        ];
        self
    }

    /// Sets every element of `f` (`const` builder; see [`FontSet`]). Never panics.
    #[must_use]
    pub const fn fonts(self, f: FontSet) -> Self {
        self.font(FONT_SMALL, f.small)
            .font(FONT_BODY, f.body)
            .font(FONT_LARGE, f.large)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{
        BACKGROUND, Element, ElementType, FILL_3, ICON_CHECK, IconElement, MotionElement, SCRIM_OPACITY,
        SEPARATOR,
    };

    static SPEC: [AnimSpec; 4] = [
        AnimSpec::new(Duration::ms(1)),
        AnimSpec::new(Duration::ms(2)),
        AnimSpec::new(Duration::ms(3)),
        AnimSpec::new(Duration::ms(4)),
    ];
    static ICONS: [ImageSource; 8] = [
        ImageSource::Symbol("0"),
        ImageSource::Symbol("1"),
        ImageSource::Symbol("2"),
        ImageSource::Symbol("3"),
        ImageSource::Symbol("4"),
        ImageSource::Symbol("5"),
        ImageSource::Symbol("6"),
        ImageSource::Symbol("7"),
    ];

    /// A color unique per index.
    const fn c(i: u8) -> Color {
        Color::new(i, 0, 0)
    }
    const fn len(i: i32) -> Length {
        Length::Px(i)
    }

    const SET: DesignSet = DesignSet {
        colors: SystemColors {
            background: c(0),
            surface: c(1),
            on_surface: c(2),
            on_surface_muted: c(3),
            surface_variant: c(4),
            outline: c(5),
            primary: c(6),
            on_primary: c(7),
            secondary: c(8),
            on_secondary: c(9),
            danger: c(10),
            warning: c(11),
            ok: c(12),
            disabled: c(13),
            focus_ring: c(14),
            scrollbar: c(15),
            shadow: c(16),
            scrim: c(17),
            placeholder: c(18),
            neutral: c(19),
            accent_tint: c(20),
            separator: c(21),
            grouped_background: c(22),
            fill_1: c(23),
            fill_2: c(24),
            fill_3: c(25),
            focus_ring_opacity: Opa::from_raw(0),
            shadow_opacity: Opa::from_raw(1),
            disabled_opacity: Opa::from_raw(2),
            scrim_opacity: Opa::from_raw(3),
        },
        spacing: Spacing {
            xs: len(0),
            s: len(1),
            m: len(2),
            l: len(3),
        },
        shape: Shape {
            s: Radius::Px(0),
            m: Radius::Px(1),
            l: Radius::Px(2),
            full: Radius::Px(3),
        },
        elevation: Elevation {
            level_1: ElevationLevel {
                width: len(4),
                offset_y: len(5),
                spread: len(6),
            },
            level_2: ElevationLevel {
                width: len(7),
                offset_y: len(8),
                spread: len(9),
            },
            level_3: ElevationLevel {
                width: len(10),
                offset_y: len(11),
                spread: len(12),
            },
        },
        motion: MotionSet {
            fast: &SPEC[0],
            standard: &SPEC[1],
            emphasized: &SPEC[2],
            exit: &SPEC[3],
        },
        icons: IconSet {
            check: Some(&ICONS[0]),
            chevron_right: Some(&ICONS[1]),
            back: Some(&ICONS[2]),
            close: Some(&ICONS[3]),
            search: Some(&ICONS[4]),
            plus: Some(&ICONS[5]),
            minus: Some(&ICONS[6]),
            clear: Some(&ICONS[7]),
        },
        fonts: FontSet::uniform(&twine_text::EMPTY_FONT),
    };

    static TABLE: ElementTable = SET.to_table();

    /// Each field lands on the element whose id is its index in the set (the `c(i)`, `len(i)`
    /// … values encode the expected id).
    #[test]
    fn sets_fill_the_elements_in_id_order() {
        for id in 0..<Color as ElementType>::STANDARD {
            assert_eq!(
                TABLE.get(Element::<Color>::standard(id)),
                Some(c(id as u8)),
                "color {id}"
            );
        }
        for id in 0..<Length as ElementType>::STANDARD {
            assert_eq!(
                TABLE.get(Element::<Length>::standard(id)),
                Some(len(i32::from(id))),
                "length {id}"
            );
        }
        for id in 0..<Radius as ElementType>::STANDARD {
            assert_eq!(
                TABLE.get(Element::<Radius>::standard(id)),
                Some(Radius::Px(i32::from(id)))
            );
        }
        for id in 0..<Opa as ElementType>::STANDARD {
            assert_eq!(
                TABLE.get(Element::<Opa>::standard(id)),
                Some(Opa::from_raw(id as u8))
            );
        }
        for (id, spec) in (0..).zip(&SPEC) {
            assert!(
                TABLE
                    .get(MotionElement::standard(id))
                    .is_some_and(|s| core::ptr::eq(s, spec))
            );
        }
        for (id, icon) in (0..).zip(&ICONS) {
            assert!(
                TABLE
                    .get(IconElement::standard(id))
                    .is_some_and(|s| core::ptr::eq(s, icon))
            );
        }
        assert_eq!(TABLE.missing_standard(), None);
    }

    #[test]
    fn icons_are_optional_and_cleared() {
        let t = TABLE.clone().icons(IconSet::NONE);
        assert_eq!(t.get(ICON_CHECK), None);
        assert_eq!(t.missing_standard(), None);
        assert_eq!(ElementTable::new().missing_standard(), Some(BACKGROUND.erase()));
    }

    #[test]
    fn monochrome_defines_every_color() {
        let t = SystemColors::monochrome(Color::WHITE, Color::BLACK).to_table();
        assert_eq!(t.get(FILL_3), Some(Color::BLACK));
        assert_eq!(t.get(SEPARATOR), Some(Color::WHITE));
        assert_eq!(t.get(SCRIM_OPACITY), Some(Opa::COVER));
    }
}
