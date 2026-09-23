//! # twine-text
//!
//! Fonts, text layout and text drawing for the Twine GUI library (layer 3, above the renderer).
//!
//! - **Fonts.** A [`Font`] holds line metrics, a [`GlyphProvider`] and an optional fallback
//!   font. The built-in provider is [`BitmapFont`], a bitmap format with the same capabilities
//!   and semantics as LVGL's `lv_font_fmt_txt` (LVGL `src/font/lv_font_fmt_txt.h`): 1/2/4/8
//!   bits per pixel, four cmap kinds, pair and class kerning, optional RLE compression with an
//!   XOR row prefilter, and subpixel ([`Subpx`]) glyphs. Ready-made fonts (Montserrat with
//!   Font Awesome [`symbols`], unscii) live in the `twine-assets` crate; the `twine font`
//!   generator (`twine-cli`) produces new ones from TTF files.
//! - **Glyph cache.** [`GlyphCache`] keeps decoded glyphs of compressed fonts in a fixed byte
//!   budget (LRU) and owns the scratch memory of text drawing, so drawing never allocates.
//! - **Layout.** [`TextLayout`] breaks text into lines (LVGL rules), measures it, aligns lines,
//!   ellipsizes and hit-tests ([`TextLayout::char_at`] / [`TextLayout::pos_of`]).
//! - **Drawing.** [`draw_text`] draws text through a [`Painter`](twine_render::Painter) with
//!   letter/line spacing, alignment, underline, strikethrough, selection, ellipsis and
//!   subpixel fonts, processing only the lines inside the clip.
//!
//! All metrics are integers: glyph advances are stored in 1/16 px and rounded **per glyph** to
//! whole pixels exactly like LVGL (`(adv + 8) >> 4`), so layout and rendering are identical on
//! every target. The crate is `no_std` + `alloc` and float-free.
//!
//! ## Example
//!
//! ```
//! use twine_assets::fonts::MONTSERRAT_14;
//! use twine_core::{Color, ColorFormat, Point, Rect};
//! use twine_render::{DrawBuf, Painter, RenderCaches};
//! use twine_text::{GlyphCache, TextAlign, TextDecor, TextDsc, TextLayout, draw_text};
//!
//! // Measure and hit-test.
//! let mut layout = TextLayout::new("Hello wide world", &MONTSERRAT_14);
//! layout.max_width = 60;
//! assert_eq!(layout.line_count(), 3);
//! let cursor = layout.pos_of(6, TextAlign::Left, 60); // before "wide"
//! assert_eq!(cursor.y, i32::from(MONTSERRAT_14.line_height));
//! assert_eq!(layout.char_at(cursor, TextAlign::Left, 60), 6);
//!
//! // Draw (once per frame / chunk; the caches are created once).
//! let mut caches = RenderCaches::default();
//! let mut glyphs = GlyphCache::default();
//! let mut px = vec![0u8; 120 * 40 * 2];
//! let area = Rect::from_xywh(0, 0, 120, 40);
//! let buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, area).unwrap();
//! let mut painter = Painter::new(buf, &mut caches);
//! let mut dsc = TextDsc::new(&MONTSERRAT_14);
//! dsc.color = Color::WHITE;
//! dsc.align = TextAlign::Center;
//! dsc.decor = TextDecor::UNDERLINE;
//! draw_text(&mut painter, area, "Hello", &dsc, &mut glyphs);
//! ```
//!
//! ## Bitmap format
//!
//! Plain bitmaps are bit-packed rows without padding (MSB first); compressed bitmaps use LVGL's
//! RLE scheme with an optional XOR prefilter. The bit-level format is documented in the
//! `decode` module source, and the encoder (feature `std`, module `encode`) is its exact
//! inverse.
//!
//! ## Features
//!
//! `std` (std conveniences and the `encode` module used by font generators), `log` / `defmt`
//! (logging backend, target `"twine::text"`).
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

mod bitmap_font;
mod cache;
mod decode;
mod draw;
mod font;
mod hit;
mod layout;
pub mod symbols;
#[cfg(test)]
mod test_font;

#[cfg(feature = "std")]
pub mod encode;

pub use bitmap_font::{BitmapFont, BitmapFormat, Cmap, CmapKind, GlyphDsc, GlyphIdOfs, Kern, KernPairIds};
pub use cache::{
    BLOCK_BYTES, CacheStats, DEFAULT_BUDGET, GlyphCache, GlyphKey, MAX_ENTRIES, MAX_GLYPH_BYTES, MAX_ROW,
};
pub use draw::{TextDrawFlags, TextDsc, draw_text};
pub use font::{
    EMPTY_FONT, Font, GlyphInfo, GlyphProvider, MAX_FALLBACK_DEPTH, Subpx, has_placeholder, provider_key,
};
pub use hit::{DOTS, Ellipsis, LongMode, TextAlign, TextDecor};
pub use layout::{BREAK_CHARS, Line, LineIter, TextFlags, TextLayout, is_wide};
