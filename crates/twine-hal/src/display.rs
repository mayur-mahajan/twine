//! Display traits and descriptions: [`DisplayDriver`], `AsyncDisplayDriver` (feature `async`),
//! [`FramebufferDisplay`], [`DisplayInfo`], [`BufferSpec`].
//!
//! This module is the complete contract between the Twine engine and a display driver. A driver
//! author needs nothing else.
//!
//! # Which trait to implement
//!
//! | Panel | Trait |
//! |-------|-------|
//! | Panel with its own frame memory (GRAM) on SPI / i80, blocking or DMA | [`DisplayDriver`] |
//! | Same, driven from an async executor (embassy) | `AsyncDisplayDriver` (feature `async`) |
//! | Memory-mapped panel scanned out of MCU RAM (LTDC, RGB, DSI, Linux fb) | [`FramebufferDisplay`] |
//!
//! # Pixel layout of a flush
//!
//! The pixels of `area` are row-major and tightly packed: row `r` starts at byte
//! `r * stride` with `stride = area.width() * bpp / 8` (for sub-byte formats rounded up to whole
//! bytes per row, see [`ColorFormat::stride`]). The format is [`DisplayInfo::format`]. The buffer
//! may be longer than `stride * area.height()`; trailing bytes are ignored.
//!
//! # Coordinates and rotation
//!
//! Areas use [`Rect`] (half-open). When [`DisplayInfo::hw_rotation`] is `true`,
//! `area` is in **logical** (rotated) coordinates and the driver/controller rotates (e.g. MIPI
//! DCS `MADCTL`). Otherwise the engine rotates the pixels in software and `area` is in physical
//! panel coordinates.

pub use twine_core::Rotation;
use twine_core::{ColorFormat, Rect};

use crate::DrawBufferMem;

/// How the simulator and the test harnesses should allocate draw buffers for a display.
///
/// This is only a *request*; the engine's `BufferMode` holds the actual memory.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::{BufferSpec, DisplayInfo};
///
/// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565);
/// assert_eq!(BufferSpec::default(), BufferSpec::PartialDouble { rows: 40 });
/// assert_eq!(BufferSpec::default().bytes_per_buffer(&info), 320 * 2 * 40);
/// assert_eq!(BufferSpec::Full.bytes_per_buffer(&info), 320 * 2 * 240);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BufferSpec {
    /// One partial buffer of `rows` full-width rows: render, flush, wait, repeat.
    PartialSingle {
        /// Rows per buffer.
        rows: u16,
    },
    /// Two partial buffers of `rows` rows each (ping-pong with DMA).
    PartialDouble {
        /// Rows per buffer.
        rows: u16,
    },
    /// Two full-screen framebuffers (memory-mapped panels, swapped at vsync).
    Full,
    /// One full-screen framebuffer rendered in place.
    Direct,
}

impl Default for BufferSpec {
    fn default() -> Self {
        BufferSpec::PartialDouble { rows: 40 }
    }
}

impl BufferSpec {
    /// Number of buffers of this layout (1 or 2).
    #[must_use]
    pub const fn buffer_count(&self) -> usize {
        match self {
            BufferSpec::PartialSingle { .. } | BufferSpec::Direct => 1,
            BufferSpec::PartialDouble { .. } | BufferSpec::Full => 2,
        }
    }

    /// Bytes of **one** buffer for the display `info`.
    ///
    /// Partial rows are clamped to `1..=info.height` (a buffer taller than the screen is
    /// useless, a zero-row buffer cannot render anything).
    #[must_use]
    pub fn bytes_per_buffer(&self, info: &DisplayInfo) -> usize {
        let rows = match *self {
            BufferSpec::PartialSingle { rows } | BufferSpec::PartialDouble { rows } => {
                rows.clamp(1, info.height.max(1)).min(info.height)
            }
            BufferSpec::Full | BufferSpec::Direct => info.height,
        };
        info.bytes_per_row() * usize::from(rows)
    }
}

/// Static description of a display, returned by the drivers' `info()`.
///
/// ```
/// use twine_core::{ColorFormat, Rotation};
/// use twine_hal::DisplayInfo;
///
/// let info = DisplayInfo::new(240, 320, ColorFormat::Rgb565Swapped)
///     .with_rotation(Rotation::Deg90)
///     .with_hw_rotation(true)
///     .with_align(2);
/// assert_eq!(info.bytes_per_row(), 480);
/// assert_eq!(info.dpi, 130);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisplayInfo {
    /// Width in pixels, **after** rotation (logical).
    pub width: u16,
    /// Height in pixels, **after** rotation (logical).
    pub height: u16,
    /// Native pixel format (e.g. `Rgb565Swapped` for SPI MIPI panels).
    pub format: ColorFormat,
    /// Rotation of the logical screen relative to the panel.
    pub rotation: Rotation,
    /// `true`: the panel rotates in hardware and flush areas are logical; `false`: the engine
    /// rotates each chunk in software before flushing.
    pub hw_rotation: bool,
    /// Flush areas' x, y, width and height must be multiples of this (1, 2, 8…; 0 is treated
    /// as 1 by the engine).
    pub align: u8,
    /// Dots per inch (used to scale default sizes, LVGL default 130).
    pub dpi: u16,
}

