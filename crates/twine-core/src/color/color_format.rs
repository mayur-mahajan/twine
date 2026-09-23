//! The runtime pixel-format tag [`ColorFormat`].

use core::fmt;

/// Pixel formats of images and draw buffers.
///
/// The discriminants are those of LVGL v9 `lv_color_format_t` (verified against LVGL
/// **v9.6.0**, `include/lvgl/draw/lv_color.h`, formerly `src/misc/lv_color.h`), so binary image
/// headers are LVGL-compatible.
///
/// ## Memory layout (identical to LVGL 9)
///
/// | Format | Bits/px (main plane) | Bytes in memory |
/// |--------|----------------------|-----------------|
/// | `Rgb565` | 16 | little-endian `u16` `RRRRRGGG_GGGBBBBB` (low byte first) |
/// | `Rgb565Swapped` | 16 | big-endian `u16` (high byte first, for SPI panels) |
/// | `Rgb565A8` | 16 + 8 | RGB565 (LE) plane of `stride × h` bytes, then an alpha plane of `w × h` bytes (stride `w`) |
/// | `Rgb888` | 24 | `B, G, R` |
/// | `Argb8888` | 32 | `B, G, R, A` (little-endian `u32` `0xAARRGGBB`), straight alpha |
/// | `Xrgb8888` | 32 | `B, G, R, X` (the fourth byte is ignored) |
/// | `Argb8888Premultiplied` | 32 | `B, G, R, A` with color channels multiplied by alpha |
/// | `L8`, `A8`, `I8` | 8 | one byte per pixel |
/// | `A1`/`A2`/`A4`, `I1`/`I2`/`I4` | 1 / 2 / 4 | packed **MSB first** within a byte; every row starts on a byte |
///
/// Indexed formats (`I1`…`I8`) store their palette of [`palette_len`](Self::palette_len)
/// `Argb8888` entries (4 bytes each, `B, G, R, A`) **before** the index data. Alpha-only
/// formats (`A1`…`A8`) are masks: they are drawn in a recolor color (black by default).
///
/// ```
/// use twine_core::ColorFormat;
/// assert_eq!(ColorFormat::Rgb565 as u8, 0x12);
/// assert_eq!(ColorFormat::from_u8(0x12), Some(ColorFormat::Rgb565));
/// assert_eq!(ColorFormat::I4.stride(3), 2);
/// assert_eq!(ColorFormat::I4.buf_size(3, 2), 16 * 4 + 2 * 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum ColorFormat {
    /// 8-bit luminance.
    L8 = 0x06,
    /// 1-bit alpha only.
    A1 = 0x0B,
    /// 2-bit alpha only.
    A2 = 0x0C,
    /// 4-bit alpha only.
    A4 = 0x0D,
    /// 8-bit alpha only.
    A8 = 0x0E,
    /// 1-bit indexed (2-entry ARGB8888 palette first).
    I1 = 0x07,
    /// 2-bit indexed (4-entry palette).
    I2 = 0x08,
    /// 4-bit indexed (16-entry palette).
    I4 = 0x09,
    /// 8-bit indexed (256-entry palette).
    I8 = 0x0A,
    /// 16-bit RGB565, little-endian in memory.
    Rgb565 = 0x12,
    /// 16-bit RGB565 with bytes swapped (big-endian in memory, for SPI panels).
    Rgb565Swapped = 0x1B,
    /// RGB565 plane followed by an 8-bit alpha plane.
    Rgb565A8 = 0x14,
    /// 24-bit, stored B, G, R.
    Rgb888 = 0x0F,
    /// 32-bit with alpha, stored B, G, R, A.
    Argb8888 = 0x10,
    /// 32-bit, alpha byte ignored.
    Xrgb8888 = 0x11,
    /// 32-bit with premultiplied alpha.
    Argb8888Premultiplied = 0x1A,
}

