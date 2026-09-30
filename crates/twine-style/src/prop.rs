//! The property catalogue: [`PropId`], [`StyleProp`], [`PropMeta`] and [`PROP_META`].
//!
//! Everything here is generated from one table (`__prop_table!` in `table.rs`) so the id enum,
//! the typed property enum, the metadata, the `StyleBuf` builder methods, the `style!` keys and
//! the `twine-view` modifiers cannot drift apart.
//!
//! The table mirrors LVGL v9.6.0 (`include/lvgl/core/lv_style.h` for the list,
//! `src/misc/lv_style.c` for the flags table `lv_style_builtin_prop_flag_lookup_table` and the
//! defaults of `lv_style_prop_get_default`). Where LVGL's generated doc comments disagree with
//! that code, the code wins. The names are Twine's own (see `table.rs`); LVGL's are aliases.

use crate::value::StyleValue;

bitflags::bitflags! {
    /// What a property change affects (LVGL `LV_STYLE_PROP_FLAG_*`, same bit values).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct PropFlags: u8 {
        /// Inherited from the parent when not set (`LV_STYLE_PROP_FLAG_INHERITABLE`).
        const INHERITABLE = 1 << 0;
        /// Changes the extra draw area around the object (`LV_STYLE_PROP_FLAG_EXT_DRAW_UPDATE`).
        const EXT_DRAW = 1 << 1;
        /// Needs a layout update (`LV_STYLE_PROP_FLAG_LAYOUT_UPDATE`).
        const LAYOUT = 1 << 2;
        /// Needs a layout update of the parent (`LV_STYLE_PROP_FLAG_PARENT_LAYOUT_UPDATE`).
        const PARENT_LAYOUT = 1 << 3;
        /// Affects layer handling (`LV_STYLE_PROP_FLAG_LAYER_UPDATE`).
        const LAYER = 1 << 4;
        /// Affects the object's transformation (`LV_STYLE_PROP_FLAG_TRANSFORM`).
        const TRANSFORM = 1 << 5;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for PropFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "PropFlags({=u8:#x})", self.bits());
    }
}

/// Static metadata of one property (see [`PROP_META`]).
#[derive(Clone, Copy, Debug)]
pub struct PropMeta {
    /// Property name (`"BgColor"`).
    pub name: &'static str,
    /// The property's key: the `style!` key, the `StyleBuf` builder method and the view
    /// modifier (`"bg_color"`).
    pub snake_name: &'static str,
    /// Rust payload type of the [`StyleProp`] variant (`"Color"`).
    pub type_name: &'static str,
    /// Inherited from the parent's `Main` part when not set.
    pub inherited: bool,
    /// A change needs a layout update.
    pub layout: bool,
    /// A change may change the extra draw area (shadow, outline, transforms…).
    pub ext_draw: bool,
    /// Every LVGL flag of the property.
    pub flags: PropFlags,
    /// Value when no style sets the property (LVGL `lv_style_prop_get_default`). `Font`
    /// holds `twine_text::EMPTY_FONT`; resolution substitutes the caller's `StyleDefaults::font`.
    pub default: StyleValue,
    /// Lookup group, `id >> 4` (see `Style::has_group`).
    pub group: u8,
}

impl PropMeta {
    const fn new(
        name: &'static str,
        snake_name: &'static str,
        type_name: &'static str,
        flags: PropFlags,
        default: StyleValue,
        id: u8,
    ) -> Self {
        Self {
            name,
            snake_name,
            type_name,
            inherited: flags.contains(PropFlags::INHERITABLE),
            layout: flags.contains(PropFlags::LAYOUT),
            ext_draw: flags.contains(PropFlags::EXT_DRAW),
            flags,
            default,
            group: id >> 4,
        }
    }
}

/// Converts `style!` values of `Length`/`Radius`/`DurationMs` properties (integers or typed values) in
/// `const` context; every other value is taken as is (typed values only).
#[doc(hidden)]
#[macro_export]
macro_rules! __style_wrap {
    (len, $v:expr) => {
        $crate::__LengthArg($v).get()
    };
    (radius, $v:expr) => {
        $crate::__RadiusArg($v).get()
    };
    (dur, $v:expr) => {
        $crate::__DurationArg($v).get()
    };
    (val, $v:expr) => {
        $v
    };
}

