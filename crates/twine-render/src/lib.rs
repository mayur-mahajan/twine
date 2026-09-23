//! # twine-render
//!
//! The integer software renderer of the Twine GUI library: pixel operations only, no knowledge
//! of widgets. The engine decides *what* to draw into which buffer; this crate draws it.
//!
//! Everything is integer or fixed-point arithmetic (no floating point), so rendering is
//! bit-identical on every target, and drawing never allocates.
//!
//! ## Pipeline
//!
//! ```text
//!  Painter::rect / line / arc / polygon / blit / layer …
//!        │  clip to Painter::clip()
//!        │  per row: the primitive's own coverage (anti-aliasing) in a scratch span
//!        ▼
//!  masked span ──► mask stack (radius, angle, line, fade, bitmap) multiplies the coverage
//!        ▼
//!  blend_span::<F>  (F = the buffer's pixel format, chosen once per span)
//!        fast paths: raw fill · per-pixel mix · same-format copy · generic
//! ```
//!
//! ## Coordinates
//!
//! All coordinates are **absolute screen coordinates**; a [`DrawBuf`] knows which screen
//! rectangle it holds (a full frame or a partial chunk), so the same drawing code renders any
//! chunk. Rectangles are half-open (`[x0, x1) × [y0, y1)`). Rendering in chunks is
//! pixel-identical to rendering the whole screen at once.
//!
//! ## Memory
//!
//! [`RenderCaches`] is created once with fixed budgets ([`RenderConfig`]): coverage/mask/row
//! scratch spans, the quarter-circle, shadow-corner and gradient caches (LRU, they evict instead
//! of growing) and the layer buffer. **No drawing call allocates.** Masks borrow their data for
//! the painter's lifetime `'a`.
//!
//! ## Primitives
//!
//! | Primitive | Call | Descriptor | LVGL equivalent |
//! |-----------|------|------------|-----------------|
//! | fill | [`Painter::fill`], [`Painter::fill_mode`] | — | `lv_draw_sw_fill` (plain) |
//! | rectangle (radius, gradient, border, outline, shadow) | [`Painter::rect`], [`Painter::rect_border_post`] | [`RectDsc`], [`ShadowDsc`], [`BorderSide`] | `lv_draw_rect` / `lv_draw_sw_fill`, `_border`, `_box_shadow` |
//! | gradient fill | [`Painter::fill_gradient`] | [`Gradient`], [`GradStop`], [`GradKind`], [`GradExtend`] | `lv_draw_sw_grad` |
//! | line, polyline | [`Painter::line`], [`Painter::polyline`] | [`LineDsc`] | `lv_draw_sw_line` |
//! | arc | [`Painter::arc`] | [`ArcDsc`] | `lv_draw_sw_arc` |
//! | triangle, polygon | [`Painter::triangle`], [`Painter::polygon`], [`Painter::polygon_with_rule`] | [`TriangleDsc`] / [`PolygonDsc`], [`FillRule`] | `lv_draw_sw_triangle` |
//! | image (opa, recolor, chroma key, tiling, clip radius, bitmap mask, rotation/scale) | [`Painter::image`], [`transformed_area`] | [`ImagePixels`], [`ImageDsc`] | `lv_draw_sw_image` |
//! | raw blit (any affine transform) | [`Painter::blit`], [`Painter::blit_transformed`] | [`BlitDsc`] | `lv_draw_sw_image` (layers) |
//! | glyph coverage (text) | [`Painter::glyph_a8`], [`Painter::glyph_lcd`] | [`SubpxOrder`] | `lv_draw_sw_letter` |
//! | masks | [`Painter::push_mask`], [`Painter::pop_mask`] | [`Mask`] | `lv_draw_sw_mask_*` |
//! | layer (group opacity, blend mode, transform) | [`Painter::layer`] | [`LayerDsc`], [`LayerTransform`] | `lv_draw_layer` |
//! | display rotation | [`rotate_buffer`], [`rotate_area`] | — | `lv_draw_sw_rotate` |
//!
//! Translucent **polylines** blend overlapping segment ends twice; draw them inside
//! [`Painter::layer`] when that matters.
//!
//! Hardware accelerators implement [`DrawAccel`]; the software path is always complete.
//!
//! ## Features
//!
//! `color-rgb565`, `color-rgb565-swapped`, `color-rgb888`, `color-xrgb8888`, `color-l8`,
//! `color-i1` select the buffer formats that are compiled (`Argb8888` is always compiled
//! because layers need it); drawing into a disabled format logs a warning once and draws
//! nothing. `log` / `defmt` select the logging backend (target `"twine::render"`); `std`
//! enables std-only conveniences.
//!
//! ## Example
//!
//! ```
//! use twine_core::{Color, ColorFormat, Opa, Rect};
//! use twine_render::{DrawBuf, Painter, RectDsc, RenderCaches};
//!
//! let mut caches = RenderCaches::default();
//! let mut pixels = vec![0u8; 64 * 48 * 2];
//! let buf = DrawBuf::new_packed(&mut pixels, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 64, 48)).unwrap();
//! let mut p = Painter::new(buf, &mut caches);
//! p.fill(Rect::from_xywh(0, 0, 64, 48), Color::WHITE, Opa::COVER);
//! p.rect(
//!     Rect::from_xywh(8, 8, 48, 32),
//!     &RectDsc {
//!         radius: 10,
//!         bg_color: Color::BLUE,
//!         bg_opa: Opa::COVER,
//!         border_width: 2,
//!         border_color: Color::BLACK,
//!         border_opa: Opa::COVER,
//!         ..RectDsc::default()
//!     },
//! );
//! // The center is blue, the corner pixel stays white (outside the rounded corner).
//! let px = |x: usize, y: usize| u16::from_le_bytes([pixels[(y * 64 + x) * 2], pixels[(y * 64 + x) * 2 + 1]]);
//! assert_eq!(px(32, 24), Color::BLUE.to_rgb565());
//! assert_eq!(px(8, 8), Color::WHITE.to_rgb565());
//! ```
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

