//! [`SimDisplay`]: the simulator's emulated panel, a real [`DisplayDriver`].

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use twine_core::{Color, ColorFormat, Rect, Rotation};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};

use crate::config::{SUPPORTED_FORMATS, SimConfig};
use crate::convert;

/// Errors of [`SimDisplay::begin_flush`]. The rejected buffer is returned by the next
/// `poll_flush` (see the [`DisplayDriver`] contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimDisplayError {
    /// Two flushes are already queued (the engine never has more than two draw buffers).
    Busy,
    /// The area is empty or not fully inside the panel.
    OutOfBounds(Rect),
    /// The buffer holds fewer bytes than the area needs.
    BufferTooSmall {
        /// Bytes needed.
        needed: usize,
        /// Bytes in the buffer.
        got: usize,
    },
}

impl fmt::Display for SimDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("two flushes are already in flight"),
            Self::OutOfBounds(r) => write!(f, "flush area {r} is empty or outside the panel"),
            Self::BufferTooSmall { needed, got } => {
                write!(f, "draw buffer too small: {got} bytes, area needs {needed}")
            }
        }
    }
}

impl std::error::Error for SimDisplayError {}

/// Statistics of one flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlushStat {
    /// The flushed area.
    pub area: Rect,
    /// Pixel bytes transferred.
    pub bytes: usize,
    /// Wall-clock start of the flush.
    pub started: StdInstant,
}

#[derive(Debug)]
struct InFlight {
    buf: DrawBufferMem,
    stat: FlushStat,
    done_at: StdInstant,
}

/// The emulated panel: a framebuffer in the panel's native format, so byte-order and
/// quantisation bugs are visible, with optional bus-speed emulation.
///
/// - Rotation is emulated like MIPI `MADCTL` by default: [`DisplayInfo::hw_rotation`] is
///   `true` and the panel stores pixels in logical orientation. With
///   [`with_hw_rotation(false)`](Self::with_hw_rotation) the panel keeps its physical
///   orientation (width and height swapped for 90°/270°) and the engine rotates in software.
/// - Without `bus_hz` a flush completes immediately. With `bus_hz` it is "in flight" for
///   `bytes * 8 / bus_hz` seconds of wall-clock time: `poll_flush` returns `None` until then and
///   the pixels become visible only when the transfer completes. Like a DMA driver with a
///   queue, a second flush may be started while one is in flight; it is transferred after the
///   first.
///
/// ```
/// use twine_core::{Color, ColorFormat, Rect, Rotation};
/// use twine_hal::{DisplayDriver, DrawBufferMem};
/// use twine_sim::SimDisplay;
///
/// let mut d = SimDisplay::new(4, 2, ColorFormat::Rgb565Swapped, Rotation::Deg0);
/// let buf = DrawBufferMem::new(Box::leak(Box::new([0xF8, 0x00])));
/// d.begin_flush(Rect::from_xywh(1, 0, 1, 1), buf).unwrap();
/// assert!(d.poll_flush().is_some());
/// assert_eq!(&d.panel_rgb888()[3..6], &[255, 0, 0]);
/// ```
#[derive(Debug)]
pub struct SimDisplay {
    info: DisplayInfo,
    stride: usize,
    panel: Vec<u8>,
    bus_hz: Option<u32>,
    mono: (Color, Color),
    in_flight: VecDeque<InFlight>,
    ready: VecDeque<DrawBufferMem>,
    /// Physical panel size.
    panel_w: u16,
    panel_h: u16,
    dirty: Option<Rect>,
    last: Option<FlushStat>,
    flushes: u64,
}

