//! Style transitions: the [`TransitionDsc`] descriptor and value interpolation.
//!
//! When a node changes state, the engine resolves every property listed in the new state's
//! `Transition` descriptor in the old and the new state (without transition entries). For each
//! property whose values differ ([`StyleValue`] equality) it adds a transition entry holding the
//! old value at the highest priority and animates it `0 → 255` with the descriptor's timing,
//! writing [`interpolate`]`(prop, old, new, t)` into the entry; the entry is removed when the
//! animation completes (LVGL `lv_obj_style_create_transition` / `trans_anim_cb`).

use twine_anim::Easing;
use twine_core::{Angle, Color, Duration, Opa, Scale};

use crate::prop::PropId;
use crate::value::StyleValue;
use crate::value_types::Length;

/// Which properties animate on a state change, and how (LVGL `lv_style_transition_dsc_t`).
///
/// Set through the `Transition` style property of the *target* state.
///
/// ```
/// use twine_anim::Easing;
/// use twine_core::Duration;
/// use twine_style::{PropId, TransitionDsc};
///
/// static FADE: TransitionDsc =
///     TransitionDsc::new(&[PropId::BgColor, PropId::BgOpa], Duration::ms(200), Easing::EaseOut).delay(Duration::ms(50));
/// assert_eq!(FADE.props.len(), 2);
/// assert_eq!(FADE.delay, Duration::ms(50));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TransitionDsc {
    /// The animated properties.
    pub props: &'static [PropId],
    /// Animation time.
    pub duration: Duration,
    /// Delay before the animation starts.
    pub delay: Duration,
    /// Easing curve.
    pub easing: Easing,
}

impl TransitionDsc {
    /// Transitions for `props` taking `duration` with `easing`, without delay.
    #[must_use]
    pub const fn new(props: &'static [PropId], duration: Duration, easing: Easing) -> Self {
        Self {
            props,
            duration,
            delay: Duration::ZERO,
            easing,
        }
    }

    /// Sets the delay before the transition starts.
    #[must_use]
    pub const fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// Whether `prop` is animated by this descriptor.
    #[must_use]
    pub fn contains(&self, prop: PropId) -> bool {
        self.props.contains(&prop)
    }
}

/// Whether values of `prop` change gradually during a transition (integers, lengths, colors,
/// opacities, angles and scales). Other properties (fonts, images, enums, flags, references)
/// switch from the old to the new value at the end.
#[must_use]
pub fn is_interpolable(prop: PropId) -> bool {
    matches!(
        prop.meta().default,
        StyleValue::Int(_)
            | StyleValue::Length(_)
            | StyleValue::Color(_)
            | StyleValue::Opa(_)
            | StyleValue::Angle(_)
            | StyleValue::Scale(_)
    )
}

/// LVGL `trans_anim_cb` number mixing: `from + ((to − from) · t) >> 8` (arithmetic shift),
/// with exact end points.
fn lerp(from: i32, to: i32, t: u8) -> i32 {
    match t {
        0 => from,
        255 => to,
        _ => {
            let v = i64::from(from) + (((i64::from(to) - i64::from(from)) * i64::from(t)) >> 8);
            v as i32 // between `from` and `to`, so in range
        }
    }
}

