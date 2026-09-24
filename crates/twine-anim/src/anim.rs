//! [`Anim`]: an animation description and its pure timing model ([`Anim::sample`]).

use alloc::boxed::Box;
use core::fmt;

use twine_core::Duration;

use crate::Easing;
use crate::timeline::AnimCx;

/// How often an animation plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Repeat {
    /// Plays once and then `n` more times (`Count(0)` = play once).
    Count(u16),
    /// Plays forever.
    Infinite,
}

impl Default for Repeat {
    /// Play once.
    fn default() -> Self {
        Repeat::Count(0)
    }
}

/// A raw engine node key (the engine converts its node ids to and from `u32`; this crate sits
/// below the engine and cannot name them).
pub type NodeKey = u32;

/// The node property an animation writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AnimProp {
    /// Local X position.
    X,
    /// Local Y position.
    Y,
    /// Width.
    Width,
    /// Height.
    Height,
    /// Opacity.
    Opa,
    /// Horizontal translation.
    TranslateX,
    /// Vertical translation.
    TranslateY,
    /// Horizontal transform scale.
    ScaleX,
    /// Vertical transform scale.
    ScaleY,
    /// Transform rotation.
    Rotation,
    /// Any integer-valued style property, by its property id (`PropId as u8`).
    StyleProp(u8),
    /// Horizontal scroll position.
    ScrollX,
    /// Vertical scroll position.
    ScrollY,
    /// A widget-defined value (e.g. a bar's value).
    Value,
    /// A widget-defined custom property.
    Custom(u16),
}

/// What an animation writes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AnimTarget {
    /// A property of an engine node. Starting an animation on a node property replaces a running
    /// animation of the same node and property (LVGL: same `var` and `exec_cb`).
    Node(NodeKey, AnimProp),
    /// A user-defined target resolved by the caller (e.g. a view-layer tween); never replaced
    /// automatically.
    Custom(u32),
}

/// The phase an animation is in at some point of its timeline (see [`Anim::sample`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Phase {
    /// The initial delay (the value is only applied with [`Anim::early_apply`]).
    Delay,
    /// Playing from `start` to `end`.
    Forward,
    /// Holding `end` before playing back.
    PlaybackDelay,
    /// Playing back from `end` to `start`.
    Backward,
    /// Holding the cycle's final value before the next repeat.
    RepeatDelay,
}

impl Phase {
    /// Whether the value changes during this phase (forward or backward play).
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Phase::Forward | Phase::Backward)
    }
}

/// The state of an animation at some elapsed time (the result of [`Anim::sample`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Sample {
    /// The animated value. During [`Phase::Delay`] it is `start`; the caller applies it only if
    /// [`Anim::early_apply`] is set.
    pub value: i32,
    /// The current phase (the last phase when finished).
    pub phase: Phase,
    /// Whether the animation has ended (`value` is then the final value).
    pub finished: bool,
    /// Completed cycles (forward + optional playback) before the current one.
    pub repeats_done: u16,
    /// Time until the current phase ends (zero when finished). During the delay phases the
    /// value does not change, so this is when it next changes.
    pub phase_left: Duration,
}

type Callback = Box<dyn FnMut(&mut AnimCx<'_>)>;

/// The event callbacks of an animation (`on_start`, `on_repeat`, `on_complete`), carried by an
/// [`Anim`] and moved into the [`Timeline`](crate::Timeline) when it is added.
#[derive(Default)]
#[allow(clippy::struct_field_names)] // `on_*` mirrors the builder methods
pub struct AnimCallbacks {
    pub(crate) on_start: Option<Callback>,
    pub(crate) on_repeat: Option<Callback>,
    pub(crate) on_complete: Option<Callback>,
}

impl AnimCallbacks {
    /// Whether no callback is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.on_start.is_none() && self.on_repeat.is_none() && self.on_complete.is_none()
    }
}

