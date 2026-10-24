//! Rasterization: grid-fitting (hinting) with `skrifa`'s auto-hinter, coverage grids with
//! `fontdue` (unhinted) or `ab_glyph_rasterizer` (hinted outlines), trimming, subpixel
//! resampling and quantization.

#![allow(clippy::cast_precision_loss)] // pixel sizes and advances are small

use std::fmt;
use std::str::FromStr;

use ab_glyph_rasterizer::{Point, point};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{
    DrawSettings, Engine, HintingInstance, HintingOptions, OutlineGlyphCollection, OutlinePen, SmoothMode,
    Target,
};
use skrifa::{FontRef, MetadataProvider};
use twine_text::{Bpp, Subpx};

/// Grid-fitting of glyph outlines before they are rasterized (the `FreeType` auto-hinter, as
/// implemented by the pure-Rust `skrifa` crate; the font's own TrueType instructions are not
/// used, so every font is hinted the same way).
///
/// Hinting moves stems and edges onto whole pixels, so small text has one-pixel stems at full
/// ink instead of two half-inked columns: crisper on low-DPI panels (ILI9341-class 2.4–3.2″
/// QVGA). It runs on the host when the font is generated; nothing is linked into the firmware
/// and drawing costs the same.
///
/// | Mode | Moves stems | Advances | Use |
/// |------|-------------|----------|-----|
/// | [`None`](Hinting::None) | no | unhinted (1/16 px) | exact outlines, large sizes, LVGL-identical glyphs |
/// | [`Light`](Hinting::Light) | vertically (horizontal stems, x-height, baseline) | unhinted (1/16 px) | keeps the layout of unhinted text |
/// | [`Full`](Hinting::Full) | in both directions | hinted (whole pixels) | the default: crispest small text |
///
/// Measured on Montserrat Medium 14 px: the stem of `l` covers two columns at 73 % / 62 %
/// unhinted and with `Light` (light hinting does not move vertical stems), and one full column
/// with `Full` (27 % / 100 % / 9 %); full hinting widens a sample line from 322 to 329 px.
///
/// ```
/// use twine_build::font::Hinting;
///
/// assert_eq!(Hinting::default(), Hinting::Full);
/// assert_eq!("light".parse::<Hinting>(), Ok(Hinting::Light));
/// assert_eq!(Hinting::None.to_string(), "none");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Hinting {
    /// No grid-fitting: the outline as designed (the generator's output before R6.S03).
    None,
    /// Vertical-only grid-fitting (as `FT_LOAD_TARGET_LIGHT`): heights snap to the pixel grid, advances
    /// and horizontal positions stay unhinted.
    Light,
    /// Grid-fitting in both directions (as `FT_LOAD_TARGET_NORMAL`, or `_LCD` / `_LCD_V` for subpixel
    /// fonts) with hinted, whole-pixel advances (the default).
    #[default]
    Full,
}

impl Hinting {
    /// Every mode.
    pub const ALL: [Hinting; 3] = [Hinting::None, Hinting::Light, Hinting::Full];

    /// The mode's name as the `twine font --hinting` option and the generated header write it:
    /// `none`, `light` or `full`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Hinting::None => "none",
            Hinting::Light => "light",
            Hinting::Full => "full",
        }
    }
}

impl fmt::Display for Hinting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Hinting {
    type Err = String;

    /// Parses `none`, `light` or `full`.
    fn from_str(s: &str) -> Result<Hinting, String> {
        Hinting::ALL
            .into_iter()
            .find(|h| h.name() == s)
            .ok_or_else(|| format!("hinting must be none, light or full (got {s:?})"))
    }
}

/// The rasterizer versions the generated header records: a different version may produce
/// different coverage, so the header says which produced it.
pub(crate) const UNHINTED_RASTERIZER: &str = "fontdue 0.9";
/// See [`UNHINTED_RASTERIZER`].
pub(crate) const HINTED_RASTERIZER: &str = "skrifa 0.49 auto-hinter, ab_glyph_rasterizer 0.1";

/// A font file prepared for rasterization: `fontdue` for unhinted glyphs and metrics, and the
/// auto-hinter when the font is hinted.
pub(crate) struct Face<'a> {
    /// The parsed font (metrics, unhinted glyphs).
    pub(crate) fd: &'a fontdue::Font,
    hinter: Option<Hinter<'a>>,
}

/// The auto-hinter of one font at one size.
struct Hinter<'a> {
    font: FontRef<'a>,
    outlines: OutlineGlyphCollection<'a>,
    instance: HintingInstance,
    /// `Full`: the hinted advance replaces the unhinted one.
    hinted_advances: bool,
}