impl ColorFormat {
    /// Every format, in declaration order.
    pub const ALL: [ColorFormat; 16] = [
        ColorFormat::L8,
        ColorFormat::A1,
        ColorFormat::A2,
        ColorFormat::A4,
        ColorFormat::A8,
        ColorFormat::I1,
        ColorFormat::I2,
        ColorFormat::I4,
        ColorFormat::I8,
        ColorFormat::Rgb565,
        ColorFormat::Rgb565Swapped,
        ColorFormat::Rgb565A8,
        ColorFormat::Rgb888,
        ColorFormat::Argb8888,
        ColorFormat::Xrgb8888,
        ColorFormat::Argb8888Premultiplied,
    ];

    /// Bits per pixel of the main plane (`Rgb565A8` → 16).
    #[must_use]
    pub const fn bpp(self) -> u8 {
        match self {
            ColorFormat::A1 | ColorFormat::I1 => 1,
            ColorFormat::A2 | ColorFormat::I2 => 2,
            ColorFormat::A4 | ColorFormat::I4 => 4,
            ColorFormat::L8 | ColorFormat::A8 | ColorFormat::I8 => 8,
            ColorFormat::Rgb565 | ColorFormat::Rgb565Swapped | ColorFormat::Rgb565A8 => 16,
            ColorFormat::Rgb888 => 24,
            ColorFormat::Argb8888 | ColorFormat::Xrgb8888 | ColorFormat::Argb8888Premultiplied => 32,
        }
    }

    /// Whether pixels carry opacity (alpha-only, indexed with ARGB palette, `Rgb565A8`,
    /// `Argb8888*`), like LVGL `lv_color_format_has_alpha`.
    #[must_use]
    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            ColorFormat::A1
                | ColorFormat::A2
                | ColorFormat::A4
                | ColorFormat::A8
                | ColorFormat::I1
                | ColorFormat::I2
                | ColorFormat::I4
                | ColorFormat::I8
                | ColorFormat::Rgb565A8
                | ColorFormat::Argb8888
                | ColorFormat::Argb8888Premultiplied
        )
    }

    /// `I1`, `I2`, `I4`, `I8`.
    #[must_use]
    pub const fn is_indexed(self) -> bool {
        matches!(
            self,
            ColorFormat::I1 | ColorFormat::I2 | ColorFormat::I4 | ColorFormat::I8
        )
    }

    /// `A1`, `A2`, `A4`, `A8`.
    #[must_use]
    pub const fn is_alpha_only(self) -> bool {
        matches!(
            self,
            ColorFormat::A1 | ColorFormat::A2 | ColorFormat::A4 | ColorFormat::A8
        )
    }

    /// Number of palette entries (`I1` → 2 … `I8` → 256; others 0).
    #[must_use]
    pub const fn palette_len(self) -> usize {
        match self {
            ColorFormat::I1 => 2,
            ColorFormat::I2 => 4,
            ColorFormat::I4 => 16,
            ColorFormat::I8 => 256,
            _ => 0,
        }
    }

    /// Bytes per row of the main plane, rounded up to whole bytes (saturating).
    #[must_use]
    pub const fn stride(self, width: u32) -> u32 {
        let bits = width as u64 * self.bpp() as u64;
        let bytes = bits.div_ceil(8);
        if bytes > u32::MAX as u64 {
            u32::MAX
        } else {
            bytes as u32
        }
    }

    /// Minimum bytes per row of the main plane for width `w`: `⌈w · bpp / 8⌉` (saturating).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// assert_eq!(ColorFormat::A1.min_stride(9), 2);
    /// assert_eq!(ColorFormat::Rgb888.min_stride(3), 9);
    /// ```
    #[must_use]
    pub const fn min_stride(self, w: u16) -> u16 {
        let s = self.stride(w as u32);
        if s > u16::MAX as u32 { u16::MAX } else { s as u16 }
    }

    /// Total bytes of a `w × h` image whose main-plane rows are `stride` bytes apart:
    /// palette (`palette_len · 4`, indexed formats) + `stride · h` + the `w · h` alpha plane
    /// of `Rgb565A8` (its stride is always `w`, LVGL 9 convention).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// assert_eq!(ColorFormat::Rgb565A8.data_size(3, 2, 8), 8 * 2 + 3 * 2);
    /// assert_eq!(ColorFormat::I2.data_size(5, 2, 2), 4 * 4 + 2 * 2);
    /// ```
    #[must_use]
    pub const fn data_size(self, w: u16, h: u16, stride: u16) -> usize {
        let (w, h, stride) = (w as usize, h as usize, stride as usize);
        let mut total = self.palette_len() * 4 + stride * h;
        if matches!(self, ColorFormat::Rgb565A8) {
            total += w * h;
        }
        total
    }

    /// Total bytes of a `w × h` image: palette (4 bytes per entry) + main plane (+ the alpha
    /// plane of `Rgb565A8`), saturating.
    #[must_use]
    pub const fn buf_size(self, w: u32, h: u32) -> u32 {
        let mut total = self.palette_len() as u64 * 4 + self.stride(w) as u64 * h as u64;
        if matches!(self, ColorFormat::Rgb565A8) {
            total += w as u64 * h as u64;
        }
        if total > u32::MAX as u64 {
            u32::MAX
        } else {
            total as u32
        }
    }

    /// The format with LVGL discriminant `v`, if supported.
    #[must_use]
    pub const fn from_u8(v: u8) -> Option<ColorFormat> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as u8 == v {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// Short name, e.g. `"RGB565"`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ColorFormat::L8 => "L8",
            ColorFormat::A1 => "A1",
            ColorFormat::A2 => "A2",
            ColorFormat::A4 => "A4",
            ColorFormat::A8 => "A8",
            ColorFormat::I1 => "I1",
            ColorFormat::I2 => "I2",
            ColorFormat::I4 => "I4",
            ColorFormat::I8 => "I8",
            ColorFormat::Rgb565 => "RGB565",
            ColorFormat::Rgb565Swapped => "RGB565_SWAPPED",
            ColorFormat::Rgb565A8 => "RGB565A8",
            ColorFormat::Rgb888 => "RGB888",
            ColorFormat::Argb8888 => "ARGB8888",
            ColorFormat::Xrgb8888 => "XRGB8888",
            ColorFormat::Argb8888Premultiplied => "ARGB8888_PREMULTIPLIED",
        }
    }
}

