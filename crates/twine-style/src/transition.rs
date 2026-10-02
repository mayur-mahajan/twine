//! Style transitions: the [`Transition`] descriptor, how a style holds it
//! ([`TransitionRef`]) and value interpolation.
//!
//! When a node changes state, the engine collects the `Transition` property of every style
//! entry that applies in the new state, works out which properties each one animates (its
//! [`Props`], or the properties the state change alters), resolves each of them in the old and
//! the new state (without transition entries) and, for each property whose values differ
//! ([`StyleValue`] equality), adds a transition entry holding the old value at the highest
//! priority and animates it `0 → 255` with the descriptor's [`AnimSpec`], writing
//! [`interpolate`]`(prop, old, new, t)` into the entry; the entry is removed when the animation
//! completes (LVGL `lv_obj_style_create_transition` / `trans_anim_cb`).
//!
//! A transition is a value (`Copy`): a `static` one lives in flash and is referenced by a
//! `'static` reference ([`TransitionRef::Static`], e.g. in `style!`); one built at run time
//! (`.transition(Transition::all(Duration::ms(150)))` on a view) is held by the style container
//! that sets it ([`TransitionRef::Local`]), like a run-time grid template: created once at build
//! time, shared by the container's clones, released with the property.

use alloc::rc::Rc;

use twine_anim::{AnimSpec, Easing};
use twine_core::{Angle, Color, Duration, Opa, Scale};

use crate::prop::PropId;
use crate::props::Props;
use crate::value::StyleValue;
use crate::value_types::Length;

/// Which properties animate when a node enters a state, and how (LVGL
/// `lv_style_transition_dsc_t`). Set as the `transition` property of the *target* state's
/// style: `.on_state(State::PRESSED, |s| s.transition(..).bg(..))`, or on the default state
/// to animate every state change.
///
/// - [`Transition::all`]: the properties the state change alters — the keys of every style of
///   the part that starts or stops applying (e.g. the pressed style's `bg` and `scale`), the
///   interpolable ones only ([`Props::INTERPOLABLE`]: fonts, images, enums and other discrete
///   values switch at once). The default: no list to keep in sync with the styles.
/// - [`Transition::of`]: exactly the properties of a [`Props`] set (e.g.
///   `Props::BG | Props::TRANSFORM`); a listed discrete property (a grid template, an image)
///   switches at the end of the transition, as in LVGL.
///
/// The timing is an [`AnimSpec`] (the one type of every animation): duration, easing, delay,
/// [`essential`](Self::essential). A transition plays its forward play once (the spec's
/// repetition and playback do not apply). Under a reduced [`Motion`](twine_anim::Motion)
/// preference transitions are shortened, and not created at all with `Motion::None`.
///
/// Where the same property of a part is covered by several transitions, the one of the style
/// whose selector has the higher [state precedence](crate::State#precedence) wins (the same
/// rule as for values); equal precedence → the earlier entry.
///
/// ```
/// use twine_core::Duration;
/// use twine_style::{Easing, PropId, Props, Transition};
///
/// // In flash, for `style!` and themes (`const` context: `union` instead of `|`).
/// static PRESS: Transition =
///     Transition::of(Props::BG.union(Props::TRANSFORM), Duration::ms(80)).delay(Duration::ms(70));
/// assert!(PRESS.props.is_some_and(|p| p.contains(PropId::TransformScaleX)));
/// assert_eq!(PRESS.spec.delay, Duration::ms(70));
///
/// // Built where it is used (a view's `.transition(..)` modifier takes it by value).
/// let t = Transition::all(Duration::ms(150)).ease_out();
/// assert_eq!((t.props, t.spec.easing), (None, Easing::EaseOut));
/// ```
#[doc(alias = "lv_style_transition_dsc_t")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Transition {
    /// The animated properties: `Some(set)` exactly these, `None` the properties the state
    /// change alters (see [`Transition::all`]).
    pub props: Option<Props>,
    /// The timing (forward play only).
    pub spec: AnimSpec,
}