impl SimDisplay {
    /// A `width × height` panel in `format` (unsupported formats fall back to `Rgb565` with a
    /// warning), cleared to zero bytes, without bus emulation.
    #[must_use]
    pub fn new(width: u16, height: u16, format: ColorFormat, rotation: Rotation) -> Self {
        let format = if SUPPORTED_FORMATS.contains(&format) {
            format
        } else {
            log::warn!(target: "twine::sim", "SimDisplay: unsupported format {format}, using RGB565");
            ColorFormat::Rgb565
        };
        let info = DisplayInfo::new(width, height, format)
            .with_rotation(rotation)
            .with_hw_rotation(true);
        let stride = info.bytes_per_row();
        Self {
            info,
            stride,
            panel: vec![0; stride * usize::from(height)],
            bus_hz: None,
            mono: (Color::BLACK, Color::WHITE),
            in_flight: VecDeque::new(),
            ready: VecDeque::new(),
            panel_w: width,
            panel_h: height,
            dirty: None,
            last: None,
            flushes: 0,
        }
    }

    /// The panel of `cfg` (size, format, rotation, hardware or software rotation, bus speed,
    /// mono colors; flush areas aligned to 8 for `I1`, like page-based mono panels).
    #[must_use]
    pub fn from_config(cfg: &SimConfig) -> Self {
        let mut d = Self::new(cfg.width, cfg.height, cfg.format, cfg.rotation)
            .with_hw_rotation(cfg.hw_rotation)
            .with_bus_hz(cfg.bus_hz)
            .with_mono_colors(cfg.mono_colors.0, cfg.mono_colors.1);
        if d.info.format == ColorFormat::I1 {
            d.info.align = 8;
        }
        d
    }

    /// `false`: the panel keeps its physical orientation and the engine rotates in software
    /// (`DisplayInfo::hw_rotation == false`).
    #[must_use]
    pub fn with_hw_rotation(mut self, hw: bool) -> Self {
        self.info.hw_rotation = hw;
        let (w, h) = (self.info.width, self.info.height);
        (self.panel_w, self.panel_h) = if !hw && self.info.rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        };
        self.stride = self.info.format.stride(u32::from(self.panel_w)) as usize;
        self.panel = vec![0; self.stride * usize::from(self.panel_h)];
        self
    }

    /// The physical panel size (see [`with_hw_rotation`](Self::with_hw_rotation)).
    #[must_use]
    pub fn panel_size(&self) -> (u16, u16) {
        (self.panel_w, self.panel_h)
    }

    /// Emulates a bus of `hz` bits per second (`None` or 0 = instant).
    #[must_use]
    pub fn with_bus_hz(mut self, hz: Option<u32>) -> Self {
        self.bus_hz = hz.filter(|h| *h > 0);
        self
    }

    /// Colors of `0` and `1` bits of an `I1` panel.
    #[must_use]
    pub fn with_mono_colors(mut self, zero: Color, one: Color) -> Self {
        self.mono = (zero, one);
        self
    }

    /// The raw panel memory (native format, stride = [`DisplayInfo::bytes_per_row`]).
    #[must_use]
    pub fn panel(&self) -> &[u8] {
        &self.panel
    }

    /// The panel as tightly packed 8-bit RGB.
    #[must_use]
    pub fn panel_rgb888(&self) -> Vec<u8> {
        convert::to_rgb888(
            &self.panel,
            self.info.format,
            usize::from(self.panel_w),
            usize::from(self.panel_h),
            self.stride,
            self.mono,
        )
    }

    /// The panel as `0x00RRGGBB` words, written into `out` (allocation reused).
    pub fn panel_xrgb_into(&self, out: &mut Vec<u32>) {
        convert::to_xrgb(
            &self.panel,
            self.info.format,
            usize::from(self.panel_w),
            usize::from(self.panel_h),
            self.stride,
            self.mono,
            out,
        );
    }

    /// The union of the areas that became visible since the last call.
    pub fn take_dirty(&mut self) -> Option<Rect> {
        self.dirty.take()
    }

    /// When the oldest flush in flight completes (`None` when idle).
    #[must_use]
    pub fn busy_until(&self) -> Option<StdInstant> {
        self.in_flight.front().map(|f| f.done_at)
    }

    /// The most recently started flush.
    #[must_use]
    pub fn last_flush(&self) -> Option<FlushStat> {
        self.last
    }

    /// Number of accepted flushes.
    #[must_use]
    pub fn flush_count(&self) -> u64 {
        self.flushes
    }

    fn validate(&self, area: Rect, len: usize) -> Result<usize, SimDisplayError> {
        let panel = Rect::new(0, 0, i32::from(self.panel_w), i32::from(self.panel_h));
        if area.is_empty() || !panel.contains_rect(&area) {
            return Err(SimDisplayError::OutOfBounds(area));
        }
        let needed = self.info.format.stride(area.width() as u32) as usize * area.height() as usize;
        if len < needed {
            return Err(SimDisplayError::BufferTooSmall { needed, got: len });
        }
        if self.in_flight.len() >= 2 {
            return Err(SimDisplayError::Busy);
        }
        Ok(needed)
    }

    /// Copies the pixels of a completed flush into the panel.
    fn complete(&mut self, f: InFlight) -> DrawBufferMem {
        let area = f.stat.area;
        copy_area(
            &mut self.panel,
            self.stride,
            self.info.format,
            area,
            f.buf.as_slice(),
        );
        self.dirty = Some(self.dirty.map_or(area, |d| d.union(&area)));
        let us = f.stat.started.elapsed().as_micros();
        log::debug!(target: "twine::sim", "flush {} {} bytes in {} µs", area, f.stat.bytes, us);
        f.buf
    }
}

