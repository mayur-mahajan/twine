//! Draw buffer ownership: [`DrawBufferMem`].
//!
//! # Why `'static`
//!
//! A DMA transfer keeps reading a buffer after the call that started it has returned. If the
//! buffer were a borrowed `&mut [u8]` with a shorter lifetime, the borrow could end (and the memory
//! be reused) while the DMA engine is still reading it — undefined behaviour that the borrow
//! checker cannot see. Requiring `&'static mut [u8]` and **moving** the buffer into the driver
//! for the duration of a flush (`DisplayDriver::begin_flush` takes it by value,
//! `DisplayDriver::poll_flush` hands it back) makes this safe without any `unsafe` in user code
//! (`docs/design/06-rendering.md` §1.2):
//!
//! - the memory lives forever, so it can never be freed under the DMA;
//! - while the driver owns the [`DrawBufferMem`], nobody else can write to it.
//!
//! Buffers are typically created once at start-up, e.g. with
//! `static_cell::ConstStaticCell::new([0u8; N]).take()` on firmware, or by leaking a boxed slice
//! on the host.

use core::fmt;

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
    fn debug_does_not_dump_contents() {
        extern crate std;
        use std::format;
        let b = DrawBufferMem::new(leak(4));
        let s = format!("{b:?}");
        assert!(s.contains("len: 4"), "{s}");
    }
}
