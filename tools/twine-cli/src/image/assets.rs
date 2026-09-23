//! Procedurally drawn sample images (`cargo xtask gen-assets`), so the repository needs no
//! third-party artwork: the Twine logo and the image gallery's sample files.

#![allow(clippy::cast_precision_loss)] // small image coordinates

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};

use super::Result;

/// Coverage of a rounded square of half-size `half` and corner radius `r` centered at `c`
/// (1-pixel anti-aliased edge).
fn rounded_square(x: f32, y: f32, c: f32, half: f32, r: f32) -> f32 {
    let qx = ((x - c).abs() - (half - r)).max(0.0);
    let qy = ((y - c).abs() - (half - r)).max(0.0);
    let d = (qx * qx + qy * qy).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Coverage of a thick sine "thread".
fn thread(x: f32, y: f32, phase: f32, center: f32, width: f32) -> f32 {
    let yc = center + (x / 64.0 * std::f32::consts::TAU + phase).sin() * 9.0;
    (width / 2.0 + 0.5 - (y - yc).abs()).clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The 64 × 64 Twine logo: a rounded square with a teal → violet gradient and two woven
/// white threads, transparent outside.
#[must_use]
pub fn logo() -> RgbaImage {
    RgbaImage::from_fn(64, 64, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let cov = rounded_square(fx, fy, 32.0, 30.0, 12.0);
        let t = (fx + fy) / 128.0;
        let mut c = [lerp(0.0, 124.0, t), lerp(170.0, 77.0, t), lerp(170.0, 255.0, t)];
        let a = thread(fx, fy, 0.0, 26.0, 5.0);
        let b = thread(fx, fy, std::f32::consts::PI, 38.0, 5.0);
        // Over/under weaving: which thread is on top alternates every 16 px.
        let (first, second) = if (x / 16) % 2 == 0 { (a, b) } else { (b, a) };
        for w in [second, first] {
            for ch in &mut c {
                *ch = lerp(*ch, 255.0, w);
            }
        }
        let alpha = (cov * 255.0).round() as u8;
        Rgba([c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, alpha])
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
pub fn all() -> Result<Vec<(&'static str, Vec<u8>)>> {
    let photo = photo();
    Ok(vec![
        ("twine_logo.png", encode(&logo(), ImageFormat::Png)?),
        ("photo.png", encode(&photo, ImageFormat::Png)?),
        ("photo.qoi", encode(&photo, ImageFormat::Qoi)?),
        ("photo.bmp", encode(&photo, ImageFormat::Bmp)?),
        ("photo.jpg", encode(&photo, ImageFormat::Jpeg)?),
        ("spinner.gif", spinner_gif()?),
    ])
}

/// Writes every asset into `dir` (with `check`, only compares); returns the stale files.
pub fn write_all(dir: &Path, check: bool) -> Result<Vec<PathBuf>> {
    let mut stale = Vec::new();
    for (name, bytes) in all()? {
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            if !check {
                std::fs::create_dir_all(dir)?;
                std::fs::write(&path, &bytes)?;
            }
            stale.push(path);
        }
    }
    Ok(stale)
}
