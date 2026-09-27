//! Built-in decoders (each behind its `img-*` feature, except the raw `.tbin` format).

#[cfg(feature = "img-bmp")]
pub mod bmp;
#[cfg(feature = "img-gif")]
pub mod gif;
#[cfg(feature = "img-jpeg")]
pub mod jpeg;
#[cfg(feature = "img-png")]
pub mod png;
#[cfg(feature = "img-qoi")]
pub mod qoi;
pub mod tbin;