impl Transition {
    /// Animates every property the state change alters (interpolable ones), over `duration`,
    /// linear, without delay. See [`Transition`].
    #[must_use]
    pub const fn all(duration: Duration) -> Self {
        Self {
            props: None,
            spec: AnimSpec::new(duration),
        }
    }

    /// Animates the properties of `props` over `duration`, linear, without delay.
    #[must_use]
    pub const fn of(props: Props, duration: Duration) -> Self {
        Self {
            props: Some(props),
            spec: AnimSpec::new(duration),
        }
    }

    /// Restricts the transition to `props` (an explicit list instead of the derived one).
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{Props, Transition};
    /// let t = Transition::all(Duration::ms(150)).props(Props::BG);
    /// assert_eq!(t.props, Some(Props::BG));
    /// ```
    #[must_use]
    pub const fn props(mut self, props: Props) -> Self {
        self.props = Some(props);
        self
    }

    /// Replaces the whole timing.
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{AnimSpec, Transition};
    /// static SHARED: AnimSpec = AnimSpec::new(Duration::ms(200)).ease_out();
    /// let t = Transition::all(Duration::ms(50)).spec(SHARED);
    /// assert_eq!(t.spec, SHARED); // the duration too
    /// ```
    #[must_use]
    pub const fn spec(mut self, spec: AnimSpec) -> Self {
        self.spec = spec;
        self
    }

    /// Sets the duration.
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::Transition;
    /// assert_eq!(Transition::all(Duration::ms(100)).duration(Duration::ms(250)).spec.duration, Duration::ms(250));
    /// ```
    #[must_use]
    pub const fn duration(mut self, d: Duration) -> Self {
        self.spec.duration = d;
        self
    }

    /// Sets the delay before the transition starts.
    #[must_use]
    pub const fn delay(mut self, d: Duration) -> Self {
        self.spec.delay = d;
        self
    }

    /// Sets the easing curve.
    #[must_use]
    pub const fn easing(mut self, e: Easing) -> Self {
        self.spec.easing = e;
        self
    }

    /// Constant speed (the default).
    #[must_use]
    pub const fn linear(self) -> Self {
        self.easing(Easing::Linear)
    }

    /// Slow start.
    #[must_use]
    pub const fn ease_in(self) -> Self {
        self.easing(Easing::EaseIn)
    }

    /// Slow end.
    #[must_use]
    pub const fn ease_out(self) -> Self {
        self.easing(Easing::EaseOut)
    }

    /// Slow start and end.
    #[must_use]
    pub const fn ease_in_out(self) -> Self {
        self.easing(Easing::EaseInOut)
    }

    /// Marks the transition as essential: the [`Motion`](twine_anim::Motion) preference leaves
    /// it unchanged (for a transition that conveys a state, e.g. an alarm color fading in).
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{Props, Transition};
    /// static ALARM_FADE: Transition = Transition::of(Props::BG, Duration::ms(300)).essential();
    /// assert!(ALARM_FADE.spec.essential);
    /// ```
    #[must_use]
    pub const fn essential(mut self) -> Self {
        self.spec.essential = true;
        self
    }

    /// The properties this transition animates for a state change that alters `changed`
    /// (the keys of the styles that start or stop applying): its list, or the interpolable
    /// members of `changed`.
    /// ```
    /// use twine_core::Duration;
    /// use twine_style::{PropId, Props, Transition};
    /// let changed = Props::from_ids(&[PropId::BgColor, PropId::Font]);
    /// // Derived: the interpolable members only (a font switches at once).
    /// let all = Transition::all(Duration::ms(100)).animated(changed);
    /// assert!(all.contains(PropId::BgColor) && !all.contains(PropId::Font));
    /// // Explicit: the list, whatever changed.
    /// assert_eq!(Transition::of(Props::BG, Duration::ms(100)).animated(changed), Props::BG);
    /// ```
    #[inline]
    #[must_use]
    pub const fn animated(&self, changed: Props) -> Props {
        match self.props {
            Some(p) => p,
            None => changed.intersection(Props::INTERPOLABLE),
        }
    }
}