impl fmt::Debug for AnimCallbacks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnimCallbacks")
            .field("on_start", &self.on_start.is_some())
            .field("on_repeat", &self.on_repeat.is_some())
            .field("on_complete", &self.on_complete.is_some())
            .finish()
    }
}

/// An animation: a value going from `start` to `end` over time, with LVGL's timing options
/// (`lv_anim_t`).
///
/// One cycle is: forward play (`duration`), then, with [`Anim::playback`], `playback_delay` and
/// a backward play; cycles are separated by `repeat_delay`. The first cycle starts after
/// `delay`. Time is wall-clock: [`Anim::sample`] maps any elapsed time directly to a value, so
/// late frames jump to the right value instead of slowing the animation down.
///
/// ```
/// use twine_anim::{Anim, Easing, Phase, Repeat};
/// use twine_core::Duration;
///
/// let a = Anim::new(0, 100)
///     .duration(Duration::ms(200))
///     .delay(Duration::ms(50))
///     .easing(Easing::Linear)
///     .playback(Duration::ms(100))
///     .repeat(Repeat::Count(1));
/// assert_eq!(a.sample(Duration::ms(150)).value, 50);
/// let back = a.sample(Duration::ms(300));
/// assert_eq!((back.phase, back.value), (Phase::Backward, 50));
/// assert!(a.sample(Duration::ms(650)).finished);
/// ```
#[derive(Debug)]
pub struct Anim {
    /// Start value.
    pub start: i32,
    /// End value.
    pub end: i32,
    /// Duration of the forward play (default 500 ms, as LVGL).
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
    /// Apply `start` during the initial delay (default `true`, as LVGL).
    pub early_apply: bool,
    /// What the animation writes to.
    pub target: AnimTarget,
    pub(crate) callbacks: AnimCallbacks,
}

impl Anim {
    /// An animation from `start` to `end`: 500 ms, linear, played once, no delay, early apply,
    /// target `Custom(0)` (LVGL `lv_anim_init` defaults).
    #[must_use]
    pub fn new(start: i32, end: i32) -> Self {
        Anim {
            start,
            end,
            duration: Duration::ms(500),
            delay: Duration::ZERO,
            easing: Easing::Linear,
            repeat: Repeat::Count(0),
            repeat_delay: Duration::ZERO,
            playback: None,
            playback_delay: Duration::ZERO,
            early_apply: true,
            target: AnimTarget::Custom(0),
            callbacks: AnimCallbacks::default(),
        }
    }

    /// Sets the forward play duration.
    #[must_use]
    pub fn duration(mut self, d: Duration) -> Self {
        self.duration = d;
        self
    }

    /// Sets the duration from a speed in units (pixels) per second (LVGL `lv_anim_speed_to_time`):
    /// `|end − start| · 1 s / speed`, at least 1 ms. A zero speed logs a warning and keeps the
    /// current duration.
    ///
    /// ```
    /// use twine_anim::Anim;
    /// use twine_core::Duration;
    /// assert_eq!(Anim::new(0, 300).speed(100).duration, Duration::secs(3));
    /// assert_eq!(Anim::new(0, 0).speed(100).duration, Duration::ms(1));
    /// ```
    #[must_use]
    pub fn speed(mut self, px_per_s: u32) -> Self {
        if px_per_s == 0 {
            twine_core::warn!(target: "twine::anim", "Anim::speed: zero speed ignored");
            return self;
        }
        let dist = u64::from(self.start.abs_diff(self.end));
        let us = dist * 1_000_000 / u64::from(px_per_s);
        self.duration = Duration::us(us.max(1_000));
        self
    }

    /// Sets the initial delay.
    #[must_use]
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// Sets the easing curve.
    #[must_use]
    pub fn easing(mut self, e: Easing) -> Self {
        self.easing = e;
        self
    }

    /// Sets the number of cycles.
    #[must_use]
    pub fn repeat(mut self, r: Repeat) -> Self {
        self.repeat = r;
        self
    }

