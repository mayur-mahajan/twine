//! [`AnimSpec`]: the timing of an animation, shared by every animation kind; [`Repeat`]; the
//! global [`Motion`] preference.

use twine_core::Duration;

use crate::Easing;

/// How often an animation plays.
///
/// ```
/// use twine_anim::Repeat;
///
/// assert_eq!(Repeat::default(), Repeat::ONCE);
/// assert_eq!(Repeat::Times(3).cycles(), Some(3));
/// assert_eq!(Repeat::Times(0).cycles(), Some(1)); // an animation plays at least once
/// assert_eq!(Repeat::Forever.cycles(), None);
/// ```
#[doc(alias = "LV_ANIM_REPEAT_INFINITE")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Repeat {
    /// Plays `n` cycles in total (LVGL `lv_anim_set_repeat_count`). `Times(0)` plays once like
    /// `Times(1)`: a started animation always plays at least one cycle.
    Times(u16),
    /// Plays until it is stopped (LVGL `LV_ANIM_REPEAT_INFINITE`).
    Forever,
}

impl Repeat {
    /// One cycle (the default).
    pub const ONCE: Repeat = Repeat::Times(1);

    /// Number of cycles (`None` = forever); at least 1.
    #[inline]
    #[must_use]
    pub const fn cycles(self) -> Option<u16> {
        match self {
            Repeat::Times(0) => Some(1),
            Repeat::Times(n) => Some(n),
            Repeat::Forever => None,
        }
    }
}

impl Default for Repeat {
    /// [`Repeat::ONCE`].
    fn default() -> Self {
        Repeat::ONCE
    }
}

/// The timing of an animation: duration, easing, delay, repetition, playback and whether it
/// is [essential](Self::essential). One type shared by every animation kind: engine
/// animations ([`Anim::spec`](crate::Anim::spec)), style transitions, the `Anim` style
/// property widgets read, and the view layer's `tween` and `animation`.
///
/// `const`-constructible, so a spec can live in a `static` (in flash). The defaults are
/// LVGL's `lv_anim_init`: 500 ms, linear, once, no delay, no playback, not essential.
///
/// ```
/// use twine_anim::{AnimSpec, Easing, Repeat};
/// use twine_core::Duration;
///
/// static PULSE: AnimSpec = AnimSpec::new(Duration::ms(600))
///     .ease_in_out()
///     .playback(Duration::ms(600))
///     .repeat(Repeat::Forever);
/// assert_eq!(PULSE.easing, Easing::EaseInOut);
/// assert_eq!(PULSE.cycle_len(), Duration::ms(1200));
///
/// // A duration alone is a linear spec played once.
/// let s: AnimSpec = Duration::ms(150).into();
/// assert_eq!(s, AnimSpec::new(Duration::ms(150)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AnimSpec {
    /// Duration of the forward play.
    pub duration: Duration,
    /// Delay before the first cycle.
    pub delay: Duration,
    /// Easing curve (used in both directions).
    pub easing: Easing,
    /// Number of cycles.
    pub repeat: Repeat,
    /// Delay between cycles.
    pub repeat_delay: Duration,
    /// Duration of the backward play after each forward play (`None` = no playback).
    pub playback: Option<Duration>,
    /// Delay between the forward and the backward play.
    pub playback_delay: Duration,
    /// Whether the animation conveys information and must play even when the user asked for
    /// less motion (see [`Motion`]). Default `false`.
    pub essential: bool,
}

impl AnimSpec {
    /// LVGL's defaults: 500 ms, linear, once.
    pub const DEFAULT: AnimSpec = AnimSpec::new(Duration::ms(500));

