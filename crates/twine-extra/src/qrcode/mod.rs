//! QR codes (LVGL `lv_qrcode`): an encoder producing a module matrix and drawing helpers.
//!
//! [`QrMatrix::encode`] turns bytes into the smallest QR code (versions 1–40) that holds them at
//! the requested error correction level. Numeric, alphanumeric or byte mode is chosen
//! automatically, and the level is raised while the data still fits the same version (like
//! LVGL). [`draw_qr`] paints a matrix into an area with a [`Painter`](twine_render::Painter).
//!
//! ```
//! use twine_extra::qrcode::{Ecc, QrMatrix};
//!
//! let qr = QrMatrix::encode(b"https://example.com", Ecc::Medium).unwrap();
//! assert_eq!(qr.version(), 2);
//! assert_eq!(qr.size(), 25);
//! // Finder pattern: the top-left module is dark, the one inside its white ring is light.
//! assert!(qr.get(0, 0));
//! assert!(!qr.get(1, 1));
//! ```
//!
//! The encoder is Nayuki's QR Code generator (`qrcodegen-no-heap`, MIT), which needs no heap by
//! itself; this module allocates its scratch buffers only while encoding (≈ 2 × 3.9 KiB for the
//! largest version, less for small codes) and keeps just the packed matrix (≤ 4 KiB).

// NOTE(P25.S01): the `QrCode` engine widget (re-encodes only when the data changes, draws with
// `draw_qr` or `draw_qr_placeholder` after warning about data that is too long) and the
// `qrcode(data, size)` view need the engine/view layers and are added with them.

mod draw;
mod encoder;

pub use draw::{QrLayout, QrStyle, draw_qr, draw_qr_placeholder};
pub use encoder::{DarkRuns, Ecc, QrError, QrMatrix};