/// The value of `prop` at transition progress `t` (`0` = `from`, `255` = `to`), as LVGL's
/// `trans_anim_cb`:
///
/// - colors: `Color::mix(to, from, t)` (LVGL `lv_color_mix`);
/// - integers, opacities, angles, scales and two lengths of the same unit (`Px`/`Pct`):
///   `from + ((to − from) · t) >> 8`;
/// - `ColorFilterDsc`: the one that is set if the other is not, else switches at `t = 128`;
/// - everything else (lengths of different units, fonts, images, enums, flags, references):
///   `from` until `t = 255`, then `to`.
///
/// ```
/// use twine_core::Color;
/// use twine_style::{PropId, StyleValue, interpolate};
///
/// let mid = interpolate(PropId::BgColor, &StyleValue::Color(Color::BLACK), &StyleValue::Color(Color::WHITE), 128);
/// assert_eq!(mid, StyleValue::Color(Color::hex(0x808080)));
/// assert_eq!(interpolate(PropId::Radius, &StyleValue::Int(0), &StyleValue::Int(100), 64), StyleValue::Int(25));
/// ```
#[must_use]
pub fn interpolate(prop: PropId, from: &StyleValue, to: &StyleValue, t: u8) -> StyleValue {
    use StyleValue as V;
    let switch = || if t == 255 { *to } else { *from };
    if prop == PropId::ColorFilterDsc {
        return match (from, to) {
            (V::None, _) => *to,
            (_, V::None) => *from,
            _ if t < 128 => *from,
            _ => *to,
        };
    }
    match (*from, *to) {
        (V::Color(a), V::Color(b)) => match t {
            0 => V::Color(a),
            255 => V::Color(b),
            _ => V::Color(Color::mix(b, a, Opa(t))),
        },
        (V::Int(a), V::Int(b)) => V::Int(lerp(a, b, t)),
        (V::Opa(a), V::Opa(b)) => V::Opa(Opa(lerp(i32::from(a.0), i32::from(b.0), t) as u8)),
        (V::Angle(a), V::Angle(b)) => V::Angle(Angle(lerp(a.0, b.0, t))),
        (V::Scale(a), V::Scale(b)) => V::Scale(Scale(lerp(i32::from(a.0), i32::from(b.0), t) as u16)),
        (V::Length(Length::Px(a)), V::Length(Length::Px(b))) => V::Length(Length::Px(lerp(a, b, t))),
        (V::Length(Length::Pct(a)), V::Length(Length::Pct(b))) => {
            V::Length(Length::Pct(lerp(i32::from(a), i32::from(b), t) as i16))
        }
        _ => switch(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn interpolate_color_endpoints_and_mid() {
        let (a, b) = (StyleValue::Color(Color::RED), StyleValue::Color(Color::BLUE));
        assert_eq!(interpolate(PropId::BgColor, &a, &b, 0), a);
        assert_eq!(interpolate(PropId::BgColor, &a, &b, 255), b);
        assert_eq!(
            interpolate(PropId::TextColor, &a, &b, 128),
            StyleValue::Color(Color::mix(Color::BLUE, Color::RED, Opa(128)))
        );
        // Every color property mixes (LVGL mixes only its 8 listed color props).
        assert_eq!(
            interpolate(PropId::LineColor, &a, &b, 128),
            StyleValue::Color(Color::mix(Color::BLUE, Color::RED, Opa(128)))
        );
    }

    proptest! {
        #[test]
        fn interpolate_int_monotonic(a in -100_000i32..100_000, b in -100_000i32..100_000) {
            let mut prev = a;
            for t in 0..=255u8 {
                let StyleValue::Int(v) = interpolate(PropId::Radius, &StyleValue::Int(a), &StyleValue::Int(b), t) else {
                    panic!("not an int");
                };
                if a <= b { prop_assert!(v >= prev && v <= b); } else { prop_assert!(v <= prev && v >= b); }
                prev = v;
            }
            prop_assert_eq!(prev, b);
        }
    }

    #[test]
    fn lerp_matches_lvgl() {
        // start + ((end - start) * v >> 8), arithmetic shift (rounds toward -inf).
        assert_eq!(lerp(0, 100, 128), 50);
        assert_eq!(lerp(100, 0, 128), 50);
        assert_eq!(lerp(0, -100, 1), -1);
        assert_eq!(lerp(10, 20, 254), 19);
        assert_eq!(lerp(i32::MIN, i32::MAX, 128), -1);
        assert_eq!(
            interpolate(
                PropId::BgOpa,
                &StyleValue::Opa(Opa(0)),
                &StyleValue::Opa(Opa(255)),
                100
            ),
            StyleValue::Opa(Opa(99))
        );
        assert_eq!(
            interpolate(
                PropId::TransformScaleX,
                &StyleValue::Scale(Scale(256)),
                &StyleValue::Scale(Scale(512)),
                128
            ),
            StyleValue::Scale(Scale(384))
        );
        assert_eq!(
            interpolate(
                PropId::TransformRotation,
                &StyleValue::Angle(Angle(0)),
                &StyleValue::Angle(Angle(900)),
                64
            ),
            StyleValue::Angle(Angle(225))
        );
    }

    #[test]
    fn non_interpolable_switches_at_end() {
        static F1: twine_text::Font = crate::test_util::font(1);
        static F2: twine_text::Font = crate::test_util::font(1);
        let (a, b) = (StyleValue::Font(&F1), StyleValue::Font(&F2));
        assert!(!is_interpolable(PropId::TextFont));
        assert_eq!(interpolate(PropId::TextFont, &a, &b, 254), a);
        assert_eq!(interpolate(PropId::TextFont, &a, &b, 255), b);
        let (a, b) = (StyleValue::Enum(1), StyleValue::Enum(9));
        assert!(!is_interpolable(PropId::Align));
        assert_eq!(interpolate(PropId::Align, &a, &b, 200), a);
        assert_eq!(interpolate(PropId::Align, &a, &b, 255), b);
        assert_eq!(
            interpolate(
                PropId::ClipCorner,
                &StyleValue::Bool(false),
                &StyleValue::Bool(true),
                128
            ),
            StyleValue::Bool(false)
        );
        assert!(
            is_interpolable(PropId::BgColor)
                && is_interpolable(PropId::Width)
                && is_interpolable(PropId::Opa)
        );
        assert!(is_interpolable(PropId::TransformRotation) && is_interpolable(PropId::TransformScaleY));
    }

    #[test]
    fn length_mixed_variants_switch() {
        let px = StyleValue::Length(Length::Px(10));
        let pct = StyleValue::Length(Length::Pct(50));
        let content = StyleValue::Length(Length::Content);
        assert_eq!(interpolate(PropId::Width, &px, &pct, 200), px);
        assert_eq!(interpolate(PropId::Width, &px, &pct, 255), pct);
        assert_eq!(interpolate(PropId::Width, &content, &px, 128), content);
        assert_eq!(interpolate(PropId::Width, &content, &px, 255), px);
        assert_eq!(
            interpolate(PropId::Width, &px, &StyleValue::Length(Length::Px(30)), 128),
            StyleValue::Length(Length::Px(20))
        );
        assert_eq!(
            interpolate(PropId::Width, &pct, &StyleValue::Length(Length::Pct(100)), 128),
            StyleValue::Length(Length::Pct(75))
        );
    }

    #[test]
    fn color_filter_switches_at_half() {
        static F: crate::ColorFilter = crate::ColorFilter::SHADE;
        static G: crate::ColorFilter = crate::ColorFilter::SHADE;
        let (f, g) = (StyleValue::ColorFilter(&F), StyleValue::ColorFilter(&G));
        assert_eq!(interpolate(PropId::ColorFilterDsc, &StyleValue::None, &f, 0), f);
        assert_eq!(interpolate(PropId::ColorFilterDsc, &f, &StyleValue::None, 255), f);
        assert_eq!(interpolate(PropId::ColorFilterDsc, &f, &g, 127), f);
        assert_eq!(interpolate(PropId::ColorFilterDsc, &f, &g, 128), g);
    }

    #[test]
    fn transition_dsc_const_in_static() {
        static PROPS: [PropId; 2] = [PropId::BgColor, PropId::TransformScaleX];
        static T: TransitionDsc = TransitionDsc::new(&PROPS, Duration::ms(150), Easing::EaseOut);
        static D: TransitionDsc = T.delay(Duration::ms(20));
        assert_eq!(T.delay, Duration::ZERO);
        assert_eq!(D.delay, Duration::ms(20));
        assert_eq!(D.easing, Easing::EaseOut);
        assert!(T.contains(PropId::BgColor) && !T.contains(PropId::Radius));
    }
}
