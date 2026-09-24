//! # twine-anim
//!
//! Animation building blocks of the Twine GUI library, below the widget engine in the layering
//! (it depends only on `twine-core`, so the style system can reference easing curves and
//! animation templates).
//!
//! - [`Easing`]: animation curves, exact integer ports of LVGL's `lv_anim_path_*` functions
//!   (no floating point; identical results on every target).
//! - [`Interpolate`]: linear interpolation of animatable values (numbers, colors, points, …).
//! - [`Anim`]: an animation (values, duration, delay, easing, repeat, playback) with a pure
//!   timing model, [`Anim::sample`].
//! - [`Timeline`]: runs many animations, applies their values through a [`TickSink`] and
//!   returns the next deadline.
//! - [`Timers`]: periodic callbacks (LVGL `lv_timer`) that also return their next deadline.
//! - [`AnimTemplate`]: `const` animation parameters (duration, easing, repeat) used by the
//!   `Anim` style property.
//!
//! Progress values use 1024 = 1.0 ([`EASING_ONE`]); time is [`Instant`](twine_core::Instant) /
//! [`Duration`](twine_core::Duration) in microseconds and always wall-clock: a late frame
//! jumps to the right value instead of slowing the animation down. Nothing here needs polling:
//! [`Timeline::tick`] and [`Timers::run`] tell the caller when to wake up next, and return
//! `None` when nothing is pending so the CPU can sleep.
//!
//! ```
//! use twine_anim::{Anim, Easing};
//! use twine_core::Duration;
//!
//! // Value of a 0 → 200 animation, halfway through, with ease-in-out.
//! assert_eq!(Easing::EaseInOut.value(512, 0, 200), 100);
//! let a = Anim::new(0, 200).duration(Duration::ms(100)).easing(Easing::EaseInOut);
//! assert_eq!(a.sample(Duration::ms(50)).value, 100);
//! ```
#![no_std]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

mod anim;
mod easing;
mod interp;
mod template;
mod timeline;
mod timer;

pub use anim::{Anim, AnimCallbacks, AnimProp, AnimTarget, NodeKey, Phase, Repeat, Sample};
pub use easing::{EASING_ONE, Easing};
pub use interp::Interpolate;
pub use template::AnimTemplate;
pub use timeline::{AnimCx, AnimId, Exec, ExecFn, TickSink, Timeline};
pub use timer::{TimerCx, TimerId, Timers};
