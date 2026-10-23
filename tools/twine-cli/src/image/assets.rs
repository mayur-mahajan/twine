//! Procedurally drawn sample images (`cargo xtask gen-assets`), so the repository needs no
//! third-party artwork: the Twine logo (the Rope mark of `assets/logo/`, redrawn at 64 × 64
//! from its geometry) and the image gallery's sample files.

#![allow(clippy::cast_precision_loss)] // small image coordinates

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};

use super::Result;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// The Rope mark (the artwork in `assets/logo/`), in its 64-unit design box: three strands
// plaited on three lanes, then opening to the right into three rays.
/// Distance between the plait's lanes (centered on y = 32).
const LANE: f32 = 9.5;
/// Length of one crossing (one strand moving from an outer lane to the middle).
const STEP: f32 = 11.0;
/// Start of the strands, and the straight lead-in before the first crossing.
const X0: f32 = 4.0;
const LEAD: f32 = 4.0;
/// Length of the S-curve from each lane to its ray, the rays' end, and the rays' heights.
const EXIT: f32 = 16.0;
const X1: f32 = 60.0;
const RAYS: [f32; 3] = [11.0, 32.0, 53.0];
/// Stroke width, and the transparent gap on each side of an over-strand at a crossing.
const STROKE: f32 = 4.4;
const GAP: f32 = 2.4;
/// Sample spacing of the strand polylines.
const RES: f32 = 0.25;
/// Strand colors (red, green, blue), by the lane each strand starts in.
const STRAND_COLORS: [[u8; 3]; 3] = [[0xEC, 0x40, 0x48], [0x22, 0xB8, 0x68], [0x30, 0x70, 0xF0]];
/// Side of the square the mark is framed in: its width (`X0..X1`, wider than its height) plus
/// the widest stroke (the gap) and a 3-unit margin on each side; the mark is centered on
/// (32, 32) already, so only scaling is needed (like `export_big.py` of the artwork).
const FRAME: f32 = (X1 - X0) + 2.0 * (STROKE / 2.0 + GAP + 3.0);

/// A point in the design box.
type Point = (f32, f32);

/// One stroke of the mark: a polyline drawn `half` wide on each side, in `paint` or cutting
/// to transparent (`None`). `flat` ends stop at the plane through each end point (butt caps);
/// otherwise the ends are round.
struct Stroke {
    pts: Vec<Point>,
    half: f32,
    paint: Option<[u8; 3]>,
    flat: bool,
}

