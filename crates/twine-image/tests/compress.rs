//! LVGL RLE and LZ4 decompression.

mod compress {
    use proptest::prelude::*;
    use twine_core::ColorFormat;
    use twine_image::{
        Compression, Error, Image, ImageData, ImageFlags, ImageHeader, lz4_decompress, rle_compress,
        rle_decompress,
    };

    proptest! {
        #[test]
        fn rle_roundtrip_proptest(blk in 1usize..=4, data in proptest::collection::vec(0u8..4, 0..600)) {
            // Few distinct values → many runs; the length is made a multiple of the block.
            let n = data.len() / blk * blk;
            let data = &data[..n];
            let packed = rle_compress(data, blk);
            let mut out = vec![0u8; n];
            prop_assert_eq!(rle_decompress(&packed, &mut out, blk), Ok(n));
            prop_assert_eq!(&out[..], data);
        }

        #[test]
        fn rle_random_input_never_panics(blk in 1usize..=4, input in proptest::collection::vec(any::<u8>(), 0..300), n in 0usize..400) {
            let mut out = vec![0u8; n];
            let _ = rle_decompress(&input, &mut out, blk);
        }

        #[test]
        fn lz4_random_input_never_panics(input in proptest::collection::vec(any::<u8>(), 0..300), n in 0usize..400) {
            let mut out = vec![0u8; n];
            let _ = lz4_decompress(&input, &mut out);
        }
    }

    /// Hand-built stream, blk = 3 (RGB888 pixels):
    /// `0x02 [10 20 30]` → the block twice; `0x81 [1 2 3]` → one literal block;
    /// `0x00 [9 9 9]` → a block repeated zero times (writes nothing).
    #[test]
    fn rle_known_vector() {
        let packed = [0x02, 10, 20, 30, 0x81, 1, 2, 3, 0x00, 9, 9, 9];
        let mut out = [0u8; 9];
        assert_eq!(rle_decompress(&packed, &mut out, 3), Ok(9));
        assert_eq!(out, [10, 20, 30, 10, 20, 30, 1, 2, 3]);
        assert_eq!(rle_compress(&out, 3), [0x02, 10, 20, 30, 0x81, 1, 2, 3]);
    }

    #[test]
    fn rle_truncated_input_errors() {
        let mut out = [0u8; 16];
        // Literal of 3 blocks with only 2 present.
        assert_eq!(rle_decompress(&[0x83, 1, 2], &mut out, 1), Err(Error::Decode("rle truncated")));
        // Repeat packet without its block.
        assert_eq!(rle_decompress(&[0x05, 1], &mut out, 2), Err(Error::Decode("rle truncated")));
        assert!(rle_decompress(&[1, 2], &mut out, 0).is_err());
    }

    #[test]
    fn rle_overflow_within_one_block_ok() {
        // Output of 5 bytes, blk 2: the third block overflows by one byte → tolerated.
        let packed = rle_compress(&[1, 2, 1, 2, 7], 2); // padded to [.. 7, 0]
        let mut out = [0u8; 5];
        assert_eq!(rle_decompress(&packed, &mut out, 2), Ok(5));
        assert_eq!(out, [1, 2, 1, 2, 7]);
        // A whole extra block is an error.
        let mut out = [0u8; 4];
        assert_eq!(rle_decompress(&[0x03, 1, 2], &mut out, 2), Err(Error::Decode("rle overflow")));
    }

    #[test]
    fn lz4_roundtrip() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i / 7 % 13) as u8).collect();
        let packed = lz4_flex::block::compress(&data);
        assert!(packed.len() < data.len() / 4);
        let mut out = vec![0u8; data.len()];
        assert_eq!(lz4_decompress(&packed, &mut out), Ok(data.len()));
        assert_eq!(out, data);
    }

    #[test]
    fn lz4_corrupt_errors_no_panic() {
        let data = vec![42u8; 1000];
        let mut packed = lz4_flex::block::compress(&data);
        let mut out = vec![0u8; 1000];
        // Too small output.
        assert!(lz4_decompress(&packed, &mut out[..500]).is_err());
        // Offset pointing before the start.
        let last = packed.len() - 1;
        packed[last] ^= 0xFF;
        for i in 0..packed.len() {
            let mut p = packed.clone();
            p[i] = p[i].wrapping_add(97);
            let _ = lz4_decompress(&p, &mut out);
        }
        assert!(lz4_decompress(&[0xF0], &mut out).is_err());
    }

    #[test]
    fn image_decompress_into_both_methods() {
        let raw: Vec<u8> = (0..16 * 8 * 2).map(|i| (i / 32) as u8).collect();
        let header = ImageHeader {
            flags: ImageFlags::COMPRESSED,
            ..ImageHeader::new(ColorFormat::Rgb565, 16, 8)
        };
        for (method, packed) in [
            (Compression::Rle, rle_compress(&raw, 2)),
            (Compression::Lz4, lz4_flex::block::compress(&raw)),
        ] {
            let img = Image {
                header,
                data: ImageData::Compressed {
                    method,
                    data: Box::leak(packed.into_boxed_slice()),
                    decompressed_size: raw.len() as u32,
                },
            };
            assert!(img.validate().is_ok());
            let mut out = vec![0u8; raw.len()];
            let h = img.decompress_into(&mut out).unwrap();
            assert!(!h.flags.contains(ImageFlags::COMPRESSED));
            assert_eq!(out, raw, "{method:?}");
            assert_eq!(
                img.decompress_into(&mut out[..10]),
                Err(Error::SizeMismatch { expected: raw.len(), got: 10 })
            );
        }
    }
}
