//! The blending core: [`blend_span`] with its fast paths, [`BlendMode`] and [`Source`].
//!
//! Every primitive ends in [`blend_span`] (through the painter's masked-span helper). Paths,
//! chosen once per call:
//!
//! 1. solid color, cover opacity, no mask, [`BlendMode::Normal`] → raw fill (the pixel is
//!    written once, then the written prefix is doubled with `copy_within`);
//! 2. solid color with opacity or mask → per-pixel mix (`a ≤ Opa::MIN` skipped,
//!    `a ≥ Opa::MAX` writes the color);
//! 3. pixels of the same (alpha-less) format, cover, no mask, Normal → `copy_from_slice`;
//! 4. everything else → the source pixel is converted to a [`Color`] with alpha and mixed.
//!
//! The effective alpha of a pixel is `udiv255(mask · opa)` (just `mask` at cover opacity),
//! multiplied by the source pixel's alpha for sources with alpha.
//!
//! An `Argb8888` destination (layers) is composited with the straight-alpha "over" operator:
//! drawing onto transparent pixels keeps the source alpha.

use core::sync::atomic::AtomicBool;

use twine_core::color::{Argb8888, L8, Rgb565, Rgb565Swapped, Rgb888, Xrgb8888, mix_channel};
use twine_core::math::udiv255;
use twine_core::{Color, ColorFormat, Opa, PixelFormat};

use crate::DrawBuf;
use crate::dispatch::{dispatch_format, warn_once};

/// How source colors combine with the destination before alpha mixing (the LVGL set).
///
/// Channel operations: Additive `min(255, d + s)`, Subtractive `max(0, d − s)`, Multiply
/// `d · s / 255`, Difference `|d − s|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BlendMode {
    /// The source replaces the destination (mixed by alpha).
    #[default]
    Normal,
    /// Channels are added (saturating).
    Additive,
    /// Source channels are subtracted from the destination (saturating at 0).
    Subtractive,
    /// Channels are multiplied.
    Multiply,
    /// Absolute channel difference.
    Difference,
}

impl BlendMode {
    /// Every mode, in declaration order.
    pub const ALL: [BlendMode; 5] = [
        BlendMode::Normal,
        BlendMode::Additive,
        BlendMode::Subtractive,
        BlendMode::Multiply,
        BlendMode::Difference,
    ];

    /// Combines one channel of the source `s` with the destination `d`.
    #[inline(always)]
    #[must_use]
    pub fn channel(self, s: u8, d: u8) -> u8 {
        match self {
            BlendMode::Normal => s,
            BlendMode::Additive => s.saturating_add(d),
            BlendMode::Subtractive => d.saturating_sub(s),
            BlendMode::Multiply => udiv255(u32::from(s) * u32::from(d)) as u8,
            BlendMode::Difference => s.abs_diff(d),
        }
    }

    /// Combines the source color `s` with the destination color `d` channel by channel.
    #[inline(always)]
    #[must_use]
    pub fn color(self, s: Color, d: Color) -> Color {
        if self == BlendMode::Normal {
            return s;
        }
        Color::new(
            self.channel(s.r, d.r),
            self.channel(s.g, d.g),
            self.channel(s.b, d.b),
        )
    }
}

/// What a span is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'s> {
    /// One color.
    Solid(Color),
    /// Pixels of a byte-aligned format, starting with the span's first pixel
    /// (`Rgb565`, `Rgb565Swapped`, `Rgb888`, `Xrgb8888`, `Argb8888`, `L8`).
    Pixels {
        /// Pixel bytes.
        data: &'s [u8],
        /// Their format.
        format: ColorFormat,
    },
    /// Straight-alpha `Argb8888` pixels (memory `B, G, R, A`), starting with the span's first pixel.
    Argb(&'s [u8]),
}

impl<'s> Source<'s> {
    /// The source advanced by `px` pixels.
    #[must_use]
    pub fn offset(self, px: usize) -> Source<'s> {
        match self {
            Source::Solid(c) => Source::Solid(c),
            Source::Pixels { data, format } => {
                let skip = (px * usize::from(format.bpp()) / 8).min(data.len());
                Source::Pixels {
                    data: &data[skip..],
                    format,
                }
            }
            Source::Argb(d) => Source::Argb(&d[(px * 4).min(d.len())..]),
        }
    }

    /// How many pixels the source can provide (`usize::MAX` for a solid color).
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Source::Solid(_) => usize::MAX,
            Source::Pixels { data, format } => (data.len() * 8)
                .checked_div(usize::from(format.bpp()))
                .unwrap_or(0),
            Source::Argb(d) => d.len() / 4,
        }
    }

    /// Whether the source provides no pixels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Effective alpha of `mask` value `m` at opacity `opa` (`opa == 255` → `m` exactly).