/// Generates the property catalogue from the rows of `__prop_table!` (see `table.rs`).
macro_rules! define_props {
    (
        [$d:tt]
        $(
            $(#[doc = $doc:literal])*
            $name:ident ( $key:ident ) : $kind:ident [$($ty:tt)+] [ $($flag:ident)* ] $default:ident
                [ $($alias:literal)* ];
        )*
    ) => {
        /// Declaration order (first property = 1), used to number [`PropId`].
        #[allow(dead_code, clippy::enum_variant_names)]
        #[repr(u8)]
        enum Order { Invalid, $($name),* }

        /// Identifier of a style property: the fieldless discriminant of [`StyleProp`]
        /// (`1..=PROP_COUNT`; LVGL `lv_style_prop_t`, with Twine's own numbering and names).
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub enum PropId {
            $( $(#[doc = $doc])* $(#[doc(alias = $alias)])* $name = Order::$name as u8, )*
        }

        /// Number of style properties.
        pub const PROP_COUNT: usize = [$($crate::PropId::$name),*].len();

        impl $crate::PropId {
            /// Every property, in id order.
            pub const ALL: [$crate::PropId; PROP_COUNT] = [$($crate::PropId::$name),*];

            /// The property with id `v` (`None` for 0 and ids past [`PROP_COUNT`]).
            #[must_use]
            pub const fn from_u8(v: u8) -> ::core::option::Option<$crate::PropId> {
                if v == 0 || v as usize > PROP_COUNT {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(Self::ALL[v as usize - 1])
                }
            }
        }

        /// One style property with its value: one variant per [`PropId`], with the natural
        /// payload type. `Copy`; lives in `static` [`Style`](crate::Style)s or in
        /// [`StyleBuf`](crate::StyleBuf)s.
        #[derive(Clone, Copy, Debug)]
        pub enum StyleProp {
            $( $(#[doc = $doc])* $(#[doc(alias = $alias)])* $name($crate::__prop_ty!($($ty)+)), )*
        }

        impl $crate::StyleProp {
            /// The property's id.
            #[inline]
            #[must_use]
            pub const fn id(&self) -> $crate::PropId {
                match self {
                    $( $crate::StyleProp::$name(_) => $crate::PropId::$name, )*
                }
            }

            /// The value as a [`StyleValue`].
            #[inline]
            #[must_use]
            pub fn value(&self) -> $crate::StyleValue {
                match *self {
                    $( $crate::StyleProp::$name(v) => $crate::PropValue::into_value(v), )*
                }
            }

            /// Builds the property `id` from a value of the matching variant (`None` if the
            /// variant or enum code does not fit the property).
            #[must_use]
            pub fn from_value(id: $crate::PropId, v: $crate::StyleValue) -> ::core::option::Option<$crate::StyleProp> {
                match id {
                    $( $crate::PropId::$name => <$crate::__prop_ty!($($ty)+) as $crate::PropValue>::from_value(v).map($crate::StyleProp::$name), )*
                }
            }
        }

        /// Metadata of every property, indexed by `id as usize - 1` (use [`PropId::meta`]).
        pub static PROP_META: [$crate::PropMeta; PROP_COUNT] = [
            $(
                $crate::PropMeta::new(
                    ::core::stringify!($name),
                    ::core::stringify!($key),
                    $crate::__prop_type_name!($($ty)+),
                    $crate::PropFlags::empty()$(.union($crate::PropFlags::$flag))*,
                    $crate::__private::$default,
                    $crate::PropId::$name as u8,
                ),
            )*
        ];

        /// The former names of every property, indexed like [`PROP_META`]: the previous Twine
        /// key (when it changed) and the LVGL constant. They are `#[doc(alias)]`es of the
        /// property, its builder method and its view modifier, and are listed in
        /// `PROPERTIES.md`; `style!` does not accept them. Kept out of [`PropMeta`] so firmware
        /// that never reads it does not link the strings.
        pub static PROP_ALIASES: [&[&str]; PROP_COUNT] = [ $( &[$($alias),*], )* ];

        /// Builder methods, one per property (named like the `style!` keys and the view
        /// modifiers).
        impl $crate::StyleBuf {
            $(
                $(#[doc = $doc])*
                ///
                #[doc = ::core::concat!("Sets [`StyleProp::", ::core::stringify!($name), "`] and returns the buffer (builder style).")]
                $(#[doc(alias = $alias)])*
                #[must_use]
                pub fn $key(mut self, v: $crate::__builder_ty!($($ty)+)) -> Self {
                    self.set($crate::StyleProp::$name(::core::convert::Into::into(v)));
                    self
                }
            )*
        }

        /// Maps a `style!` key to its `StyleProp` (generated from the property table).
        #[doc(hidden)]
        #[macro_export]
        macro_rules! __style_prop {
            $(
                ($key, $d v:expr) => { $crate::StyleProp::$name($crate::__style_wrap!($kind, $d v)) };
            )*
            ($d other:ident, $d v:expr) => {
                ::core::compile_error!(::core::concat!(
                    "unknown style property: ", ::core::stringify!($d other),
                    " (the keys and the former names they replace are listed in twine-style/PROPERTIES.md)"
                ))
            };
        }

        #[cfg(test)]
        pub(crate) mod generated_tests {
            /// `(id, key)` of every row, for tests.
            pub(crate) const ROWS: &[($crate::PropId, &str)] = &[$(($crate::PropId::$name, ::core::stringify!($key))),*];
        }
    };
}

crate::__prop_table!(define_props $);

impl PropId {
    /// The property's metadata.
    #[inline]
    #[must_use]
    pub fn meta(self) -> &'static PropMeta {
        &PROP_META[self as usize - 1]
    }

    /// Lookup group: `id >> 4` (see [`Style::has_group`](crate::Style::has_group)).
    #[inline]
    #[must_use]
    pub const fn group(self) -> u8 {
        self as u8 >> 4
    }

    /// Bit of this property's group in a `has_group` mask.
    #[inline]
    #[must_use]
    pub const fn group_bit(self) -> u16 {
        1 << self.group()
    }

    /// Property name (`"BgColor"`).
    #[must_use]
    pub fn name(self) -> &'static str {
        self.meta().name
    }
}

impl core::fmt::Display for PropId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

// A style property must stay small (12 bytes on 32-bit targets): styles are scanned linearly
// and live in flash.
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<StyleProp>() <= 12);

#[cfg(test)]
mod tests {
    use twine_anim::AnimTemplate;
    use twine_core::{Color, Opa, Scale};
    use twine_image::ImageSource;

    use super::generated_tests::ROWS;
    use super::*;
    use crate::transition::TransitionDsc;
    use crate::value_types::{
        Align, BaseDir, ColorFilter, FlexFlow, Gradient, GridTrack, ImageColorkey, LayoutKind, Length,
        TextAlign,
    };

    #[test]
    fn prop_count_le_128() {
        // LVGL v9.6 has 129 built-in properties: more than 128, so the group mask is a `u16`
        // (groups 0..=8), see DEVIATIONS.
        assert_eq!(PROP_COUNT, 129);
        assert!(u8::try_from(PROP_COUNT).is_ok());
        assert!(
            usize::from(PropId::ALL[PROP_COUNT - 1].group()) < 16,
            "group mask is u16"
        );
        for (i, id) in PropId::ALL.iter().enumerate() {
            assert_eq!(*id as usize, i + 1);
            assert_eq!(PropId::from_u8(i as u8 + 1), Some(*id));
        }
        assert_eq!(PropId::from_u8(0), None);
        assert_eq!(PropId::from_u8(PROP_COUNT as u8 + 1), None);
    }

    /// A value of the right variant for every property, including those whose default is
    /// `None`.
    pub(crate) fn sample(id: PropId) -> StyleValue {
        static GRAD: Gradient = Gradient::new(twine_render::GradKind::Ver, &[]);
        static IMG: ImageSource = ImageSource::Symbol("s");
        static KEY: ImageColorkey = ImageColorkey {
            low: Color::BLACK,
            high: Color::WHITE,
        };
        static FILTER: ColorFilter = ColorFilter::SHADE;
        static ANIM: AnimTemplate =
            AnimTemplate::new(twine_core::Duration::ms(1), twine_anim::Easing::Linear);
        static TR: TransitionDsc = TransitionDsc::new(
            &[PropId::BgColor],
            twine_core::Duration::ms(1),
            twine_anim::Easing::Linear,
        );
        static TRACKS: [GridTrack; 2] = [GridTrack::Px(10), GridTrack::Fr(1)];
        match id.meta().type_name {
            "&'static Gradient" => StyleValue::Grad(&GRAD),
            "&'static ImageSource" => StyleValue::Image(&IMG),
            "&'static ImageColorkey" => StyleValue::Colorkey(&KEY),
            "&'static ColorFilter" => StyleValue::ColorFilter(&FILTER),
            "&'static AnimTemplate" => StyleValue::AnimTemplate(&ANIM),
            "&'static TransitionDsc" => StyleValue::Transition(&TR),
            "&'static [GridTrack]" => StyleValue::GridTracks(&TRACKS),
            _ => id.meta().default,
        }
    }

    #[test]
    fn every_prop_has_meta_and_roundtrips() {
        assert_eq!(ROWS.len(), PROP_COUNT);
        for &(id, snake) in ROWS {
            let m = id.meta();
            assert_eq!(m.snake_name, snake);
            assert_eq!(m.name, alloc::format!("{id:?}"));
            assert_eq!(m.group, id as u8 >> 4);
            let v = sample(id);
            assert!(!v.is_none(), "{id:?} has no sample");
            let p = StyleProp::from_value(id, v).unwrap_or_else(|| panic!("{id:?}: {v:?} does not fit"));
            assert_eq!(p.id(), id);
            assert_eq!(p.value(), v, "{id:?}");
            // A value of another kind is rejected.
            let wrong = if matches!(v, StyleValue::Bool(_)) {
                StyleValue::Int(0)
            } else {
                StyleValue::Bool(true)
            };
            assert!(
                StyleProp::from_value(id, wrong).is_none(),
                "{id:?} accepts {wrong:?}"
            );
        }
    }

    #[test]
    fn every_prop_is_aliased_to_its_lvgl_constant() {
        for (id, aliases) in PropId::ALL.iter().zip(PROP_ALIASES.iter()) {
            let lvgl = aliases.last().copied().unwrap_or_default();
            assert!(lvgl.starts_with("LV_STYLE_"), "{id:?}: {aliases:?}");
            // Former Twine keys are snake_case and differ from the current key.
            for a in &aliases[..aliases.len() - 1] {
                assert_ne!(*a, id.meta().snake_name, "{id:?}");
                assert!(
                    a.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                    "{id:?}: {a}"
                );
            }
        }
    }

    #[test]
    fn inherited_set_matches_lvgl_list() {
        // LVGL v9.6.0 src/misc/lv_style.c: properties with LV_STYLE_PROP_FLAG_INHERITABLE.
        let lvgl = [
            PropId::TextColor,
            PropId::TextOpacity,
            PropId::Font,
            PropId::LetterSpacing,
            PropId::LineSpacing,
            PropId::TextDecoration,
            PropId::TextAlign,
            PropId::ColorFilter,
            PropId::ColorFilterOpacity,
            PropId::BaseDir,
            PropId::TextLeadingTrim,
        ];
        for id in PropId::ALL {
            assert_eq!(id.meta().inherited, lvgl.contains(&id), "{id:?}");
        }
    }

    #[test]
    fn layout_and_ext_draw_flags_match_lvgl() {
        use PropFlags as F;
        // Copied from LVGL v9.6.0 src/misc/lv_style.c `lv_style_builtin_prop_flag_lookup_table`
        // (properties missing there have no flags).
        let lvgl: &[(PropId, PropFlags)] = &[
            (PropId::Width, F::LAYOUT),
            (PropId::MinWidth, F::LAYOUT),
            (PropId::MaxWidth, F::LAYOUT),
            (PropId::Height, F::LAYOUT),
            (PropId::MinHeight, F::LAYOUT),
            (PropId::MaxHeight, F::LAYOUT),
            (PropId::Length, F::EXT_DRAW),
            (PropId::X, F::LAYOUT),
            (PropId::Y, F::LAYOUT),
            (PropId::Align, F::LAYOUT),
            (PropId::TransformWidth, F::EXT_DRAW.union(F::TRANSFORM)),
            (PropId::TransformHeight, F::EXT_DRAW.union(F::TRANSFORM)),
            (PropId::TranslateX, F::LAYOUT.union(F::PARENT_LAYOUT)),
            (PropId::TranslateY, F::LAYOUT.union(F::PARENT_LAYOUT)),
            (
                PropId::TransformScaleX,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformScaleY,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformSkewX,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformSkewY,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (
                PropId::TransformRotation,
                F::EXT_DRAW.union(F::LAYER).union(F::TRANSFORM),
            ),
            (PropId::PaddingTop, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PaddingBottom, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PaddingLeft, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::PaddingRight, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::RowGap, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::ColumnGap, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginTop, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginBottom, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginLeft, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::MarginRight, F::EXT_DRAW.union(F::LAYOUT)),
            (PropId::BgImage, F::EXT_DRAW),
            (PropId::BorderWidth, F::LAYOUT),
            (PropId::OutlineWidth, F::EXT_DRAW),
            (PropId::OutlineOpacity, F::EXT_DRAW),
            (PropId::OutlineOffset, F::EXT_DRAW),
            (PropId::ShadowWidth, F::EXT_DRAW),
            (PropId::ShadowOffsetX, F::EXT_DRAW),
            (PropId::ShadowOffsetY, F::EXT_DRAW),
            (PropId::ShadowSpread, F::EXT_DRAW),
            (PropId::ShadowOpacity, F::EXT_DRAW),
            (PropId::LineWidth, F::EXT_DRAW),
            (PropId::ArcWidth, F::EXT_DRAW),
            (PropId::TextColor, F::INHERITABLE),
            (PropId::TextOpacity, F::INHERITABLE),
            (PropId::Font, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::LetterSpacing, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::LineSpacing, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::TextDecoration, F::INHERITABLE),
            (PropId::TextAlign, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::Opacity, F::LAYER),
            (PropId::ColorFilter, F::INHERITABLE),
            (PropId::ColorFilterOpacity, F::INHERITABLE),
            (PropId::BlendMode, F::LAYER),
            (PropId::Layout, F::LAYOUT),
            (PropId::BaseDir, F::INHERITABLE.union(F::LAYOUT)),
            (PropId::BitmapMask, F::LAYER),
            (PropId::DropShadowRadius, F::EXT_DRAW),
            (PropId::DropShadowOffsetX, F::EXT_DRAW),
            (PropId::DropShadowOffsetY, F::EXT_DRAW),
            (PropId::DropShadowOpacity, F::EXT_DRAW),
            (PropId::FlexFlow, F::LAYOUT),
            (PropId::FlexMainAlign, F::LAYOUT),
            (PropId::FlexCrossAlign, F::LAYOUT),
            (PropId::FlexTrackAlign, F::LAYOUT),
            (PropId::FlexGrow, F::LAYOUT),
            (PropId::GridColumnTracks, F::LAYOUT),
            (PropId::GridRowTracks, F::LAYOUT),
            (PropId::GridColumnAlign, F::LAYOUT),
            (PropId::GridRowAlign, F::LAYOUT),
            (PropId::GridCellRowSpan, F::LAYOUT),
            (PropId::GridCellRow, F::LAYOUT),
            (PropId::GridCellColumnSpan, F::LAYOUT),
            (PropId::GridCellColumn, F::LAYOUT),
            (PropId::GridCellXAlign, F::LAYOUT),
            (PropId::GridCellYAlign, F::LAYOUT),
            (PropId::TextLeadingTrim, F::INHERITABLE.union(F::LAYOUT)),
        ];
        for id in PropId::ALL {
            let want = lvgl
                .iter()
                .find(|(p, _)| *p == id)
                .map_or(F::empty(), |(_, f)| *f);
            let m = id.meta();
            assert_eq!(m.flags, want, "{id:?}");
            assert_eq!(m.layout, want.contains(F::LAYOUT), "{id:?}");
            assert_eq!(m.ext_draw, want.contains(F::EXT_DRAW), "{id:?}");
        }
    }

    #[test]
    fn defaults_match_lvgl() {
        use StyleValue as V;
        // LVGL v9.6.0 src/misc/lv_style.c `lv_style_prop_get_default` (every other property: 0 /
        // NULL). Width/Height: LVGL takes them from the widget class; Twine defaults to Content.
        let non_zero: &[(PropId, StyleValue)] = &[
            (PropId::Width, V::Length(Length::Content)),
            (PropId::Height, V::Length(Length::Content)),
            (PropId::TransformScaleX, V::Scale(Scale::from_raw_256(256))),
            (PropId::TransformScaleY, V::Scale(Scale::from_raw_256(256))),
            (PropId::BgColor, V::Color(Color::WHITE)),
            (PropId::BgGradientColor, V::Color(Color::BLACK)),
            (PropId::BorderColor, V::Color(Color::BLACK)),
            (PropId::ShadowColor, V::Color(Color::BLACK)),
            (PropId::OutlineColor, V::Color(Color::BLACK)),
            (PropId::ArcColor, V::Color(Color::BLACK)),
            (PropId::LineColor, V::Color(Color::BLACK)),
            (PropId::TextColor, V::Color(Color::BLACK)),
            (PropId::DropShadowColor, V::Color(Color::BLACK)),
            (PropId::ImageRecolor, V::Color(Color::BLACK)),
            (PropId::Recolor, V::Color(Color::BLACK)),
            (PropId::PartOpacity, V::Opa(Opa::COVER)),
            (PropId::Opacity, V::Opa(Opa::COVER)),
            (PropId::BorderOpacity, V::Opa(Opa::COVER)),
            (PropId::TextOpacity, V::Opa(Opa::COVER)),
            (PropId::ImageOpacity, V::Opa(Opa::COVER)),
            (PropId::BgGradientEndOpacity, V::Opa(Opa::COVER)),
            (PropId::BgGradientStartOpacity, V::Opa(Opa::COVER)),
            (PropId::BgImageOpacity, V::Opa(Opa::COVER)),
            (PropId::OutlineOpacity, V::Opa(Opa::COVER)),
            (PropId::LineOpacity, V::Opa(Opa::COVER)),
            (PropId::ArcOpacity, V::Opa(Opa::COVER)),
            (PropId::ShadowOpacity, V::Opa(Opa::COVER)),
            (PropId::BgGradientEnd, V::Int(255)),
            (PropId::BorderSide, V::Enum(0x0F)),
            (PropId::Font, V::Font(&twine_text::EMPTY_FONT)),
            (PropId::MaxWidth, V::Length(Length::Px((1 << 29) - 1))),
            (PropId::MaxHeight, V::Length(Length::Px((1 << 29) - 1))),
            (PropId::RotarySensitivity, V::Scale(Scale::ONE)),
            (PropId::DropShadowQuality, V::Enum(2)),
            (PropId::GridCellRowSpan, V::Int(1)),
            (PropId::GridCellColumnSpan, V::Int(1)),
            // LVGL's zero of an enum is its first value; in Twine's encoding that is only
            // different for `TextAlign` (LVGL `LV_TEXT_ALIGN_AUTO` = 0).
            (PropId::TextAlign, V::Enum(TextAlign::Auto as u8)),
        ];
        for id in PropId::ALL {
            let d = id.meta().default;
            if let Some((_, want)) = non_zero.iter().find(|(p, _)| *p == id) {
                assert_eq!(d, *want, "{id:?}");
                continue;
            }
            let zero = match d {
                V::None | V::Int(0) | V::Length(Length::Px(0)) | V::Bool(false) | V::Enum(0) => true,
                V::Color(c) => c == Color::BLACK,
                V::Opa(o) => o == Opa::TRANSP,
                V::Angle(a) => a.as_deci_deg() == 0,
                _ => false,
            };
            assert!(zero, "{id:?}: default {d:?} is not LVGL's 0/NULL");
        }
        assert_eq!(PropId::Align.meta().default.as_align(), Some(Align::Default));
        assert_eq!(PropId::BaseDir.meta().default.as_base_dir(), Some(BaseDir::Ltr));
        assert_eq!(
            PropId::FlexFlow.meta().default.as_flex_flow(),
            Some(FlexFlow::ROW)
        );
        assert_eq!(PropId::Layout.meta().default.as_layout(), Some(LayoutKind::None));
    }

    #[test]
    fn group_is_id_shift_4() {
        for id in PropId::ALL {
            assert_eq!(id.group(), id as u8 >> 4);
            assert_eq!(id.meta().group, id.group());
            assert_eq!(id.group_bit(), 1u16 << (id as u8 >> 4));
        }
        assert_eq!(PropId::Width.group(), 0);
        assert_eq!(PropId::GridCellYAlign.group(), 8);
    }

    #[test]
    fn size_of_style_prop() {
        // 12 bytes on 32-bit targets (checked at compile time there); two words + tag on 64-bit.
        assert!(core::mem::size_of::<StyleProp>() <= 3 * core::mem::size_of::<usize>());
        assert!(core::mem::size_of::<StyleValue>() <= 3 * core::mem::size_of::<usize>());
    }
}
