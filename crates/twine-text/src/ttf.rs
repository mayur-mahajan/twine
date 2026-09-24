//! Runtime TrueType/OpenType fonts (feature `ttf`): [`TtfProvider`] rasterizes glyphs on
//! demand with `fontdue` and keeps them in the [`GlyphCache`]; [`TtfFont`] builds the
//! `'static` [`Font`] around it.
//!
//! Float arithmetic is used here, and only here: `fontdue` measures and rasterizes outlines in
//! `f32`. Every result is converted to the crate's integer metrics once (advances in 1/16 px,
//! boxes in whole pixels), so layout and drawing stay integer. On MCUs without an FPU the
//! rasterization of each new glyph is slow; cached glyphs cost the same as bitmap fonts.
// Exception to the crate-wide float ban (runtime TTF is feature-gated): outline metrics and
// rasterization are inherently floating point; results are converted to integers right away.
#![allow(clippy::float_arithmetic, clippy::cast_precision_loss)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::cache::GlyphCache;
use crate::font::{Font, GlyphInfo, GlyphProvider, Subpx, provider_key};

/// Errors of [`TtfFont`] / [`TtfProvider`] creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TtfError {
    /// The data is not a TrueType/OpenType font (message from the parser).
    Parse(&'static str),
    /// The pixel size is 0 or above [`TTF_MAX_PX`].
    InvalidSize(u16),
    /// The `StaticCell` passed to [`TtfFont::new_in`] is already initialized.
    AlreadyInitialized,
    /// The font file could not be read (see [`FontFileSource`]).
    File,
}

impl core::fmt::Display for TtfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "invalid TrueType font: {m}"),
            Self::InvalidSize(px) => write!(f, "invalid font size {px} px (1..={TTF_MAX_PX})"),
            Self::AlreadyInitialized => f.write_str("font cell already initialized"),
            Self::File => f.write_str("cannot read font file"),
        }
    }
}

impl core::error::Error for TtfError {}

/// Largest supported pixel size (glyph boxes must fit the 255 × 255 glyph scratch).
pub const TTF_MAX_PX: u16 = 200;

/// Rounds half away from zero (no `f32::round` in `core`).
fn round(v: f32) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        (v - 0.5) as i32
    }
}

/// Smallest integer ≥ `v` (no `f32::ceil` in `core`).
fn ceil(v: f32) -> i32 {
    let t = v as i32;
    if (t as f32) < v { t + 1 } else { t }
}

/// A [`GlyphProvider`] rasterizing a TrueType/OpenType font at one pixel size.
///
/// Glyph ids are the font's glyph indices; the glyph-cache key is `(provider address, glyph
/// index)`, and since a provider has exactly one size, a font used at several sizes needs
/// one provider per size (each with its own cache entries). Kerning comes from the legacy
/// `kern` table (`fontdue`); GPOS kerning is not applied at runtime.
///
/// Rasterizing a glyph allocates once (fontdue returns a `Vec`), then the coverage is copied
/// into the cache; glyphs that are cached cost no allocation and no float math.
pub struct TtfProvider {
    font: fontdue::Font,
    px: u16,
    ascent: i32,
    descent: i32,
    underline_position: i8,
    underline_thickness: u8,
}

impl core::fmt::Debug for TtfProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TtfProvider")
            .field("name", &self.font.name())
            .field("px", &self.px)
            .field("glyphs", &self.font.glyph_count())
            .finish_non_exhaustive()
    }
}

impl TtfProvider {
    /// Parses `data` (the font is parsed completely; `data` is not kept) for rendering at
    /// `px` pixels per em.
    pub fn new(data: &[u8], px: u16) -> Result<Self, TtfError> {
        if px == 0 || px > TTF_MAX_PX {
            return Err(TtfError::InvalidSize(px));
        }
        let settings = fontdue::FontSettings {
            scale: f32::from(px),
            ..fontdue::FontSettings::default()
        };
        let font = fontdue::Font::from_bytes(data, settings).map_err(TtfError::Parse)?;
        let size = f32::from(px);
        let (ascent, descent) = font
            .horizontal_line_metrics(size)
            .map_or((i32::from(px), 0), |m| (ceil(m.ascent), ceil(-m.descent)));
        let scale = size / font.units_per_em();
        let (underline_position, underline_thickness) = ttf_parser::Face::parse(data, 0)
            .ok()
            .and_then(|f| f.underline_metrics())
            .map_or((-(descent / 2).max(1) as i8, 1), |u| {
                (
                    round(f32::from(u.position) * scale).clamp(-128, 127) as i8,
                    round(f32::from(u.thickness) * scale).clamp(1, 255) as u8,
                )
            });
        twine_core::debug!(
            target: "twine::text",
            "ttf font {} px: ascent {} descent {} glyphs {}",
            px,
            ascent,
            descent,
            font.glyph_count()
        );
        Ok(Self {
            font,
            px,
            ascent: ascent.max(0),
            descent: descent.max(0),
            underline_position,
            underline_thickness,
        })
    }

