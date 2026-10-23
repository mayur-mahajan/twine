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
//! | [`math`] | [`Fx`] (16.16), [`Angle`] (0.1°), [`Scale`] (256 = 1.0), [`Fraction`] (255 = 1.0), [`AngularSpeed`] (°/s), sqrt, sin/cos/atan2, bezier, `udiv255`, `map`, shared 128-bit division ([`math::div_rem_i128`]) |
//! | [`transform`] | [`Transform`] (2-D affine, fixed point) |
//! | [`color`] | [`Color`], [`Opa`], [`ColorFormat`], [`PixelFormat`] and format marker types, the WCAG [`ContrastRatio`](color::ContrastRatio) |
//! | [`time`] | [`Instant`], [`Duration`] (µs) |
//! | [`arena`] | [`Arena<T>`], generational [`Id<T>`], its [`Generation`] (16-bit; 48-bit in the safety profile, [`arena::MAX_GENERATION`], [`arena::next_generation`]) and packed form [`RawId`] |
//! | [`small_vec`] | [`SmallVec`] (inline-first vector) |
//! | [`symbol`] | [`Symbol`] (built-in icon glyphs, LVGL `LV_SYMBOL_*`) |
//! | [`rect_set`] | [`RectSet`] (dirty areas, LVGL join policy) |
//! | [`mod@error`] | [`Error`] |
//! | [`fault`] | [`FaultKind`], [`Faults`], [`FaultCounts`], [`CapacityFault`](fault::CapacityFault): failures Twine recovered from, reported to the application |
//! | [`limits`] | [`Limits`]: the memory bounds of an application (enforced as `Capacity` faults, optionally preallocated) |
//! | [`partition`] | [`Partition`]: the identity of a partitioned runtime (one reactive runtime and its UIs; feature `partitions`) |
//! | [`alloc_guard`] | sealing per partition: allocation after start-up detected at Twine's growth points ([`AllocSite`](alloc_guard::AllocSite)) |
//! | [`audit`] | [`Audit`](audit::Audit), [`AuditSink`](audit::AuditSink), [`KeyValues`](audit::KeyValues): machine-readable configuration evidence |
//! | [`crc`] | [`Crc32`]: the CRC-32 (IEEE 802.3) checksum of display-integrity checks and digests |
//! | [`validity`] | [`Validity`], [`ValidityStatus`]: a value and how far it can be trusted (valid, stale, failed, no data) |
//! | [`rng`] | [`XorShift32`] (deterministic, non-cryptographic) |
//! | [`log`] | `trace!`, `debug!`, `info!`, `warn!`, `error!` (log / defmt / none backends) |
//! | [`prelude`] | the most used types |
//!
//! ## Units and conventions
//!
//! - Pixels are `i32`; [`Rect`] is **half-open** `[x0, x1) × [y0, y1)` (LVGL areas are
//!   inclusive — ported algorithms convert explicitly).
//! - Opacity is [`Opa`] (`u8`, 255 = cover); angles are [`Angle`] in 0.1°, clockwise on screen;
//!   scale factors are [`Scale`] (`u16`, 256 = 1.0); fractions of a whole (gradient stops,
//!   brightness) are [`Fraction`] (`u8`, 255 = 1.0); angular speeds are [`AngularSpeed`]
//!   (`u16`, whole degrees per second); time is [`Instant`]/[`Duration`] in µs (`u64`).
//! - Arithmetic saturates rather than overflowing; invalid arguments log `warn!` and fall back
//!   to a documented value instead of panicking (P7).
//!
//! ## Features
//!
//! - `log` / `defmt`: logging backend (see [`log`]); `defmt` also derives `defmt::Format` for
//!   every public type.
//! - `std`: std-only conveniences.
//! - `safety`: the safety profile (enabled by the `twine` facade's `safety` feature; see
//!   [`SAFETY_PROFILE`] and `docs/safety/profile.md`). Among other things it widens arena
//!   generations to 48 bits ([`Generation`]), so no arena slot retires in practice.
//! - `partitions`: [`Partition::COUNT`] = 2 partitioned runtimes instead of 1 (ADR-001,
//!   `docs/design/adr-001-partitioned-runtimes.md`); per-partition seal domains in
//!   [`alloc_guard`]. Not implied by `safety`. Without it [`Partition`] is zero-sized.
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
// The safety profile's lints (`docs/safety/profile.md` § Lints): denied in library code under
// the `safety` feature (`cargo xtask ci` stage `clippy-safety`).
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
    )
)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
// Unit tests use `std` for the per-thread allocation-guard state (tests run in parallel).
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod alloc_guard;
pub mod arena;
pub mod audit;
pub mod color;
pub mod crc;
pub mod error;
pub mod fault;
pub mod geometry;
pub mod limits;
pub mod log;
pub mod math;
pub mod partition;
pub mod prelude;
pub mod rect_set;
pub mod rng;
pub mod small_vec;
pub mod symbol;
pub mod time;
pub mod transform;
pub mod validity;

pub use arena::{Arena, Generation, Id, RawId};
pub use color::{Color, ColorFormat, Opa, PixelFormat};
pub use crc::Crc32;
pub use error::Error;
pub use fault::{FaultCounts, FaultKind, Faults};
pub use geometry::*;
pub use limits::Limits;
pub use math::{Angle, AngularSpeed, Fraction, Fx, Scale};
pub use partition::Partition;
pub use rect_set::{AddResult, RectSet};
pub use rng::XorShift32;
pub use small_vec::SmallVec;
pub use symbol::Symbol;
pub use time::{Duration, Instant};
pub use transform::Transform;
pub use validity::{Validity, ValidityStatus};

/// Whether this build is the **safety profile** (the `twine` facade's `safety` feature, which
/// enables `twine-core/safety`).
///
/// Cargo unifies features, so every crate of one build sees the same value. Crates the profile
/// excludes as a whole (`twine-fs`, `twine-lottie`) assert on it in a constant, so linking one
/// of them into a safety build is a compile error naming the crate, even when the application
/// depends on it directly instead of through the facade. Application code can check it the same
/// way, e.g. a certified firmware refusing to build without the profile. See
/// `docs/safety/profile.md` for what the profile contains.
///
/// ```
/// // In a certified firmware: refuse to build without the profile.
/// // const _: () = assert!(twine_core::SAFETY_PROFILE, "build with `twine/safety`");
/// let profile = if twine_core::SAFETY_PROFILE { "safety" } else { "standard" };
/// assert!(!profile.is_empty());
/// ```
pub const SAFETY_PROFILE: bool = cfg!(feature = "safety");

/// Re-exports used by the logging macros. Not public API.
#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "defmt")]
    pub use defmt;
    #[cfg(feature = "log")]
    pub use log;
}
