//! Image cache, header cache and source resolution.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::ColorFormat;
use twine_image::decoders::qoi::qoi_encode;
use twine_image::{
    Compression, Decoder, DecoderRegistry, Error, FileSource, Image, ImageCache, ImageContext, ImageData,
    ImageFlags, ImageHeader, ImageHeaderCache, ImageSource, SourceKey, header_of, rle_compress, with_pixels,
};
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[allow(clippy::unnecessary_wraps)] // the signature of a decode callback
fn l8(out: &mut Vec<u8>, n: usize, v: u8) -> Result<ImageHeader, Error> {
    out.resize(n * n, v);
    Ok(ImageHeader::new(ColorFormat::L8, n as u16, n as u16))
}

fn leak(v: Vec<u8>) -> &'static [u8] {
    Box::leak(v.into_boxed_slice())
}

mod cache {
    use super::*;

    #[test]
    fn cache_hit_miss_counts() {
        let mut c = ImageCache::new(1000, 4);
        let decoded = Cell::new(0);
        for k in [1, 2, 1, 1, 2, 3] {
            let img = c
                .get_or_decode(SourceKey::Ptr(k), |o| {
                    decoded.set(decoded.get() + 1);
                    l8(o, 10, k as u8)
                })
                .unwrap();
            assert_eq!(img.data[0], k as u8);
        }
        let s = c.stats();
        assert_eq!((s.hits, s.misses, s.evictions, s.bytes_used), (3, 3, 0, 300));
        assert_eq!(decoded.get(), 3);
        assert!(c.contains(&SourceKey::Ptr(3)));
        c.invalidate(&SourceKey::Ptr(3));
        assert!(!c.contains(&SourceKey::Ptr(3)));
        assert_eq!(c.stats().bytes_used, 200);
        c.clear();
        assert_eq!(c.stats().bytes_used, 0);
        // Decode errors are returned and nothing is cached.
        assert_eq!(
            c.get_or_decode(SourceKey::Ptr(9), |_| Err(Error::Decode("x")))
                .unwrap_err(),
            Error::Decode("x")
        );
        assert!(!c.contains(&SourceKey::Ptr(9)));
    }

    #[test]
    fn cache_evicts_lru() {
        // Budget for 3 images of 100 bytes.
        let mut c = ImageCache::new(300, 8);
        for k in 1..=3 {
            c.get_or_decode(SourceKey::Ptr(k), |o| l8(o, 10, 0)).unwrap();
        }
        // Touch 1, then add 4: 2 is the least recently used.
        c.get_or_decode(SourceKey::Ptr(1), |o| l8(o, 10, 0)).unwrap();
        c.get_or_decode(SourceKey::Ptr(4), |o| l8(o, 10, 0)).unwrap();
        assert!(!c.contains(&SourceKey::Ptr(2)));
        assert!(c.contains(&SourceKey::Ptr(1)) && c.contains(&SourceKey::Ptr(3)));
        assert_eq!(c.stats().evictions, 1);
        // The entry limit evicts too.
        let mut c = ImageCache::new(10_000, 2);
        for k in 1..=3 {
            c.get_or_decode(SourceKey::Ptr(k), |o| l8(o, 2, 0)).unwrap();
        }
        assert!(!c.contains(&SourceKey::Ptr(1)));
        // A big image evicts several small ones.
        let mut c = ImageCache::new(300, 8);
        for k in 1..=3 {
            c.get_or_decode(SourceKey::Ptr(k), |o| l8(o, 10, 0)).unwrap();
        }
        c.get_or_decode(SourceKey::Ptr(9), |o| l8(o, 15, 0)).unwrap();
        assert_eq!(c.stats().bytes_used, 225);
        assert_eq!(c.stats().evictions, 3);
    }

    #[test]
    fn oversize_is_transient_and_warns_once() {
        twine_testing::init_test_logging();
        let mut c = ImageCache::new(50, 4);
        let img = c.get_or_decode(SourceKey::Ptr(1), |o| l8(o, 10, 5)).unwrap();
        assert_eq!(img.data.len(), 100);
        assert!(!c.contains(&SourceKey::Ptr(1)));
        assert_eq!(c.stats().bytes_used, 0);
        // Decoded again next time.
        c.get_or_decode(SourceKey::Ptr(1), |o| l8(o, 10, 5)).unwrap();
        assert_eq!(c.stats().misses, 2);
        assert_eq!(c.stats().hits, 0);
    }