    /// The pixel size.
    #[must_use]
    pub fn px(&self) -> u16 {
        self.px
    }

    /// Line height in px: `ceil(ascent) + ceil(descent)` (hhea metrics, same rule as the
    /// font generator).
    #[must_use]
    pub fn line_height(&self) -> i16 {
        (self.ascent + self.descent).clamp(0, i32::from(i16::MAX)) as i16
    }

    /// Distance of the baseline from the bottom of the line: `ceil(descent)`.
    #[must_use]
    pub fn base_line(&self) -> i16 {
        self.descent.clamp(0, i32::from(i16::MAX)) as i16
    }

    /// A [`Font`] over this provider with the given fallback.
    #[must_use]
    pub fn font(&'static self, fallback: Option<&'static Font>) -> Font {
        Font {
            line_height: self.line_height(),
            base_line: self.base_line(),
            underline_position: self.underline_position,
            underline_thickness: self.underline_thickness,
            provider: self,
            fallback,
            subpx: Subpx::None,
        }
    }

    /// Rasterizes the glyph of `info` into `out` (`box_w × box_h`, stride `box_w`).
    fn rasterize(&self, info: &GlyphInfo, out: &mut [u8]) -> bool {
        let Ok(id) = u16::try_from(info.id) else {
            return false;
        };
        let (m, bmp) = self.font.rasterize_indexed(id, f32::from(self.px));
        let len = m.width * m.height;
        if m.width != usize::from(info.box_w) || m.height != usize::from(info.box_h) || out.len() < len {
            return false;
        }
        out[..len].copy_from_slice(&bmp[..len]);
        true
    }

    fn index(&self, cp: char) -> Option<u16> {
        match self.font.lookup_glyph_index(cp) {
            0 => None,
            i => Some(i),
        }
    }
}

impl GlyphProvider for TtfProvider {
    fn glyph_info(&self, cp: char, next: Option<char>) -> Option<GlyphInfo> {
        let id = self.index(cp)?;
        let size = f32::from(self.px);
        let m = self.font.metrics_indexed(id, size);
        let kern = next
            .and_then(|n| self.index(n))
            .and_then(|n| self.font.horizontal_kern_indexed(id, n, size))
            .unwrap_or(0.0);
        let adv = round((m.advance_width + kern) * 16.0).clamp(0, i32::from(u16::MAX));
        Some(GlyphInfo {
            adv_w: adv as u16,
            box_w: m.width.min(usize::from(u16::MAX)) as u16,
            box_h: m.height.min(usize::from(u16::MAX)) as u16,
            ofs_x: m.xmin.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            ofs_y: m.ymin.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            bpp: 8,
            id: u32::from(id),
            is_placeholder: false,
        })
    }

    fn render_a8(&self, info: &GlyphInfo, out: &mut [u8], _cache: &mut GlyphCache) -> bool {
        self.rasterize(info, out)
    }

    fn render_rows(
        &self,
        info: &GlyphInfo,
        cache: &mut GlyphCache,
        sink: &mut dyn FnMut(usize, &[u8]),
    ) -> bool {
        let (w, h) = (usize::from(info.box_w), usize::from(info.box_h));
        if w == 0 || h == 0 {
            return true;
        }
        let key = (provider_key(self), info.id);
        if let Some(bmp) = cache.get(key) {
            for (y, row) in bmp.chunks_exact(w).take(h).enumerate() {
                sink(y, row);
            }
            return true;
        }
        twine_core::trace!(target: "twine::text", "ttf rasterizes glyph {} at {} px", info.id, self.px);
        if let Some(slot) = cache.insert(key, w, h) {
            if !self.rasterize(info, slot) {
                cache.remove(key);
                return false;
            }
            for (y, row) in slot.chunks_exact(w).enumerate() {
                sink(y, row);
            }
            return true;
        }
        // Larger than the cache budget: rasterize into the scratch every time.
        let Some(mut buf) = cache.take_glyph_scratch(w * h) else {
            return false;
        };
        let ok = self.rasterize(info, &mut buf);
        if ok {
            for (y, row) in buf.chunks_exact(w).enumerate() {
                sink(y, row);
            }
        }
        cache.put_glyph_scratch(buf);
        ok
    }
}