impl DisplayInfo {
    /// A display of `width × height` in `format` with defaults: rotation `Deg0`,
    /// `hw_rotation = false`, `align = 1`, `dpi = 130`.
    #[must_use]
    pub const fn new(width: u16, height: u16, format: ColorFormat) -> Self {
        Self {
            width,
            height,
            format,
            rotation: Rotation::Deg0,
            hw_rotation: false,
            align: 1,
            dpi: 130,
        }
    }

    /// Sets the rotation.
    #[must_use]
    pub const fn with_rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets whether the panel rotates in hardware.
    #[must_use]
    pub const fn with_hw_rotation(mut self, hw_rotation: bool) -> Self {
        self.hw_rotation = hw_rotation;
        self
    }

    /// Sets the flush area alignment.
    #[must_use]
    pub const fn with_align(mut self, align: u8) -> Self {
        self.align = align;
        self
    }

    /// Sets the DPI.
    #[must_use]
    pub const fn with_dpi(mut self, dpi: u16) -> Self {
        self.dpi = dpi;
        self
    }

    /// Bytes of one full-width row in [`format`](Self::format) (sub-byte formats round up).
    #[must_use]
    pub const fn bytes_per_row(&self) -> usize {
        self.format.stride(self.width as u32) as usize
    }

    /// The whole (logical) screen as a rectangle at the origin.
    #[must_use]
    pub const fn area(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }
}

/// A blocking or DMA-capable display driver with an embedded frame memory (GRAM).
///
/// # Buffer ownership
///
/// [`begin_flush`](Self::begin_flush) **moves** the buffer into the driver.
/// The engine never touches a buffer between `begin_flush` and the
/// [`poll_flush`](Self::poll_flush) call that returns it. A driver that holds several
/// buffers returns them in the order they were submitted.
///
/// If `begin_flush` returns an error, the driver must still hand the buffer back through the
/// next `poll_flush` call, so the engine never loses a draw buffer.
///
/// # Example: a blocking driver
///
/// ```
/// use twine_core::{ColorFormat, Rect};
/// use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
///
/// /// A 4×2 RGB565 "panel" backed by an array.
/// struct Fake {
///     gram: [u8; 4 * 2 * 2],
///     returned: Option<DrawBufferMem>,
/// }
///
/// impl DisplayDriver for Fake {
///     type Error = ();
///     fn info(&self) -> DisplayInfo {
///         DisplayInfo::new(4, 2, ColorFormat::Rgb565)
///     }
///     fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), ()> {
///         let stride = area.width() as usize * 2;
///         for (r, row) in buf.as_slice().chunks(stride).take(area.height() as usize).enumerate() {
///             let start = ((area.y0 as usize + r) * 4 + area.x0 as usize) * 2;
///             self.gram[start..start + stride].copy_from_slice(row);
///         }
///         self.returned = Some(buf); // blocking: done before returning
///         Ok(())
///     }
///     fn poll_flush(&mut self) -> Option<DrawBufferMem> {
///         self.returned.take()
///     }
/// }
///
/// let mut d = Fake { gram: [0; 16], returned: None };
/// let buf = DrawBufferMem::new(Box::leak(Box::new([0xFFu8; 4])));
/// d.begin_flush(Rect::from_xywh(1, 1, 2, 1), buf).unwrap();
/// let buf = d.poll_flush().expect("blocking driver returns the buffer immediately");
/// assert_eq!(buf.len(), 4);
/// assert_eq!(&d.gram[10..14], &[0xFF; 4]);
/// ```
pub trait DisplayDriver {
    /// The driver's error type (bus errors, invalid areas…).
    type Error: core::fmt::Debug;

    /// Static description of the display. Must not change while the engine runs.
    fn info(&self) -> DisplayInfo;

    /// Starts sending `buf` (the pixels of `area`, row-major, native format, `stride =
    /// area.width() * bpp / 8`) to the panel.
    ///
    /// - A DMA-capable driver starts the transfer and returns immediately, keeping `buf`.
    /// - A blocking driver sends everything before returning.
    ///
    /// `area` is in logical coordinates when [`DisplayInfo::hw_rotation`] is `true` and lies
    /// within the screen; its x/y/width/height are multiples of [`DisplayInfo::align`]. The
    /// engine calls this only when it holds the buffer, and at most as many times without an
    /// intervening successful [`poll_flush`](Self::poll_flush) as it has buffers.
    ///
    /// On error the buffer must be returned by the next `poll_flush`.
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error>;

    /// Polls for completion of the oldest flush in progress.
    ///
    /// Returns its buffer once the transfer has finished (the panel shows or has latched the
    /// pixels), `None` while it is still running or when nothing is in flight. Blocking drivers
    /// return the buffer on the first call. Must not block.
    fn poll_flush(&mut self) -> Option<DrawBufferMem>;

