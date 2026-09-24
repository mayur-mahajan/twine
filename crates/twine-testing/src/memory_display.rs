//! [`MemoryDisplay`]: an in-memory [`DisplayDriver`] that records every flush, with optional
//! DMA-like latency, plus [`leak_buffer`] for creating draw buffers in tests.

use std::collections::VecDeque;
use std::fmt;

use twine_core::{Color, Rect};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};

use crate::convert;

/// One flush received by a [`MemoryDisplay`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlushRecord {
    /// The flushed area (screen coordinates).
    pub area: Rect,
    /// Pixel bytes of the area (`stride * height`).
    pub bytes: usize,
    /// Address of the draw buffer ([`DrawBufferMem::addr`]); identifies ping-pong buffers.
    pub buffer_addr: usize,
    /// Value of the display's `poll_flush` call counter when the flush began.
    pub begin_at: u64,
    /// Value of the `poll_flush` call counter when the buffer was returned (equal to
    /// `begin_at` while the flush is still in flight).
    pub end_at: u64,
    /// Index of the draw buffer in order of first use (0 = first buffer seen, 1 = second…):
    /// a stable name for [`buffer_addr`](Self::buffer_addr).
    pub buffer: u8,
    /// Frame number of the flush, filled in by harnesses that know it (0 otherwise).
    pub frame: u32,
}

/// Errors of [`MemoryDisplay::begin_flush`]. The rejected buffer is returned by the next
/// `poll_flush` (see the [`DisplayDriver`] contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryDisplayError {
    /// `max_in_flight` flushes are already in progress.
    Busy,
    /// The area is empty or not fully inside the screen.
    OutOfBounds(Rect),
    /// The buffer holds fewer bytes than the area needs.
    BufferTooSmall {
        /// Bytes needed for the area.
        needed: usize,
        /// Bytes in the buffer.
        got: usize,
    },
}

impl fmt::Display for MemoryDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("display busy: too many flushes in flight"),
            Self::OutOfBounds(r) => write!(f, "flush area {r} is empty or outside the screen"),
            Self::BufferTooSmall { needed, got } => {
                write!(f, "draw buffer too small: {got} bytes, area needs {needed}")
            }
        }
    }
}

impl std::error::Error for MemoryDisplayError {}

#[derive(Debug)]
struct InFlight {
    buf: DrawBufferMem,
    polls_left: u32,
    seq: u64,
}

/// An in-memory display: a framebuffer in `info.format` that records every flush.
///
/// - Pixels are copied into the framebuffer **when the flush begins** (their content is known).
/// - `latency_polls = 0` (default) behaves like a blocking driver: the next `poll_flush`
///   returns the buffer. With `N > 0` the buffer is returned by the `N`-th `poll_flush` call,
///   emulating a DMA transfer so the engine's ping-pong logic is testable.
/// - At most `max_in_flight` (default 1) buffers may be held; one more `begin_flush` fails with
///   [`MemoryDisplayError::Busy`].
///
/// ```
/// use twine_core::{Color, ColorFormat, Rect};
/// use twine_hal::{DisplayDriver, DisplayInfo};
/// use twine_testing::{MemoryDisplay, leak_buffer};
///
/// let mut d = MemoryDisplay::new(DisplayInfo::new(4, 4, ColorFormat::Rgb565));
/// let mut buf = leak_buffer(2 * 2 * 2);
/// buf.as_mut_slice().copy_from_slice(&[0x00, 0xF8].repeat(4)); // 2×2 red
/// d.begin_flush(Rect::from_xywh(1, 1, 2, 2), buf).unwrap();
/// assert!(d.poll_flush().is_some());
/// assert_eq!(d.pixel(2, 2), Color::RED);
/// assert_eq!(d.pixel(0, 0), Color::BLACK);
/// assert_eq!(d.flushes().len(), 1);
/// ```
#[derive(Debug)]
pub struct MemoryDisplay {
    info: DisplayInfo,
    stride: usize,
    framebuffer: Vec<u8>,
    flushes: Vec<FlushRecord>,
    /// Sequence number of `flushes[0]`.
    seq_base: u64,
    next_seq: u64,
    latency_polls: u32,
    max_in_flight: usize,
    in_flight: VecDeque<InFlight>,
    rejected: VecDeque<DrawBufferMem>,
    polls: u64,
    mono: (Color, Color),
    /// Physical panel size (logical size with the axes swapped for 90°/270° software rotation).
    panel_w: u16,
    panel_h: u16,
    /// Buffer addresses in order of first use.
    buffers_seen: Vec<usize>,
}

