//! [`SimFramebufferDisplay`]: a memory-mapped panel for the engine's `Full` and `Direct`
//! buffer modes.

use twine_core::{ColorFormat, Rotation};
use twine_hal::{DisplayInfo, DrawBufferMem, FramebufferDisplay};

/// An emulated memory-mapped panel (like an LTDC or RGB interface): it hands one or two
/// framebuffers to the engine and "scans out" the presented one. The simulator reads the
/// presented framebuffer back through the engine (`Engine::framebuffer`) to show it. Swaps
/// take effect immediately (no tearing is possible in the simulator).
///
/// ```
/// use twine_core::{ColorFormat, Rotation};
/// use twine_hal::FramebufferDisplay;
/// use twine_sim::SimFramebufferDisplay;
///
/// let mut d = SimFramebufferDisplay::new(8, 4, ColorFormat::Rgb565, Rotation::Deg0, true);
/// let (a, b) = d.framebuffers().unwrap();
/// assert_eq!((a.len(), b.map(|b| b.len())), (64, Some(64)));
/// d.present(1).unwrap();
/// assert_eq!(d.presented(), Some(1));
/// assert!(d.present_done());
/// ```
#[derive(Debug)]
pub struct SimFramebufferDisplay {
    info: DisplayInfo,
    fbs: Option<(DrawBufferMem, Option<DrawBufferMem>)>,
    presented: Option<u8>,
    presents: u64,
}

/// Leaks a zeroed buffer of `len` bytes (4-byte aligned).
fn leak(len: usize) -> DrawBufferMem {
    let v: &'static mut [u8] = Box::leak(vec![0u8; len + 3].into_boxed_slice());
    let off = v.as_ptr().align_offset(4).min(3);
    DrawBufferMem::new(&mut v[off..off + len])
}

impl SimFramebufferDisplay {
    /// A `width × height` panel in `format` with two (`double`) or one framebuffer. Rotation
    /// is emulated in hardware.
    #[must_use]
    pub fn new(width: u16, height: u16, format: ColorFormat, rotation: Rotation, double: bool) -> Self {
        let mut info = DisplayInfo::new(width, height, format)
            .with_rotation(rotation)
            .with_hw_rotation(true);
        if format == ColorFormat::I1 {
            info.align = 8;
        }
        let len = info.bytes_per_row() * usize::from(height);
        Self {
            info,
            fbs: Some((leak(len), double.then(|| leak(len)))),
            presented: None,
            presents: 0,
        }
    }

    /// The index of the framebuffer currently shown.
    #[must_use]
    pub fn presented(&self) -> Option<u8> {
        self.presented
    }

    /// Number of `present` calls so far.
    #[must_use]
    pub fn present_count(&self) -> u64 {
        self.presents
    }
}

impl FramebufferDisplay for SimFramebufferDisplay {
    type Error = core::convert::Infallible;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    fn framebuffers(&mut self) -> Option<(DrawBufferMem, Option<DrawBufferMem>)> {
        self.fbs.take()
    }

    fn present(&mut self, index: u8) -> Result<(), Self::Error> {
        self.presented = Some(index);
        self.presents += 1;
        Ok(())
    }

    fn present_done(&mut self) -> bool {
        true
    }
}
