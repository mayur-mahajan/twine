//! Code 128 barcodes (LVGL `lv_barcode`): an encoder producing the symbol and drawing helpers.
//!
//! [`Code128::encode`] turns ASCII text into Code 128 symbols with automatic code set selection
//! (B by default, C for runs of 4 or more digits, A for control characters), the modulo-103
//! check symbol and the start/stop patterns. [`draw_barcode`] paints the bars with a
//! [`Painter`](twine_render::Painter), horizontally or vertically.
//!
//! ```
//! use twine_extra::barcode::Code128;
//!
//! let code = Code128::encode("Twine 2026").unwrap();
//! // Start B, "Twine ", switch to C, "20" "26", check symbol, stop.
//! assert_eq!(code.symbols().len(), 12);
//! assert_eq!(code.module_count(), 11 * 11 + 13);
//! ```

// NOTE(P25.S02): the `Barcode` engine widget (idempotent `set_data`, warning on invalid text,
// content size from `Code128::module_count`) and the `barcode(data)` view need the engine/view
// layers and are added with them.

mod code128;
mod draw;

pub use code128::{Bars, Code128, Code128Error};
pub use draw::{BarcodeStyle, Orientation, draw_barcode};