impl<'a> Face<'a> {
    /// Prepares `bytes` (parsed as `fd`) for `raster`'s size, subpixel layout and hinting.
    ///
    /// # Errors
    /// When the auto-hinter cannot read the font.
    pub(crate) fn new(
        bytes: &'a [u8],
        fd: &'a fontdue::Font,
        raster: &Rasterizer,
    ) -> Result<Face<'a>, String> {
        let mode = match (raster.hinting, raster.subpx) {
            (Hinting::None, _) => return Ok(Face { fd, hinter: None }),
            (Hinting::Light, _) => SmoothMode::Light,
            (Hinting::Full, Subpx::None) => SmoothMode::Normal,
            (Hinting::Full, Subpx::Hor) => SmoothMode::Lcd,
            (Hinting::Full, Subpx::Ver) => SmoothMode::VerticalLcd,
        };
        let font = FontRef::new(bytes).map_err(|e| format!("the auto-hinter cannot read the font: {e}"))?;
        let outlines = font.outline_glyphs();
        let options = HintingOptions {
            engine: Engine::Auto(None),
            target: Target::Smooth {
                mode,
                symmetric_rendering: true,
                preserve_linear_metrics: raster.hinting == Hinting::Light,
            },
        };
        let instance = HintingInstance::new(
            &outlines,
            Size::new(f32::from(raster.size)),
            LocationRef::default(),
            options,
        )
        .map_err(|e| format!("the auto-hinter cannot hint the font: {e}"))?;
        Ok(Face {
            fd,
            hinter: Some(Hinter {
                font,
                outlines,
                instance,
                hinted_advances: raster.hinting == Hinting::Full,
            }),
        })
    }
}

/// How glyph outlines become coverage grids: the pixel size, the subpixel layout and the
/// hinting of one generated font. Every glyph of the font (main and symbol fonts) goes through
/// [`Rasterizer::glyph`], the one place where outline → coverage options apply.
pub(crate) struct Rasterizer {
    size: u16,
    subpx: Subpx,
    hinting: Hinting,
}

impl Rasterizer {
    pub(crate) fn new(size: u16, subpx: Subpx, hinting: Hinting) -> Rasterizer {
        Rasterizer { size, subpx, hinting }
    }

    /// Rasterizes `cp` of `face` (turned 90° clockwise when `turn`): its coverage, trimmed to
    /// the inked box (subpixel grids keep their alignment), and its advance in 1/16 px.
    ///
    /// Unhinted glyphs are rasterized by `fontdue` (subpixel glyphs at 3× size). Hinted
    /// outlines are grid-fitted at the font's size and then rasterized (subpixel glyphs from
    /// the same outline scaled 3×, so the hinting of the real pixel grid is kept); `Full`
    /// takes the hinted advance.
    ///
    /// # Errors
    /// When the auto-hinter fails on the glyph.
    pub(crate) fn glyph(&self, face: &Face<'_>, cp: char, turn: bool) -> Result<(Cov, u16), String> {
        let px = f32::from(self.size);
        let scale = match self.subpx {
            Subpx::None => 1.0,
            Subpx::Hor | Subpx::Ver => 3.0,
        };
        let hinted = match &face.hinter {
            Some(h) => h.outline(cp)?.map(|o| (h, o)),
            None => None,
        };
        let (cov, adv) = if let Some((h, (cmds, hinted_adv))) = &hinted {
            let hinted_adv = *hinted_adv;
            let c = raster_path(cmds, scale);
            let c = if turn { c.turn_cw() } else { c };
            let unhinted = face.fd.metrics(cp, px).advance_width;
            let adv = if turn {
                px
            } else if h.hinted_advances {
                hinted_adv.unwrap_or(unhinted)
            } else {
                unhinted
            };
            (c, adv)
        } else {
            let (c, a) = raster(face.fd, cp, px * scale);
            if turn { (c.turn_cw(), px * scale) } else { (c, a) }
        };
        let (cov, adv) = match self.subpx {
            Subpx::None => (cov.trim(1, 1), adv),
            Subpx::Hor | Subpx::Ver if hinted.is_some() => (subpixel(&cov, self.subpx), adv),
            Subpx::Hor | Subpx::Ver => (subpixel(&cov, self.subpx), adv / 3.0),
        };
        let adv16 = (adv * 16.0).round().clamp(0.0, f32::from(u16::MAX)) as u16;
        Ok((cov, adv16))
    }
}

