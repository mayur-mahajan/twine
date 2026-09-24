//! # twine-lottie
//!
//! Plays Lottie (Bodymovin JSON) animations for the Twine GUI library: [`load`] parses a file
//! into a typed [`Composition`], and [`LottiePlayer`] renders any frame of
//! it into a `twine-render` [`Painter`](twine_render::Painter) through `twine-vector`
//! (anti-aliased fills and strokes, gradients, layers).
//!
//! ## Supported subset
//!
//! - Layers: shape, solid, null (as parents), precomposition (with time remapping `tm`, time
//!   stretch, start offset and a size clip); layer opacity, parenting, hidden layers, in/out
//!   points.
//! - Shapes: group (with transform and opacity), rectangle (rounded), ellipse, path, fill (fill
//!   rule), stroke (width, caps, joins, miter limit, dashes), gradient fill and stroke (linear;
//!   radial with highlight), trim paths (simultaneous and individual), repeater (copies,
//!   offset, transform, start/end opacity, stacking order).
//! - Animated properties with linear, hold and cubic-bezier keyframes (per-axis easing),
//!   spatial tangents for positions, old files with end values `e`.
//! - Masks (add, subtract, intersect, none; opacity; inverted) and alpha track mattes (normal
//!   and inverted).
//!
//! Not supported: expressions, text, image and audio layers, effects, blend modes, luma mattes,
//! mask expansion, 3-D layers, auto-orient, merge paths, rounded corners, offset paths,
//! polystars and other shape modifiers. They are ignored and reported once per file with
//! `warn!(target: "twine::lottie", "lottie: unsupported {}", what)` and listed in
//! [`Composition::unsupported`](model::Composition::unsupported).
//!
//! ## Floating point
//!
//! Lottie data is float-based, so this crate uses `f32` (it is the documented exception to
//! Twine's fixed-point rule): an FPU is needed for good performance. Geometry is converted to
//! 16.16 fixed point before it reaches `twine-vector`, so rasterization stays integer and
//! deterministic; MCUs without an FPU (RP2040, ESP32-C3) run it with soft-float, only slower.
//! The small math functions needed (`sin`, `cos`, `atan2`, `sqrt`, `exp`, `ln`) are implemented
//! in the crate, so no `libm` is required.
//!
//! ## Memory
//!
//! The composition model lives on the heap (roughly the size of the JSON). Rendering reuses
//! scratch buffers owned by the player: the contours of the largest shape layer, and per
//! offscreen nesting level an `Argb8888` strip of at most
//! [`RenderConfig::layer_buf_bytes`](twine_render::RenderConfig) bytes plus coverage strips
//! (¼ of that per mask/matte buffer). Nothing is allocated per frame once they have grown.
//!
//! ## Example
//!
//! ```
//! use twine_core::{ColorFormat, Rect};
//! use twine_render::{DrawBuf, Painter, RenderCaches};
//! use twine_lottie::{LottiePlayer, load};
//!
//! let json = br#"{"fr":30,"ip":0,"op":60,"w":64,"h":64,"layers":[
//!   {"ty":4,"ks":{"r":{"a":1,"k":[{"t":0,"s":[0]},{"t":60,"s":[360]}]},
//!                 "p":{"k":[32,32]}},
//!    "shapes":[
//!     {"ty":"rc","p":{"k":[0,0]},"s":{"k":[30,30]},"r":{"k":4}},
//!     {"ty":"fl","c":{"k":[0.2,0.5,1,1]},"o":{"k":100}}]}]}"#;
//! let mut player = LottiePlayer::new(load(json)?);
//! let mut caches = RenderCaches::default();
//! let mut px = vec![0u8; 64 * 64 * 2];
//! let area = Rect::from_xywh(0, 0, 64, 64);
//! let mut p = Painter::new(DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, area)?, &mut caches);
//! player.render_frame(15.0, &mut p, area); // the square rotated by 90°
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![no_std]
#![forbid(unsafe_code)]
// A float crate by design: frame numbers, counts and indices become `f32`, NaN-aware negated
// comparisons (`!(x > 0.0)`) are intentional, and exact float comparisons test for "unset"/
// "identical" values rather than computed results.
#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::neg_cmp_op_on_partial_ord
)]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod error;
pub mod eval;
mod fmath;
mod geom;
mod load;
pub mod model;
mod modifier;
mod render;

pub use error::LottieError;
pub use load::load;
pub use model::Composition;
pub use render::{LottiePlayer, MAX_LAYER_DRAWS, MAX_OFFSCREEN_DEPTH, MAX_PRECOMP_DEPTH};

// NOTE(P24.S06): the `Lottie` widget (own render buffer, frame-change-only redraw, `Anim`
// playback) builds on `LottiePlayer` once the engine exists.