impl Stroke {
    /// Whether `(x, y)` lies on the stroke.
    fn covers(&self, x: f32, y: f32) -> bool {
        let p = &self.pts;
        let n = p.len() - 1;
        let dot = |a: Point, b: Point| (x - a.0) * (b.0 - a.0) + (y - a.1) * (b.1 - a.1);
        if self.flat && (dot(p[0], p[1]) < 0.0 || dot(p[n], p[n - 1]) < 0.0) {
            return false;
        }
        p.windows(2).any(|s| {
            let (a, b) = (s[0], s[1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 { 0.0 } else { dot(a, b) / len2 }.clamp(0.0, 1.0);
            let (qx, qy) = (a.0 + t * dx - x, a.1 + t * dy - y);
            qx * qx + qy * qy <= self.half * self.half
        })
    }
}

/// The three strands as polylines (index = starting lane = color) and the crossings as
/// `(strand on top, crossing point)`.
fn plait() -> ([Vec<Point>; 3], Vec<(usize, Point)>) {
    let lanes = [32.0 - LANE, 32.0, 32.0 + LANE];
    // Cosine ease: crossings and exits are S-curves that leave and arrive level.
    let ease = |t: f32| (1.0 - (std::f32::consts::PI * t).cos()) / 2.0;
    let mut lane_of = [0, 1, 2];
    let mut x = X0 + LEAD;
    let mut paths = lanes.map(|y| vec![(X0, y), (x, y)]);
    let mut crossings = Vec::new();
    let n = (STEP / RES) as usize;
    // Three crossings: the outer strand (top, then bottom, then top) moves over the middle
    // one into the middle lane while that one moves under it to the outer lane.
    for outer in [0, 2, 0] {
        let over = lane_of.iter().position(|&l| l == outer).unwrap_or(0);
        let under = lane_of.iter().position(|&l| l == 1).unwrap_or(0);
        for (i, path) in paths.iter_mut().enumerate() {
            let y0 = lanes[lane_of[i]];
            let y1 = if i == over {
                lanes[1]
            } else if i == under {
                lanes[outer]
            } else {
                y0
            };
            for k in 1..=n {
                let t = k as f32 / n as f32;
                path.push((x + STEP * t, lerp(y0, y1, ease(t))));
            }
        }
        crossings.push((over, (x + STEP / 2.0, f32::midpoint(lanes[1], lanes[outer]))));
        lane_of.swap(over, under);
        x += STEP;
    }
    // Exit: each lane's strand eases to its ray (the middle one stays level), then runs level
    // to `X1`.
    let m = (EXIT / RES) as usize;
    for (path, &lane) in paths.iter_mut().zip(&lane_of) {
        let (y0, y1) = (lanes[lane], RAYS[lane]);
        for k in 1..=m {
            let t = k as f32 / m as f32;
            path.push((x + EXIT * t, lerp(y0, y1, ease(t))));
        }
        path.push((X1, y1));
    }
    (paths, crossings)
}

/// The strokes of the mark, back to front: the three strands, then for every crossing a
/// transparent gap cut through the under-strand and the over-strand's segment redrawn on top.
fn rope_strokes() -> Vec<Stroke> {
    let (paths, crossings) = plait();
    let near = |i: usize, c: Point, r: f32| -> Vec<Point> {
        let in_reach = |p: &&Point| (p.0 - c.0).hypot(p.1 - c.1) <= r;
        paths[i].iter().filter(in_reach).copied().collect()
    };
    let mut strokes: Vec<Stroke> = paths
        .iter()
        .zip(STRAND_COLORS)
        .map(|(p, c)| Stroke {
            pts: p.clone(),
            half: STROKE / 2.0,
            paint: Some(c),
            flat: false,
        })
        .collect();
    // The gap reaches just past where the strands' outlines meet; the over-strand's core runs
    // one stroke width further, so its bend refills the notch the gap cut into itself.
    let reach = (STROKE + 2.0 * GAP) * 0.62;
    for (i, c) in crossings {
        let (gap, core) = (near(i, c, reach), near(i, c, reach + STROKE));
        strokes.push(Stroke {
            pts: gap,
            half: STROKE / 2.0 + GAP,
            paint: None,
            flat: true,
        });
        strokes.push(Stroke {
            pts: core,
            half: STROKE / 2.0,
            paint: Some(STRAND_COLORS[i]),
            flat: true,
        });
    }
    strokes
}

/// The 64 × 64 Twine logo, the Rope mark: red, green and blue strands woven as a 3-strand
/// plait (with transparent gaps where one strand passes over another), opening on the right
/// into three rays; transparent background. Drawn from the same geometry as the artwork in
/// `assets/logo/`, centered, with 4 × 4 supersampling.
#[must_use]
pub fn logo() -> RgbaImage {
    const SS: u32 = 4;
    let strokes = rope_strokes();
    let scale = FRAME / 64.0; // pixels → design units
    RgbaImage::from_fn(64, 64, |x, y| {
        let (mut sum, mut hits) = ([0u32; 3], 0u32);
        for (sx, sy) in (0..SS * SS).map(|s| (s % SS, s / SS)) {
            let px = (x as f32 + (sx as f32 + 0.5) / SS as f32 - 32.0) * scale + 32.0;
            let py = (y as f32 + (sy as f32 + 0.5) / SS as f32 - 32.0) * scale + 32.0;
            // The last stroke covering the sample decides its color (or cuts it).
            if let Some(c) = strokes
                .iter()
                .rev()
                .find(|s| s.covers(px, py))
                .and_then(|s| s.paint)
            {
                sum.iter_mut().zip(c).for_each(|(a, v)| *a += u32::from(v));
                hits += 1;
            }
        }
        if hits == 0 {
            return Rgba([0, 0, 0, 0]);
        }
        let [r, g, b] = sum.map(|v| (v / hits) as u8);
        Rgba([r, g, b, (255 * hits / (SS * SS)) as u8])
    })
}

/// A 96 × 64 landscape: sky gradient, sun, two hills (opaque).
#[must_use]
pub fn photo() -> RgbaImage {
    RgbaImage::from_fn(96, 64, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let t = fy / 64.0;
        let mut c = [lerp(40.0, 255.0, t), lerp(90.0, 190.0, t), lerp(200.0, 150.0, t)];
        let sun = (6.5 - ((fx - 70.0).powi(2) + (fy - 18.0).powi(2)).sqrt()).clamp(0.0, 1.0);
        for (ch, s) in c.iter_mut().zip([255.0, 230.0, 90.0]) {
            *ch = lerp(*ch, s, sun);
        }
        for (base, amp, freq, col) in [
            (44.0, 7.0, 0.07, [60.0, 140.0, 70.0]),
            (52.0, 5.0, 0.11, [30.0, 100.0, 50.0]),
        ] {
            let h = base + (fx * freq).sin() * amp;
            let cov = (fy - h + 0.5).clamp(0.0, 1.0);
            for (ch, s) in c.iter_mut().zip(col) {
                *ch = lerp(*ch, s + (fy - h) * 1.5, cov);
            }
        }
        Rgba([
            c[0].clamp(0.0, 255.0) as u8,
            c[1].clamp(0.0, 255.0) as u8,
            c[2].clamp(0.0, 255.0) as u8,
            255,
        ])
    })
}

/// Avatar colors of the multilang demo, one per language.
pub const AVATARS: [(&str, [u8; 3]); 8] = [
    ("en", [0x1E, 0x88, 0xE5]),
    ("de", [0xE5, 0x39, 0x35]),
    ("fr", [0x43, 0xA0, 0x47]),
    ("zh", [0xFB, 0x8C, 0x00]),
    ("ja", [0x8E, 0x24, 0xAA]),
    ("he", [0x00, 0x89, 0x7B]),
    ("ar", [0x6D, 0x4C, 0x41]),
    ("fa", [0x39, 0x49, 0xAB]),
];

/// A 40 × 40 avatar: a `color` disc with a lighter head-and-shoulders silhouette, transparent
/// outside the disc.
#[must_use]
pub fn avatar(color: [u8; 3]) -> RgbaImage {
    let cov = |d: f32| (0.5 - d).clamp(0.0, 1.0);
    RgbaImage::from_fn(40, 40, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let disc = cov(((fx - 20.0).powi(2) + (fy - 20.0).powi(2)).sqrt() - 19.5);
        if disc <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let head = cov(((fx - 20.0).powi(2) + (fy - 16.0).powi(2)).sqrt() - 7.0);
        let body = if fy > 26.0 {
            cov((((fx - 20.0) / 1.25).powi(2) + (fy - 38.0).powi(2)).sqrt() - 11.0)
        } else {
            0.0
        };
        let s = head.max(body);
        let ch = |c: u8| {
            let c = f32::from(c);
            lerp(c, lerp(c, 255.0, 0.75), s).round() as u8
        };
        Rgba([
            ch(color[0]),
            ch(color[1]),
            ch(color[2]),
            (disc * 255.0).round() as u8,
        ])
    })
}

