//! [`EgDisplay`]: any embedded-graphics [`DrawTarget`] as a twine [`DisplayDriver`].

use core::fmt;

use embedded_graphics_core::draw_target::DrawTarget;
use embedded_graphics_core::geometry::{Point, Size};
use embedded_graphics_core::primitives::Rectangle;
use twine_core::Rect;
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};

use crate::EgColor;

/// Error of an [`EgDisplay`] flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EgError<E> {
    /// The draw target failed.
    Target(E),
    /// The flushed buffer is shorter than the area needs.
    BufferTooSmall {
        /// Bytes needed for the area.
        needed: usize,
        /// Bytes in the buffer.
        got: usize,
    },
}

impl<E: fmt::Debug> fmt::Display for EgError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EgError::Target(e) => write!(f, "draw target error: {e:?}"),
            EgError::BufferTooSmall { needed, got } => {
                write!(f, "flush buffer too small: {got} bytes, {needed} needed")
            }
        }
    }
}

impl<E: fmt::Debug> core::error::Error for EgError<E> {}

/// An embedded-graphics [`DrawTarget`] used as a blocking twine display.
///
/// The display size is the target's bounding box; the pixel format follows the target's colour
/// type ([`EgColor`]). Each flush converts the rendered rows into colours and hands them to
/// [`DrawTarget::fill_contiguous`] for the flushed rectangle (offset by the bounding box origin).
/// The flush completes before `begin_flush` returns, so `poll_flush` hands the buffer back
/// immediately.
///
/// ```
/// use core::convert::Infallible;
/// use embedded_graphics_core::{pixelcolor::Gray8, prelude::*};
/// use twine_core::{ColorFormat, Rect};
/// use twine_embedded_graphics::EgDisplay;
/// use twine_hal::{DisplayDriver, DrawBufferMem};
///
/// /// A 4×2 grayscale frame buffer.
/// struct Fb([u8; 8]);
/// impl OriginDimensions for Fb {
///     fn size(&self) -> Size { Size::new(4, 2) }
/// }
/// impl DrawTarget for Fb {
///     type Color = Gray8;
///     type Error = Infallible;
///     fn draw_iter<I: IntoIterator<Item = Pixel<Gray8>>>(&mut self, px: I) -> Result<(), Infallible> {
///         for Pixel(p, c) in px {
///             self.0[(p.y * 4 + p.x) as usize] = c.luma();
///         }
///         Ok(())
///     }
/// }
///
/// let mut d = EgDisplay::new(Fb([0; 8]));
/// assert_eq!(d.info().format, ColorFormat::L8);
/// let buf = DrawBufferMem::new(Box::leak(Box::new([7u8, 8])));
/// d.begin_flush(Rect::from_xywh(1, 1, 2, 1), buf).unwrap();
/// assert!(d.poll_flush().is_some());
/// assert_eq!(d.target().0, [0, 0, 0, 0, 0, 7, 8, 0]);
/// ```
#[derive(Debug)]
pub struct EgDisplay<T: DrawTarget>
where
    T::Color: EgColor,
{
    target: T,
    info: DisplayInfo,
    returned: Option<DrawBufferMem>,
}

impl<T: DrawTarget> EgDisplay<T>
where
    T::Color: EgColor,
{
    /// Wraps `target`. Size and format are read once, here.
    pub fn new(target: T) -> Self {
        let size = target.bounding_box().size;
        let dim = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
        let format = <T::Color as EgColor>::FORMAT;
        let mut info = DisplayInfo::new(dim(size.width), dim(size.height), format);
        if format.bpp() < 8 {
            info = info.with_align(8); // whole bytes per row start
        }
        Self {
            target,
            info,
            returned: None,
        }
    }

    /// Replaces the display description (e.g. to set the DPI). Width, height and format must
    /// stay those of the target.
    #[must_use]
    pub fn with_info(mut self, f: impl FnOnce(DisplayInfo) -> DisplayInfo) -> Self {
        let (w, h, format) = (self.info.width, self.info.height, self.info.format);
        self.info = f(self.info);
        self.info.width = w;
        self.info.height = h;
        self.info.format = format;
        self
    }

    /// The draw target.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// The draw target, mutably (e.g. to call a simulator window's `update`).
    pub fn target_mut(&mut self) -> &mut T {
        &mut self.target
    }

    /// Unwraps the draw target.
    pub fn into_inner(self) -> T {
        self.target
    }

    /// Sends the pixels of `area` (`buf` rows packed) to the target.
    fn draw(&mut self, area: Rect, buf: &[u8]) -> Result<(), EgError<T::Error>> {
        if area.is_empty() {
            return Ok(());
        }
        let (w, h) = (area.width() as usize, area.height() as usize);
        let stride = <T::Color as EgColor>::FORMAT.stride(w as u32) as usize;
        let needed = stride * h;
        if buf.len() < needed {
            return Err(EgError::BufferTooSmall {
                needed,
                got: buf.len(),
            });
        }
        let origin = self.target.bounding_box().top_left;
        let rect = Rectangle::new(
            origin + Point::new(area.x0, area.y0),
            Size::new(w as u32, h as u32),
        );
        let colors = buf[..needed]
            .chunks_exact(stride)
            .flat_map(move |row| (0..w).map(move |x| <T::Color as EgColor>::from_row(row, x)));
        self.target
            .fill_contiguous(&rect, colors)
            .map_err(EgError::Target)
    }
}

impl<T: DrawTarget> DisplayDriver for EgDisplay<T>
where
    T::Color: EgColor,
    T::Error: fmt::Debug,
{
    type Error = EgError<T::Error>;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let r = self.draw(area, buf.as_slice());
        if r.is_err() {
            twine_core::warn!(target: "twine::eg", "[eg] flush of {} failed", area);
        }
        // Blocking: done either way, the buffer goes back on the next poll.
        self.returned = Some(buf);
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.returned.take()
    }
}