#[inline(always)]
fn eff(m: u8, opa: u8) -> u8 {
    if opa == 255 {
        m
    } else {
        udiv255(u32::from(m) * u32::from(opa)) as u8
    }
}

/// Writes the raw pixel bytes `px` repeatedly over `dst` (doubling copies).
#[inline]
fn fill_raw(dst: &mut [u8], px: &[u8]) {
    let bytes = dst.len();
    if bytes == 0 {
        return;
    }
    if px.iter().all(|&b| b == px[0]) {
        dst.fill(px[0]);
        return;
    }
    let first = px.len().min(bytes);
    dst[..first].copy_from_slice(&px[..first]);
    let mut filled = first;
    while filled < bytes {
        let n = filled.min(bytes - filled);
        dst.copy_within(0..n, filled);
        filled += n;
    }
}

/// Composites `fg` with alpha `a` over the straight-alpha `Argb8888` pixel `px` (LVGL
/// `lv_color_32_32_mix` rules).
#[inline(always)]
fn put_argb(px: &mut [u8], fg: Color, a: u8, mode: BlendMode) {
    let da = px[3];
    let dc = Color::new(px[2], px[1], px[0]);
    let fg = if mode == BlendMode::Normal || da == 0 {
        fg
    } else {
        mode.color(fg, dc)
    };
    let (c, out_a) = if a >= Opa::MAX.0 {
        (fg, 255)
    } else if da <= Opa::MIN.0 {
        (fg, a)
    } else if da == 255 {
        (Color::mix(fg, dc, Opa(a)), 255)
    } else {
        let ra = 255 - udiv255(u32::from(255 - a) * u32::from(255 - da)) as u8;
        let ratio = (u32::from(a) * 255 / u32::from(ra)).min(255) as u8;
        (Color::mix(fg, dc, Opa(ratio)), ra)
    };
    px[0] = c.b;
    px[1] = c.g;
    px[2] = c.r;
    px[3] = out_a;
}

/// Mixes `fg` with alpha `a` into the pixel `px` of an alpha-less format `F`.
#[inline(always)]
fn put<F: PixelFormat>(px: &mut [u8], fg: Color, a: u8, mode: BlendMode) {
    if F::FORMAT == ColorFormat::Argb8888 {
        put_argb(px, fg, a, mode);
        return;
    }
    if mode == BlendMode::Normal && a >= Opa::MAX.0 {
        F::write(px, F::from_color(fg));
        return;
    }
    let bg = F::to_color(F::read(px));
    let fg = mode.color(fg, bg);
    let out = if a >= Opa::MAX.0 {
        fg
    } else {
        Color::mix(fg, bg, Opa(a))
    };
    F::write(px, F::from_color(out));
}

