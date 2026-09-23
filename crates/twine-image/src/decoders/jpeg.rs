//! JPEG decoder over `zune-jpeg` (baseline and progressive, gray / YCbCr / CMYK input, all
//! chroma subsamplings). Output is always opaque `Xrgb8888`.
//!
//! Each decode parses the headers twice (probe, then decode); that is cheap next to the
//! entropy decoding.

use alloc::vec::Vec;

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder as Zune;

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

/// Largest accepted width or height.
pub const JPEG_MAX_DIM: usize = 16384;

/// Decodes JPEG files.
#[derive(Clone, Copy, Debug, Default)]
pub struct JpegDecoder;

fn options(out: ColorSpace) -> DecoderOptions {
    DecoderOptions::new_safe()
        .set_max_width(JPEG_MAX_DIM)
        .set_max_height(JPEG_MAX_DIM)
        .jpeg_set_out_colorspace(out)
}

/// Header and whether the image is grayscale.
fn parse(bytes: &[u8]) -> Option<(ImageHeader, bool)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut d = Zune::new_with_options(ZCursor::new(bytes), options(ColorSpace::BGRA));
    d.decode_headers().ok()?;
    let (w, h) = d.dimensions()?;
    if w == 0 || h == 0 {
        return None;
    }
    let gray = matches!(d.input_colorspace(), Some(ColorSpace::Luma | ColorSpace::LumaA));
    Some((out_header(w as u16, h as u16, false), gray))
}

impl Decoder for JpegDecoder {
    fn name(&self) -> &'static str {
        "jpeg"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        parse(bytes).map(|(h, _)| h)
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let (header, gray) = parse(bytes).ok_or(Error::InvalidHeader)?;
        let pixels = usize::from(header.w) * usize::from(header.h);
        let n = pixels * 4;
        prepare_out(out, n)?;
        if gray {
            // Decode luma into the first quarter, then expand in place (back to front).
            let mut d = Zune::new_with_options(ZCursor::new(bytes), options(ColorSpace::Luma));
            d.decode_into(&mut out[..pixels])
                .map_err(|_| Error::Decode("jpeg corrupt"))?;
            for i in (0..pixels).rev() {
                let g = out[i];
                out[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, 255]);
            }
        } else {
            let mut d = Zune::new_with_options(ZCursor::new(bytes), options(ColorSpace::BGRA));
            d.decode_into(out).map_err(|_| Error::Decode("jpeg corrupt"))?;
            for px in out.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
        log_decoded("jpeg", header, n);
        Ok(header)
    }
}
