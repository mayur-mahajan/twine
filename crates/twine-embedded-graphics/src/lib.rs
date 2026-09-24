//! [embedded-graphics] adapter for the Twine GUI library, in both directions.
//!
//! - [`EgDisplay`] turns any `embedded_graphics_core::draw_target::DrawTarget` — an existing
//!   embedded-graphics display driver, the embedded-graphics simulator, a framebuffer — into a
//!   twine `DisplayDriver`. The pixel format follows the target's colour type ([`EgColor`]:
//!   `Rgb565`, `Rgb888`, `Gray8`, `BinaryColor`).
//! - `PainterTarget` (feature `drawtarget`) goes the other way: embedded-graphics primitives,
//!   fonts and images drawn inside a twine canvas through a `Painter`
//!   (`painter.as_draw_target::<Rgb565>()` with `PainterExt`).
//!
//! [embedded-graphics]: https://docs.rs/embedded-graphics
#![no_std]

mod color;
mod display;
#[cfg(feature = "drawtarget")]
mod target;

pub use color::EgColor;
pub use display::{EgDisplay, EgError};
#[cfg(feature = "drawtarget")]
pub use target::{PainterExt, PainterTarget};