/// One outline command in pixels (y up): move, line, quadratic, cubic, close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A hinted outline (pixels, y up) and its hinted advance, when the hinter reports one.
type HintedOutline = (Vec<PathCmd>, Option<f32>);

/// Collects the commands of a drawn outline.
#[derive(Default)]
struct PathPen(Vec<PathCmd>);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(PathCmd::MoveTo(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(PathCmd::LineTo(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.push(PathCmd::QuadTo(cx0, cy0, x, y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push(PathCmd::CurveTo(cx0, cy0, cx1, cy1, x, y));
    }
    fn close(&mut self) {
        self.0.push(PathCmd::Close);
    }
}

impl Hinter<'_> {
    /// The hinted outline of `cp` in pixels and its hinted advance; `None` when the
    /// auto-hinter does not find the glyph (it is then rasterized unhinted, as `fontdue` reads
    /// it).
    fn outline(&self, cp: char) -> Result<Option<HintedOutline>, String> {
        let Some(glyph) = self.font.charmap().map(cp).and_then(|gid| self.outlines.get(gid)) else {
            return Ok(None);
        };
        let mut pen = PathPen::default();
        let metrics = glyph
            .draw(DrawSettings::hinted(&self.instance, false), &mut pen)
            .map_err(|e| format!("the auto-hinter failed on U+{:04X}: {e}", u32::from(cp)))?;
        Ok(Some((pen.0, metrics.advance_width)))
    }
}

/// Rasterizes an outline (pixels, y up) scaled by `scale` into a coverage grid over its
/// bounding box (whole pixels; [`Cov::trim`] removes empty borders).
pub(crate) fn raster_path(cmds: &[PathCmd], scale: f32) -> Cov {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut grow = |x: f32, y: f32| {
        let (x, y) = (x * scale, y * scale);
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    };
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => grow(x, y),
            PathCmd::QuadTo(a, b, x, y) => {
                grow(a, b);
                grow(x, y);
            }
            PathCmd::CurveTo(a, b, c, d, x, y) => {
                grow(a, b);
                grow(c, d);
                grow(x, y);
            }
            PathCmd::Close => {}
        }
    }
    let Some((minx, miny, maxx, maxy)) = bounds else {
        return Cov::default();
    };
    let (x0, y0) = (minx.floor() as i32, miny.floor() as i32);
    let (x1, y1) = (maxx.ceil() as i32, maxy.ceil() as i32);
    let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
    let mut r = ab_glyph_rasterizer::Rasterizer::new(w as usize, h as usize);
    // Grid space: x right from the box's left edge, y down from its top edge.
    let tp = |x: f32, y: f32| -> Point { point(x * scale - x0 as f32, y1 as f32 - y * scale) };
    let (mut cur, mut start) = (point(0.0, 0.0), point(0.0, 0.0));
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => {
                if cur != start {
                    r.draw_line(cur, start);
                }
                cur = tp(x, y);
                start = cur;
            }
            PathCmd::LineTo(x, y) => {
                let n = tp(x, y);
                r.draw_line(cur, n);
                cur = n;
            }
            PathCmd::QuadTo(a, b, x, y) => {
                let n = tp(x, y);
                r.draw_quad(cur, tp(a, b), n);
                cur = n;
            }
            PathCmd::CurveTo(a, b, c, d, x, y) => {
                let n = tp(x, y);
                r.draw_cubic(cur, tp(a, b), tp(c, d), n);
                cur = n;
            }
            PathCmd::Close => {
                if cur != start {
                    r.draw_line(cur, start);
                }
                cur = start;
            }
        }
    }
    if cur != start {
        r.draw_line(cur, start);
    }
    let mut data = vec![0u8; (w * h) as usize];
    r.for_each_pixel(|i, a| {
        if let Some(d) = data.get_mut(i) {
            *d = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    Cov { x0, y0, w, h, data }
}

/// A coverage grid in glyph space: columns `[x0, x0 + w)` right of the pen, rows `[y0, y0 + h)`
/// **up** from the baseline; `data` is row-major, top row first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cov {
    /// Left edge.
    pub(crate) x0: i32,
    /// Bottom edge (above the baseline).
    pub(crate) y0: i32,
    /// Width.
    pub(crate) w: i32,
    /// Height.
    pub(crate) h: i32,
    /// Coverage values, top row first.
    pub(crate) data: Vec<u8>,
}

impl Cov {
    /// Value at glyph-space `(x, y)` (0 outside).
    #[must_use]
    pub(crate) fn at(&self, x: i32, y: i32) -> u8 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            return 0;
        }
        let row = self.y0 + self.h - 1 - y;
        self.data[(row * self.w + (x - self.x0)) as usize]
    }

    /// Builds a grid over `[x0, x1) × [y0, y1)` from `f(x, y)`.
    fn build(x0: i32, x1: i32, y0: i32, y1: i32, mut f: impl FnMut(i32, i32) -> u8) -> Cov {
        let (w, h) = ((x1 - x0).max(0), (y1 - y0).max(0));
        let mut data = Vec::with_capacity((w * h) as usize);
        for y in (y0..y1).rev() {
            for x in x0..x1 {
                data.push(f(x, y));
            }
        }
        Cov { x0, y0, w, h, data }
    }

    /// Removes empty borders. `step_x`/`step_y` keep the box aligned to multiples of that many
    /// columns/rows (relative to the glyph origin) for subpixel grids.
    #[must_use]
    pub(crate) fn trim(&self, step_x: i32, step_y: i32) -> Cov {
        let col_empty = |x: i32| (self.y0..self.y0 + self.h).all(|y| self.at(x, y) == 0);
        let row_empty = |y: i32| (self.x0..self.x0 + self.w).all(|x| self.at(x, y) == 0);
        let (mut x0, mut x1) = (self.x0, self.x0 + self.w);
        let (mut y0, mut y1) = (self.y0, self.y0 + self.h);
        while x0 < x1 && (x0..x0 + step_x).all(col_empty) {
            x0 += step_x;
        }
        while x1 > x0 && (x1 - step_x..x1).all(col_empty) {
            x1 -= step_x;
        }
        while y0 < y1 && (y0..y0 + step_y).all(row_empty) {
            y0 += step_y;
        }
        while y1 > y0 && (y1 - step_y..y1).all(row_empty) {
            y1 -= step_y;
        }
        if x0 >= x1 || y0 >= y1 {
            return Cov::default();
        }
        Cov::build(x0, x1, y0, y1, |x, y| self.at(x, y))
    }

    /// The grid turned 90° clockwise around its bottom-left corner (the box keeps its
    /// bottom-left position; width and height swap).
    #[must_use]
    pub(crate) fn turn_cw(&self) -> Cov {
        let (w, h) = (self.h, self.w);
        let mut data = Vec::with_capacity(self.data.len());
        for r in 0..h {
            for c in 0..w {
                // New (c, r) comes from old row (h_old - 1 - c) from the top, column r.
                let old_row = self.h - 1 - c;
                data.push(self.data[(old_row * self.w + r) as usize]);
            }
        }
        Cov {
            x0: self.x0,
            y0: self.y0,
            w,
            h,
            data,
        }
    }

    /// Moves the grid vertically by `dy`.
    #[must_use]
    pub(crate) fn shifted_y(mut self, dy: i32) -> Cov {
        self.y0 += dy;
        self
    }
}