    #[test]
    fn zero_budget_always_decodes() {
        let mut c = ImageCache::new(0, 0);
        let n = Cell::new(0);
        for _ in 0..5 {
            c.get_or_decode(SourceKey::Ptr(7), |o| {
                n.set(n.get() + 1);
                l8(o, 1, 1)
            })
            .unwrap();
        }
        assert_eq!(n.get(), 5);
        assert_eq!(c.stats().bytes_used, 0);
    }

    #[test]
    fn cache_hits_do_not_allocate() {
        let mut c = ImageCache::new(10_000, 8);
        for k in 0..8 {
            c.get_or_decode(SourceKey::Ptr(k), |o| l8(o, 20, 0)).unwrap();
        }
        let file = SourceKey::File("/a.png".try_into().unwrap());
        c.get_or_decode(file.clone(), |o| l8(o, 5, 0)).unwrap();
        let ((), stats) = count_allocs(|| {
            for k in 1..8 {
                assert_eq!(
                    c.get_or_decode(SourceKey::Ptr(k), |_| Err(Error::NotFound))
                        .unwrap()
                        .data
                        .len(),
                    400
                );
            }
            c.get_or_decode(file.clone(), |_| Err(Error::NotFound)).unwrap();
        });
        assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    }

    #[test]
    fn header_cache_round_robin() {
        let mut hc = ImageHeaderCache::new();
        for k in 0..10 {
            hc.insert(SourceKey::Ptr(k), ImageHeader::new(ColorFormat::L8, k as u16, 1));
        }
        // 0 and 1 were replaced by 8 and 9.
        assert!(hc.get(&SourceKey::Ptr(0)).is_none() && hc.get(&SourceKey::Ptr(1)).is_none());
        assert_eq!(hc.get(&SourceKey::Ptr(9)).unwrap().w, 9);
        assert_eq!(hc.get(&SourceKey::Ptr(2)).unwrap().w, 2);
    }
}

mod resolve {
    use super::*;