    /// A linear animation of `duration`, played once, without delay or playback.
    #[must_use]
    pub const fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: Easing::Linear,
            repeat: Repeat::ONCE,
            repeat_delay: Duration::ZERO,
            playback: None,
            playback_delay: Duration::ZERO,
            essential: false,
        }
    }

    /// Sets the forward play duration.
    #[must_use]
    pub const fn duration(mut self, d: Duration) -> Self {
        self.duration = d;
        self
    }

    /// Sets the easing curve.
    #[must_use]
    pub const fn easing(mut self, e: Easing) -> Self {
        self.easing = e;
        self
    }

    /// Constant speed ([`Easing::Linear`], the default).
    #[must_use]
    pub const fn linear(self) -> Self {
        self.easing(Easing::Linear)
    }

    /// Slow start ([`Easing::EaseIn`]).
    #[must_use]
    pub const fn ease_in(self) -> Self {
        self.easing(Easing::EaseIn)
    }

    /// Slow end ([`Easing::EaseOut`]).
    #[must_use]
    pub const fn ease_out(self) -> Self {
        self.easing(Easing::EaseOut)
    }

    /// Slow start and end ([`Easing::EaseInOut`]).
    #[must_use]
    pub const fn ease_in_out(self) -> Self {
        self.easing(Easing::EaseInOut)
    }

    /// Sets the delay before the first cycle.
    #[must_use]
    pub const fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// Sets the number of cycles.
    #[must_use]
    pub const fn repeat(mut self, r: Repeat) -> Self {
        self.repeat = r;
        self
    }

    /// Plays forever (`.repeat(Repeat::Forever)`).
    #[must_use]
    pub const fn forever(self) -> Self {
        self.repeat(Repeat::Forever)
    }

    /// Sets the delay between cycles.
    #[must_use]
    pub const fn repeat_delay(mut self, d: Duration) -> Self {
        self.repeat_delay = d;
        self
    }

    /// Plays back to the start value after each forward play, taking `d`.
    #[must_use]
    pub const fn playback(mut self, d: Duration) -> Self {
        self.playback = Some(d);
        self
    }

    /// Sets the delay between the forward and the backward play.
    #[must_use]
    pub const fn playback_delay(mut self, d: Duration) -> Self {
        self.playback_delay = d;
        self
    }

    /// Marks the animation as essential: it conveys information (a progress or busy
    /// indicator, a scrolling text that is otherwise unreadable, a clock), so [`Motion`]
    /// leaves it unchanged.
    #[must_use]
    pub const fn essential(mut self) -> Self {
        self.essential = true;
        self
    }

    /// Length of one cycle's played part: forward play, plus playback delay and backward play
    /// with [`playback`](Self::playback) (saturating).
    #[must_use]
    pub const fn cycle_len(&self) -> Duration {
        match self.playback {
            Some(pb) => self
                .duration
                .saturating_add(self.playback_delay)
                .saturating_add(pb),
            None => self.duration,
        }
    }
}

impl Default for AnimSpec {
    /// [`AnimSpec::DEFAULT`]: 500 ms, linear, once (LVGL `lv_anim_init`).
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<Duration> for AnimSpec {
    /// A linear animation of that duration, played once ([`AnimSpec::new`]).
    fn from(d: Duration) -> Self {
        Self::new(d)
    }
}

/// How much motion the user wants: a global accessibility preference (the platforms' "reduce
/// motion" setting), set on the engine (`Engine::set_motion` in `twine-engine`, or the `Ui`
/// builder and `use_motion` in `twine-view`). The default is [`Full`](Self::Full).
///
/// It is applied **once per animation start**, never per frame: when an animation is started
/// (or restarted), its [`AnimSpec`] is replaced by [`apply`](Self::apply)`(spec)`. Essential
/// animations ([`AnimSpec::essential`]) are never changed. What counts as non-essential is
/// everything decorative or transitional:
///
/// | Animation | Default |
/// |-----------|---------|
/// | style transitions (also those of themes) | non-essential |
/// | screen load animations | non-essential |
/// | scroll animations (animated `scroll_to`, momentum, snapping, elastic return) | non-essential |
/// | widget value animations (bar, switch knob, roller, dropdown fade, …) | non-essential |
/// | application `tween`s and `animation`s | non-essential unless marked [`essential`](AnimSpec::essential) |
/// | spinner, scrolling label text, animated images, Lottie, the arc's drag clock | essential |
///
/// Effects on a non-essential animation:
///
/// - [`Reduced`](Self::Reduced): the forward and backward plays are capped at
///   [`REDUCED_CAP`](Self::REDUCED_CAP) (100 ms) and it plays one cycle
///   ([`Repeat::ONCE`]: a decorative loop ends). Delays are kept (they are timing, not
///   motion).
/// - [`None`](Self::None): durations and delays become zero and it plays one cycle: it jumps
///   to its final value at the next animation step (its callbacks still run). The engine does
///   not even create style transitions then.
///
/// ```
/// use twine_anim::{AnimSpec, Motion, Repeat};
/// use twine_core::Duration;
///
/// let wave = AnimSpec::new(Duration::ms(400)).delay(Duration::ms(50)).forever();
/// let r = Motion::Reduced.apply(wave);
/// assert_eq!((r.duration, r.delay, r.repeat), (Duration::ms(100), Duration::ms(50), Repeat::ONCE));
/// let n = Motion::None.apply(wave);
/// assert_eq!((n.duration, n.delay, n.repeat), (Duration::ZERO, Duration::ZERO, Repeat::ONCE));
/// // A progress indicator keeps moving.
/// let busy = wave.essential();
/// assert_eq!(Motion::None.apply(busy), busy);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Motion {
    /// Every animation plays as specified.
    #[default]
    Full,
    /// Non-essential animations are shortened to at most [`REDUCED_CAP`](Self::REDUCED_CAP)
    /// per play and do not loop.
    Reduced,
    /// Non-essential animations jump to their end values.
    None,
}