impl fmt::Display for ColorFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl TryFrom<u8> for ColorFormat {
    type Error = crate::Error;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Self::from_u8(v).ok_or(crate::Error::Unsupported("color format"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_format_stride_and_buf_size_table() {
        // (format, [stride for widths 1, 7, 8, 320], buf_size(320, 2))
        let table: [(ColorFormat, [u32; 4], u32); 16] = [
            (ColorFormat::L8, [1, 7, 8, 320], 640),
            (ColorFormat::A1, [1, 1, 1, 40], 80),
            (ColorFormat::A2, [1, 2, 2, 80], 160),
            (ColorFormat::A4, [1, 4, 4, 160], 320),
            (ColorFormat::A8, [1, 7, 8, 320], 640),
            (ColorFormat::I1, [1, 1, 1, 40], 8 + 80),
            (ColorFormat::I2, [1, 2, 2, 80], 16 + 160),
            (ColorFormat::I4, [1, 4, 4, 160], 64 + 320),
            (ColorFormat::I8, [1, 7, 8, 320], 1024 + 640),
            (ColorFormat::Rgb565, [2, 14, 16, 640], 1280),
            (ColorFormat::Rgb565Swapped, [2, 14, 16, 640], 1280),
            (ColorFormat::Rgb565A8, [2, 14, 16, 640], 1280 + 640),
            (ColorFormat::Rgb888, [3, 21, 24, 960], 1920),
            (ColorFormat::Argb8888, [4, 28, 32, 1280], 2560),
            (ColorFormat::Xrgb8888, [4, 28, 32, 1280], 2560),
            (ColorFormat::Argb8888Premultiplied, [4, 28, 32, 1280], 2560),
        ];
        for (f, strides, size) in table {
            for (w, s) in [1, 7, 8, 320].into_iter().zip(strides) {
                assert_eq!(f.stride(w), s, "{f} stride({w})");
            }
            assert_eq!(f.buf_size(320, 2), size, "{f} buf_size");
            assert_eq!(ColorFormat::from_u8(f as u8), Some(f));
            assert_eq!(ColorFormat::try_from(f as u8), Ok(f));
        }
        assert_eq!(ColorFormat::from_u8(0), None);
        assert!(ColorFormat::try_from(0x30).is_err());
        assert_eq!(ColorFormat::Argb8888.stride(u32::MAX), u32::MAX);
        assert_eq!(ColorFormat::Argb8888.buf_size(u32::MAX, 2), u32::MAX);
    }

    #[test]
    fn color_format_stride_and_size_all_formats() {
        // (format, bpp, has_alpha, palette_len)
        let table: [(ColorFormat, u8, bool, usize); 16] = [
            (ColorFormat::L8, 8, false, 0),
            (ColorFormat::A1, 1, true, 0),
            (ColorFormat::A2, 2, true, 0),
            (ColorFormat::A4, 4, true, 0),
            (ColorFormat::A8, 8, true, 0),
            (ColorFormat::I1, 1, true, 2),
            (ColorFormat::I2, 2, true, 4),
            (ColorFormat::I4, 4, true, 16),
            (ColorFormat::I8, 8, true, 256),
            (ColorFormat::Rgb565, 16, false, 0),
            (ColorFormat::Rgb565Swapped, 16, false, 0),
            (ColorFormat::Rgb565A8, 16, true, 0),
            (ColorFormat::Rgb888, 24, false, 0),
            (ColorFormat::Argb8888, 32, true, 0),
            (ColorFormat::Xrgb8888, 32, false, 0),
            (ColorFormat::Argb8888Premultiplied, 32, true, 0),
        ];
        assert_eq!(table.len(), ColorFormat::ALL.len());
        for (f, bpp, alpha, pal) in table {
            assert_eq!(f.bpp(), bpp, "{f}");
            assert_eq!(f.has_alpha(), alpha, "{f}");
            assert_eq!(f.palette_len(), pal, "{f}");
            assert_eq!(f.is_indexed(), pal > 0, "{f}");
            for w in [1u16, 7, 8, 33] {
                let stride = (u32::from(w) * u32::from(bpp)).div_ceil(8) as u16;
                assert_eq!(f.min_stride(w), stride, "{f} w={w}");
                for h in [1u16, 5] {
                    let mut size = pal * 4 + usize::from(stride) * usize::from(h);
                    if f == ColorFormat::Rgb565A8 {
                        size += usize::from(w) * usize::from(h);
                    }
                    assert_eq!(f.data_size(w, h, stride), size, "{f} {w}x{h}");
                    assert_eq!(f.buf_size(u32::from(w), u32::from(h)) as usize, size, "{f}");
                    // A padded stride only grows the main plane.
                    assert_eq!(f.data_size(w, h, stride + 4) - size, 4 * usize::from(h), "{f}");
                }
            }
        }
        assert_eq!(ColorFormat::Argb8888.min_stride(u16::MAX), u16::MAX);
    }

    #[test]
    fn lvgl_discriminants() {
        let expected = [
            (ColorFormat::L8, 0x06),
            (ColorFormat::I1, 0x07),
            (ColorFormat::I8, 0x0A),
            (ColorFormat::A1, 0x0B),
            (ColorFormat::A8, 0x0E),
            (ColorFormat::Rgb888, 0x0F),
            (ColorFormat::Argb8888, 0x10),
            (ColorFormat::Xrgb8888, 0x11),
            (ColorFormat::Rgb565, 0x12),
            (ColorFormat::Rgb565A8, 0x14),
            (ColorFormat::Argb8888Premultiplied, 0x1A),
            (ColorFormat::Rgb565Swapped, 0x1B),
        ];
        for (f, v) in expected {
            assert_eq!(f as u8, v, "{f}");
        }
    }

    #[test]
    fn predicates() {
        assert!(ColorFormat::I2.is_indexed() && !ColorFormat::A2.is_indexed());
        assert!(ColorFormat::A4.is_alpha_only() && ColorFormat::A4.has_alpha());
        assert!(!ColorFormat::Rgb565.has_alpha() && ColorFormat::Rgb565A8.has_alpha());
        assert_eq!(ColorFormat::I8.palette_len(), 256);
        assert_eq!(ColorFormat::Rgb888.palette_len(), 0);
        assert_eq!(alloc::format!("{}", ColorFormat::Rgb565Swapped), "RGB565_SWAPPED");
    }
}