mod accel;
mod arc;
mod blend;
mod buf;
mod caches;
mod circle;
mod dispatch;
mod error;
mod glyph;
mod gradient;
mod image;
mod image_pixels;
mod layer;
mod line;
mod mask;
mod painter;
mod polygon;
mod rect;
mod rotate;
mod rrect;
mod shadow;
mod transform_blit;

pub use accel::{AccelResult, DrawAccel};
pub use arc::ArcDsc;
pub use blend::{BlendMode, Source, argb_raw, blend_span, mix_color};
pub use buf::{DrawBuf, is_draw_format};
pub use caches::{CacheStats, RenderCacheStats, RenderCaches, RenderConfig};
pub use circle::{CircleCache, MAX_CACHED_RADIUS};
pub use dispatch::is_format_enabled;
pub use error::RenderError;
pub use glyph::SubpxOrder;
pub use gradient::{GradExtend, GradKind, GradStop, Gradient, GradientCache, MAX_STOPS};
pub use image::read::read_row_argb;
pub use image::{ImageDsc, transformed_area};
pub use image_pixels::ImagePixels;
pub use layer::{LayerDsc, LayerTransform, transformed_bounds};
pub use line::LineDsc;
pub use mask::{LineSide, MAX_MASKS, Mask, MaskId, MaskResult, MaskStack};
pub use painter::{ACCEL_MIN_PX, Painter};
pub use polygon::{FillRule, MAX_POLYGON_POINTS, PolygonDsc, TriangleDsc};
pub use rect::{BorderSide, RADIUS_CIRCLE, RectDsc};
pub use rotate::{rotate_area, rotate_buffer};
pub use shadow::{ShadowCache, ShadowDsc, shadow_ext_size};
pub use transform_blit::BlitDsc;
