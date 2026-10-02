//! Draw buffer memory: [`DrawBufferMem`] (an owned `'static` buffer), [`DrawBuffer`] (the
//! storage of one buffer, 4-byte aligned by its type) and [`buffer_bytes`] (the size of a
//! partial buffer, computed at compile time).
//!
//! # Why `'static`
//!
//! A DMA transfer keeps reading a buffer after the call that started it has returned. If the
//! buffer were a borrowed `&mut [u8]` with a shorter lifetime, the borrow could end (and the memory
//! be reused) while the DMA engine is still reading it — undefined behaviour that the borrow
//! checker cannot see. Requiring `&'static mut [u8]` and **moving** the buffer into the driver
//! for the duration of a flush (`DisplayDriver::begin_flush` takes it by value,
//! `DisplayDriver::poll_flush` hands it back) makes this safe without any `unsafe` in user code:
//!
//! - the memory lives forever, so it can never be freed under the DMA;
//! - while the driver owns the [`DrawBufferMem`], nobody else can write to it.
//!
//! # Where the memory comes from
//!
//! - **Firmware:** `twine::draw_buffers!` declares a `static` of [`DrawBuffer`]s sized with
//!   [`buffer_bytes`] at compile time and hands them out once
//!   (`BufferMode::partial_double_from(BUFS.take()..)` in `twine-engine`). The alignment the
//!   engine needs is a property of the type, the buffers are zeroed `.bss` (no flash), and no
//!   `static mut` or `unsafe` is involved.
//! - **Heap:** `BufferMode::alloc(..)` lets the engine allocate (and leak) the buffers once the
//!   display has been accepted.
//! - **Other memory** (a linker section, external RAM): any `&'static mut [u8]`
//!   (`BufferMode::partial_single` / `partial_double`); the engine checks its alignment when
//!   the display is added.

use core::fmt;

use twine_core::ColorFormat;

/// Bytes of one partial draw buffer of `rows` full-width rows of a `width`-pixel display in
/// `format`: `rows × ⌈width × bpp / 8⌉` (sub-byte formats round each row up to whole bytes,
/// exactly as the engine lays out a chunk; see [`ColorFormat::stride`]).
///
/// `const`, so firmware sizes its `static` buffers at compile time (`twine::draw_buffers!`
/// calls it). `width` is the **logical** width (after rotation), like
/// [`DisplayInfo::width`](crate::DisplayInfo::width). Never panics (saturates at
/// `usize::MAX`, far beyond any memory).
///
/// | Format | bpp | 480 px × 40 rows |
/// |--------|-----|------------------|
/// | `I1` | 1 | 2 400 |
/// | `L8`, `I8`, `A8` | 8 | 19 200 |
/// | `Rgb565`, `Rgb565Swapped` | 16 | 38 400 |
/// | `Rgb888` | 24 | 57 600 |
/// | `Xrgb8888`, `Argb8888` | 32 | 76 800 |
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::buffer_bytes;
///
/// const BYTES: usize = buffer_bytes(480, 40, ColorFormat::Rgb565Swapped);
/// assert_eq!(BYTES, 480 * 2 * 40);
/// assert_eq!(buffer_bytes(128, 64, ColorFormat::I1), 128 * 64 / 8);
/// assert_eq!(buffer_bytes(5, 2, ColorFormat::I1), 2); // 5 px round up to 1 byte per row
/// ```
#[must_use]
pub const fn buffer_bytes(width: u16, rows: u16, format: ColorFormat) -> usize {
    (format.stride(width as u32) as usize).saturating_mul(rows as usize)
}

/// The storage of one draw buffer: `BYTES` zeroed bytes, 4-byte aligned **by its type**
/// (the alignment the engine requires of draw buffers; word-wise renderers and most DMA
/// engines need it too).
///
/// Declare buffers with `twine::draw_buffers!` (a `static` of `N` of them, handed out once)
/// rather than by hand. A `&'static mut DrawBuffer<BYTES>` converts into a [`DrawBufferMem`]
/// that is aligned by construction, so the engine's run-time alignment check cannot fail for
/// it.
///
/// ```
/// use twine_hal::{DrawBuffer, DrawBufferMem};
///
/// assert_eq!(core::mem::align_of::<DrawBuffer<6>>(), DrawBuffer::<6>::ALIGN);
/// assert_eq!(core::mem::size_of::<DrawBuffer<6>>(), 8); // padded to the alignment
///
/// let buf: &'static mut DrawBuffer<64> = Box::leak(Box::new(DrawBuffer::ZEROED));
/// let mem = DrawBufferMem::from(buf);
/// assert_eq!(mem.len(), 64);
/// assert!(mem.is_aligned(4));
/// ```
#[repr(C, align(4))]
pub struct DrawBuffer<const BYTES: usize>([u8; BYTES]);

impl<const BYTES: usize> DrawBuffer<BYTES> {
    /// The alignment of every `DrawBuffer` in bytes (4).
    pub const ALIGN: usize = 4;

    /// A zeroed buffer (a constant, so `[DrawBuffer::ZEROED; N]` initialises a `static`
    /// array in `.bss`).
    pub const ZEROED: Self = Self([0; BYTES]);

    /// Length in bytes (`BYTES`).
    #[must_use]
    pub const fn len(&self) -> usize {
        BYTES
    }

    /// Whether the buffer has no bytes (`BYTES == 0`).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        BYTES == 0
    }

    /// The bytes of the buffer.
    #[must_use]
    pub const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// The bytes of the buffer, mutably.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl<const BYTES: usize> fmt::Debug for DrawBuffer<BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump the (possibly huge) contents.
        f.debug_struct("DrawBuffer").field("len", &BYTES).finish()
    }
}

