//! # twine-core
//!
//! Foundations of the Twine GUI library: integer geometry, fixed-point math and trigonometry,
//! transforms, colors and pixel formats, opacity, time, generational arenas, a small inline
//! vector, the dirty-rectangle set, errors, a deterministic RNG and the logging macros.
//!
//! `twine-core` is the lowest layer: every other Twine crate
//! builds on it and it depends on no other Twine crate. It is `no_std` + `alloc`, contains no
//! `unsafe` and no floating-point arithmetic, so results are bit-identical on every target (P5,
//! P6).
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`geometry`] | [`Point`], [`Size`], [`Rect`] (half-open), [`Insets`], [`Rotation`] |
//! | [`math`] | [`Fx`] (16.16), [`Angle`] (0.1°), [`Scale`] (256 = 1.0), sqrt, sin/cos/atan2, bezier, `udiv255`, `map` |
//! | [`transform`] | [`Transform`] (2-D affine, fixed point) |
//! | [`color`] | [`Color`], [`Opa`], [`ColorFormat`], [`PixelFormat`] and format marker types |
//! | [`time`] | [`Instant`], [`Duration`] (µs) |
//! | [`arena`] | [`Arena<T>`], generational [`Id<T>`] |
//! | [`small_vec`] | [`SmallVec`] (inline-first vector) |
//! | [`rect_set`] | [`RectSet`] (dirty areas, LVGL join policy) |
//! | [`mod@error`] | [`Error`] |
//! | [`rng`] | [`XorShift32`] (deterministic, non-cryptographic) |
//! | [`log`] | `trace!`, `debug!`, `info!`, `warn!`, `error!` (log / defmt / none backends) |
//! | [`prelude`] | the most used types |
//!
//! ## Units and conventions
//!
//! - Pixels are `i32`; [`Rect`] is **half-open** `[x0, x1) × [y0, y1)` (LVGL areas are
//!   inclusive — ported algorithms convert explicitly).
//! - Opacity is [`Opa`] (`u8`, 255 = cover); angles are [`Angle`] in 0.1°, clockwise on screen;
//!   scale factors are [`Scale`] (`u16`, 256 = 1.0); time is [`Instant`]/[`Duration`] in µs
//!   (`u64`).
//! - Arithmetic saturates rather than overflowing; invalid arguments log `warn!` and fall back
//!   to a documented value instead of panicking (P7).
//!
//! ## Features
//!
//! - `log` / `defmt`: logging backend (see [`log`]); `defmt` also derives `defmt::Format` for
//!   every public type.
//! - `std`: std-only conveniences.
//!
//! ```
//! use twine_core::prelude::*;
//!
//! let area = Rect::from_xywh(10, 10, 100, 50);
//! let c = Color::mix(Color::RED, Color::BLUE, Opa::P50);
//! let t = Transform::rotate(Angle::deg(90));
//! assert_eq!(t.map_point(Point::new(1, 0)), Point::new(0, 1));
//! assert_eq!(area.size(), Size::new(100, 50));
//! assert_eq!(c, Color::new(127, 0, 128));
//! ```
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod arena;
pub mod color;
pub mod error;
pub mod geometry;
pub mod log;
pub mod math;
pub mod prelude;
pub mod rect_set;
pub mod rng;
pub mod small_vec;
pub mod time;
pub mod transform;

pub use arena::{Arena, Id};
pub use color::{Color, ColorFormat, Opa, PixelFormat};
pub use error::Error;
pub use geometry::*;
pub use math::{Angle, Fx, Scale};
pub use rect_set::{AddResult, RectSet};
pub use rng::XorShift32;
pub use small_vec::SmallVec;
pub use time::{Duration, Instant};
pub use transform::Transform;

/// Re-exports used by the logging macros. Not public API.
#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "defmt")]
    pub use defmt;
    #[cfg(feature = "log")]
    pub use log;
}
