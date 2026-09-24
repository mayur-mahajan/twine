//! # twine-image
//!
//! Images for the Twine GUI library: the [`Image`] data model (every [`ColorFormat`] of the
//! renderer, palettes, `Rgb565A8` alpha planes, premultiplied alpha), compressed images (LVGL
//! RLE, LZ4), decoders for encoded files (QOI, PNG, JPEG, BMP, GIF), a byte-budgeted
//! [`ImageCache`] and the resolution of any [`ImageSource`] to drawable
//! [`ImagePixels`](twine_render::ImagePixels).
//!
//! Drawing itself is done by `twine-render` (`Painter::image`); this crate only produces
//! pixels. It is `no_std` + `alloc`.
//!
//! ## Features
//!
//! | Feature | Enables |
//! |---------|---------|
//! | `img-qoi` | in-house QOI decoder |
//! | `img-png` | in-house PNG decoder over `miniz_oxide` (all color types, Adam7) |
//! | `img-jpeg` | JPEG decoder (`zune-jpeg`, baseline + progressive) |
//! | `img-bmp` | in-house BMP decoder |
//! | `img-gif` | in-house GIF decoder and [`GifPlayer`](decoders::gif::GifPlayer) animation |
//! | `img-lz4` | LZ4-compressed images |
//! | `std` | encoders used by tools and tests |
//! | `log` / `defmt` | logging backend (target `"twine::image"`) |
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod cache;
mod compress;
mod decoder;
pub mod decoders;
mod error;
mod image;
mod resolve;

pub use cache::{CacheStats, CachedImage, HEADER_CACHE_ENTRIES, ImageCache, ImageHeaderCache, SourceKey};
pub use compress::*;
pub use decoder::{Decoder, DecoderRegistry, MAX_DECODERS};
pub use error::Error;
pub use image::{
    Compression, Image, ImageData, ImageFlags, ImageHeader, ImageSource, MAX_PATH_LEN, pixels_of,
};
pub use resolve::{FileSource, ImageContext, header_of, with_pixels};
pub use twine_core::ColorFormat;