/// The identity of a run-time transition held by a container (opaque: it only identifies, it
/// cannot be dereferenced).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransitionId(usize);

impl TransitionId {
    /// The identity of `t`.
    #[inline]
    pub(crate) fn of(t: &Rc<Transition>) -> Self {
        Self(Rc::as_ptr(t) as usize)
    }
}

/// The payload of the `transition` property in a [`StyleProp`](crate::StyleProp) and a
/// [`StyleValue`]: `Copy`, two words, so `StyleProp` stays 12 bytes.
///
/// - `Static`: a `'static` transition (in flash).
/// - `Local`: the run-time transition that the **container** of the property holds (a
///   [`StyleBuf`](crate::StyleBuf) slot), with its identity. Read it through the container
///   ([`StyleBuf::get_transition`](crate::StyleBuf::get_transition),
///   [`StyleRef::get_transition`](crate::StyleRef::get_transition)); there is no API that dereferences
///   a `TransitionRef` on its own, so no path reads a freed transition. A `StyleBuf` accepts a
///   `Local` property only if it holds that very transition.
///
/// Equality: static transitions by value, local ones by identity.
#[derive(Clone, Copy, Debug)]
pub enum TransitionRef {
    /// A `'static` transition.
    Static(&'static Transition),
    /// The transition the property's container holds.
    Local(TransitionId),
}

impl PartialEq for TransitionRef {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Static(a), Self::Static(b)) => core::ptr::eq(a, b) || a == b,
            (Self::Local(a), Self::Local(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for TransitionRef {}

impl From<&'static Transition> for TransitionRef {
    fn from(t: &'static Transition) -> Self {
        Self::Static(t)
    }
}

impl crate::PropValue for TransitionRef {
    #[inline]
    fn to_value(&self) -> StyleValue {
        StyleValue::Transition(*self)
    }

    #[inline]
    fn from_value(v: StyleValue) -> Option<Self> {
        match v {
            StyleValue::Transition(t) => Some(t),
            _ => None,
        }
    }
}

/// A transition to set: a `'static` one (stored as a reference) or one built at run time
/// (shared by reference counting; the style that sets it holds it). The `transition` builder
/// method and view modifier take `impl Into<TransitionValue>`: a `&'static Transition`, a
/// [`Transition`] (moved into an `Rc` once, at build time) or an `Rc<Transition>`. Equality
/// compares the transitions.
///
/// ```
/// use twine_core::Duration;
/// use twine_style::{PropId, StyleBuf, Transition, TransitionValue};
///
/// static T: Transition = Transition::all(Duration::ms(100));
/// let a = StyleBuf::new().transition(&T);
/// let b = StyleBuf::new().transition(Transition::all(Duration::ms(100)));
/// assert_eq!(a.get_transition(), Some(&T));
/// assert_eq!(b.get_transition(), Some(&T)); // same value, held by `b`
/// assert_eq!(TransitionValue::from(&T), TransitionValue::from(T));
/// ```
#[derive(Clone, Debug)]
pub enum TransitionValue {
    /// A transition in flash.
    Static(&'static Transition),
    /// A transition built at run time.
    Shared(Rc<Transition>),
}

impl TransitionValue {
    /// The transition.
    #[inline]
    #[must_use]
    pub fn get(&self) -> &Transition {
        match self {
            Self::Static(t) => t,
            Self::Shared(t) => t,
        }
    }
}

impl PartialEq for TransitionValue {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Eq for TransitionValue {}

impl From<&'static Transition> for TransitionValue {
    fn from(t: &'static Transition) -> Self {
        Self::Static(t)
    }
}

impl From<Transition> for TransitionValue {
    /// Moves `t` into an `Rc` (one allocation, at build time).
    fn from(t: Transition) -> Self {
        Self::Shared(Rc::new(t))
    }
}

impl From<Rc<Transition>> for TransitionValue {
    fn from(t: Rc<Transition>) -> Self {
        Self::Shared(t)
    }
}

/// Converts `style!` values of the `transition` property in `const` context: a
/// `&'static Transition` (the only transitions a `static` style can hold).
#[doc(hidden)]
pub struct __TransitionArg<T>(pub T);

impl __TransitionArg<&'static Transition> {
    #[doc(hidden)]
    #[must_use]
    pub const fn get(self) -> TransitionRef {
        TransitionRef::Static(self.0)
    }
}

/// Whether values of `prop` change gradually during a transition (integers, lengths, colors,
/// opacities, angles and scales). Other properties (fonts, images, enums, flags, references)
/// switch from the old to the new value at the end.
#[must_use]
pub const fn is_interpolable(prop: PropId) -> bool {
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
/// - `ColorFilter`: the one that is set if the other is not, else switches at `t = 128`;
/// - everything else (lengths of different units, fonts, images, enums, flags, references):
///   `from` until `t = 255`, then `to`.
///
/// ```
/// use twine_core::Color;
/// use twine_style::{Length, PropId, StyleValue, interpolate};
///
/// let mid = interpolate(PropId::BgColor, &StyleValue::Color(Color::BLACK), &StyleValue::Color(Color::WHITE), 128);
/// assert_eq!(mid, StyleValue::Color(Color::hex(0x808080)));
/// assert_eq!(interpolate(PropId::Radius, &StyleValue::Length(Length::Px(0)), &StyleValue::Length(Length::Px(100)), 64), StyleValue::Length(Length::Px(25)));
/// ```
#[must_use]
pub fn interpolate(prop: PropId, from: &StyleValue, to: &StyleValue, t: u8) -> StyleValue {
    use StyleValue as V;
    let switch = || if t == 255 { *to } else { *from };
    if prop == PropId::ColorFilter {
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
            _ => V::Color(Color::mix(b, a, Opa::from_raw(t))),
        },
        (V::Int(a), V::Int(b)) => V::Int(lerp(a, b, t)),
        (V::Opa(a), V::Opa(b)) => V::Opa(Opa::from_raw(
            lerp(i32::from(a.raw()), i32::from(b.raw()), t) as u8
        )),
        (V::Angle(a), V::Angle(b)) => V::Angle(Angle::deci_deg(lerp(a.as_deci_deg(), b.as_deci_deg(), t))),
        (V::Scale(a), V::Scale(b)) => {
            V::Scale(Scale::from_raw_256(
                lerp(i32::from(a.raw_256()), i32::from(b.raw_256()), t) as u16,
            ))
        }
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
            StyleValue::Color(Color::mix(Color::BLUE, Color::RED, Opa::from_raw(128)))
        );
        // Every color property mixes (LVGL mixes only its 8 listed color props).
        assert_eq!(
            interpolate(PropId::LineColor, &a, &b, 128),
            StyleValue::Color(Color::mix(Color::BLUE, Color::RED, Opa::from_raw(128)))
        );
    }

    proptest! {
        #[test]
        fn interpolate_int_monotonic(a in -100_000i32..100_000, b in -100_000i32..100_000) {
            let mut prev = a;
            for t in 0..=255u8 {
                let StyleValue::Length(Length::Px(v)) = interpolate(PropId::Radius, &StyleValue::Length(Length::Px(a)), &StyleValue::Length(Length::Px(b)), t) else {
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
                PropId::BgOpacity,
                &StyleValue::Opa(Opa::from_raw(0)),
                &StyleValue::Opa(Opa::from_raw(255)),
                100
            ),
            StyleValue::Opa(Opa::from_raw(99))
        );
        assert_eq!(
            interpolate(
                PropId::TransformScaleX,
                &StyleValue::Scale(Scale::from_raw_256(256)),
                &StyleValue::Scale(Scale::from_raw_256(512)),
                128
            ),
            StyleValue::Scale(Scale::from_raw_256(384))
        );
        assert_eq!(
            interpolate(
                PropId::TransformRotation,
                &StyleValue::Angle(Angle::deci_deg(0)),
                &StyleValue::Angle(Angle::deci_deg(900)),
                64
            ),
            StyleValue::Angle(Angle::deci_deg(225))
        );
    }

    #[test]
    fn non_interpolable_switches_at_end() {
        static F1: twine_text::Font = crate::test_util::font(1);
        static F2: twine_text::Font = crate::test_util::font(1);
        let (a, b) = (StyleValue::Font(&F1), StyleValue::Font(&F2));
        assert!(!is_interpolable(PropId::Font));
        assert_eq!(interpolate(PropId::Font, &a, &b, 254), a);
        assert_eq!(interpolate(PropId::Font, &a, &b, 255), b);
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
                && is_interpolable(PropId::PartOpacity)
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
        assert_eq!(interpolate(PropId::ColorFilter, &StyleValue::None, &f, 0), f);
        assert_eq!(interpolate(PropId::ColorFilter, &f, &StyleValue::None, 255), f);
        assert_eq!(interpolate(PropId::ColorFilter, &f, &g, 127), f);
        assert_eq!(interpolate(PropId::ColorFilter, &f, &g, 128), g);
    }

    #[test]
    fn transition_const_in_static() {
        static T: Transition = Transition::of(
            Props::from_ids(&[PropId::BgColor, PropId::TransformScaleX]),
            Duration::ms(150),
        )
        .ease_out();
        static D: Transition = T.delay(Duration::ms(20));
        assert_eq!(T.spec.delay, Duration::ZERO);
        assert_eq!(D.spec.delay, Duration::ms(20));
        assert_eq!(D.spec.easing, Easing::EaseOut);
        let p = T.animated(Props::ALL);
        assert!(p.contains(PropId::BgColor) && !p.contains(PropId::Radius));
        assert!(!T.spec.essential && T.essential().spec.essential);
        let t = Transition::all(Duration::ms(5)).props(Props::BG);
        assert_eq!(t.props, Some(Props::BG));
        assert_eq!(
            t.spec(twine_anim::AnimSpec::new(Duration::ms(9))).spec.duration,
            Duration::ms(9)
        );
    }

    #[test]
    fn derived_list_keeps_interpolable_changes_only() {
        let t = Transition::all(Duration::ms(10));
        let changed = Props::from_ids(&[
            PropId::BgColor,
            PropId::Font,
            PropId::GridColumnTracks,
            PropId::Radius,
        ]);
        assert_eq!(
            t.animated(changed),
            Props::from_ids(&[PropId::BgColor, PropId::Radius])
        );
        // An explicit list is taken as is (a listed grid template switches at the end).
        let g = Transition::of(Props::GRID, Duration::ms(10));
        assert!(g.animated(Props::EMPTY).contains(PropId::GridColumnTracks));
    }

    #[test]
    fn transition_refs_compare_by_value_or_identity() {
        const S: TransitionRef = __TransitionArg(&A).get();
        static A: Transition = Transition::all(Duration::ms(1));
        static B: Transition = Transition::all(Duration::ms(1));
        static C: Transition = Transition::all(Duration::ms(2));
        assert_eq!(TransitionRef::Static(&A), TransitionRef::Static(&B));
        assert_ne!(TransitionRef::Static(&A), TransitionRef::Static(&C));
        let x = Rc::new(A);
        let y = Rc::new(A);
        assert_eq!(
            TransitionRef::Local(TransitionId::of(&x)),
            TransitionRef::Local(TransitionId::of(&x))
        );
        assert_ne!(
            TransitionRef::Local(TransitionId::of(&x)),
            TransitionRef::Local(TransitionId::of(&y))
        );
        assert_ne!(
            TransitionRef::Static(&A),
            TransitionRef::Local(TransitionId::of(&x))
        );
        assert_eq!(S, TransitionRef::from(&A));
        assert_eq!(TransitionValue::from(x.clone()).get(), &A);
        // `StyleProp` stays 12 bytes on 32-bit targets (checked at compile time in `prop.rs`).
        assert_eq!(
            core::mem::size_of::<TransitionRef>(),
            2 * core::mem::size_of::<usize>()
        );
    }
}
