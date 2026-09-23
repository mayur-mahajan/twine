//! Deterministic 8-bit RGB PNG output (screenshots, recordings, headless shots).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Writes `rgb` (`w * h * 3` bytes) as an 8-bit RGB PNG without metadata, creating parent
/// directories.
pub fn write_rgb_png(path: &Path, w: u32, h: u32, rgb: &[u8]) -> std::io::Result<()> {
    if rgb.len() != w as usize * h as usize * 3 {
        return Err(std::io::Error::other(format!(
            "{w}x{h} RGB image needs {} bytes, got {}",
            w as usize * h as usize * 3,
            rgb.len()
        )));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = BufWriter::new(File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Default);
    enc.set_filter(png::FilterType::Paeth);
    enc.set_adaptive_filter(png::AdaptiveFilterType::NonAdaptive);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgb).map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)
}
