//! Decoders (QOI, BMP, PNG, JPEG) and the decoder registry. Test images are generated here
//! (procedural patterns encoded with independent encoders: `image`, `png`, `jpeg-encoder`), so
//! no third-party image files are needed.

mod decoders {
    use std::io::Cursor;

    use proptest::prelude::*;
    use twine_core::ColorFormat;
    use twine_image::decoders::bmp::BmpDecoder;
    use twine_image::decoders::jpeg::JpegDecoder;
    use twine_image::decoders::png::PngDecoder;
    use twine_image::decoders::qoi::{QoiDecoder, qoi_encode};
    use twine_image::{Decoder, DecoderRegistry, Error, ImageHeader, MAX_DECODERS};

    /// A photo-like RGBA pattern: gradients, a hash-based noise and an alpha ramp.
    fn photo(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 27;
                let r = (x * 255 / w.max(1)) as u8;
                let g = (y * 255 / h.max(1)) as u8;
                let b = (((x + y) * 4) as u8).wrapping_add(n as u8);
                let a = if alpha { ((x * 7 + y * 3) % 256) as u8 } else { 255 };
                v.extend_from_slice(&[r, g, b, a]);
            }
        }
        v
    }

    /// RGBA → the decoders' B, G, R, A.
    fn bgra(rgba: &[u8]) -> Vec<u8> {
        rgba.chunks_exact(4)
            .flat_map(|c| [c[2], c[1], c[0], c[3]])
            .collect()
    }

    fn encode_with_image(
        rgba: &[u8],
        w: u32,
        h: u32,
        fmt: image::ImageFormat,
        color: image::ColorType,
    ) -> Vec<u8> {
        let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap();
        let dynimg = image::DynamicImage::ImageRgba8(img);
        let converted = match color {
            image::ColorType::Rgb8 => image::DynamicImage::ImageRgb8(dynimg.to_rgb8()),
            image::ColorType::L8 => image::DynamicImage::ImageLuma8(dynimg.to_luma8()),
            _ => dynimg,
        };
        let mut out = Cursor::new(Vec::new());
        converted.write_to(&mut out, fmt).unwrap();
        out.into_inner()
    }

    fn decode(d: &dyn Decoder, bytes: &[u8]) -> (ImageHeader, Vec<u8>) {
        let mut out = Vec::new();
        let h = d.decode(bytes, &mut out).unwrap();
        assert_eq!(d.probe(bytes), Some(h), "probe agrees with decode");
        assert_eq!(out.len(), usize::from(h.w) * usize::from(h.h) * 4);
        (h, out)
    }

    // ------------------------------------------------------------------ QOI

    #[test]
    fn qoi_decode_reference_images() {
        for (w, h, alpha) in [(64, 48, false), (37, 29, true), (1, 1, true), (300, 2, false)] {
            let rgba = photo(w, h, alpha);
            let color = if alpha {
                image::ColorType::Rgba8
            } else {
                image::ColorType::Rgb8
            };
            let file = encode_with_image(&rgba, w, h, image::ImageFormat::Qoi, color);
            let (hd, px) = decode(&QoiDecoder, &file);
            assert_eq!((u32::from(hd.w), u32::from(hd.h)), (w, h));
            assert_eq!(
                hd.format,
                if alpha {
                    ColorFormat::Argb8888
                } else {
                    ColorFormat::Xrgb8888
                }
            );
            assert_eq!(px, bgra(&rgba), "{w}x{h}");
            // The same pixels through PNG, decoded by the `png` crate.
            let png = encode_with_image(&rgba, w, h, image::ImageFormat::Png, color);
            let mut dec = png::Decoder::new(Cursor::new(png));
            dec.set_transformations(png::Transformations::ALPHA);
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size()];
            r.next_frame(&mut buf).unwrap();
            assert_eq!(px, bgra(&buf), "qoi vs png {w}x{h}");
        }
    }

    proptest! {
        #[test]
        fn qoi_roundtrip_proptest(w in 1u32..40, h in 1u32..20, seed in any::<u32>(), channels in 3u8..=4) {
            let mut s = seed | 1;
            let mut rgba = Vec::new();
            for _ in 0..w * h {
                s ^= s << 13; s ^= s >> 17; s ^= s << 5;
                // Small alphabet → runs, index hits, diffs and lumas.
                let v = s.to_le_bytes().map(|b| b % 6 * 3);
                rgba.extend_from_slice(&[v[0], v[1].wrapping_add(v[0]), v[2], if channels == 4 { v[3].wrapping_mul(40) } else { 255 }]);
            }
            let file = qoi_encode(&rgba, w, h, channels);
            let mut out = Vec::new();
            QoiDecoder.decode(&file, &mut out).unwrap();
            prop_assert_eq!(out, bgra(&rgba));
        }

        #[test]
        fn qoi_random_bytes_never_panic(tail in proptest::collection::vec(any::<u8>(), 0..400), w in 0u32..70, h in 0u32..70) {
            let mut b = b"qoif".to_vec();
            b.extend_from_slice(&w.to_be_bytes());
            b.extend_from_slice(&h.to_be_bytes());
            b.extend_from_slice(&[4, 0]);
            b.extend_from_slice(&tail);
            let _ = QoiDecoder.decode(&b, &mut Vec::new());
            let _ = QoiDecoder.decode(&tail, &mut Vec::new());
        }

        #[test]
        fn bmp_random_bytes_never_panic(i in 0usize..2000, v in any::<u8>(), cut in 0usize..3000, junk in proptest::collection::vec(any::<u8>(), 0..200)) {
            for file in [bmp24(), bmp8()] {
                let mut f = file.clone();
                let i = i % f.len();
                f[i] = v;
                f.truncate(cut.max(2));
                let _ = BmpDecoder.decode(&f, &mut Vec::new());
            }
            let mut j = b"BM".to_vec();
            j.extend_from_slice(&junk);
            let _ = BmpDecoder.decode(&j, &mut Vec::new());
        }

        #[test]
        fn png_random_bytes_never_panic(i in 0usize..4000, v in any::<u8>(), cut in 0usize..5000) {
            let mut f = png_file(png::ColorType::Rgba, 20, 20);
            let i = i % f.len();
            f[i] = v;
            f.truncate(cut.max(8));
            let _ = PngDecoder.decode(&f, &mut Vec::new());
        }

        #[test]
        fn jpeg_random_bytes_never_panic(i in 0usize..4000, v in any::<u8>(), cut in 0usize..5000) {
            let mut f = jpeg_file(jpeg_encoder::SamplingFactor::R_4_2_0, false);
            let i = i % f.len();
            f[i] = v;
            f.truncate(cut.max(2));
            let _ = JpegDecoder.decode(&f, &mut Vec::new());
        }
    }

    #[test]
    fn qoi_rejects_bad_magic_and_huge_dims() {
        let good = qoi_encode(&[1, 2, 3, 255], 1, 1, 4);
        assert!(QoiDecoder.probe(&good).is_some());
        let mut bad = good.clone();
        bad[0] = b'Q';
        assert_eq!(
            QoiDecoder.decode(&bad, &mut Vec::new()),
            Err(Error::InvalidHeader)
        );
        let mut huge = good.clone();
        huge[4..8].copy_from_slice(&20_000u32.to_be_bytes());
        assert_eq!(
            QoiDecoder.decode(&huge, &mut Vec::new()),
            Err(Error::InvalidHeader)
        );
        let mut zero = good.clone();
        zero[8..12].copy_from_slice(&0u32.to_be_bytes());
        assert!(QoiDecoder.probe(&zero).is_none());
        // Truncated op stream.
        let long = qoi_encode(&photo(16, 16, true), 16, 16, 4);
        assert_eq!(
            QoiDecoder.decode(&long[..40], &mut Vec::new()),
            Err(Error::Decode("qoi truncated"))
        );
    }

    // ------------------------------------------------------------------ BMP

    fn bmp24() -> Vec<u8> {
        encode_with_image(
            &photo(21, 13, false),
            21,
            13,
            image::ImageFormat::Bmp,
            image::ColorType::Rgb8,
        )
    }

    fn bmp8() -> Vec<u8> {
        encode_with_image(
            &photo(21, 13, false),
            21,
            13,
            image::ImageFormat::Bmp,
            image::ColorType::L8,
        )
    }

    #[test]
    fn bmp_decodes_24_and_8bit() {
        let src = photo(21, 13, false);
        let (h, px) = decode(&BmpDecoder, &bmp24());
        assert_eq!((h.w, h.h, h.format), (21, 13, ColorFormat::Xrgb8888));
        assert_eq!(px, bgra(&src));
        // 8-bit paletted gray.
        let (h, px) = decode(&BmpDecoder, &bmp8());
        assert_eq!((h.w, h.h), (21, 13));
        let gray =
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(21, 13, src).unwrap()).to_luma8();
        let expected: Vec<u8> = gray
            .pixels()
            .flat_map(|p| [p.0[0], p.0[0], p.0[0], 255])
            .collect();
        assert_eq!(px, expected);
        // 32-bit with alpha.
        let rgba = photo(9, 7, true);
        let file = encode_with_image(&rgba, 9, 7, image::ImageFormat::Bmp, image::ColorType::Rgba8);
        let (h, px) = decode(&BmpDecoder, &file);
        assert_eq!(h.format, ColorFormat::Argb8888);
        assert_eq!(px, bgra(&rgba));
    }

    // ------------------------------------------------------------------ PNG

    /// A PNG in `color` (8 bit) encoded with the `png` crate; palette images use a 256-entry
    /// palette with a transparency chunk.
    fn png_file(color: png::ColorType, w: u32, h: u32) -> Vec<u8> {
        let rgba = photo(w, h, true);
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(color);
            e.set_depth(png::BitDepth::Eight);
            if color == png::ColorType::Indexed {
                let pal: Vec<u8> = (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect();
                let trns: Vec<u8> = (0..=255u8).map(|i| i.wrapping_mul(3)).collect();
                e.set_palette(pal);
                e.set_trns(trns);
            }
            let mut wr = e.write_header().unwrap();
            let data: Vec<u8> = match color {
                png::ColorType::Grayscale => rgba.chunks_exact(4).map(|c| c[0]).collect(),
                png::ColorType::Rgb => rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect(),
                png::ColorType::GrayscaleAlpha => rgba.chunks_exact(4).flat_map(|c| [c[1], c[3]]).collect(),
                png::ColorType::Indexed => rgba.chunks_exact(4).map(|c| c[2]).collect(),
                png::ColorType::Rgba => rgba.clone(),
            };
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    /// Reference decode through the `image` crate, as B, G, R, A.
    fn reference(bytes: &[u8]) -> Vec<u8> {
        bgra(&image::load_from_memory(bytes).unwrap().to_rgba8().into_raw())
    }

    #[test]
    fn png_decodes_rgb_rgba_gray_palette() {
        for (color, fmt) in [
            (png::ColorType::Grayscale, ColorFormat::Xrgb8888), // basn0g08
            (png::ColorType::Rgb, ColorFormat::Xrgb8888),       // basn2c08
            (png::ColorType::Indexed, ColorFormat::Argb8888),   // basn3p08 (+ tRNS)
            (png::ColorType::Rgba, ColorFormat::Argb8888),      // basn6a08
            (png::ColorType::GrayscaleAlpha, ColorFormat::Argb8888),
        ] {
            let f = png_file(color, 32, 23);
            let (h, px) = decode(&PngDecoder, &f);
            assert_eq!((h.w, h.h, h.format), (32, 23, fmt), "{color:?}");
            assert_eq!(px, reference(&f), "{color:?}");
        }
    }

    #[test]
    fn png_probe_header_only() {
        let f = png_file(png::ColorType::Rgb, 40, 30);
        // Only the signature and IHDR (8 + 25 bytes) are needed to probe.
        let h = PngDecoder.probe(&f[..33]).unwrap();
        assert_eq!((h.w, h.h, h.format), (40, 30, ColorFormat::Xrgb8888));
        assert!(PngDecoder.decode(&f[..33], &mut Vec::new()).is_err());
        assert!(PngDecoder.probe(b"\x89PNG\r\n\x1a\nxx").is_none());
        assert!(PngDecoder.probe(&bmp24()).is_none());
    }

    // ------------------------------------------------------------------ JPEG

    fn jpeg_file(sampling: jpeg_encoder::SamplingFactor, progressive: bool) -> Vec<u8> {
        let (w, h) = (48u16, 40u16);
        let rgba = photo(u32::from(w), u32::from(h), false);
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
        let mut out = Vec::new();
        let mut e = jpeg_encoder::Encoder::new(&mut out, 90);
        e.set_sampling_factor(sampling);
        e.set_progressive(progressive);
        e.encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
        out
    }

    #[test]
    fn jpeg_decodes_baseline_420_444() {
        use jpeg_encoder::SamplingFactor as S;
        for (sampling, progressive) in [
            (S::R_4_2_0, false),
            (S::R_4_4_4, false),
            (S::R_4_2_2, false),
            (S::R_4_2_0, true),
        ] {
            let f = jpeg_file(sampling, progressive);
            let mut out = Vec::new();
            let h = JpegDecoder
                .decode(&f, &mut out)
                .unwrap_or_else(|e| panic!("{sampling:?} {progressive}: {e}"));
            let px = out;
            assert_eq!((h.w, h.h, h.format), (48, 40, ColorFormat::Xrgb8888));
            let r = reference(&f);
            let worst = px.iter().zip(&r).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(
                worst <= 2,
                "{sampling:?} progressive={progressive}: max delta {worst}"
            );
            // Close to the source as well (lossy, quality 90).
            let src = bgra(&photo(48, 40, false));
            let mean: u64 = px
                .iter()
                .zip(&src)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>()
                / px.len() as u64;
            assert!(mean <= 6, "{sampling:?}: mean error {mean}");
        }
        // Grayscale JPEG (from the `image` crate).
        let g = encode_with_image(
            &photo(30, 20, false),
            30,
            20,
            image::ImageFormat::Jpeg,
            image::ColorType::L8,
        );
        let (h, px) = decode(&JpegDecoder, &g);
        assert_eq!((h.w, h.h), (30, 20));
        assert!(
            px.chunks_exact(4)
                .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255)
        );
    }

    #[test]
    fn jpeg_probe() {
        let f = jpeg_file(jpeg_encoder::SamplingFactor::R_4_2_0, true);
        let h = JpegDecoder.probe(&f).unwrap();
        assert_eq!((h.w, h.h), (48, 40));
        assert!(JpegDecoder.probe(&f[..2]).is_none());
        assert!(JpegDecoder.probe(b"\xFF\xD8garbage").is_none());
        // A truncated file never panics (zune-jpeg may fill missing data with gray).
        let _ = JpegDecoder.decode(&f[..f.len() / 3], &mut Vec::new());
    }

    // ------------------------------------------------------------------ registry

    struct Fake(&'static str, u16);
    impl Decoder for Fake {
        fn name(&self) -> &'static str {
            self.0
        }
        fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
            bytes
                .starts_with(b"fake")
                .then(|| ImageHeader::new(ColorFormat::Argb8888, self.1, 1))
        }
        fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
            let h = self.probe(bytes).ok_or(Error::InvalidHeader)?;
            out.clear();
            out.resize(usize::from(self.1) * 4, 7);
            Ok(h)
        }
    }

    static FAKE_A: Fake = Fake("fake-a", 1);
    static FAKE_B: Fake = Fake("fake-b", 2);

    #[test]
    fn registry_probes_in_order() {
        let mut r = DecoderRegistry::with_defaults();
        assert_eq!(
            r.names().collect::<Vec<_>>(),
            ["qoi", "png", "jpeg", "bmp", "gif"]
        );
        r.register(&FAKE_A).unwrap();
        r.register(&FAKE_B).unwrap();
        let (d, h) = r.probe(b"fake!").unwrap();
        assert_eq!((d.name(), h.w), ("fake-a", 1));
        for (file, name) in [
            (qoi_encode(&[0, 0, 0, 255], 1, 1, 3), "qoi"),
            (png_file(png::ColorType::Rgb, 2, 2), "png"),
            (jpeg_file(jpeg_encoder::SamplingFactor::R_4_2_0, false), "jpeg"),
            (bmp24(), "bmp"),
        ] {
            assert_eq!(r.probe(&file).unwrap().0.name(), name);
            let mut out = Vec::new();
            r.decode(&file, &mut out).unwrap();
            assert!(!out.is_empty());
        }
        assert_eq!(r.decode(b"nothing", &mut Vec::new()), Err(Error::InvalidHeader));
    }

    #[test]
    fn registry_full_errors() {
        let mut r = DecoderRegistry::new();
        for _ in 0..MAX_DECODERS {
            r.register(&FAKE_A).unwrap();
        }
        assert_eq!(r.register(&FAKE_B), Err(Error::CacheFull));
    }
}
