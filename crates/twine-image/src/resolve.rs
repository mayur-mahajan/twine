//! Turning any [`ImageSource`] into drawable pixels: [`with_pixels`] and [`header_of`].

use alloc::vec::Vec;

use twine_render::ImagePixels;

use crate::{
    DecoderRegistry, Error, Image, ImageCache, ImageData, ImageFlags, ImageHeader, ImageHeaderCache, ImageSource,
    SourceKey,
};

/// Reads whole files for [`ImageSource::File`] (implemented over the file system by the
/// engine; `twine-image` has no file system of its own).
pub trait FileSource {
    /// Replaces the contents of `out` with the file at `path`; [`Error::NotFound`] when it
    /// does not exist.
    fn read_all(&mut self, path: &str, out: &mut Vec<u8>) -> Result<(), Error>;
}

/// Everything needed to resolve image sources.
pub struct ImageContext<'a> {
    /// Decoded / decompressed pixels.
    pub cache: &'a mut ImageCache,
    /// Probed headers.
    pub header_cache: &'a mut ImageHeaderCache,
    /// Decoders for encoded sources.
    pub registry: &'a DecoderRegistry,
    /// File access (`None`: file sources are not found).
    pub fs: Option<&'a mut dyn FileSource>,
}

impl core::fmt::Debug for ImageContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageContext")
            .field("cache", &self.cache.stats())
            .field("fs", &self.fs.is_some())
            .finish_non_exhaustive()
    }
}

fn no_fs() -> Error {
    twine_core::warn!(target: "twine::image", "file image source without a file system");
    Error::NotFound
}

/// Reads the file `path` through `fs`.
fn read_file<'f>(fs: Option<&mut (dyn FileSource + 'f)>, path: &str) -> Result<Vec<u8>, Error> {
    let fs = fs.ok_or_else(no_fs)?;
    let mut bytes = Vec::new();
    fs.read_all(path, &mut bytes)?;
    Ok(bytes)
}

/// Calls `f` with the pixels of `src`.
///
/// - uncompressed static images are used in place (no allocation, no cache);
/// - compressed static images are decompressed into the cache;
/// - encoded bytes and files are decoded through the registry into the cache;
/// - symbols and SVG are not images here ([`Error::UnsupportedSource`]; the engine draws them).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{DecoderRegistry, Image, ImageCache, ImageContext, ImageHeader, ImageHeaderCache, ImageSource, with_pixels};
///
/// static IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::L8, 2, 1), &[1, 2]);
/// let (mut cache, mut hc, reg) = (ImageCache::new(0, 0), ImageHeaderCache::new(), DecoderRegistry::new());
/// let mut cx = ImageContext { cache: &mut cache, header_cache: &mut hc, registry: &reg, fs: None };
/// let w = with_pixels(&ImageSource::Static(&IMG), &mut cx, |px| px.w).unwrap();
/// assert_eq!(w, 2);
/// ```
pub fn with_pixels<R>(
    src: &ImageSource,
    cx: &mut ImageContext<'_>,
    f: impl FnOnce(&ImagePixels<'_>) -> R,
) -> Result<R, Error> {
    let ImageContext {
        cache,
        header_cache,
        registry,
        fs,
    } = cx;
    let cached = match src {
        ImageSource::Static(img) => {
            if let ImageData::Compressed { .. } = img.data {
                cache.get_or_decode(SourceKey::of_ptr(*img), |out| decompress(img, out))?
            } else {
                let px = img.pixels().ok_or(Error::SizeMismatch {
                    expected: img.header.data_size(),
                    got: img.bytes().map_or(0, <[u8]>::len),
                })?;
                return Ok(f(&px));
            }
        }
        ImageSource::Encoded(bytes) => {
            let key = SourceKey::of_ptr(*bytes);
            let c = cache.get_or_decode(key.clone(), |out| registry.decode(bytes, out))?;
            header_cache.insert(key, c.header);
            c
        }
        ImageSource::File(path) => {
            let key = SourceKey::File(path.clone());
            let c = cache.get_or_decode(key.clone(), |out| {
                let bytes = read_file(fs.as_deref_mut(), path)?;
                registry.decode(&bytes, out)
            })?;
            header_cache.insert(key, c.header);
            c
        }
        ImageSource::Symbol(_) => return Err(Error::UnsupportedSource("symbol")),
        ImageSource::Svg(_) => return Err(Error::UnsupportedSource("svg")),
    };
    let px = cached.pixels().ok_or(Error::InvalidHeader)?;
    Ok(f(&px))
}

fn decompress(img: &Image, out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
    let n = img.header.data_size();
    out.try_reserve_exact(n)
        .map_err(|_| Error::Decode("out of memory"))?;
    out.resize(n, 0);
    img.decompress_into(out)
}

/// The header of `src` without decoding pixels (probing encoded data and files, cached in
/// the header cache). The `COMPRESSED` flag is cleared: the header describes the pixels.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_image::{DecoderRegistry, Image, ImageCache, ImageContext, ImageHeader, ImageHeaderCache, ImageSource, header_of};
///
/// static IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 5, 3), &[0; 15]);
/// let (mut cache, mut hc, reg) = (ImageCache::new(0, 0), ImageHeaderCache::new(), DecoderRegistry::new());
/// let mut cx = ImageContext { cache: &mut cache, header_cache: &mut hc, registry: &reg, fs: None };
/// assert_eq!(header_of(&ImageSource::Static(&IMG), &mut cx).unwrap().w, 5);
/// ```
pub fn header_of(src: &ImageSource, cx: &mut ImageContext<'_>) -> Result<ImageHeader, Error> {
    let (key, bytes) = match src {
        ImageSource::Static(img) => {
            let mut h = img.header;
            h.flags.remove(ImageFlags::COMPRESSED);
            return Ok(h);
        }
        ImageSource::Encoded(bytes) => (SourceKey::of_ptr(*bytes), None),
        ImageSource::File(path) => {
            let key = SourceKey::File(path.clone());
            if let Some(h) = cx.header_cache.get(&key) {
                return Ok(h);
            }
            (key, Some(read_file(cx.fs.as_deref_mut(), path)?))
        }
        ImageSource::Symbol(_) => return Err(Error::UnsupportedSource("symbol")),
        ImageSource::Svg(_) => return Err(Error::UnsupportedSource("svg")),
    };
    if let Some(h) = cx.header_cache.get(&key) {
        return Ok(h);
    }
    let data: &[u8] = match (&bytes, src) {
        (Some(b), _) => b,
        (None, ImageSource::Encoded(b)) => b,
        _ => return Err(Error::InvalidHeader),
    };
    let (_, h) = cx.registry.probe(data).ok_or(Error::InvalidHeader)?;
    cx.header_cache.insert(key, h);
    Ok(h)
}