/// RGB565 mix with a pre-expanded foreground, bit-identical to `Rgb565::mix` (generic formula).
#[inline(always)]
fn mix565(fr: u32, fg: u32, fb: u32, bg: u16, a: u32) -> u16 {
    let r5 = u32::from(bg >> 11);
    let g6 = u32::from((bg >> 5) & 0x3F);
    let b5 = u32::from(bg & 0x1F);
    let (br, bgc, bb) = (
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    );
    let ia = 255 - a;
    let r = udiv255(fr * a + br * ia);
    let g = udiv255(fg * a + bgc * ia);
    let b = udiv255(fb * a + bb * ia);
    (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16
}

/// Solid color paths.
#[inline]
fn solid<F: PixelFormat>(dst: &mut [u8], n: usize, c: Color, mask: Option<&[u8]>, opa: u8, mode: BlendMode) {
    let raw = F::from_color(c);
    let dst = &mut dst[..n * F::BYTES];
    if mode == BlendMode::Normal && mask.is_none() && opa >= Opa::MAX.0 {
        let mut px = [0u8; 4];
        F::write(&mut px, raw);
        fill_raw(dst, &px[..F::BYTES]);
        return;
    }
    // The normative semantics: `F::mix(from_color(c), bg, a)`, i.e. the color quantized to the
    // destination format first.
    let fgc = F::to_color(raw);
    if mode == BlendMode::Normal
        && (F::FORMAT == ColorFormat::Rgb565 || F::FORMAT == ColorFormat::Rgb565Swapped)
    {
        let swapped = F::FORMAT == ColorFormat::Rgb565Swapped;
        let fg565 = fgc.to_rgb565();
        let (fr, fgn, fb) = (u32::from(fgc.r), u32::from(fgc.g), u32::from(fgc.b));
        let write = |px: &mut [u8], a: u8| {
            if a <= Opa::MIN.0 {
                return;
            }
            let v = if a >= Opa::MAX.0 {
                fg565
            } else {
                let bg = u16::from_le_bytes([px[0], px[1]]);
                let bg = if swapped { bg.swap_bytes() } else { bg };
                mix565(fr, fgn, fb, bg, u32::from(a))
            };
            let v = if swapped { v.swap_bytes() } else { v };
            px.copy_from_slice(&v.to_le_bytes());
        };
        match mask {
            None => dst.chunks_exact_mut(2).for_each(|px| write(px, opa)),
            Some(m) => dst
                .chunks_exact_mut(2)
                .zip(m)
                .for_each(|(px, &m)| write(px, eff(m, opa))),
        }
        return;
    }
    let write = |px: &mut [u8], a: u8| {
        if a > Opa::MIN.0 {
            put::<F>(px, fgc, a, mode);
        }
    };
    match mask {
        None => dst.chunks_exact_mut(F::BYTES).for_each(|px| write(px, opa)),
        Some(m) => dst
            .chunks_exact_mut(F::BYTES)
            .zip(m)
            .for_each(|(px, &m)| write(px, eff(m, opa))),
    }
}

/// `Argb8888` source pixels.
#[inline]
fn argb<F: PixelFormat>(dst: &mut [u8], n: usize, src: &[u8], mask: Option<&[u8]>, opa: u8, mode: BlendMode) {
    let dst = &mut dst[..n * F::BYTES];
    let src = &src[..n * 4];
    let each = |px: &mut [u8], s: &[u8], m: u8| {
        let a = eff(eff(s[3], m), opa);
        if a > Opa::MIN.0 {
            put::<F>(px, Color::new(s[2], s[1], s[0]), a, mode);
        }
    };
    match mask {
        None => dst
            .chunks_exact_mut(F::BYTES)
            .zip(src.chunks_exact(4))
            .for_each(|(px, s)| each(px, s, 255)),
        Some(m) => dst
            .chunks_exact_mut(F::BYTES)
            .zip(src.chunks_exact(4))
            .zip(m)
            .for_each(|((px, s), &m)| each(px, s, m)),
    }
}

/// Opaque source pixels of format `S`.
#[inline]
fn pixels<F: PixelFormat, S: PixelFormat>(
    dst: &mut [u8],
    n: usize,
    src: &[u8],
    mask: Option<&[u8]>,
    opa: u8,
    mode: BlendMode,
) {
    let dst = &mut dst[..n * F::BYTES];
    let src = &src[..n * S::BYTES];
    if S::FORMAT == F::FORMAT && mask.is_none() && opa >= Opa::MAX.0 && mode == BlendMode::Normal {
        dst.copy_from_slice(src);
        return;
    }
    let each = |px: &mut [u8], s: &[u8], a: u8| {
        if a > Opa::MIN.0 {
            put::<F>(px, S::to_color(S::read(s)), a, mode);
        }
    };
    match mask {
        None => dst
            .chunks_exact_mut(F::BYTES)
            .zip(src.chunks_exact(S::BYTES))
            .for_each(|(px, s)| each(px, s, opa)),
        Some(m) => dst
            .chunks_exact_mut(F::BYTES)
            .zip(src.chunks_exact(S::BYTES))
            .zip(m)
            .for_each(|((px, s), &m)| each(px, s, eff(m, opa))),
    }
}

static WARN_SOURCE: AtomicBool = AtomicBool::new(false);

#[cold]
fn unsupported_source(format: ColorFormat) {
    if warn_once(&WARN_SOURCE) {
        twine_core::warn!(target: "twine::render", "blend: unsupported source format {}; nothing drawn", format);
    }
}

/// Blends `n` pixels of `src` onto `dst` (row bytes of format `F`).
///
/// `mask`: per-pixel coverage (at least `n` values) or `None` (full coverage). `n` is clamped
/// to what `dst`, `src` and `mask` hold.
///
/// The effective alpha of a pixel is `udiv255(mask · opa)` (just `mask` at cover opacity),
/// multiplied by the source pixel's alpha; `a ≤ Opa::MIN` leaves the pixel, `a ≥ Opa::MAX`
/// replaces it. Paths, chosen once per call: solid + cover + no mask + [`BlendMode::Normal`] →
/// raw fill; solid → per-pixel mix (the color quantized to `F` first, like `F::mix`); pixels of
/// the same alpha-less format + cover + no mask + Normal → `copy_from_slice`; otherwise the
/// source pixel is converted to a [`Color`] and mixed. An `Argb8888` destination is
/// composited with the straight-alpha "over" operator.
///
/// ```
/// use twine_core::{Color, Opa, PixelFormat, color::Rgb565};
/// use twine_render::{BlendMode, Source, blend_span};
///
/// let mut row = [0u8; 2 * 3];
/// blend_span::<Rgb565>(&mut row, 3, Source::Solid(Color::WHITE), Some(&[255, 0, 255]), Opa::COVER, BlendMode::Normal);
/// assert_eq!(row, [0xFF, 0xFF, 0, 0, 0xFF, 0xFF]);
/// ```
pub fn blend_span<F: PixelFormat>(
    dst: &mut [u8],
    n: usize,
    src: Source<'_>,
    mask: Option<&[u8]>,
    opa: Opa,
    mode: BlendMode,
) {
    if opa.is_transparent() {
        return;
    }
    let opa = if opa.is_cover() { 255 } else { opa.0 };
    let mut n = n.min(dst.len() / F::BYTES).min(src.len());
    if let Some(m) = mask {
        n = n.min(m.len());
    }
    if n == 0 {
        return;
    }
    match src {
        Source::Solid(c) => solid::<F>(dst, n, c, mask, opa, mode),
        Source::Argb(s) => argb::<F>(dst, n, s, mask, opa, mode),
        Source::Pixels { data, format } => match format {
            ColorFormat::Rgb565 => pixels::<F, Rgb565>(dst, n, data, mask, opa, mode),
            ColorFormat::Rgb565Swapped => pixels::<F, Rgb565Swapped>(dst, n, data, mask, opa, mode),
            ColorFormat::Rgb888 => pixels::<F, Rgb888>(dst, n, data, mask, opa, mode),
            ColorFormat::Xrgb8888 => pixels::<F, Xrgb8888>(dst, n, data, mask, opa, mode),
            ColorFormat::L8 => pixels::<F, L8>(dst, n, data, mask, opa, mode),
            ColorFormat::Argb8888 => argb::<F>(dst, n, data, mask, opa, mode),
            other => unsupported_source(other),
        },
    }
}

/// Color and alpha of pixel `i` of `src` (slow, per-pixel dispatch; used for `I1`).
#[cfg(feature = "color-i1")]
fn source_px(src: Source<'_>, i: usize) -> Option<(Color, u8)> {
    match src {
        Source::Solid(c) => Some((c, 255)),
        Source::Argb(d) => d
            .get(i * 4..i * 4 + 4)
            .map(|s| (Color::new(s[2], s[1], s[0]), s[3])),
        Source::Pixels { data, format } => {
            fn rd<S: PixelFormat>(d: &[u8], i: usize) -> Option<(Color, u8)> {
                d.get(i * S::BYTES..(i + 1) * S::BYTES)
                    .map(|b| (S::to_color(S::read(b)), 255))
            }
            match format {
                ColorFormat::Rgb565 => rd::<Rgb565>(data, i),
                ColorFormat::Rgb565Swapped => rd::<Rgb565Swapped>(data, i),
                ColorFormat::Rgb888 => rd::<Rgb888>(data, i),
                ColorFormat::Xrgb8888 => rd::<Xrgb8888>(data, i),
                ColorFormat::L8 => rd::<L8>(data, i),
                ColorFormat::Argb8888 => data
                    .get(i * 4..i * 4 + 4)
                    .map(|s| (Color::new(s[2], s[1], s[0]), s[3])),
                other => {
                    unsupported_source(other);
                    None
                }
            }
        }
    }
}

/// Blends onto 1-bit pixels (`1` = white): the pixel becomes white when the blended color's
/// luminance is at least 128.
#[cfg(feature = "color-i1")]
fn blend_span_i1(
    row: &mut [u8],
    bit0: usize,
    n: usize,
    src: Source<'_>,
    mask: Option<&[u8]>,
    opa: Opa,
    mode: BlendMode,
) {
    use twine_core::color::I1;
    if opa.is_transparent() {
        return;
    }
    let opa = if opa.is_cover() { 255 } else { opa.0 };
    for i in 0..n {
        let m = match mask {
            Some(m) => match m.get(i) {
                Some(&v) => v,
                None => break,
            },
            None => 255,
        };
        let Some((fg, sa)) = source_px(src, i) else { break };
        let a = eff(eff(sa, m), opa);
        if a <= Opa::MIN.0 {
            continue;
        }
        let x = bit0 + i;
        let bg = if I1::get(row, x) {
            Color::WHITE
        } else {
            Color::BLACK
        };
        let fg = mode.color(fg, bg);
        let out = if a >= Opa::MAX.0 {
            fg
        } else {
            Color::mix(fg, bg, Opa(a))
        };
        I1::set(row, x, out.luminance() >= 128);
    }
}

/// Blends pixels `[x0, x1)` of row `y` of `buf` (already clipped to the buffer), dispatching
/// on the buffer's runtime format.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_row(
    buf: &mut DrawBuf<'_>,
    y: i32,
    x0: i32,
    x1: i32,
    src: Source<'_>,
    mask: Option<&[u8]>,
    opa: Opa,
    mode: BlendMode,
) {
    if x1 <= x0 {
        return;
    }
    let n = (x1 - x0) as usize;
    let format = buf.format();
    if format == ColorFormat::I1 {
        #[cfg(feature = "color-i1")]
        {
            let bit0 = (x0 - buf.area().x0) as usize % 8;
            let row = buf.row_mut(y, x0, x1);
            blend_span_i1(row, bit0, n, src, mask, opa, mode);
        }
        #[cfg(not(feature = "color-i1"))]
        {
            let _ = (y, n, src, mask, opa, mode);
            crate::dispatch::format_disabled(format);
        }
        return;
    }
    let row = buf.row_mut(y, x0, x1);
    dispatch_format!(format, F => blend_span::<F>(row, n, src, mask, opa, mode));
}

