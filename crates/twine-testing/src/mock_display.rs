//! Display mocks for engine tests (feature `engine`): [`MockDmaDisplay`] (a DMA-like
//! [`DisplayDriver`] recording an event log that proves render/transfer overlap) and
//! [`MockFramebufferDisplay`] (a [`FramebufferDisplay`] with configurable swap latency).

use std::cell::RefCell;

use twine_core::{ColorFormat, Rect};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, FramebufferDisplay};

use crate::memory_display::{MemoryDisplay, MemoryDisplayError, leak_buffer};

/// One event of the DMA log (see [`dma_log`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DmaEvent {
    /// A transfer of `area` from buffer `buf` started (`begin_flush`).
    Begin {
        /// Buffer index (order of first use).
        buf: u8,
        /// The flushed area.
        area: Rect,
    },
    /// The transfer from buffer `buf` completed (`poll_flush` returned it).
    Complete {
        /// Buffer index.
        buf: u8,
    },
    /// The engine started rendering into buffer `buf` (from its render hook).
    RenderStart {
        /// Buffer index.
        buf: u8,
    },
}

thread_local! {
    static DMA_LOG: RefCell<Vec<DmaEvent>> = const { RefCell::new(Vec::new()) };
}

/// The events recorded on this thread since the last [`clear_dma_log`].
#[must_use]
pub fn dma_log() -> Vec<DmaEvent> {
    DMA_LOG.with(|l| l.borrow().clone())
}

/// Clears this thread's DMA log.
pub fn clear_dma_log() {
    DMA_LOG.with(|l| l.borrow_mut().clear());
}

fn log_event(e: DmaEvent) {
    DMA_LOG.with(|l| l.borrow_mut().push(e));
}

/// Render hook for `Engine::set_render_hook`: logs [`DmaEvent::RenderStart`] (installed by
/// `EngineHarness::dma` with the `debug-checks` feature).
pub fn record_render_start(buf: u8) {
    log_event(DmaEvent::RenderStart { buf });
}

/// A DMA-like display: a [`MemoryDisplay`] whose transfers complete after `polls_to_complete`
/// `poll_flush` calls, holding up to two buffers, and logging every begin/complete to the
/// thread's [`dma_log`] (buffers are named by order of first use, like the engine's slots).
///
/// ```
/// use twine_core::{ColorFormat, Rect};
/// use twine_hal::{DisplayDriver, DisplayInfo};
/// use twine_testing::{DmaEvent, MockDmaDisplay, clear_dma_log, dma_log, leak_buffer};
///
/// clear_dma_log();
/// let mut d = MockDmaDisplay::new(DisplayInfo::new(4, 4, ColorFormat::L8), 2);
/// d.begin_flush(Rect::from_xywh(0, 0, 4, 1), leak_buffer(4)).unwrap();
/// assert!(d.poll_flush().is_none());
/// assert!(d.poll_flush().is_some());
/// assert_eq!(dma_log(), [DmaEvent::Begin { buf: 0, area: Rect::from_xywh(0, 0, 4, 1) }, DmaEvent::Complete { buf: 0 }]);
/// ```
#[derive(Debug)]
pub struct MockDmaDisplay {
    inner: MemoryDisplay,
    seen: Vec<usize>,
}

impl MockDmaDisplay {
    /// A display described by `info` whose transfers take `polls_to_complete` polls (≥ 1).
    #[must_use]
    pub fn new(info: DisplayInfo, polls_to_complete: u32) -> Self {
        Self {
            inner: MemoryDisplay::new(info)
                .with_latency(polls_to_complete.max(1))
                .with_max_in_flight(2),
            seen: Vec::new(),
        }
    }

    /// The wrapped memory display (panel pixels, flush records).
    #[must_use]
    pub fn memory(&self) -> &MemoryDisplay {
        &self.inner
    }

    /// The wrapped memory display, mutably.
    pub fn memory_mut(&mut self) -> &mut MemoryDisplay {
        &mut self.inner
    }

    fn index(&mut self, addr: usize) -> u8 {
        if let Some(i) = self.seen.iter().position(|a| *a == addr) {
            i as u8
        } else {
            self.seen.push(addr);
            (self.seen.len() - 1) as u8
        }
    }
}

impl DisplayDriver for MockDmaDisplay {
    type Error = MemoryDisplayError;

    fn info(&self) -> DisplayInfo {
        self.inner.info()
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let b = self.index(buf.addr());
        let r = self.inner.begin_flush(area, buf);
        if r.is_ok() {
            log_event(DmaEvent::Begin { buf: b, area });
        }
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        let buf = self.inner.poll_flush()?;
        let b = self.index(buf.addr());
        log_event(DmaEvent::Complete { buf: b });
        Some(buf)
    }
}

/// A memory-mapped panel mock: one or two leaked framebuffers handed out once, `present`
/// recorded, and `present_done` true only after `present_delay` calls.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::{DisplayInfo, FramebufferDisplay};
/// use twine_testing::MockFramebufferDisplay;
///
/// let mut d = MockFramebufferDisplay::new(DisplayInfo::new(4, 2, ColorFormat::Rgb565), true, 1);
/// let (a, b) = d.framebuffers().unwrap();
/// assert_eq!(a.len(), 16);
/// assert!(b.is_some() && d.framebuffers().is_none());
/// d.present(1).unwrap();
/// assert!(!d.present_done());
/// assert!(d.present_done());
/// assert_eq!(d.presents(), &[1]);
/// ```
#[derive(Debug)]
pub struct MockFramebufferDisplay {
    info: DisplayInfo,
    fbs: Option<(DrawBufferMem, Option<DrawBufferMem>)>,
    presents: Vec<u8>,
    present_delay: u32,
    polls_left: u32,
}

impl MockFramebufferDisplay {
    /// A panel described by `info` with two (`double`) or one framebuffer; a present takes
    /// effect after `present_delay` calls of `present_done` (0 = immediately).
    #[must_use]
    pub fn new(info: DisplayInfo, double: bool, present_delay: u32) -> Self {
        let len = info.bytes_per_row() * usize::from(info.height);
        Self {
            info,
            fbs: Some((leak_buffer(len), double.then(|| leak_buffer(len)))),
            presents: Vec::new(),
            present_delay,
            polls_left: 0,
        }
    }

    /// A panel whose framebuffers have the wrong size (for error tests).
    #[must_use]
    pub fn with_buffer_len(mut self, len: usize) -> Self {
        let double = self.fbs.as_ref().is_some_and(|f| f.1.is_some());
        self.fbs = Some((leak_buffer(len), double.then(|| leak_buffer(len))));
        self
    }

    /// Every presented index, in order.
    #[must_use]
    pub fn presents(&self) -> &[u8] {
        &self.presents
    }

    /// The most recently presented index.
    #[must_use]
    pub fn last_presented(&self) -> Option<u8> {
        self.presents.last().copied()
    }

    /// The pixel format.
    #[must_use]
    pub fn format(&self) -> ColorFormat {
        self.info.format
    }
}

impl FramebufferDisplay for MockFramebufferDisplay {
    type Error = core::convert::Infallible;

    fn info(&self) -> DisplayInfo {
        self.info
    }

    fn framebuffers(&mut self) -> Option<(DrawBufferMem, Option<DrawBufferMem>)> {
        self.fbs.take()
    }

    fn present(&mut self, index: u8) -> Result<(), Self::Error> {
        self.presents.push(index);
        self.polls_left = self.present_delay;
        Ok(())
    }

    fn present_done(&mut self) -> bool {
        if self.polls_left == 0 {
            return true;
        }
        self.polls_left -= 1;
        false
    }
}
