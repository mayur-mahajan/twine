//! Rasterization with fontdue: coverage grids, trimming, subpixel resampling and quantization.

#![allow(clippy::cast_precision_loss)] // pixel sizes and advances are small

use twine_text::Subpx;

/// A coverage grid in glyph space: columns `[x0, x0 + w)` right of the pen, rows `[y0, y0 + h)`
/// **up** from the baseline; `data` is row-major, top row first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cov {
    /// Left edge.
    pub x0: i32,
    /// Bottom edge (above the baseline).
    pub y0: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
    /// Coverage values, top row first.
    pub data: Vec<u8>,
}

impl Cov {
    /// Value at glyph-space `(x, y)` (0 outside).
    #[must_use]
    pub fn at(&self, x: i32, y: i32) -> u8 {
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
    pub fn trim(&self, step_x: i32, step_y: i32) -> Cov {
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
    pub fn turn_cw(&self) -> Cov {
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
    pub fn shifted_y(mut self, dy: i32) -> Cov {
        self.y0 += dy;
        self
    }
}

/// Rasterizes `ch` at `px` with fontdue; returns the coverage and the advance in pixels.
#[must_use]
pub fn raster(font: &fontdue::Font, ch: char, px: f32) -> (Cov, f32) {
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

fn div_floor(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

fn div_ceil(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
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
pub fn subpixel(c3: &Cov, mode: Subpx) -> Cov {
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
            let x0 = div_floor(c3.x0 - 2, 3) * 3;
            let x1 = div_ceil(c3.x0 + c3.w + 2, 3) * 3;
            let y0 = div_floor(c3.y0, 3);
            let y1 = div_ceil(c3.y0 + c3.h, 3);
            Cov::build(x0, x1, y0, y1, |x, y| filt(x, y, true)).trim(3, 1)
        }
        Subpx::Ver => {
            let y0 = div_floor(c3.y0 - 2, 3) * 3;
            let y1 = div_ceil(c3.y0 + c3.h + 2, 3) * 3;
            let x0 = div_floor(c3.x0, 3);
            let x1 = div_ceil(c3.x0 + c3.w, 3);
            Cov::build(x0, x1, y0, y1, |x, y| filt(y, x, false)).trim(1, 3)
        }
    }
}

/// Quantizes A8 coverage to `bpp` bits: `q = (v × max + 127) / 255`, `max = 2^bpp − 1`.
#[must_use]
pub fn quantize(v: u8, bpp: u8) -> u8 {
    let max = (1u32 << bpp) - 1;
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
        assert_eq!(quantize(0, 4), 0);
        assert_eq!(quantize(255, 4), 15);
        assert_eq!(quantize(8, 4), 0);
        assert_eq!(quantize(9, 4), 1);
        assert_eq!(quantize(128, 1), 1);
        assert_eq!(quantize(127, 1), 0);
        assert_eq!(quantize(200, 8), 200);
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