/// Reads whole font files for [`TtfFont::from_file`]. `twine-text` has no file system of its
/// own: an adapter over the application's file system implements this (for example over
/// `twine_fs::Vfs`: `vfs.read_to_vec(path)`).
// NOTE(P22.S03): the engine's `fs` feature provides the adapter over `twine_fs::Vfs`.
pub trait FontFileSource {
    /// Replaces the contents of `out` with the file at `path`; [`TtfError::File`] if it cannot
    /// be read.
    fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), TtfError>;
}

/// A runtime TrueType font: a [`TtfProvider`] plus the [`Font`] that uses it.
///
/// Fonts live for the whole program, so the pair is either leaked ([`TtfFont::new`]) or placed
/// in a caller-provided `static` [`StaticCell`](static_cell::StaticCell)
/// ([`TtfFont::new_in`]).
///
/// ```
/// use static_cell::StaticCell;
/// use twine_text::{TextLayout, TtfFont};
///
/// # let ttf: &'static [u8] = include_bytes!("../../../assets/fonts/Montserrat-Medium.ttf");
/// static HEADING: StaticCell<TtfFont> = StaticCell::new();
/// let font = TtfFont::new_in(&HEADING, ttf, 24).unwrap();
/// assert!(TextLayout::new("Hello", font).measure().w > 50);
/// ```
pub struct TtfFont {
    provider: TtfProvider,
    font: Option<Font>,
}

impl core::fmt::Debug for TtfFont {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TtfFont")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl TtfFont {
    /// Initializes `slot` in place and returns the font inside it.
    fn install(slot: &'static mut TtfFont, fallback: Option<&'static Font>) -> &'static Font {
        let TtfFont { provider, font } = slot;
        let provider: &'static TtfProvider = provider;
        font.insert(provider.font(fallback))
    }

    /// Parses `data` and returns a `'static` font of `px` pixels. The provider and the font
    /// are leaked (`Box::leak`): call this once per font and size at startup.
    #[allow(clippy::new_ret_no_self)] // the font is the useful result; the slot is leaked
    pub fn new(data: &[u8], px: u16) -> Result<&'static Font, TtfError> {
        Self::new_with_fallback(data, px, None)
    }

    /// Like [`new`](Self::new), with a fallback font for characters the TTF lacks.
    pub fn new_with_fallback(
        data: &[u8],
        px: u16,
        fallback: Option<&'static Font>,
    ) -> Result<&'static Font, TtfError> {
        let provider = TtfProvider::new(data, px)?;
        let slot = Box::leak(Box::new(TtfFont { provider, font: None }));
        Ok(Self::install(slot, fallback))
    }

    /// Parses `data` into the caller's `static` cell instead of leaking.
    /// [`TtfError::AlreadyInitialized`] if `cell` was used before (a
    /// [`StaticCell`](static_cell::StaticCell) is initialized once).
    pub fn new_in(
        cell: &'static static_cell::StaticCell<TtfFont>,
        data: &[u8],
        px: u16,
    ) -> Result<&'static Font, TtfError> {
        let provider = TtfProvider::new(data, px)?;
        let slot = cell
            .try_init(TtfFont { provider, font: None })
            .ok_or(TtfError::AlreadyInitialized)?;
        Ok(Self::install(slot, None))
    }

    /// Reads the font file `path` (e.g. `"A:/fonts/heading.ttf"`) from `files` and returns a
    /// leaked font of `px` pixels (the file data is freed after parsing).
    pub fn from_file(files: &mut dyn FontFileSource, path: &str, px: u16) -> Result<&'static Font, TtfError> {
        let mut data = Vec::new();
        files.read_all(path, &mut data)?;
        Self::new(&data, px)
    }
}
