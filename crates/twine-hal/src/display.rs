//! Display traits and descriptions: [`DisplayDriver`], `AsyncDisplayDriver` (feature `async`),
//! [`FramebufferDisplay`], [`DisplayInfo`].
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
//!
//! # Power and rotation at run time
//!
//! Besides flushing, a driver may offer three controls, each a provided method that answers
//! [`ControlError::Unsupported`] unless the driver overrides it:
//!
//! | Control | Method | When the driver does not support it |
//! |---------|--------|-------------------------------------|
//! | brightness | [`DisplayDriver::set_brightness`] | the engine raises `FaultKind::DisplayControl`; panels dimmed by a backlight pin are dimmed by the application (its PWM) |
//! | sleep / wake | [`DisplayDriver::sleep`] | the engine still stops drawing while "asleep" (and raises the fault) |
//! | rotation | [`DisplayDriver::set_rotation`] | the engine rotates in software (needs `Engine::reserve_rotation`) |
//!
//! The engine calls them only between frames, with no flush in flight (never in the middle of
//! a transfer), and never sends anything to the panel before the settle time a
//! [`sleep`](DisplayDriver::sleep) call returned has passed — so drivers need no delay of their
//! own.

pub use twine_core::Rotation;
use twine_core::{ColorFormat, Duration, Fraction, Rect};

use crate::DrawBufferMem;

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

    /// The panel's native (unrotated) size `(width, height)`: the logical size with the axes
    /// swapped back for [`Rotation::Deg90`] and [`Rotation::Deg270`]. Touch transforms
    /// ([`TouchTransform::for_display`](crate::TouchTransform::for_display)) and anything else
    /// that works in panel coordinates start from it.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Rotation};
    /// use twine_hal::DisplayInfo;
    ///
    /// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped).with_rotation(Rotation::Deg90);
    /// assert_eq!(info.native_size(), (240, 320));
    /// assert_eq!(info.with_rotation(Rotation::Deg180).native_size(), (320, 240));
    /// ```
    #[must_use]
    pub const fn native_size(&self) -> (u16, u16) {
        if self.rotation.swaps_axes() {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// Why a display control ([`DisplayDriver::set_brightness`], [`DisplayDriver::sleep`],
/// [`DisplayDriver::set_rotation`] and their async and framebuffer counterparts) was not
/// applied.
///
/// ```
/// use twine_hal::ControlError;
///
/// let e: ControlError<&str> = ControlError::Driver("bus error");
/// assert_eq!(e.map(str::len), ControlError::Driver(9));
/// assert!(ControlError::<()>::Unsupported.is_unsupported());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControlError<E> {
    /// The panel or driver cannot do this (the provided methods' answer). Nothing was sent.
    Unsupported,
    /// The driver tried and failed (bus error, …); the panel's state is unknown.
    Driver(E),
}

impl<E> ControlError<E> {
    /// Maps the driver error with `f` (e.g. to an error code); `Unsupported` stays.
    #[must_use]
    pub fn map<F>(self, f: impl FnOnce(E) -> F) -> ControlError<F> {
        match self {
            ControlError::Unsupported => ControlError::Unsupported,
            ControlError::Driver(e) => ControlError::Driver(f(e)),
        }
    }

    /// Whether this is [`ControlError::Unsupported`].
    #[must_use]
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, ControlError::Unsupported)
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

    /// Static description of the display. Must not change while the engine runs, except
    /// through [`set_rotation`](Self::set_rotation) (which returns the new description).
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
    /// On error the buffer must be returned by the next `poll_flush`. The engine treats the
    /// area as not shown: by default it redraws it on a later frame (its flush policy decides).
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

    /// A numeric code for `error`, reported with the flush fault (the engine's `FaultRecord`
    /// `code` and its `DriverErrorCode`) so the application can tell errors apart without
    /// parsing text. Default: `0` (no code). Called only on the error path.
    fn error_code(&self, error: &Self::Error) -> u32 {
        let _ = error;
        0
    }

    /// Sets the panel brightness (`Fraction::ZERO` = darkest the panel allows, `ONE` =
    /// brightest), for panels that dim themselves (AMOLED/OLED: MIPI DCS `WRDISBV`, SSD1306
    /// contrast). Default: [`ControlError::Unsupported`] — LCDs are dimmed by their backlight,
    /// which the application drives (PWM).
    ///
    /// Called by the engine between frames with no flush in flight (`Engine::set_display_brightness`,
    /// `DisplayCmd::Brightness`).
    ///
    /// # Errors
    /// [`ControlError::Unsupported`] (default), or [`ControlError::Driver`] when sending failed.
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<Self::Error>> {
        let _ = level;
        Err(ControlError::Unsupported)
    }

    /// Puts the panel to sleep (`true`: e.g. MIPI DCS `SLPIN`, frame memory kept) or wakes it
    /// (`false`: `SLPOUT`). Returns the settle time: how long the panel needs before it accepts
    /// the next command or pixels (MIPI DCS: 5 ms after `SLPIN`, 120 ms after `SLPOUT`). The
    /// engine sends nothing to the driver before it has passed, so the driver never blocks.
    /// Default: [`ControlError::Unsupported`].
    ///
    /// # Errors
    /// [`ControlError::Unsupported`] (default), or [`ControlError::Driver`] when sending failed.
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<Self::Error>> {
        let _ = sleep;
        Err(ControlError::Unsupported)
    }

    /// Rotates the picture in hardware (e.g. MIPI DCS `MADCTL`) and returns the new
    /// description: logical size and `rotation` of `rotation`, and `hw_rotation` telling
    /// whether flush areas are now logical (`true`) or still native. The format may not
    /// change. Default: [`ControlError::Unsupported`], and the engine rotates in software
    /// instead (with the scratch buffers `Engine::reserve_rotation` allocated at start-up).
    ///
    /// Called by the engine between frames with no flush in flight; the whole screen is
    /// redrawn afterwards.
    ///
    /// # Errors
    /// [`ControlError::Unsupported`] (default), or [`ControlError::Driver`] when sending failed
    /// (the driver must then keep reporting its previous rotation).
    fn set_rotation(&mut self, rotation: Rotation) -> Result<DisplayInfo, ControlError<Self::Error>> {
        let _ = rotation;
        Err(ControlError::Unsupported)
    }
}