impl MemoryDisplay {
    /// A display described by `info`, framebuffer cleared to zero bytes, blocking behaviour.
    ///
    /// # Panics
    /// If `info.format` is not supported by [`convert`].
    #[must_use]
    pub fn new(info: DisplayInfo) -> Self {
        assert!(
            convert::is_supported(info.format),
            "MemoryDisplay: unsupported format {}",
            info.format
        );
        let (panel_w, panel_h) = if !info.hw_rotation && info.rotation.swaps_axes() {
            (info.height, info.width)
        } else {
            (info.width, info.height)
        };
        let stride = info.format.stride(u32::from(panel_w)) as usize;
        Self {
            info,
            stride,
            framebuffer: vec![0; stride * usize::from(panel_h)],
            flushes: Vec::new(),
            seq_base: 0,
            next_seq: 0,
            latency_polls: 0,
            max_in_flight: 1,
            in_flight: VecDeque::new(),
            rejected: VecDeque::new(),
            polls: 0,
            mono: (Color::BLACK, Color::WHITE),
            panel_w,
            panel_h,
            buffers_seen: Vec::new(),
        }
    }

    /// The physical panel size: the logical size, with width and height swapped when the
    /// display is rotated by 90° or 270° in software (`hw_rotation == false`). Flush areas and
    /// the framebuffer use these physical coordinates.
    #[must_use]
    pub fn panel_size(&self) -> (u16, u16) {
        (self.panel_w, self.panel_h)
    }

    /// Moves the flush records into `out` (keeping this display's allocation, so a harness
    /// that drains every frame does not allocate in steady state).
    pub fn drain_flushes_into(&mut self, out: &mut Vec<FlushRecord>) {
        self.seq_base += self.flushes.len() as u64;
        out.append(&mut self.flushes);
    }

    /// Returns buffers on the `polls`-th `poll_flush` call (0 = blocking).
    #[must_use]
    pub fn with_latency(mut self, polls: u32) -> Self {
        self.latency_polls = polls;
        self
    }

    /// Allows `n` (≥ 1) buffers in flight at once.
    #[must_use]
    pub fn with_max_in_flight(mut self, n: usize) -> Self {
        self.max_in_flight = n.max(1);
        self
    }

    /// Colors used for `0` and `1` bits of an `I1` framebuffer (default black, white).
    #[must_use]
    pub fn with_mono_colors(mut self, zero: Color, one: Color) -> Self {
        self.mono = (zero, one);
        self
    }

    /// The display description.
    #[must_use]
    pub fn display_info(&self) -> DisplayInfo {
        self.info
    }

    /// The framebuffer bytes (`info.format`, rows of the physical panel width, see
    /// [`panel_size`](Self::panel_size)).
    #[must_use]
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Every flush since creation or the last [`take_flushes`](Self::take_flushes).
    #[must_use]
    pub fn flushes(&self) -> &[FlushRecord] {
        &self.flushes
    }

    /// Returns and clears the flush records.
    pub fn take_flushes(&mut self) -> Vec<FlushRecord> {
        self.seq_base += self.flushes.len() as u64;
        std::mem::take(&mut self.flushes)
    }

    /// Number of buffers currently held for a flush in progress.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Number of `poll_flush` calls so far.
    #[must_use]
    pub fn poll_count(&self) -> u64 {
        self.polls
    }

    /// The framebuffer as tightly packed 8-bit RGB.
    #[must_use]
    pub fn to_rgb888(&self) -> Vec<u8> {
        convert::to_rgb888(
            &self.framebuffer,
            self.info.format,
            u32::from(self.panel_w),
            u32::from(self.panel_h),
            self.stride as u32,
            self.mono,
        )
    }

    /// The color of framebuffer pixel `(x, y)`.
    ///
    /// # Panics
    /// If the pixel is outside the screen.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Color {
        assert!(
            x < u32::from(self.panel_w) && y < u32::from(self.panel_h),
            "MemoryDisplay::pixel({x}, {y}) outside {}x{}",
            self.panel_w,
            self.panel_h
        );
        convert::pixel_color(
            &self.framebuffer,
            self.info.format,
            self.stride as u32,
            x,
            y,
            self.mono,
        )
    }

    /// Fills the whole framebuffer with `c` (see [`convert::write_pixel`] for mono formats).
    pub fn clear(&mut self, c: Color) {
        for y in 0..u32::from(self.panel_h) {
            for x in 0..u32::from(self.panel_w) {
                convert::write_pixel(
                    &mut self.framebuffer,
                    self.info.format,
                    self.stride as u32,
                    x,
                    y,
                    c,
                );
            }
        }
    }

    fn validate(&self, area: Rect, len: usize) -> Result<(), MemoryDisplayError> {
        let panel = Rect::new(0, 0, i32::from(self.panel_w), i32::from(self.panel_h));
        if area.is_empty() || !panel.contains_rect(&area) {
            return Err(MemoryDisplayError::OutOfBounds(area));
        }
        let needed = self.info.format.stride(area.width() as u32) as usize * area.height() as usize;
        if len < needed {
            return Err(MemoryDisplayError::BufferTooSmall { needed, got: len });
        }
        if self.in_flight.len() >= self.max_in_flight {
            return Err(MemoryDisplayError::Busy);
        }
        Ok(())
    }

    fn copy_in(&mut self, area: Rect, src: &[u8]) {
        let bpp = usize::from(self.info.format.bpp());
        let src_stride = self.info.format.stride(area.width() as u32) as usize;
        let w = area.width() as usize;
        for (r, row) in src.chunks(src_stride).take(area.height() as usize).enumerate() {
            let dst_row = (area.y0 as usize + r) * self.stride;
            if bpp % 8 == 0 {
                let start = dst_row + area.x0 as usize * bpp / 8;
                self.framebuffer[start..start + src_stride].copy_from_slice(row);
            } else {
                let dst = &mut self.framebuffer[dst_row..dst_row + self.stride];
                for x in 0..w {
                    let v = twine_core::color::unpack_bits(row, bpp as u8, x);
                    set_bits(dst, bpp, area.x0 as usize + x, v);
                }
            }
        }
    }
}

