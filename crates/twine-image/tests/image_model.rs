//! The image data model: header sizes, validation, pixel access and sources.

use twine_core::ColorFormat;
use twine_image::{Compression, Error, Image, ImageData, ImageFlags, ImageHeader, ImageSource};

#[test]
fn color_format_stride_and_size_all_formats() {
    for f in ColorFormat::ALL {
        for w in [1u16, 7, 8, 33] {
            let h = ImageHeader::new(f, w, 3);
            let stride = (u32::from(w) * u32::from(f.bpp())).div_ceil(8) as usize;
            assert_eq!(usize::from(h.stride), stride, "{f} w={w}");
            let mut size = f.palette_len() * 4 + stride * 3;
            if f == ColorFormat::Rgb565A8 {
                size += usize::from(w) * 3;
            }
            assert_eq!(h.data_size(), size, "{f} w={w}");
            let img = Image::new_owned(h, Box::from(vec![0u8; size]));
            assert_eq!(img.validate(), Ok(()), "{f} w={w}");
            let px = img.pixels().expect("pixels");
            assert_eq!((px.w, px.h, px.stride, px.format), (w, 3, h.stride, f));
        }
    }
}

#[test]
fn validate_rejects_short_data() {
    let h = ImageHeader::new(ColorFormat::Argb8888, 4, 4);
    let img = Image::new_owned(h, Box::from(vec![0u8; 63]));
    assert_eq!(
        img.validate(),
        Err(Error::SizeMismatch {
            expected: 64,
            got: 63
        })
    );
    assert!(img.pixels().is_none());
    let bad_stride = ImageHeader { stride: 15, ..h };
    let img = Image::new_owned(bad_stride, Box::from(vec![0u8; 64]));
    assert_eq!(img.validate(), Err(Error::InvalidHeader));
    let compressed = Image {
        header: ImageHeader {
            flags: ImageFlags::COMPRESSED,
            ..h
        },
        data: ImageData::Compressed {
            method: Compression::Rle,
            data: &[],
            decompressed_size: 60,
        },
    };
    assert_eq!(
        compressed.validate(),
        Err(Error::SizeMismatch {
            expected: 64,
            got: 60
        })
    );
    assert!(compressed.pixels().is_none(), "compressed images need the cache");
}

#[test]
fn pixels_exposes_palette_and_alpha_plane() {
    // I2: 4 palette entries, then 2 rows of 1 byte (3 px).
    let mut d = vec![0u8; 16 + 2];
    d[4..8].copy_from_slice(&[1, 2, 3, 255]);
    d[16] = 0b0100_0000;
    let img = Image::new_owned(ImageHeader::new(ColorFormat::I2, 3, 2), Box::from(d));
    assert_eq!(img.palette().unwrap()[4..8], [1, 2, 3, 255]);
    let px = img.pixels().unwrap();
    assert_eq!(px.palette.unwrap().len(), 16);
    assert_eq!(px.row(0), &[0b0100_0000]);
    let mut out = [0u8; 4];
    twine_render::read_row_argb(&px, 0, 0, 1, &mut out);
    assert_eq!(out, [1, 2, 3, 255]);

    // Rgb565A8: 2x2 color plane (stride 4), then 2x2 alpha plane.
    let d: Vec<u8> = (0..8).chain([10, 20, 30, 40]).collect();
    let img = Image::new_owned(ImageHeader::new(ColorFormat::Rgb565A8, 2, 2), Box::from(d));
    assert!(img.palette().is_none());
    let px = img.pixels().unwrap();
    assert_eq!(px.row(1), &[4, 5, 6, 7]);
    assert_eq!(px.alpha_row(1), &[30, 40]);

    // Premultiplied flag reaches the pixels.
    let h = ImageHeader {
        flags: ImageFlags::PREMULTIPLIED,
        ..ImageHeader::new(ColorFormat::Argb8888, 1, 1)
    };
    assert!(
        Image::new_owned(h, Box::from(vec![0u8; 4]))
            .pixels()
            .unwrap()
            .premultiplied
    );
}

#[test]
fn symbol_and_svg_sources_construct() {
    static IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::L8, 1, 1), &[0]);
    let sources = [
        ImageSource::Static(&IMG),
        ImageSource::Encoded(b"qoif"),
        ImageSource::file("/a.png").unwrap(),
        ImageSource::Symbol("\u{f00c}"),
        ImageSource::Svg(b"<svg/>"),
    ];
    let text: Vec<String> = sources.iter().map(ToString::to_string).collect();
    assert_eq!(text[0], "static 1x1 L8");
    assert!(text[3].starts_with("symbol"));
    assert!(text[4].starts_with("svg"));
    assert_eq!(sources[3].clone(), ImageSource::Symbol("\u{f00c}"));
}