/// Mixes `fg` over `bg` with alpha `a` and blend `mode`, like [`blend_span`] does for one pixel
/// of an alpha-less destination (without quantization).
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_render::{BlendMode, mix_color};
/// assert_eq!(mix_color(Color::WHITE, Color::BLACK, Opa::COVER, BlendMode::Normal), Color::WHITE);
/// assert_eq!(mix_color(Color::RED, Color::BLUE, Opa::COVER, BlendMode::Additive), Color::MAGENTA);
/// ```
#[must_use]
pub fn mix_color(fg: Color, bg: Color, a: Opa, mode: BlendMode) -> Color {
    let fg = mode.color(fg, bg);
    if a.is_cover() {
        fg
    } else if a.is_transparent() {
        bg
    } else {
        Color::new(
            mix_channel(fg.r, bg.r, a.0),
            mix_channel(fg.g, bg.g, a.0),
            mix_channel(fg.b, bg.b, a.0),
        )
    }
}

/// The raw `Argb8888` value of `c` with alpha `opa` (helper for building layer rows).
#[inline]
#[must_use]
pub fn argb_raw(c: Color, opa: Opa) -> u32 {
    Argb8888::from_color_opa(c, opa)
}

#[cfg(test)]
mod tests {
    use super::*;
    use twine_core::XorShift32;

