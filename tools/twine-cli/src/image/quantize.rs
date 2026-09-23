//! Palette quantization: deterministic median cut over RGBA colors, nearest-color mapping and
//! optional Floyd–Steinberg error diffusion.

use std::collections::{BTreeMap, HashMap};

/// An RGBA color.
pub type Rgba = [u8; 4];

/// A box of the median cut: distinct colors with their pixel counts.
#[derive(Debug, Clone)]
struct CutBox {
    colors: Vec<(Rgba, u32)>,
}

impl CutBox {
    /// `(channel, range)` of the widest channel (lowest channel index on ties).
    fn widest(&self) -> (usize, u8) {
        let mut best = (0, 0);
        for c in 0..4 {
            let (lo, hi) = self
                .colors
                .iter()
                .fold((255u8, 0u8), |(lo, hi), (p, _)| (lo.min(p[c]), hi.max(p[c])));
            let range = hi.saturating_sub(lo);
            if range > best.1 {
                best = (c, range);
            }
        }
        best
    }

    fn count(&self) -> u64 {
        self.colors.iter().map(|&(_, n)| u64::from(n)).sum()
    }

    /// Pixel-count weighted mean color (rounded).
    fn mean(&self) -> Rgba {
        let total = self.count().max(1);
        let mut out = [0u8; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let sum: u64 = self
                .colors
                .iter()
                .map(|&(p, n)| u64::from(p[c]) * u64::from(n))
                .sum();
            *o = ((sum + total / 2) / total) as u8;
        }
        out
    }

    /// Splits at the pixel-weighted median of `channel` (colors sorted by that channel, then by
    /// the whole color, so the result does not depend on input order).
    fn split(mut self, channel: usize) -> (CutBox, CutBox) {
        self.colors
            .sort_by(|a, b| a.0[channel].cmp(&b.0[channel]).then(a.0.cmp(&b.0)));
        let half = self.count() / 2;
        let mut acc = 0u64;
        let mut at = 1;
        for (i, &(_, n)) in self.colors.iter().enumerate() {
            acc += u64::from(n);
            if acc >= half {
                at = i + 1;
                break;
            }
        }
        let at = at.clamp(1, self.colors.len() - 1);
        let right = self.colors.split_off(at);
        (self, CutBox { colors: right })
    }
}

/// A palette of at most `n` colors for `pixels` (median cut). When the image has at most `n`
/// distinct colors the palette holds exactly those. The palette is sorted and padded with
/// transparent black to `n` entries; the result does not depend on pixel order.
#[must_use]
pub fn median_cut(pixels: &[Rgba], n: usize) -> Vec<Rgba> {
    let mut hist: BTreeMap<Rgba, u32> = BTreeMap::new();
    for &p in pixels {
        *hist.entry(p).or_default() += 1;
    }
    let mut palette: Vec<Rgba> = if hist.len() <= n {
        hist.keys().copied().collect()
    } else {
        let mut boxes = vec![CutBox {
            colors: hist.into_iter().collect(),
        }];
        while boxes.len() < n {
            // The box with the widest channel range (larger pixel count, then lower index,
            // on ties).
            let Some((i, ch)) = boxes
                .iter()
                .enumerate()
                .filter(|(_, b)| b.colors.len() > 1)
                .max_by(|(ia, a), (ib, b)| {
                    let (wa, wb) = (a.widest(), b.widest());
                    wa.1.cmp(&wb.1).then(a.count().cmp(&b.count())).then(ib.cmp(ia))
                })
                .map(|(i, b)| (i, b.widest().0))
            else {
                break;
            };
            let b = boxes.remove(i);
            let (l, r) = b.split(ch);
            boxes.push(l);
            boxes.push(r);
        }
        boxes.iter().map(CutBox::mean).collect()
    };
    palette.sort_unstable();
    palette.dedup();
    palette.resize(n, [0, 0, 0, 0]);
    palette
}

fn dist(a: [i32; 4], b: Rgba) -> i32 {
    (0..4).map(|c| (a[c] - i32::from(b[c])).pow(2)).sum()
}

/// Index of the palette entry nearest to `c` (lowest index on ties).
fn nearest(palette: &[Rgba], c: [i32; 4]) -> u8 {
    let mut best = (i32::MAX, 0usize);
    for (i, &p) in palette.iter().enumerate() {
        let d = dist(c, p);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1 as u8
}

/// Maps every pixel to its nearest palette index, optionally with Floyd–Steinberg dithering
/// (errors of all four channels are diffused).
#[must_use]
pub fn map_to_palette(pixels: &[Rgba], w: usize, palette: &[Rgba], dither: bool) -> Vec<u8> {
    if !dither {
        let mut memo: HashMap<Rgba, u8> = HashMap::new();
        return pixels
            .iter()
            .map(|&p| {
                *memo
                    .entry(p)
                    .or_insert_with(|| nearest(palette, p.map(i32::from)))
            })
            .collect();
    }
    let h = pixels.len() / w.max(1);
    let mut err = vec![[0i32; 4]; w * h];
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let want: [i32; 4] =
                core::array::from_fn(|c| (i32::from(pixels[i][c]) + err[i][c] / 16).clamp(0, 255));
            let k = nearest(palette, want);
            out[i] = k;
            let got = palette[usize::from(k)];
            let e: [i32; 4] = core::array::from_fn(|c| want[c] - i32::from(got[c]));
            let mut spread = |xx: usize, yy: usize, f: i32| {
                if xx < w && yy < h {
                    for c in 0..4 {
                        err[yy * w + xx][c] += e[c] * f;
                    }
                }
            };
            spread(x + 1, y, 7);
            if x > 0 {
                spread(x - 1, y + 1, 3);
            }
            spread(x, y + 1, 5);
            spread(x + 1, y + 1, 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_colors_are_kept_exactly() {
        let px = [[1, 2, 3, 255], [9, 9, 9, 0], [1, 2, 3, 255]];
        let pal = median_cut(&px, 4);
        assert_eq!(pal, [[1, 2, 3, 255], [9, 9, 9, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert_eq!(map_to_palette(&px, 3, &pal, false), [0, 1, 0]);
    }

    #[test]
    fn gradient_reduces_to_n_colors() {
        let px: Vec<Rgba> = (0..=255u8).map(|v| [v, v, v, 255]).collect();
        let pal = median_cut(&px, 16);
        assert_eq!(pal.len(), 16);
        let idx = map_to_palette(&px, 256, &pal, false);
        let worst = px
            .iter()
            .zip(&idx)
            .map(|(p, &i)| p[0].abs_diff(pal[usize::from(i)][0]))
            .max()
            .unwrap();
        assert!(worst <= 10, "{worst}");
    }
}