impl Motion {
    /// The longest forward or backward play of a non-essential animation under
    /// [`Motion::Reduced`]: 100 ms (a short fade still shows that something changed).
    pub const REDUCED_CAP: Duration = Duration::ms(100);

    /// The spec an animation started under this preference plays with (see [`Motion`]).
    /// Pure, `const`, allocation-free; never panics.
    #[must_use]
    pub const fn apply(self, spec: AnimSpec) -> AnimSpec {
        if spec.essential {
            return spec;
        }
        match self {
            Motion::Full => spec,
            Motion::Reduced => {
                let mut s = spec;
                s.duration = cap(s.duration);
                if let Some(pb) = s.playback {
                    s.playback = Some(cap(pb));
                }
                s.repeat = Repeat::ONCE;
                s
            }
            Motion::None => {
                let mut s = spec;
                s.duration = Duration::ZERO;
                s.delay = Duration::ZERO;
                if s.playback.is_some() {
                    s.playback = Some(Duration::ZERO);
                }
                s.playback_delay = Duration::ZERO;
                s.repeat = Repeat::ONCE;
                s
            }
        }
    }

    /// Whether [`apply`](Self::apply) can change `spec` (`false` for [`Full`](Self::Full) and
    /// essential specs). A cheap test (two comparisons) for skipping `apply`; it may return
    /// `true` for a spec that `apply` leaves equal (e.g. an already short one under
    /// [`Reduced`](Self::Reduced)). `const`; never panics.
    ///
    /// ```
    /// use twine_anim::{AnimSpec, Motion};
    /// use twine_core::Duration;
    ///
    /// let spec = AnimSpec::new(Duration::ms(400));
    /// assert!(!Motion::Full.affects(&spec));
    /// assert!(Motion::Reduced.affects(&spec));
    /// assert!(!Motion::None.affects(&spec.essential()));
    /// ```
    #[inline]
    #[must_use]
    pub const fn affects(self, spec: &AnimSpec) -> bool {
        !spec.essential && !matches!(self, Motion::Full)
    }
}

/// `d` capped at [`Motion::REDUCED_CAP`].
const fn cap(d: Duration) -> Duration {
    if d.as_micros() > Motion::REDUCED_CAP.as_micros() {
        Motion::REDUCED_CAP
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_defaults_and_builders() {
        const S: AnimSpec = AnimSpec::new(Duration::ms(10))
            .ease_out()
            .delay(Duration::ms(1))
            .repeat_delay(Duration::ms(2))
            .playback(Duration::ms(3))
            .playback_delay(Duration::ms(4))
            .forever()
            .essential();
        let s = AnimSpec::default();
        assert_eq!(s.duration, Duration::ms(500));
        assert_eq!(
            (s.easing, s.repeat, s.essential),
            (Easing::Linear, Repeat::ONCE, false)
        );
        assert_eq!(S.easing, Easing::EaseOut);
        assert_eq!(S.cycle_len(), Duration::ms(17));
        assert!(S.essential && S.repeat == Repeat::Forever);
        assert_eq!(S.linear().ease_in().easing, Easing::EaseIn);
        assert_eq!(
            S.ease_in_out().duration(Duration::ms(5)).duration,
            Duration::ms(5)
        );
    }

    #[test]
    fn repeat_cycles() {
        assert_eq!(Repeat::Times(0).cycles(), Some(1));
        assert_eq!(Repeat::Times(1).cycles(), Some(1));
        assert_eq!(Repeat::Times(7).cycles(), Some(7));
        assert_eq!(Repeat::Forever.cycles(), None);
    }

    #[test]
    fn motion_reduced_caps_and_stops_loops() {
        let s = AnimSpec::new(Duration::ms(80))
            .delay(Duration::ms(70))
            .playback(Duration::secs(2))
            .repeat(Repeat::Times(3));
        let r = Motion::Reduced.apply(s);
        assert_eq!(r.duration, Duration::ms(80), "short plays are kept");
        assert_eq!(r.playback, Some(Motion::REDUCED_CAP));
        assert_eq!((r.delay, r.repeat), (Duration::ms(70), Repeat::ONCE));
        assert_eq!(Motion::Full.apply(s), s);
        assert!(Motion::Reduced.affects(&s) && !Motion::Full.affects(&s));
        assert!(!Motion::None.affects(&s.essential()));
    }

    #[test]
    fn motion_none_jumps() {
        let s = AnimSpec::new(Duration::secs(1))
            .delay(Duration::ms(70))
            .playback(Duration::ms(10))
            .playback_delay(Duration::ms(5))
            .forever();
        let n = Motion::None.apply(s);
        assert_eq!(n.cycle_len(), Duration::ZERO);
        assert_eq!((n.delay, n.repeat), (Duration::ZERO, Repeat::ONCE));
        assert_eq!(n.playback, Some(Duration::ZERO), "still ends at the start value");
        assert_eq!(Motion::None.apply(s.essential()), s.essential());
    }
}