    /// Blocks until the panel's tearing-effect / vsync signal. Default: no-op.
    ///
    /// Called before the first chunk of a frame when the display is configured with vsync.
    fn wait_vsync(&mut self) {}

    /// Called when the refresher becomes idle, for power saving (e.g. put the bus to sleep).
    /// Default: no-op. The next `begin_flush` must wake the bus again.
    fn idle(&mut self) {}
}

/// Async display driver (embassy / embedded-hal-async), feature `async`.
///
/// The future returned by [`flush`](Self::flush) **MUST start the transfer on its first poll**
/// (before returning `Pending`), so that `join(driver.flush(..), render_next_chunk)` overlaps the
/// DMA transfer with rendering.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AsyncDisplayDriver {
    /// The driver's error type.
    type Error: core::fmt::Debug;

    /// Static description of the display.
    fn info(&self) -> DisplayInfo;

    /// Sends the pixels of `area` (layout as for [`DisplayDriver::begin_flush`]) and completes
    /// when the transfer has finished; `buf` is borrowed for the whole transfer.
    async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), Self::Error>;

    /// Waits for the tearing-effect / vsync signal. Default: completes immediately.
    async fn wait_vsync(&mut self) {}
}

/// A memory-mapped panel with one or two framebuffers owned by the driver / display controller
/// (STM32 LTDC, ESP32 RGB/DSI, Linux framebuffer).
pub trait FramebufferDisplay {
    /// The driver's error type.
    type Error: core::fmt::Debug;

    /// Static description of the display.
    fn info(&self) -> DisplayInfo;

    /// Hands out the framebuffers: `Some((front, back))` on the **first** call, `None` on every
    /// later call (handing them out once avoids aliasing `&'static mut` memory). Each buffer
    /// holds `info().bytes_per_row() * info().height` bytes.
    fn framebuffers(&mut self) -> Option<(DrawBufferMem, Option<DrawBufferMem>)>;

    /// Scans out framebuffer `index` (0 = first, 1 = second) from the next vsync on.
    /// Non-blocking.
    fn present(&mut self, index: u8) -> Result<(), Self::Error>;

    /// `true` once the last [`present`](Self::present) has taken effect (the buffer swap
    /// happened and the previous buffer may be drawn into).
    fn present_done(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_swaps_axes() {
        assert!(!Rotation::Deg0.swaps_axes());
        assert!(Rotation::Deg90.swaps_axes());
        assert!(!Rotation::Deg180.swaps_axes());
        assert!(Rotation::Deg270.swaps_axes());
        assert_eq!(Rotation::Deg270.degrees(), 270);
    }

    #[test]
    fn display_info_defaults() {
        let i = DisplayInfo::new(320, 240, ColorFormat::Rgb565);
        assert_eq!(
            (
                i.width,
                i.height,
                i.format,
                i.rotation,
                i.hw_rotation,
                i.align,
                i.dpi
            ),
            (320, 240, ColorFormat::Rgb565, Rotation::Deg0, false, 1, 130)
        );
        let j = i
            .with_rotation(Rotation::Deg180)
            .with_hw_rotation(true)
            .with_align(8)
            .with_dpi(200);
        assert_eq!(
            (j.rotation, j.hw_rotation, j.align, j.dpi),
            (Rotation::Deg180, true, 8, 200)
        );
        assert_eq!(i.area(), Rect::new(0, 0, 320, 240));
    }

    #[test]
    fn bytes_per_row_rgb565() {
        assert_eq!(
            DisplayInfo::new(320, 240, ColorFormat::Rgb565).bytes_per_row(),
            640
        );
        assert_eq!(DisplayInfo::new(128, 64, ColorFormat::I1).bytes_per_row(), 16);
        assert_eq!(DisplayInfo::new(129, 64, ColorFormat::I1).bytes_per_row(), 17);
        assert_eq!(DisplayInfo::new(10, 1, ColorFormat::Rgb888).bytes_per_row(), 30);
    }

    #[test]
    fn buffer_spec_sizes() {
        let info = DisplayInfo::new(100, 50, ColorFormat::Rgb565);
        assert_eq!(
            BufferSpec::PartialSingle { rows: 10 }.bytes_per_buffer(&info),
            2000
        );
        assert_eq!(
            BufferSpec::PartialDouble { rows: 500 }.bytes_per_buffer(&info),
            10_000
        );
        assert_eq!(BufferSpec::PartialDouble { rows: 0 }.bytes_per_buffer(&info), 200);
        assert_eq!(BufferSpec::Direct.bytes_per_buffer(&info), 10_000);
        assert_eq!(BufferSpec::Full.buffer_count(), 2);
        assert_eq!(BufferSpec::PartialSingle { rows: 1 }.buffer_count(), 1);
        let empty = DisplayInfo::new(0, 0, ColorFormat::Rgb565);
        assert_eq!(BufferSpec::default().bytes_per_buffer(&empty), 0);
    }
}
