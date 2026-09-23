//! GIF decoding and animation. Animated test GIFs are produced by the `image` crate's encoder;
//! GIFs that need specific disposal methods are assembled byte by byte here.

mod gif {
    use std::io::Cursor;

    use image::AnimationDecoder;
    use image::codecs::gif::{GifEncoder, Repeat};
    use proptest::prelude::*;
    use twine_image::Decoder;
    use twine_image::decoders::gif::{GifDecoder, GifPlayer};
    use twine_testing::alloc::{CountingAllocator, count_allocs};

    #[global_allocator]
    static A: CountingAllocator = CountingAllocator;

    fn leak(v: Vec<u8>) -> &'static [u8] {
        Box::leak(v.into_boxed_slice())
    }

    /// A 3-frame 24 × 16 animation: a moving square over a gradient (delays 50, 120, 0 ms).
    fn animation() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for (i, delay) in [50u32, 120, 0].into_iter().enumerate() {
                let img = image::RgbaImage::from_fn(24, 16, |x, y| {
                    if x >= i as u32 * 6 && x < i as u32 * 6 + 8 && (4..12).contains(&y) {
                        image::Rgba([255, 255, 255, 255])
                    } else {
                        image::Rgba([(x * 10) as u8, (y * 15) as u8, 128, 255])
                    }
                });
                let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(delay, 1));
                enc.encode_frame(frame).unwrap();
            }
        }
        out
    }

    /// Frames of `bytes` as composited by the `image` crate (RGBA).
    fn reference_frames(bytes: &[u8]) -> Vec<Vec<u8>> {
        let d = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).unwrap();
        d.into_frames().map(|f| f.unwrap().into_buffer().into_raw()).collect()
    }

    fn bgra(rgba: &[u8]) -> Vec<u8> {
        rgba.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect()
    }

    #[test]
    fn gif_static_first_frame() {
        let file = animation();
        let mut out = Vec::new();
        let h = GifDecoder.decode(&file, &mut out).unwrap();
        assert_eq!((h.w, h.h), (24, 16));
        assert_eq!(GifDecoder.probe(&file), Some(h));
        assert_eq!(out, bgra(&reference_frames(&file)[0]));
    }

    #[test]
    fn gif_animation_frame_count_and_delays() {
        let file = leak(animation());
        let reference = reference_frames(file);
        let mut p = GifPlayer::new(file).unwrap();
        assert_eq!(p.frame_count(), 3);
        assert_eq!(p.loop_count(), Some(0));
        let delays: Vec<u64> = (0..6).map(|_| p.advance().as_millis()).collect();
        // 0 → 100 ms (browser behaviour); the animation loops forever.
        assert_eq!(delays, [50, 120, 100, 50, 120, 100]);
        p.reset();
        for (i, r) in reference.iter().enumerate() {
            p.advance();
            assert_eq!(p.frame_index(), i);
            assert_eq!(p.pixels().data, &bgra(r)[..], "frame {i}");
        }
        assert!(!p.is_finished());
    }

    #[test]
    fn gif_advance_does_not_allocate() {
        let file = leak(animation());
        let mut p = GifPlayer::new(file).unwrap();
        p.advance();
        let ((), stats) = count_allocs(|| {
            for _ in 0..10 {
                p.advance();
            }
        });
        assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    }

    // ------------------------------------------------------------------ hand-built GIFs

    /// One frame of a hand-built GIF.
    struct F {
        rect: (u16, u16, u16, u16),
        indices: Vec<u8>,
        disposal: u8,
        transparent: Option<u8>,
        delay_cs: u16,
    }

    /// LZW data with minimum code size 2 that only uses literal codes (a clear code every
    /// two literals keeps the code size at 3 bits).
    fn lzw_literals(indices: &[u8]) -> Vec<u8> {
        let (clear, eoi) = (4u32, 5u32);
        let mut codes = Vec::new();
        for pair in indices.chunks(2) {
            codes.push(clear);
            codes.extend(pair.iter().map(|&i| u32::from(i)));
        }
        codes.push(eoi);
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        for c in codes {
            acc |= c << n;
            n += 3;
            while n >= 8 {
                bytes.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        bytes
    }

    /// A `GIF89a` with a 4-color global palette (black, red, green, blue), no loop extension.
    fn build(w: u16, h: u16, frames: &[F]) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&h.to_le_bytes());
        g.extend_from_slice(&[0x81, 0, 0]);
        g.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
        for f in frames {
            let flags = (f.disposal << 2) | u8::from(f.transparent.is_some());
            g.extend_from_slice(&[0x21, 0xF9, 4, flags]);
            g.extend_from_slice(&f.delay_cs.to_le_bytes());
            g.extend_from_slice(&[f.transparent.unwrap_or(0), 0]);
            g.push(0x2C);
            for v in [f.rect.0, f.rect.1, f.rect.2, f.rect.3] {
                g.extend_from_slice(&v.to_le_bytes());
            }
            g.extend_from_slice(&[0, 2]);
            for block in lzw_literals(&f.indices).chunks(255) {
                g.push(block.len() as u8);
                g.extend_from_slice(block);
            }
            g.push(0);
        }
        g.push(0x3B);
        g
    }

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const RED: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    fn px(p: &GifPlayer, x: usize, y: usize) -> [u8; 4] {
        let d = p.pixels().data;
        let o = (y * 4 + x) * 4;
        [d[o], d[o + 1], d[o + 2], d[o + 3]]
    }

    #[test]
    fn gif_disposal_previous_restores() {
        let file = leak(build(
            4,
            2,
            &[
                // Full red background.
                F { rect: (0, 0, 4, 2), indices: vec![1; 8], disposal: 1, transparent: None, delay_cs: 10 },
                // A green 2×1 patch, restored to the previous canvas afterwards.
                F { rect: (1, 0, 2, 1), indices: vec![2, 2], disposal: 3, transparent: None, delay_cs: 10 },
                // A blue pixel at (3, 1), then cleared to transparent.
                F { rect: (3, 1, 1, 1), indices: vec![3], disposal: 2, transparent: None, delay_cs: 10 },
                // Nothing new: 1×1 fully transparent frame.
                F { rect: (0, 0, 1, 1), indices: vec![0], disposal: 0, transparent: Some(0), delay_cs: 10 },
            ],
        ));
        let mut p = GifPlayer::new(file).unwrap();
        assert_eq!(p.frame_count(), 4);
        assert_eq!(p.loop_count(), None);
        p.advance();
        assert_eq!(px(&p, 1, 0), RED);
        p.advance();
        assert_eq!([px(&p, 1, 0), px(&p, 2, 0), px(&p, 3, 0)], [GREEN, GREEN, RED]);
        p.advance(); // the green patch is gone again
        assert_eq!([px(&p, 1, 0), px(&p, 2, 0), px(&p, 3, 1)], [RED, RED, BLUE]);
        p.advance(); // the blue pixel was cleared (background disposal)
        assert_eq!([px(&p, 3, 1), px(&p, 0, 0)], [CLEAR, RED]);
        // No loop extension: play once, then stay on the last frame.
        p.advance();
        assert!(p.is_finished());
        assert_eq!(px(&p, 0, 0), RED);
    }

    #[test]
    fn gif_transparency() {
        let file = leak(build(
            4,
            2,
            &[
                F { rect: (0, 0, 4, 2), indices: vec![3; 8], disposal: 1, transparent: None, delay_cs: 5 },
                // Index 0 is transparent: only the red pixels replace the blue ones.
                F { rect: (0, 0, 4, 1), indices: vec![0, 1, 0, 1], disposal: 1, transparent: Some(0), delay_cs: 5 },
            ],
        ));
        let mut p = GifPlayer::new(file).unwrap();
        assert_eq!(p.advance().as_millis(), 50);
        p.advance();
        assert_eq!([px(&p, 0, 0), px(&p, 1, 0), px(&p, 2, 0), px(&p, 3, 0)], [BLUE, RED, BLUE, RED]);
        // The first frame alone (static decoder): transparent index → transparent pixel.
        let mut out = Vec::new();
        let single = build(2, 1, &[F { rect: (0, 0, 2, 1), indices: vec![0, 2], disposal: 0, transparent: Some(0), delay_cs: 0 }]);
        GifDecoder.decode(&single, &mut out).unwrap();
        assert_eq!(out, [CLEAR, GREEN].concat());
        let _ = BLACK;
    }

    proptest! {
        #[test]
        fn gif_random_bytes_never_panic(i in 0usize..3000, v in any::<u8>(), cut in 0usize..4000, junk in proptest::collection::vec(any::<u8>(), 0..300)) {
            let mut f = animation();
            let i = i % f.len();
            f[i] = v;
            f.truncate(cut.max(6));
            let f = leak(f);
            let _ = GifDecoder.decode(f, &mut Vec::new());
            if let Ok(mut p) = GifPlayer::new(f) {
                for _ in 0..5 {
                    p.advance();
                }
            }
            let mut j = b"GIF89a".to_vec();
            j.extend_from_slice(&junk);
            let j = leak(j);
            let _ = GifDecoder.decode(j, &mut Vec::new());
            if let Ok(mut p) = GifPlayer::new(j) {
                p.advance();
                p.advance();
            }
        }
    }
}