    /// Sets the delay between cycles.
    #[must_use]
    pub fn repeat_delay(mut self, d: Duration) -> Self {
        self.repeat_delay = d;
        self
    }

    /// Plays back to `start` after each forward play, taking `d`.
    #[must_use]
    pub fn playback(mut self, d: Duration) -> Self {
        self.playback = Some(d);
        self
    }

    /// Sets the delay between the forward and the backward play.
    #[must_use]
    pub fn playback_delay(mut self, d: Duration) -> Self {
        self.playback_delay = d;
        self
    }

    /// Whether `start` is applied during the initial delay.
    #[must_use]
    pub fn early_apply(mut self, on: bool) -> Self {
        self.early_apply = on;
        self
    }

    /// Sets the target.
    #[must_use]
    pub fn target(mut self, t: AnimTarget) -> Self {
        self.target = t;
        self
    }

    /// Called once when the animation leaves its initial delay (before the first value of the
    /// forward play is applied).
    #[must_use]
    pub fn on_start(mut self, f: impl FnMut(&mut AnimCx<'_>) + 'static) -> Self {
        self.callbacks.on_start = Some(Box::new(f));
        self
    }

    /// Called when a new cycle starts ([`AnimCx::repeats_done`] = completed cycles).
    #[must_use]
    pub fn on_repeat(mut self, f: impl FnMut(&mut AnimCx<'_>) + 'static) -> Self {
        self.callbacks.on_repeat = Some(Box::new(f));
        self
    }

    /// Called when the animation ends, after its final value was applied and it was removed
    /// from the timeline (not called when it is removed early).
    #[must_use]
    pub fn on_complete(mut self, f: impl FnMut(&mut AnimCx<'_>) + 'static) -> Self {
        self.callbacks.on_complete = Some(Box::new(f));
        self
    }

    /// Length of one cycle's played part (forward + playback delay + backward), in µs.
    fn play_len(&self) -> u128 {
        let d = u128::from(self.duration.as_micros());
        match self.playback {
            Some(pb) => d + u128::from(self.playback_delay.as_micros()) + u128::from(pb.as_micros()),
            None => d,
        }
    }

    /// The final value after the last cycle.
    fn final_value(&self) -> i32 {
        if self.playback.is_some() {
            self.start
        } else {
            self.end
        }
    }

    /// Total time from the end of the initial delay to the end of the animation, `None` if
    /// infinite. Used for the finished check.
    fn active_len(&self) -> Option<u128> {
        match self.repeat {
            Repeat::Infinite => None,
            Repeat::Count(n) => {
                let cycles = u128::from(n) + 1;
                Some(cycles * self.play_len() + (cycles - 1) * u128::from(self.repeat_delay.as_micros()))
            }
        }
    }

    /// The state of the animation `elapsed` after it was started. Pure and allocation-free.
    ///
    /// Values are `start + ((end − start) · eased) >> 10` in 64 bits (exactly LVGL's path
    /// callbacks, see [`Easing::value`]), so any `i32` range works. During the backward play the
    /// curve runs from `end` to `start` (LVGL swaps the values). A finished animation reports
    /// `end`, or `start` with playback.
    #[must_use]
    pub fn sample(&self, elapsed: Duration) -> Sample {
        let e = elapsed.as_micros();
        let delay = self.delay.as_micros();
        if e < delay {
            return Sample {
                value: self.start,
                phase: Phase::Delay,
                finished: false,
                repeats_done: 0,
                phase_left: Duration::us(delay - e),
            };
        }
        let t = u128::from(e - delay);
        let last_phase = if self.playback.is_some() {
            Phase::Backward
        } else {
            Phase::Forward
        };
        let finished = |repeats_done: u16| Sample {
            value: self.final_value(),
            phase: last_phase,
            finished: true,
            repeats_done,
            phase_left: Duration::ZERO,
        };
        let play = self.play_len();
        let cycle = play + u128::from(self.repeat_delay.as_micros());
        if let (Repeat::Count(n), Some(total)) = (self.repeat, self.active_len()) {
            if t >= total {
                return finished(n);
            }
        }
        if cycle == 0 {
            // Infinite animation without any duration: sits at its final value.
            return Sample {
                value: self.final_value(),
                phase: last_phase,
                finished: false,
                repeats_done: 0,
                phase_left: Duration::ZERO,
            };
        }
        let k = t / cycle;
        let w = (t % cycle) as u64;
        let repeats_done = k.min(u128::from(u16::MAX)) as u16;
        let left = |end: u128| Duration::us((end - u128::from(w)).min(u128::from(u64::MAX)) as u64);

        let d = self.duration.as_micros();
        if w < d {
            let p = Duration::fraction_1024(Duration::us(w), self.duration);
            return Sample {
                value: self.easing.value(p, self.start, self.end),
                phase: Phase::Forward,
                finished: false,
                repeats_done,
                phase_left: left(d.into()),
            };
        }
        if let Some(pb) = self.playback {
            let pd_end = d.saturating_add(self.playback_delay.as_micros());
            if w < pd_end {
                return Sample {
                    value: self.end,
                    phase: Phase::PlaybackDelay,
                    finished: false,
                    repeats_done,
                    phase_left: left(pd_end.into()),
                };
            }
            let back_end = pd_end.saturating_add(pb.as_micros());
            if w < back_end {
                let p = Duration::fraction_1024(Duration::us(w - pd_end), pb);
                return Sample {
                    value: self.easing.value(p, self.end, self.start),
                    phase: Phase::Backward,
                    finished: false,
                    repeats_done,
                    phase_left: left(back_end.into()),
                };
            }
        }
        Sample {
            value: self.final_value(),
            phase: Phase::RepeatDelay,
            finished: false,
            repeats_done,
            phase_left: left(cycle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us(v: u64) -> Duration {
        Duration::us(v)
    }

    #[test]
    fn anim_defaults_match_lvgl() {
        let a = Anim::new(0, 100);
        assert_eq!(a.duration, Duration::ms(500));
        assert!(a.early_apply);
        assert_eq!(a.repeat, Repeat::Count(0));
        assert_eq!(a.easing, Easing::Linear);
        assert!(a.callbacks.is_empty());
        assert!(!Anim::new(0, 1).on_start(|_| {}).callbacks.is_empty());
    }

    #[test]
    fn linear_value_at_quarter_half_end() {
        let a = Anim::new(0, 1000).duration(Duration::ms(100));
        let cases = [
            (0, 0, false),
            (25_000, 250, false),
            (50_000, 500, false),
            (99_999, 999, false),
            (100_000, 1000, true),
            (5_000_000, 1000, true),
        ];
        for (t, v, fin) in cases {
            let s = a.sample(us(t));
            assert_eq!((s.value, s.finished), (v, fin), "t = {t}");
        }
        assert_eq!(a.sample(us(25_000)).phase, Phase::Forward);
        assert_eq!(a.sample(us(25_000)).phase_left, us(75_000));
    }

    #[test]
    fn delay_holds_start() {
        let a = Anim::new(10, 20)
            .duration(Duration::ms(10))
            .delay(Duration::ms(30));
        for t in [0, 1, 15_000, 29_999] {
            let s = a.sample(us(t));
            assert_eq!(
                (s.value, s.phase, s.finished),
                (10, Phase::Delay, false),
                "t = {t}"
            );
        }
        assert_eq!(a.sample(us(0)).phase_left, Duration::ms(30));
        assert_eq!(a.sample(us(30_000)).phase, Phase::Forward);
        assert_eq!(a.sample(us(35_000)).value, 15);
        assert!(a.sample(us(40_000)).finished);
    }

    #[test]
    fn early_apply_flag_reported() {
        let a = Anim::new(5, 6).delay(Duration::ms(10)).early_apply(false);
        assert!(!a.early_apply);
        assert_eq!(a.sample(us(0)).phase, Phase::Delay);
        assert_eq!(a.sample(us(0)).value, 5);
        assert!(a.early_apply(true).early_apply);
    }

    #[test]
    fn playback_returns_to_start() {
        let a = Anim::new(0, 100)
            .duration(Duration::ms(100))
            .playback(Duration::ms(200));
        let cases = [
            (50_000, 50, Phase::Forward),
            (100_000, 100, Phase::Backward),
            (200_000, 50, Phase::Backward),
            (250_000, 25, Phase::Backward),
            (299_999, 0, Phase::Backward),
        ];
        for (t, v, ph) in cases {
            let s = a.sample(us(t));
            assert_eq!((s.value, s.phase, s.finished), (v, ph, false), "t = {t}");
        }
        let end = a.sample(us(300_000));
        assert_eq!((end.value, end.finished), (0, true));
    }

    #[test]
    fn playback_delay_holds_end() {
        let a = Anim::new(0, 100)
            .duration(Duration::ms(100))
            .playback(Duration::ms(100))
            .playback_delay(Duration::ms(50));
        for t in [100_000, 120_000, 149_999] {
            let s = a.sample(us(t));
            assert_eq!((s.value, s.phase), (100, Phase::PlaybackDelay), "t = {t}");
        }
        assert_eq!(a.sample(us(120_000)).phase_left, us(30_000));
        assert_eq!(a.sample(us(200_000)).value, 50);
        assert!(a.sample(us(250_000)).finished);
    }

    #[test]
    fn repeat_count_three_cycles() {
        let a = Anim::new(0, 100)
            .duration(Duration::ms(100))
            .repeat(Repeat::Count(2));
        let cases = [
            (50_000, 50, 0),
            (150_000, 50, 1),
            (250_000, 50, 2),
            (299_000, 98, 2),
        ];
        for (t, v, r) in cases {
            let s = a.sample(us(t));
            assert_eq!((s.value, s.repeats_done, s.finished), (v, r, false), "t = {t}");
        }
        let s = a.sample(us(300_000));
        assert_eq!((s.value, s.repeats_done, s.finished), (100, 2, true));
    }

    #[test]
    fn repeat_delay_between_cycles() {
        let a = Anim::new(0, 100)
            .duration(Duration::ms(100))
            .repeat(Repeat::Count(1))
            .repeat_delay(Duration::ms(40));
        let s = a.sample(us(120_000));
        assert_eq!(
            (s.value, s.phase, s.repeats_done, s.phase_left),
            (100, Phase::RepeatDelay, 0, us(20_000))
        );
        let s = a.sample(us(140_000));
        assert_eq!((s.value, s.phase, s.repeats_done), (0, Phase::Forward, 1));
        assert_eq!(a.sample(us(190_000)).value, 50);
        // No repeat delay after the last cycle.
        assert!(a.sample(us(240_000)).finished);
        assert!(!a.sample(us(239_999)).finished);

        let pb = Anim::new(0, 100)
            .duration(Duration::ms(100))
            .playback(Duration::ms(100))
            .repeat(Repeat::Count(1))
            .repeat_delay(Duration::ms(40));
        let s = pb.sample(us(220_000));
        assert_eq!((s.value, s.phase), (0, Phase::RepeatDelay));
        assert_eq!(pb.sample(us(290_000)).value, 50);
        assert!(pb.sample(us(440_000)).finished);
    }

    #[test]
    fn infinite_never_finishes() {
        let a = Anim::new(0, 100)
            .duration(Duration::ms(10))
            .playback(Duration::ms(10))
            .repeat(Repeat::Infinite);
        for t in [0, 10_000, 1_000_000, 3_600_000_000, u64::MAX] {
            assert!(!a.sample(us(t)).finished, "t = {t}");
        }
        assert_eq!(a.sample(us(1_005_000)).value, 50);
        assert_eq!(a.sample(us(1_005_000)).repeats_done, 50);
        assert_eq!(a.sample(us(u64::MAX)).repeats_done, u16::MAX);
        // Degenerate: no duration at all.
        let z = Anim::new(1, 2).duration(Duration::ZERO).repeat(Repeat::Infinite);
        assert_eq!((z.sample(us(5)).value, z.sample(us(5)).finished), (2, false));
    }

    #[test]
    fn zero_duration_finishes_immediately() {
        let a = Anim::new(1, 2).duration(Duration::ZERO);
        let s = a.sample(Duration::ZERO);
        assert_eq!((s.value, s.finished), (2, true));
    }

    #[test]
    fn speed_computes_duration() {
        assert_eq!(Anim::new(0, 300).speed(100).duration, Duration::secs(3));
        assert_eq!(Anim::new(100, -100).speed(400).duration, Duration::ms(500));
        assert_eq!(Anim::new(0, 1).speed(10_000).duration, Duration::ms(1)); // min 1 ms
        assert_eq!(Anim::new(0, 7).speed(3).duration, Duration::us(2_333_333));
        assert_eq!(
            Anim::new(0, 7).duration(Duration::ms(9)).speed(0).duration,
            Duration::ms(9)
        );
        assert_eq!(
            Anim::new(i32::MIN, i32::MAX).speed(1).duration.as_micros(),
            u64::from(u32::MAX) * 1_000_000
        );
    }

    #[test]
    fn large_values_no_overflow() {
        let a = Anim::new(-1_000_000_000, 1_000_000_000).duration(Duration::ms(100));
        assert_eq!(a.sample(us(0)).value, -1_000_000_000);
        assert_eq!(a.sample(us(50_000)).value, 0);
        assert_eq!(a.sample(us(100_000)).value, 1_000_000_000);
        let o = Anim::new(i32::MIN, i32::MAX)
            .easing(Easing::Overshoot)
            .duration(Duration::ms(100));
        assert_eq!(o.sample(us(90_000)).value, i32::MAX); // saturates
        let long = Anim::new(0, 10)
            .duration(Duration::MAX)
            .playback(Duration::MAX)
            .repeat(Repeat::Count(u16::MAX));
        assert!(!long.sample(us(u64::MAX)).finished);
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn monotonic_easing() -> impl Strategy<Value = Easing> {
            prop_oneof![
                Just(Easing::Linear),
                Just(Easing::EaseIn),
                Just(Easing::EaseOut),
                Just(Easing::EaseInOut),
                Just(Easing::Step),
                (0i16..=1024, 0i16..=1024, 0i16..=1024, 0i16..=1024)
                    .prop_map(|(a, b, c, d)| Easing::CubicBezier(a, b, c, d)),
            ]
        }

        proptest! {
            #[test]
            fn sample_value_within_bounds_for_monotonic_easings(
                start in any::<i32>(),
                end in any::<i32>(),
                easing in monotonic_easing(),
                dur in 0u64..10_000_000,
                delay in 0u64..1_000_000,
                pb in proptest::option::of(0u64..10_000_000),
                pbd in 0u64..1_000_000,
                rep in 0u16..5,
                rep_delay in 0u64..1_000_000,
                t in any::<u64>(),
            ) {
                let mut a = Anim::new(start, end)
                    .easing(easing)
                    .duration(Duration::us(dur))
                    .delay(Duration::us(delay))
                    .playback_delay(Duration::us(pbd))
                    .repeat(Repeat::Count(rep))
                    .repeat_delay(Duration::us(rep_delay));
                if let Some(pb) = pb {
                    a = a.playback(Duration::us(pb));
                }
                let s = a.sample(Duration::us(t % 100_000_000));
                prop_assert!(s.value >= start.min(end) && s.value <= start.max(end), "{:?}", s);
                prop_assert!(s.repeats_done <= rep);
                if s.finished {
                    prop_assert_eq!(s.phase_left, Duration::ZERO);
                }
            }
        }
    }
}