/// Sets pixel `x` of a packed row of `bpp`-bit pixels (1, 2 or 4, MSB first) to `v`.
fn set_bits(row: &mut [u8], bpp: usize, x: usize, v: u8) {
    let per_byte = 8 / bpp;
    let shift = 8 - bpp * (x % per_byte + 1);
    let mask = (((1u16 << bpp) - 1) as u8) << shift;
    let b = &mut row[x / per_byte];
    *b = (*b & !mask) | ((v << shift) & mask);
}

impl DisplayDriver for MemoryDisplay {
    type Error = MemoryDisplayError;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        if let Err(e) = self.validate(area, buf.len()) {
            log::warn!(target: "twine::driver", "MemoryDisplay: rejected flush: {e}");
            self.rejected.push_back(buf);
            return Err(e);
        }
        self.copy_in(area, buf.as_slice());
        let bytes = self.info.format.stride(area.width() as u32) as usize * area.height() as usize;
        let seq = self.next_seq;
        self.next_seq += 1;
        let addr = buf.addr();
        let buffer = if let Some(i) = self.buffers_seen.iter().position(|a| *a == addr) {
            i as u8
        } else {
            self.buffers_seen.push(addr);
            (self.buffers_seen.len() - 1) as u8
        };
        self.flushes.push(FlushRecord {
            area,
            bytes,
            buffer_addr: addr,
            begin_at: self.polls,
            end_at: self.polls,
            buffer,
            frame: 0,
        });
        self.in_flight.push_back(InFlight {
            buf,
            polls_left: self.latency_polls.max(1),
            seq,
        });
        Ok(())
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.polls += 1;
        if let Some(buf) = self.rejected.pop_front() {
            return Some(buf);
        }
        let front = self.in_flight.front_mut()?;
        front.polls_left -= 1;
        if front.polls_left > 0 {
            return None;
        }
        let done = self.in_flight.pop_front()?;
        if let Some(i) = done.seq.checked_sub(self.seq_base) {
            if let Some(rec) = self.flushes.get_mut(i as usize) {
                rec.end_at = self.polls;
            }
        }
        Some(done.buf)
    }
}

/// Creates a 4-byte aligned `len`-byte draw buffer by **leaking** heap memory (tests only).
///
/// ```
/// let buf = twine_testing::leak_buffer(100);
/// assert_eq!(buf.len(), 100);
/// assert!(buf.is_aligned(4));
/// ```
#[must_use]
pub fn leak_buffer(len: usize) -> DrawBufferMem {
    let v: &'static mut [u8] = Box::leak(vec![0u8; len + 3].into_boxed_slice());
    let off = v.as_ptr().align_offset(4);
    DrawBufferMem::new(&mut v[off..off + len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use twine_core::ColorFormat;

    #[test]
    fn set_bits_packs_msb_first() {
        let mut row = [0u8; 2];
        set_bits(&mut row, 1, 0, 1);
        set_bits(&mut row, 1, 9, 1);
        assert_eq!(row, [0x80, 0x40]);
        let mut row = [0xFFu8];
        set_bits(&mut row, 4, 1, 0x3);
        assert_eq!(row, [0xF3]);
        set_bits(&mut row, 2, 0, 0);
        assert_eq!(row, [0x33]);
    }

    #[test]
    fn i1_flush_at_unaligned_x() {
        let mut d = MemoryDisplay::new(DisplayInfo::new(16, 1, ColorFormat::I1));
        let mut buf = leak_buffer(1);
        buf.as_mut_slice()[0] = 0b1010_0000; // 3 px: 1 0 1
        d.begin_flush(Rect::from_xywh(6, 0, 3, 1), buf).unwrap();
        assert!(d.poll_flush().is_some());
        assert_eq!(d.framebuffer(), &[0b0000_0010, 0b1000_0000]);
    }

    #[test]
    fn take_flushes_keeps_in_flight_bookkeeping() {
        let mut d = MemoryDisplay::new(DisplayInfo::new(4, 4, ColorFormat::L8)).with_latency(2);
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), leak_buffer(1))
            .unwrap();
        assert_eq!(d.take_flushes().len(), 1);
        assert!(d.poll_flush().is_none());
        assert!(d.poll_flush().is_some());
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), leak_buffer(1))
            .unwrap();
        assert!(d.poll_flush().is_none());
        assert!(d.poll_flush().is_some());
        assert_eq!(d.flushes()[0].begin_at, 2);
        assert_eq!(d.flushes()[0].end_at, 4);
    }
}
