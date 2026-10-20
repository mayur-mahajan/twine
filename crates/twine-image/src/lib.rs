//! # twine-image
//!
//! Images for the Twine GUI library: the [`Image`] data model (every [`ColorFormat`] of the
//! renderer, palettes, `Rgb565A8` alpha planes, premultiplied alpha), compressed images (LVGL
//! RLE, LZ4), decoders for encoded files (QOI, PNG, JPEG, BMP, GIF and the raw `.tbin`
//! format of [`decoders::tbin`]), a byte-budgeted
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
#![cfg_attr(
    feature = "img-gif",
    doc = "| `img-gif` | in-house GIF decoder and [`GifPlayer`](decoders::gif::GifPlayer) animation |"
)]
#![cfg_attr(
    not(feature = "img-gif"),
    doc = "| `img-gif` | in-house GIF decoder and `GifPlayer` animation |"
)]
//! | `img-lz4` | LZ4-compressed images |
//! | `std` | encoders used by tools and tests |
//! | `log` / `defmt` | logging backend (target `"twine::image"`) |
//! | `safety` | the safety profile (enabled by the `twine` facade's `safety` feature, see `docs/safety/profile.md` in the repository): the profile's lints; refuses `img-jpeg` |
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]
// The safety profile's lints (`docs/safety/profile.md` § Lints): denied in library code under
// the `safety` feature (`cargo xtask ci` stage `clippy-safety`).
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable,)
)]
// NOTE(R5.S13): sites of these lints remain (counts: `docs/safety/profile.md` § Lint status,
// `cargo xtask safety-lints`); a lint moves to `deny` above when its last site is fixed.
#![cfg_attr(
    all(feature = "safety", not(test)),
    allow(clippy::indexing_slicing, clippy::arithmetic_side_effects,)
)]

// The safety profile refuses the parts it excludes (`docs/safety/profile.md` § Exclusions).
#[cfg(all(feature = "safety", feature = "img-jpeg"))]
compile_error!(
    "the `safety` profile excludes the JPEG decoder (`img-jpeg`: third-party decoder with \
     floating-point code); convert images at build time (QOI, PNG, raw); see \
     docs/safety/profile.md"
);

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
pub use resolve::{FileSource, HEADER_PROBE_BYTES, ImageContext, header_of, with_pixels};
pub use twine_core::{ColorFormat, Symbol};