/// Rasterizes `ch` at `px` with fontdue; returns the coverage and the advance in pixels.
#[must_use]
pub(crate) fn raster(font: &fontdue::Font, ch: char, px: f32) -> (Cov, f32) {
    let (m, bmp) = font.rasterize(ch, px);
    let cov = Cov {
        x0: m.xmin,
        y0: m.ymin,
        w: m.width as i32,
        h: m.height as i32,
        data: bmp,
    };
    (cov, m.advance_width)
}

/// The subpixel factor: subpixel glyphs are rasterized at 3× size.
const SUB: i32 = 3;

/// `⌊a / 3⌋`.
fn div_floor3(a: i32) -> i32 {
    a.div_euclid(SUB)
}

/// `⌈a / 3⌉`.
fn div_ceil3(a: i32) -> i32 {
    -(-a).div_euclid(SUB)
}

/// LVGL's 5-tap FIR filter `[1, 2, 3, 2, 1] / 9` (reduces color fringes).
const FIR: [u32; 5] = [1, 2, 3, 2, 1];

/// Resamples a coverage grid rasterized at **3×** size into a subpixel grid:
/// - [`Subpx::Hor`]: rows are averaged in groups of 3 (whole pixels), columns stay subpixels
///   and are filtered horizontally with LVGL's `[1, 2, 3, 2, 1] / 9` FIR filter; `x0` and `w` are multiples of 3.
/// - [`Subpx::Ver`]: the transposed operation; `y0` and `h` are multiples of 3.
///
/// fontdue cannot scale one axis only, so the glyph is rasterized at 3× size and the other
/// axis is averaged back down.
#[must_use]
pub(crate) fn subpixel(c3: &Cov, mode: Subpx) -> Cov {
    let pre = |sx: i32, sy: i32, hor: bool| -> u32 {
        // Sum of 3 samples across the whole-pixel axis.
        (0..3)
            .map(|r| {
                if hor {
                    u32::from(c3.at(sx, sy * 3 + r))
                } else {
                    u32::from(c3.at(sx * 3 + r, sy))
                }
            })
            .sum()
    };
    let filt = |s: i32, p: i32, hor: bool| -> u8 {
        let mut acc = 0u32;
        for (k, wgt) in FIR.iter().enumerate() {
            let o = s + k as i32 - 2;
            acc += wgt * if hor { pre(o, p, true) } else { pre(p, o, false) };
        }
        // acc / 3 (row average) / 9 (FIR), rounded.
        ((acc + 13) / 27).min(255) as u8
    };
    match mode {
        Subpx::None => c3.clone(),
        Subpx::Hor => {
            let x0 = div_floor3(c3.x0 - 2) * 3;
            let x1 = div_ceil3(c3.x0 + c3.w + 2) * 3;
            let y0 = div_floor3(c3.y0);
            let y1 = div_ceil3(c3.y0 + c3.h);
            Cov::build(x0, x1, y0, y1, |x, y| filt(x, y, true)).trim(3, 1)
        }
        Subpx::Ver => {
            let y0 = div_floor3(c3.y0 - 2) * 3;
            let y1 = div_ceil3(c3.y0 + c3.h + 2) * 3;
            let x0 = div_floor3(c3.x0);
            let x1 = div_ceil3(c3.x0 + c3.w);
            Cov::build(x0, x1, y0, y1, |x, y| filt(y, x, false)).trim(1, 3)
        }
    }
}

