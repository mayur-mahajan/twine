//! Deterministic PNG reading and writing of 8-bit RGB images.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

/// A decoded 8-bit RGB image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed `R, G, B` bytes, `width * height * 3` long.
    pub data: Vec<u8>,
}

/// Encodes `rgb` (`w * h * 3` bytes) as an 8-bit RGB PNG into a byte vector.
///
/// The output is deterministic: no metadata chunks, fixed compression and filter settings.
///
/// # Panics
/// If `rgb.len() != w * h * 3`.
pub fn encode_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    assert_eq!(
        rgb.len(),
        w as usize * h as usize * 3,
        "encode_rgb: {w}x{h} image needs {} bytes",
        w as usize * h as usize * 3
    );
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Default);
        enc.set_filter(png::FilterType::Paeth);
        enc.set_adaptive_filter(png::AdaptiveFilterType::NonAdaptive);
        let mut writer = enc.write_header()?;
        writer.write_image_data(rgb)?;
        writer.finish()?;
    }
    Ok(out)
}

/// Writes `rgb` as a PNG file (see [`encode_rgb`]), creating parent directories.
pub fn write_rgb_png(path: &Path, w: u32, h: u32, rgb: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = encode_rgb(w, h, rgb).map_err(std::io::Error::other)?;
    let mut f = BufWriter::new(File::create(path)?);
    std::io::Write::write_all(&mut f, &bytes)?;
    std::io::Write::flush(&mut f)
}

/// Reads a PNG file and converts it to 8-bit RGB (gray, palette, alpha and 16-bit inputs are
/// expanded; alpha is dropped).
pub fn read_rgb_png(path: &Path) -> std::io::Result<RgbImage> {
    let mut dec = png::Decoder::new(BufReader::new(File::open(path)?));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    buf.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect(),
        png::ColorType::Indexed => {
            return Err(std::io::Error::other("indexed PNG was not expanded"));
        }
    };
    Ok(RgbImage {
        width: info.width,
        height: info.height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_is_deterministic_and_roundtrips() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 7) as u8).collect();
        let a = encode_rgb(4, 3, &rgb).unwrap();
        let b = encode_rgb(4, 3, &rgb).unwrap();
        assert_eq!(a, b);
        let dir = std::env::temp_dir().join(format!("twine-png-io-{}", std::process::id()));
        let p = dir.join("x.png");
        write_rgb_png(&p, 4, 3, &rgb).unwrap();
        let img = read_rgb_png(&p).unwrap();
        assert_eq!((img.width, img.height), (4, 3));
        assert_eq!(img.data, rgb);
        let _ = std::fs::remove_dir_all(dir);
    }
}