/// Frame `i` of 8 of a 48 × 48 spinner: 8 dots on a ring, the head dot bright, a fading tail;
/// transparent background.
#[must_use]
pub fn spinner_frame(i: u32) -> RgbaImage {
    RgbaImage::from_fn(48, 48, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut px = Rgba([0, 0, 0, 0]);
        for k in 0..8u32 {
            let ang = k as f32 / 8.0 * std::f32::consts::TAU;
            let (cx, cy) = (24.0 + ang.cos() * 16.0, 24.0 + ang.sin() * 16.0);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            if d < 5.0 {
                let age = (i + 8 - k) % 8;
                let v = 255 - age * 28;
                px = Rgba([30, (80 + age * 20) as u8, v as u8, 255]);
            }
        }
        px
    })
}

fn encode(img: &RgbaImage, fmt: ImageFormat) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    match fmt {
        ImageFormat::Bmp | ImageFormat::Jpeg => {
            image::DynamicImage::ImageRgba8(img.clone())
                .to_rgb8()
                .write_to(&mut out, fmt)?;
        }
        _ => img.write_to(&mut out, fmt)?,
    }
    Ok(out.into_inner())
}

/// The spinner as an endlessly looping 8-frame GIF (80 ms per frame).
///
/// # Errors
/// The GIF encoder's error (not expected for in-memory output).
pub fn spinner_gif() -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        enc.set_repeat(Repeat::Infinite)?;
        for i in 0..8 {
            enc.encode_frame(Frame::from_parts(
                spinner_frame(i),
                0,
                0,
                Delay::from_numer_denom_ms(80, 1),
            ))?;
        }
    }
    Ok(out)
}

/// Every generated asset: `(file name, bytes)`.
///
/// # Errors
/// An encoder's error (not expected for in-memory output).
pub fn all() -> Result<Vec<(&'static str, Vec<u8>)>> {
    let photo = photo();
    let logo = logo();
    Ok(vec![
        ("twine_logo.png", encode(&logo, ImageFormat::Png)?),
        ("twine_logo.qoi", encode(&logo, ImageFormat::Qoi)?),
        ("photo.png", encode(&photo, ImageFormat::Png)?),
        ("photo.qoi", encode(&photo, ImageFormat::Qoi)?),
        ("photo.bmp", encode(&photo, ImageFormat::Bmp)?),
        ("photo.jpg", encode(&photo, ImageFormat::Jpeg)?),
        ("spinner.gif", spinner_gif()?),
        ("avatars/en.qoi", encode(&avatar(AVATARS[0].1), ImageFormat::Qoi)?),
        ("avatars/de.qoi", encode(&avatar(AVATARS[1].1), ImageFormat::Qoi)?),
        ("avatars/fr.qoi", encode(&avatar(AVATARS[2].1), ImageFormat::Qoi)?),
        ("avatars/zh.qoi", encode(&avatar(AVATARS[3].1), ImageFormat::Qoi)?),
        ("avatars/ja.qoi", encode(&avatar(AVATARS[4].1), ImageFormat::Qoi)?),
        ("avatars/he.qoi", encode(&avatar(AVATARS[5].1), ImageFormat::Qoi)?),
        ("avatars/ar.qoi", encode(&avatar(AVATARS[6].1), ImageFormat::Qoi)?),
        ("avatars/fa.qoi", encode(&avatar(AVATARS[7].1), ImageFormat::Qoi)?),
    ])
}

/// Writes every asset into `dir` (with `check`, only compares); returns the stale files.
///
/// # Errors
/// The errors of [`all`], or an I/O error reading or writing `dir`.
pub fn write_all(dir: &Path, check: bool) -> Result<Vec<PathBuf>> {
    let mut stale = Vec::new();
    for (name, bytes) in all()? {
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            if !check {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &bytes)?;
            }
            stale.push(path);
        }
    }
    Ok(stale)
}
