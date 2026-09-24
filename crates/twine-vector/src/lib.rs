//! # twine-vector
//!
//! Vector graphics for the Twine GUI library (the equivalent of LVGL's `lv_vector` API and SVG
//! support): paths, curve flattening, an integer anti-aliasing rasterizer with non-zero and
//! even-odd fill rules, strokes (joins, caps, dashes), paints (solid, linear and radial
//! gradients, image patterns), transforms, and — with the `svg` feature — an SVG Tiny subset
//! parser.
//!
//! Everything is fixed-point (coordinates are 16.16 [`Fx`](twine_core::Fx), the rasterizer
//! works in 24.8), so results are bit-identical on every target. Drawing goes through
//! `twine-render`'s [`Painter`](twine_render::Painter): the rasterizer produces anti-aliased
//! coverage rows for the rows inside the painter's clip only and hands them to
//! [`Painter::coverage_span`](twine_render::Painter::coverage_span), which applies masks and
//! blends. Rendering a path in horizontal chunks (partial display buffers) is pixel-identical
//! to rendering it at once.
//!
//! ## Pipeline
//!
//! ```text
//!  Path ──(transform, flatten 0.25 px)──► lines ──► rasterizer (clip rows) ──► coverage rows
//!   └──(flatten in path space)──► polylines ──► stroker (joins, caps, dashes) ──┘      │
//!                                                  Paint ──► SpanSource ──► Painter::coverage_span
//! ```
//!
//! ## Memory
//!
//! Scratch buffers live in [`VectorCaches`]; they grow to the largest path drawn and are then
//! reused, so steady-state drawing does not allocate. [`PainterVectorExt::vector`] keeps them
//! in the painter's `RenderCaches` extension slot; [`PainterVectorExt::vector_with`] takes
//! them explicitly.
//!
//! ## Example
//!
//! ```
//! use twine_core::{Color, ColorFormat, Fx, Rect};
//! use twine_render::{DrawBuf, FillRule, Painter, RenderCaches};
//! use twine_vector::{FxPoint, Paint, PainterVectorExt, Path, Stroke, VectorDsc};
//!
//! let mut caches = RenderCaches::default();
//! let mut px = vec![0u8; 64 * 64 * 2];
//! let buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 64, 64)).unwrap();
//! let mut p = Painter::new(buf, &mut caches);
//!
//! let mut star = Path::new();
//! star.move_to(FxPoint::from_int(32, 4))
//!     .line_to(FxPoint::from_int(50, 60))
//!     .line_to(FxPoint::from_int(4, 24))
//!     .line_to(FxPoint::from_int(60, 24))
//!     .line_to(FxPoint::from_int(14, 60))
//!     .close();
//! p.vector(
//!     &star,
//!     &VectorDsc {
//!         fill: Some((Paint::Solid(Color::RED), FillRule::EvenOdd)),
//!         stroke: Some((Paint::Solid(Color::BLACK), Stroke { width: Fx::from_int(2), ..Stroke::default() })),
//!         ..VectorDsc::default()
//!     },
//! );
//! ```
//!
//! ## Features
//!
//! - `svg`: `parse_svg` and `SvgDocument` in the `svg` module (SVG Tiny subset).
//! - `log` / `defmt`: logging backend (target `"twine::vector"`); `std`: std conveniences.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

mod draw;
mod flatten;
mod geom;
mod paint;
mod path;
mod raster;
mod scene;
mod stroke;
#[cfg(feature = "svg")]
pub mod svg;

pub use draw::{PainterVectorExt, VectorCaches, VectorDsc};
pub use flatten::{
    DEFAULT_TOLERANCE, Line, MAX_CURVE_SEGMENTS, Polyline, Polylines, flatten, flatten_for_stroke,
};
pub use geom::{FxPoint, FxRect, FxSize};
pub use paint::{Paint, Stops};
pub use path::{KAPPA, Path, PathEl, PathIter, Verb};
pub use scene::VectorScene;
pub use stroke::{Dash, LineCap, LineJoin, MAX_DASHES, MAX_DASHES_PER_POLYLINE, Stroke};
#[cfg(feature = "svg")]
pub use svg::{SvgDocument, SvgError, parse_svg};

/// Re-exports of the `twine-render` types used in this crate's API.
pub use twine_render::{BlendMode, FillRule, GradExtend, GradStop};
