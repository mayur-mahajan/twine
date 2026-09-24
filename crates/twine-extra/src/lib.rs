//! # twine-extra
//!
//! Extras for the Twine GUI library, the equivalent of LVGL's "others" and "libs": QR codes,
//! Code 128 barcodes, and (later) a file explorer, a Pinyin IME, a monkey tester, snapshots and a
//! system monitor. Every module sits behind a cargo feature of the same name; no feature is
//! enabled by default.
//!
//! ## Modules
//!
//! - `qrcode` (feature `qrcode`): `QrMatrix` encodes bytes into a QR code module matrix
//!   (versions 1–40, error correction levels L/M/Q/H, numeric/alphanumeric/byte mode chosen
//!   automatically); `draw_qr` paints it with a `twine_render::Painter`, scaled to whole pixels
//!   per module.
//! - `barcode` (feature `barcode`): `Code128` encodes ASCII text into a Code 128 symbol with
//!   automatic code set selection; `draw_barcode` paints it horizontally or vertically.
//!
//! Encoders are pure (`no_std` + `alloc`, no floating point) and allocate only when encoding.
//! Drawing never allocates and touches only the rows inside the painter's clip, so drawing in
//! horizontal chunks (partial display buffers) is pixel-identical to drawing at once.
//!
//! ## Other features
//!
//! - `log` / `defmt`: logging backend (target `"twine::extra"`); `std`: std conveniences.
#![no_std]

#[allow(unused_extern_crates)] // unused when no encoder feature is enabled
extern crate alloc;
#[cfg(feature = "std")]
#[allow(unused_extern_crates)] // only `std` error impls use it
extern crate std;

#[cfg(feature = "barcode")]
pub mod barcode;
#[cfg(feature = "qrcode")]
pub mod qrcode;