    static PIX: [u8; 6] = [1, 2, 3, 4, 5, 6];
    static IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::L8, 3, 2), &PIX);

    struct Ctx {
        cache: ImageCache,
        hc: ImageHeaderCache,
        reg: DecoderRegistry,
    }

    impl Ctx {
        fn new(budget: usize) -> Self {
            Self {
                cache: ImageCache::new(budget, 8),
                hc: ImageHeaderCache::new(),
                reg: DecoderRegistry::with_defaults(),
            }
        }
        fn cx(&mut self) -> ImageContext<'_> {
            ImageContext {
                cache: &mut self.cache,
                header_cache: &mut self.hc,
                registry: &self.reg,
                fs: None,
            }
        }
    }

    #[test]
    fn resolve_static_no_alloc() {
        let mut c = Ctx::new(0);
        let mut cx = c.cx();
        let src = ImageSource::Static(&IMG);
        let (r, stats) = count_allocs(|| with_pixels(&src, &mut cx, |px| (px.w, px.h, px.row(1)[2])));
        assert_eq!(r, Ok((3, 2, 6)));
        assert_eq!(stats.allocs + stats.reallocs, 0);
        assert_eq!(c.cache.stats().misses, 0);
    }

    #[test]
    fn resolve_compressed_uses_cache() {
        let raw: Vec<u8> = (0..64u32 * 4).map(|i| (i / 16) as u8).collect();
        let img: &'static Image = Box::leak(Box::new(Image {
            header: ImageHeader {
                flags: ImageFlags::COMPRESSED,
                ..ImageHeader::new(ColorFormat::Argb8888, 8, 8)
            },
            data: ImageData::Compressed {
                method: Compression::Rle,
                data: leak(rle_compress(&raw, 4)),
                decompressed_size: raw.len() as u32,
            },
        }));
        let mut c = Ctx::new(4096);
        let src = ImageSource::Static(img);
        for _ in 0..3 {
            let got = with_pixels(&src, &mut c.cx(), |px| px.data.to_vec()).unwrap();
            assert_eq!(got, raw);
        }
        let s = c.cache.stats();
        assert_eq!((s.misses, s.hits, s.bytes_used), (1, 2, raw.len()));
        assert_eq!(header_of(&src, &mut c.cx()).unwrap().flags, ImageFlags::empty());
    }

    #[test]
    fn resolve_encoded_qoi() {
        let rgba: Vec<u8> = (0..5 * 4).flat_map(|i| [i as u8 * 10, 0, 255, 255]).collect();
        let file = leak(qoi_encode(&rgba, 5, 4, 3));
        let mut c = Ctx::new(4096);
        let src = ImageSource::Encoded(file);
        let (w, first) = with_pixels(&src, &mut c.cx(), |px| (px.w, px.row(0)[..4].to_vec())).unwrap();
        assert_eq!((w, first), (5, vec![255, 0, 0, 255]));
        with_pixels(&src, &mut c.cx(), |_| ()).unwrap();
        assert_eq!(c.cache.stats().hits, 1);
        assert_eq!(
            header_of(&src, &mut c.cx()).unwrap().format,
            ColorFormat::Xrgb8888
        );
        // Garbage is an error, not a panic.
        let bad = ImageSource::Encoded(b"not an image");
        assert_eq!(with_pixels(&bad, &mut c.cx(), |_| ()), Err(Error::InvalidHeader));
        assert_eq!(header_of(&bad, &mut c.cx()), Err(Error::InvalidHeader));
        // Symbols and SVG are drawn elsewhere.
        assert_eq!(
            with_pixels(&ImageSource::Symbol("x"), &mut c.cx(), |_| ()),
            Err(Error::UnsupportedSource("symbol"))
        );
        assert_eq!(
            header_of(&ImageSource::Svg(b"<svg/>"), &mut c.cx()),
            Err(Error::UnsupportedSource("svg"))
        );
    }

    static PROBES: AtomicU32 = AtomicU32::new(0);
    static DECODES: AtomicU32 = AtomicU32::new(0);

    struct Counting;
    impl Decoder for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
            PROBES.fetch_add(1, Ordering::Relaxed);
            bytes
                .starts_with(b"cnt")
                .then(|| ImageHeader::new(ColorFormat::Argb8888, 7, 3))
        }
        fn decode(&self, _: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
            DECODES.fetch_add(1, Ordering::Relaxed);
            out.resize(7 * 3 * 4, 0);
            Ok(ImageHeader::new(ColorFormat::Argb8888, 7, 3))
        }
    }
    static COUNTING: Counting = Counting;

    #[test]
    fn header_of_without_decode() {
        let mut reg = DecoderRegistry::new();
        reg.register(&COUNTING).unwrap();
        let (mut cache, mut hc) = (ImageCache::new(4096, 4), ImageHeaderCache::new());
        let mut cx = ImageContext {
            cache: &mut cache,
            header_cache: &mut hc,
            registry: &reg,
            fs: None,
        };
        let src = ImageSource::Encoded(b"cnt-data");
        for _ in 0..3 {
            assert_eq!(header_of(&src, &mut cx).unwrap().w, 7);
        }
        assert_eq!(PROBES.load(Ordering::Relaxed), 1, "probed once, then cached");
        assert_eq!(DECODES.load(Ordering::Relaxed), 0);
        with_pixels(&src, &mut cx, |px| assert_eq!(px.h, 3)).unwrap();
        assert_eq!(DECODES.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn file_source_without_fs_is_not_found() {
        let mut c = Ctx::new(4096);
        let src = ImageSource::file("/sd/a.qoi").unwrap();
        assert_eq!(with_pixels(&src, &mut c.cx(), |_| ()), Err(Error::NotFound));
        assert_eq!(header_of(&src, &mut c.cx()), Err(Error::NotFound));
    }

    struct MemFs(Vec<(&'static str, Vec<u8>)>, u32);
    impl FileSource for MemFs {
        fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), Error> {
            self.1 += 1;
            let f = self.0.iter().find(|(p, _)| *p == path).ok_or(Error::NotFound)?;
            out.clear();
            out.extend_from_slice(&f.1);
            Ok(())
        }
    }

    #[test]
    fn file_source_with_fs_decodes_and_caches() {
        let mut fs = MemFs(vec![("/logo.qoi", qoi_encode(&[9, 8, 7, 255], 1, 1, 4))], 0);
        let mut c = Ctx::new(4096);
        let src = ImageSource::file("/logo.qoi").unwrap();
        for _ in 0..2 {
            let mut cx = ImageContext {
                cache: &mut c.cache,
                header_cache: &mut c.hc,
                registry: &c.reg,
                fs: Some(&mut fs),
            };
            let px = with_pixels(&src, &mut cx, |px| px.row(0).to_vec()).unwrap();
            assert_eq!(px, [7, 8, 9, 255]);
            assert_eq!(header_of(&src, &mut cx).unwrap().w, 1);
        }
        assert_eq!(fs.1, 1, "read once: pixels and header are cached");
        let missing = ImageSource::file("/nope.png").unwrap();
        let mut cx = ImageContext {
            cache: &mut c.cache,
            header_cache: &mut c.hc,
            registry: &c.reg,
            fs: Some(&mut fs),
        };
        assert_eq!(with_pixels(&missing, &mut cx, |_| ()), Err(Error::NotFound));
    }
}