/// Async display driver (embassy / embedded-hal-async), feature `async`.
///
/// The future returned by [`flush`](Self::flush) **MUST start the transfer on its first poll**
/// (before returning `Pending`), so that `join(driver.flush(..), render_next_chunk)` overlaps the
/// DMA transfer with rendering.
///
/// **Bounded by the runtime.** `AsyncUi` races every flush still pending after its first poll
/// against `EngineConfig::flush_timeout` on its [`AsyncPlatform`](crate::AsyncPlatform)'s
/// timer. When the timeout wins, the flush future is **dropped** (so it must be cancel-safe: an
/// embassy DMA transfer is aborted on drop) and the engine raises `FaultKind::FlushTimeout`
/// and marks the display failed, exactly as for a blocking driver whose buffer never comes
/// back. A driver therefore needs no timeout of its own; it may still return an error early
/// when it detects a failure (a bus error, its own shorter deadline), which the engine handles
/// as `FaultKind::FlushError`.
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

    /// A numeric code for `error` (see [`DisplayDriver::error_code`]). Default: `0`.
    fn error_code(&self, error: &Self::Error) -> u32 {
        let _ = error;
        0
    }

    /// Called when the UI has nothing more to draw (after a frame that left nothing due), for
    /// power saving; the next [`flush`](Self::flush) must wake the bus again. Default: completes
    /// immediately. Same contract as [`DisplayDriver::idle`].
    async fn idle(&mut self) {}

    /// Sets the panel brightness (see [`DisplayDriver::set_brightness`]). Default:
    /// [`ControlError::Unsupported`].
    ///
    /// # Errors
    /// As [`DisplayDriver::set_brightness`].
    async fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<Self::Error>> {
        let _ = level;
        Err(ControlError::Unsupported)
    }

    /// Sleeps (`true`) or wakes (`false`) the panel and returns its settle time (see
    /// [`DisplayDriver::sleep`]). Default: [`ControlError::Unsupported`].
    ///
    /// # Errors
    /// As [`DisplayDriver::sleep`].
    async fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<Self::Error>> {
        let _ = sleep;
        Err(ControlError::Unsupported)
    }

    /// Rotates in hardware and returns the new description (see
    /// [`DisplayDriver::set_rotation`]). Default: [`ControlError::Unsupported`] (software
    /// rotation).
    ///
    /// # Errors
    /// As [`DisplayDriver::set_rotation`].
    async fn set_rotation(&mut self, rotation: Rotation) -> Result<DisplayInfo, ControlError<Self::Error>> {
        let _ = rotation;
        Err(ControlError::Unsupported)
    }
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
    ///
    /// On error the engine assumes nothing was scanned out: it does not swap, keeps rendering
    /// into the same back buffer and never waits for [`present_done`](Self::present_done) of
    /// the failed call.
    fn present(&mut self, index: u8) -> Result<(), Self::Error>;

    /// `true` once the last [`present`](Self::present) has taken effect (the buffer swap
    /// happened and the previous buffer may be drawn into).
    fn present_done(&mut self) -> bool;

    /// A numeric code for `error` (see [`DisplayDriver::error_code`]). Default: `0`.
    fn error_code(&self, error: &Self::Error) -> u32 {
        let _ = error;
        0
    }

    /// Sets the panel brightness (see [`DisplayDriver::set_brightness`]). Default:
    /// [`ControlError::Unsupported`]. Framebuffer displays cannot be rotated at run time.
    ///
    /// # Errors
    /// As [`DisplayDriver::set_brightness`].
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<Self::Error>> {
        let _ = level;
        Err(ControlError::Unsupported)
    }

    /// Sleeps (`true`) or wakes (`false`) the panel and returns its settle time (see
    /// [`DisplayDriver::sleep`]). Default: [`ControlError::Unsupported`].
    ///
    /// # Errors
    /// As [`DisplayDriver::sleep`].
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<Self::Error>> {
        let _ = sleep;
        Err(ControlError::Unsupported)
    }
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
}