/// Quantizes A8 coverage to `bpp`: `q = (v × max + 127) / 255`, `max = 2^bpp − 1`.
pub(crate) fn quantize(v: u8, bpp: Bpp) -> u8 {
    let max = u32::from(bpp.max_value());
    ((u32::from(v) * max + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_removes_empty_borders() {
        let c = Cov {
            x0: -1,
            y0: -1,
            w: 3,
            h: 3,
            data: vec![0, 0, 0, 0, 9, 0, 0, 0, 0],
        };
        let t = c.trim(1, 1);
        assert_eq!((t.x0, t.y0, t.w, t.h, t.data), (0, 0, 1, 1, vec![9]));
        assert_eq!(
            Cov {
                w: 2,
                h: 1,
                data: vec![0, 0],
                ..Cov::default()
            }
            .trim(1, 1),
            Cov::default()
        );
    }

    #[test]
    fn turn_cw_rotates() {
        // 2 wide × 3 tall, top row first: [1 2 / 3 4 / 5 6] → 3 wide × 2 tall [5 3 1 / 6 4 2].
        let c = Cov {
            x0: 0,
            y0: 0,
            w: 2,
            h: 3,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        let t = c.turn_cw();
        assert_eq!((t.w, t.h), (3, 2));
        assert_eq!(t.data, vec![5, 3, 1, 6, 4, 2]);
    }

    #[test]
    fn quantize_rounds() {
        assert_eq!(quantize(0, Bpp::Four), 0);
        assert_eq!(quantize(255, Bpp::Four), 15);
        assert_eq!(quantize(8, Bpp::Four), 0);
        assert_eq!(quantize(9, Bpp::Four), 1);
        assert_eq!(quantize(128, Bpp::One), 1);
        assert_eq!(quantize(127, Bpp::One), 0);
        assert_eq!(quantize(200, Bpp::Eight), 200);
        assert_eq!(quantize(255, Bpp::Two), 3);
    }

    #[test]
    fn subpixel_hor_is_aligned_and_filtered() {
        // A 3×3 fully covered block at the origin (3× raster) = one pixel.
        let c3 = Cov {
            x0: 0,
            y0: 0,
            w: 3,
            h: 3,
            data: vec![255; 9],
        };
        let s = subpixel(&c3, Subpx::Hor);
        assert_eq!(s.x0 % 3, 0);
        assert_eq!(s.w % 3, 0);
        assert_eq!(s.h, 1);
        // The filter spreads one pixel's coverage over its neighbours, symmetric.
        let row: Vec<u8> = (s.x0..s.x0 + s.w).map(|x| s.at(x, 0)).collect();
        assert_eq!(s.x0, -3);
        assert_eq!(row, vec![0, 28, 85, 170, 198, 170, 85, 28, 0]);
    }
}