    #[test]
    fn mix565_matches_generic() {
        let mut rng = XorShift32::new(7);
        for _ in 0..200_000 {
            let fg = Rgb565::to_color(rng.next_u32() as u16);
            let bg = rng.next_u32() as u16;
            let a = rng.next_u32() as u8;
            let expect = Rgb565::mix(Rgb565::from_color(fg), bg, Opa(a));
            let got = mix565(
                u32::from(fg.r),
                u32::from(fg.g),
                u32::from(fg.b),
                bg,
                u32::from(a),
            );
            assert_eq!(got, expect);
        }
    }

    #[test]
    fn fill_raw_patterns() {
        let mut d = [0u8; 9];
        fill_raw(&mut d, &[1, 2, 3]);
        assert_eq!(d, [1, 2, 3, 1, 2, 3, 1, 2, 3]);
        let mut d = [0u8; 7];
        fill_raw(&mut d, &[1, 2]);
        assert_eq!(d, [1, 2, 1, 2, 1, 2, 1]);
        fill_raw(&mut d, &[5, 5]);
        assert_eq!(d, [5; 7]);
    }

    #[test]
    fn argb_over_transparent_keeps_alpha() {
        let mut px = [0u8; 4];
        put_argb(&mut px, Color::new(10, 20, 30), 100, BlendMode::Normal);
        assert_eq!(px, [30, 20, 10, 100]);
        put_argb(&mut px, Color::new(200, 200, 200), 100, BlendMode::Normal);
        // result alpha = 255 - (155 * 155 / 255) = 161
        assert_eq!(px[3], 161);
    }
}