#[cfg(feature = "defmt")]
impl<const BYTES: usize> defmt::Format for DrawBuffer<BYTES> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "DrawBuffer {{ len: {=usize} }}", BYTES);
    }
}

impl<const BYTES: usize> From<&'static mut DrawBuffer<BYTES>> for DrawBufferMem {
    /// Wraps a [`DrawBuffer`]: aligned by construction.
    fn from(buf: &'static mut DrawBuffer<BYTES>) -> Self {
        Self(&mut buf.0)
    }
}

/// An exclusively owned, `'static` byte buffer that the engine renders into and hands to a
/// display driver for flushing.
///
/// ```
/// use twine_hal::DrawBufferMem;
///
/// let mem: &'static mut [u8] = Box::leak(Box::new([0u8; 16]));
/// let mut buf = DrawBufferMem::new(mem);
/// buf.as_mut_slice()[0] = 0xAB;
/// assert_eq!(buf.len(), 16);
/// assert_eq!(buf.as_slice()[0], 0xAB);
/// assert!(buf.is_aligned(1));
/// ```
pub struct DrawBufferMem(&'static mut [u8]);

impl DrawBufferMem {
    /// Wraps a `'static` buffer.
    #[must_use]
    pub fn new(buf: &'static mut [u8]) -> Self {
        Self(buf)
    }

    /// The bytes of the buffer.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.0
    }

    /// The bytes of the buffer, mutably.
    #[inline]
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0
    }

    /// Length in bytes.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the buffer has no bytes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Unwraps the underlying `'static` slice.
    #[must_use]
    pub fn into_inner(self) -> &'static mut [u8] {
        self.0
    }

    /// Whether the start address is a multiple of `align` (some DMA engines require 4).
    ///
    /// `align` of 0 is treated as 1 (always aligned).
    #[must_use]
    pub fn is_aligned(&self, align: usize) -> bool {
        self.addr() % align.max(1) == 0
    }

    /// The start address, used to identify buffers in logs and tests (e.g. which of the two
    /// ping-pong buffers a flush used).
    #[must_use]
    pub fn addr(&self) -> usize {
        self.0.as_ptr() as usize
    }
}

impl fmt::Debug for DrawBufferMem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump the (possibly huge) contents.
        f.debug_struct("DrawBufferMem")
            .field("addr", &format_args!("{:#x}", self.addr()))
            .field("len", &self.len())
            .finish()
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for DrawBufferMem {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(
            f,
            "DrawBufferMem {{ addr: {=usize:#x}, len: {=usize} }}",
            self.addr(),
            self.len()
        );
    }
}

impl AsRef<[u8]> for DrawBufferMem {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl AsMut<[u8]> for DrawBufferMem {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leak(n: usize) -> &'static mut [u8] {
        extern crate std;
        std::boxed::Box::leak(std::vec![0u8; n].into_boxed_slice())
    }

    #[test]
    fn accessors() {
        let mut b = DrawBufferMem::new(leak(8));
        assert_eq!(b.len(), 8);
        assert!(!b.is_empty());
        b.as_mut_slice()[7] = 3;
        let addr = b.addr();
        let inner = b.into_inner();
        assert_eq!(inner[7], 3);
        assert_eq!(inner.as_ptr() as usize, addr);
        assert!(DrawBufferMem::new(leak(0)).is_empty());
    }

    #[test]
    fn alignment() {
        let mem = leak(16);
        let off = mem.as_ptr().align_offset(4);
        let b = DrawBufferMem::new(&mut mem[off + 1..off + 9]);
        assert!(!b.is_aligned(4));
        assert!(b.is_aligned(1));
        assert!(b.is_aligned(0));
    }

    #[test]
    fn draw_buffer_is_aligned_by_type() {
        // Compile-time: the alignment is a property of the type, and arrays of buffers keep
        // every element aligned (sizes are padded to the alignment).
        const _: () = assert!(core::mem::align_of::<DrawBuffer<3>>() == 4);
        const _: () = assert!(core::mem::size_of::<[DrawBuffer<6>; 2]>() == 16);
        extern crate std;
        let bufs: &'static mut [DrawBuffer<6>; 2] =
            std::boxed::Box::leak(std::boxed::Box::new([DrawBuffer::ZEROED; 2]));
        let [a, b] = bufs.each_mut();
        let (a, b) = (DrawBufferMem::from(a), DrawBufferMem::from(b));
        assert!(a.is_aligned(4) && b.is_aligned(4));
        assert_eq!((a.len(), b.len()), (6, 6));
        assert_ne!(a.addr(), b.addr());
    }

    #[test]
    fn buffer_bytes_per_format() {
        use twine_core::ColorFormat as F;
        for (format, bytes) in [
            (F::I1, 60),
            (F::I2, 120),
            (F::I4, 240),
            (F::L8, 480),
            (F::Rgb565, 960),
            (F::Rgb565Swapped, 960),
            (F::Rgb888, 1440),
            (F::Xrgb8888, 1920),
            (F::Argb8888, 1920),
        ] {
            assert_eq!(buffer_bytes(480, 1, format), bytes, "{format:?}");
            assert_eq!(buffer_bytes(480, 40, format), bytes * 40, "{format:?}");
        }
        assert_eq!(buffer_bytes(0, 40, F::Rgb565), 0);
        assert_eq!(buffer_bytes(7, 3, F::I1), 3);
        assert_eq!(buffer_bytes(u16::MAX, u16::MAX, F::Argb8888), 4 * 65_535 * 65_535);
    }

    #[test]
    fn debug_does_not_dump_contents() {
        extern crate std;
        use std::format;
        let b = DrawBufferMem::new(leak(4));
        let s = format!("{b:?}");
        assert!(s.contains("len: 4"), "{s}");
    }
}