/// Copies the rows of `area` from `src` (stride `area.width() * bpp / 8`) into `panel`.
fn copy_area(panel: &mut [u8], stride: usize, format: ColorFormat, area: Rect, src: &[u8]) {
    let bpp = usize::from(format.bpp());
    let src_stride = format.stride(area.width() as u32) as usize;
    for (r, row) in src.chunks(src_stride).take(area.height() as usize).enumerate() {
        let dst_row = (area.y0 as usize + r) * stride;
        if bpp % 8 == 0 {
            let start = dst_row + area.x0 as usize * bpp / 8;
            panel[start..start + src_stride].copy_from_slice(row);
        } else {
            let dst = &mut panel[dst_row..dst_row + stride];
            for x in 0..area.width() as usize {
                let v = twine_core::color::unpack_bits(row, bpp as u8, x);
                let per_byte = 8 / bpp;
                let xx = area.x0 as usize + x;
                let shift = 8 - bpp * (xx % per_byte + 1);
                let mask = (((1u16 << bpp) - 1) as u8) << shift;
                dst[xx / per_byte] = (dst[xx / per_byte] & !mask) | ((v << shift) & mask);
            }
        }
    }
}

impl DisplayDriver for SimDisplay {
    type Error = SimDisplayError;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let bytes = match self.validate(area, buf.len()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!(target: "twine::sim", "SimDisplay: rejected flush: {e}");
                self.ready.push_back(buf);
                return Err(e);
            }
        };
        let started = StdInstant::now();
        let stat = FlushStat { area, bytes, started };
        self.last = Some(stat);
        self.flushes += 1;
        let transfer = self.bus_hz.map_or(StdDuration::ZERO, |hz| {
            StdDuration::from_nanos((bytes as u64 * 8).saturating_mul(1_000_000_000) / u64::from(hz))
        });
        // A queued transfer starts when the previous one ends.
        let start = self.in_flight.back().map_or(started, |p| p.done_at.max(started));
        let f = InFlight {
            buf,
            stat,
            done_at: start + transfer,
        };
        if transfer.is_zero() && self.in_flight.is_empty() {
            let buf = self.complete(f);
            self.ready.push_back(buf);
        } else {
            self.in_flight.push_back(f);
        }
        Ok(())
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        if let Some(buf) = self.ready.pop_front() {
            return Some(buf);
        }
        if self.in_flight.front()?.done_at > StdInstant::now() {
            return None;
        }
        let f = self.in_flight.pop_front()?;
        Some(self.complete(f))
    }
}
