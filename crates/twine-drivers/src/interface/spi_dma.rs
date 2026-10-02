//! [`DmaSpiInterface`]: 4-wire SPI whose pixel transfers run by DMA (feature `spi-dma`).

use embedded_hal::digital::OutputPin;
use heapless::Deque;
use twine_core::log::{error, warn};
use twine_hal::DrawBufferMem;

use super::DcsInterface;

/// The chip-specific half of [`DmaSpiInterface`]: an SPI peripheral that writes bytes
/// (blocking) and transfers a draw buffer by DMA (non-blocking). Implement it for your MCU in a
/// few register writes (DMA channel paced by the SPI's TX request, read address incrementing,
/// write address the SPI data register); the interface does the rest (CS, DC, ordering, buffer
/// hand-back).
///
/// The interface calls [`write`](Self::write) and [`start_dma`](Self::start_dma) only when no
/// transfer is running (after [`finish_dma`](Self::finish_dma) returned its buffer).
///
/// ```
/// use twine_drivers::interface::DmaSpiBus;
/// use twine_hal::DrawBufferMem;
///
/// /// A bus whose "DMA" completes on the first poll (a stand-in for the registers).
/// struct Bus(Option<DrawBufferMem>);
/// impl DmaSpiBus for Bus {
///     type Error = core::convert::Infallible;
///     fn write(&mut self, _bytes: &[u8]) -> Result<(), Self::Error> {
///         Ok(()) // write the data register byte by byte, then wait until the SPI is idle
///     }
///     fn start_dma(&mut self, buf: DrawBufferMem, _len: usize) -> Result<(), (Self::Error, DrawBufferMem)> {
///         self.0 = Some(buf); // program the channel: buf.addr(), len, SPI TX request
///         Ok(())
///     }
///     fn finish_dma(&mut self) -> Option<DrawBufferMem> {
///         self.0.take() // `None` while the channel or the SPI shifter is busy
///     }
/// }
/// # let _ = Bus(None);
/// ```
pub trait DmaSpiBus {
    /// The bus error.
    type Error: core::fmt::Debug;

    /// Writes `bytes` and returns once the last bit has left the bus (commands and their
    /// parameters).
    ///
    /// # Errors
    ///
    /// The bus error.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Starts a DMA transfer of the first `len` bytes of `buf` (`len` ≤ `buf.len()`) and
    /// returns at once; the bus keeps `buf` until [`finish_dma`](Self::finish_dma) returns it.
    ///
    /// # Errors
    ///
    /// The bus error, with the buffer (nothing was started).
    fn start_dma(&mut self, buf: DrawBufferMem, len: usize) -> Result<(), (Self::Error, DrawBufferMem)>;

    /// The buffer of the transfer started by [`start_dma`](Self::start_dma) once it has
    /// completely left the bus (the DMA channel is done **and** the SPI has shifted out its
    /// last bit, so CS and DC may change); `None` while it runs or when none was started.
    /// Non-blocking.
    fn finish_dma(&mut self) -> Option<DrawBufferMem>;
}

/// The error of a [`DmaSpiInterface`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DmaSpiError<B, P> {
    /// The bus failed.
    Bus(B),
    /// The CS or DC pin failed.
    Pin(P),
    /// A DMA transfer did not finish within the interface's poll limit
    /// ([`DmaSpiInterface::with_max_polls`]); it is still considered running.
    Timeout,
}

/// A command/data transport ([`DcsInterface`]) over a [`DmaSpiBus`] with CS and DC pins, whose
/// pixel transfers run by DMA: [`start_pixels`](DcsInterface::start_pixels) only starts them,
/// so a panel driver's `begin_flush` returns at once and the engine renders the next chunk
/// into its second draw buffer while the first one is sent (the blocking runtime's
/// `begin_flush` / `poll_flush` contract; use two draw buffers to get the overlap).
/// [`poll_pixels`](DcsInterface::poll_pixels) hands a buffer back once its transfer is done.
///
/// Commands (and [`write_pixels`](DcsInterface::write_pixels)) first wait for a transfer in
/// progress (the bus is shared), polling [`DmaSpiBus::finish_dma`] at most
/// [`with_max_polls`](Self::with_max_polls) times (default `u32::MAX`), then fail with
/// [`DmaSpiError::Timeout`]: a hung DMA channel becomes a flush error and, as its buffer never
/// comes back, the engine's `flush_timeout` fault — never an endless spin.
///
/// ```
/// use twine_core::Rect;
/// use twine_drivers::interface::{DmaSpiBus, DmaSpiInterface};
/// use twine_drivers::mipi_dcs::MipiDcs;
/// use twine_drivers::testkit::Recorder;
/// use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};
///
/// /// A bus whose transfer takes two polls.
/// struct Bus(Option<(DrawBufferMem, u8)>);
/// impl DmaSpiBus for Bus {
///     type Error = core::convert::Infallible;
///     fn write(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
///     fn start_dma(&mut self, buf: DrawBufferMem, _: usize) -> Result<(), (Self::Error, DrawBufferMem)> {
///         self.0 = Some((buf, 2));
///         Ok(())
///     }
///     fn finish_dma(&mut self) -> Option<DrawBufferMem> {
///         match self.0.take()? {
///             (buf, 0) => Some(buf),
///             (buf, n) => { self.0 = Some((buf, n - 1)); None }
///         }
///     }
/// }
///
/// let rec = Recorder::new();
/// let iface = DmaSpiInterface::new(Bus(None), rec.quiet_pin("cs"), rec.quiet_pin("dc"));
/// let mut lcd = MipiDcs::new(iface, None::<twine_drivers::testkit::RecordingPin>, &twine_drivers::ili9341::ILI9341, Rotation::Deg0, &mut rec.delay()).unwrap();
/// let buf = DrawBufferMem::new(Box::leak(Box::new([0u8; 8])));
/// lcd.begin_flush(Rect::from_xywh(0, 0, 2, 2), buf).unwrap(); // returns while the DMA runs
/// assert!(lcd.poll_flush().is_none());
/// assert!(lcd.poll_flush().is_none());
/// assert!(lcd.poll_flush().is_some()); // done: the engine gets its buffer back
/// ```
pub struct DmaSpiInterface<B, CS, DC> {
    bus: B,
    cs: CS,
    dc: DC,
    /// A `start_dma` transfer has not been finished yet.
    running: bool,
    /// Buffers whose transfer finished, not yet handed back (at most the engine's two).
    done: Deque<DrawBufferMem, 2>,
    max_polls: u32,
}

impl<B, CS, DC> core::fmt::Debug for DmaSpiInterface<B, CS, DC> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DmaSpiInterface")
            .field("running", &self.running)
            .field("done", &self.done.len())
            .finish_non_exhaustive()
    }
}

type Error<B, CS> = DmaSpiError<<B as DmaSpiBus>::Error, <CS as embedded_hal::digital::ErrorType>::Error>;

impl<B, CS, DC> DmaSpiInterface<B, CS, DC>
where
    B: DmaSpiBus,
    CS: OutputPin,
    DC: OutputPin<Error = CS::Error>,
{
    /// The interface over `bus`, with the chip select `cs` (active low) and the data/command
    /// pin `dc`. Never panics.
    pub fn new(bus: B, cs: CS, dc: DC) -> Self {
        Self {
            bus,
            cs,
            dc,
            running: false,
            done: Deque::new(),
            max_polls: u32::MAX,
        }
    }

    /// Polls of [`DmaSpiBus::finish_dma`] a command waits for a running transfer before it
    /// fails with [`DmaSpiError::Timeout`] (default `u32::MAX`; size it from the longest
    /// transfer, e.g. a whole draw buffer at the SPI clock, times a safety factor). `0`
    /// counts as `1`. Never panics.
    ///
    /// ```
    /// # use twine_drivers::interface::{DmaSpiBus, DmaSpiInterface};
    /// # use twine_drivers::testkit::Recorder;
    /// # use twine_hal::DrawBufferMem;
    /// # struct Bus;
    /// # impl DmaSpiBus for Bus {
    /// #     type Error = core::convert::Infallible;
    /// #     fn write(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
    /// #     fn start_dma(&mut self, _: DrawBufferMem, _: usize) -> Result<(), (Self::Error, DrawBufferMem)> { Ok(()) }
    /// #     fn finish_dma(&mut self) -> Option<DrawBufferMem> { None }
    /// # }
    /// let rec = Recorder::new();
    /// // 240 x 40 rows RGB565 at 40 MHz is ~3.8 ms; one poll costs ~1 µs here: allow 20 ms.
    /// let iface = DmaSpiInterface::new(Bus, rec.quiet_pin("cs"), rec.quiet_pin("dc")).with_max_polls(20_000);
    /// # let _ = iface;
    /// ```
    #[must_use]
    pub fn with_max_polls(mut self, polls: u32) -> Self {
        self.max_polls = polls.max(1);
        self
    }

    /// Returns the bus and the pins, e.g. to share the SPI with another device or to put it to
    /// sleep. It does not wait for a running transfer: call it once the last transfer has
    /// finished (the engine got its buffers back); buffers still queued are dropped. Never
    /// panics.
    ///
    /// ```
    /// # use twine_drivers::interface::{DmaSpiBus, DmaSpiInterface};
    /// # use twine_drivers::testkit::Recorder;
    /// # use twine_hal::DrawBufferMem;
    /// # struct Bus;
    /// # impl DmaSpiBus for Bus {
    /// #     type Error = core::convert::Infallible;
    /// #     fn write(&mut self, _: &[u8]) -> Result<(), Self::Error> { Ok(()) }
    /// #     fn start_dma(&mut self, _: DrawBufferMem, _: usize) -> Result<(), (Self::Error, DrawBufferMem)> { Ok(()) }
    /// #     fn finish_dma(&mut self) -> Option<DrawBufferMem> { None }
    /// # }
    /// let rec = Recorder::new();
    /// let iface = DmaSpiInterface::new(Bus, rec.quiet_pin("cs"), rec.quiet_pin("dc"));
    /// let (_bus, _cs, _dc) = iface.release();
    /// ```
    #[must_use]
    pub fn release(self) -> (B, CS, DC) {
        (self.bus, self.cs, self.dc)
    }

    /// Ends the running transfer, if it has finished: CS goes high and the buffer is queued for
    /// `poll_pixels`.
    fn try_finish(&mut self) -> Result<bool, Error<B, CS>> {
        if !self.running {
            return Ok(true);
        }
        let Some(buf) = self.bus.finish_dma() else {
            return Ok(false);
        };
        self.running = false;
        if self.done.push_back(buf).is_err() {
            error!(target: "twine::driver", "dma spi: more than 2 finished buffers not taken; buffer dropped");
        }
        self.cs.set_high().map_err(DmaSpiError::Pin)?;
        Ok(true)
    }

    /// Waits (bounded) until no transfer runs.
    fn settle(&mut self) -> Result<(), Error<B, CS>> {
        for _ in 0..self.max_polls {
            if self.try_finish()? {
                return Ok(());
            }
        }
        warn!(target: "twine::driver", "dma spi: transfer still running after {} polls", self.max_polls);
        Err(DmaSpiError::Timeout)
    }

    /// One CS-framed write: `cmd` with DC low (if any), then `data` with DC high.
    fn framed(&mut self, cmd: Option<u8>, data: &[u8]) -> Result<(), Error<B, CS>> {
        self.settle()?;
        self.cs.set_low().map_err(DmaSpiError::Pin)?;
        let r = self.framed_inner(cmd, data);
        let high = self.cs.set_high().map_err(DmaSpiError::Pin);
        r.and(high)
    }

    fn framed_inner(&mut self, cmd: Option<u8>, data: &[u8]) -> Result<(), Error<B, CS>> {
        if let Some(c) = cmd {
            self.dc.set_low().map_err(DmaSpiError::Pin)?;
            self.bus.write(&[c]).map_err(DmaSpiError::Bus)?;
        }
        if !data.is_empty() {
            self.dc.set_high().map_err(DmaSpiError::Pin)?;
            self.bus.write(data).map_err(DmaSpiError::Bus)?;
        }
        Ok(())
    }
}

impl<B, CS, DC> DcsInterface for DmaSpiInterface<B, CS, DC>
where
    B: DmaSpiBus,
    CS: OutputPin,
    DC: OutputPin<Error = CS::Error>,
{
    type Error = Error<B, CS>;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        self.framed(Some(cmd), params)
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.framed(None, data)
    }

    /// Starts the DMA transfer (after the previous one finished) with CS low and DC high, and
    /// keeps `buf` until [`poll_pixels`](DcsInterface::poll_pixels) returns it.
    fn start_pixels(
        &mut self,
        buf: DrawBufferMem,
        len: usize,
    ) -> Result<Option<DrawBufferMem>, (Self::Error, DrawBufferMem)> {
        if let Err(e) = self.settle() {
            return Err((e, buf));
        }
        let pins = self
            .cs
            .set_low()
            .and_then(|()| self.dc.set_high())
            .map_err(DmaSpiError::Pin);
        if let Err(e) = pins {
            let _ = self.cs.set_high();
            return Err((e, buf));
        }
        let len = len.min(buf.len());
        match self.bus.start_dma(buf, len) {
            Ok(()) => {
                self.running = true;
                Ok(None)
            }
            Err((e, buf)) => {
                let _ = self.cs.set_high();
                Err((DmaSpiError::Bus(e), buf))
            }
        }
    }

    fn poll_pixels(&mut self) -> Option<DrawBufferMem> {
        if self.try_finish().is_err() {
            warn!(target: "twine::driver", "dma spi: releasing CS after a transfer failed");
        }
        self.done.pop_front()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::boxed::Box;
    use std::vec::Vec;

    use super::*;

    #[derive(Debug, PartialEq)]
    enum Op {
        Write(Vec<u8>),
        Dma(usize),
    }

    /// A bus whose DMA takes `polls` polls; records what it was asked to do.
    struct Bus {
        ops: Vec<Op>,
        running: Option<(DrawBufferMem, u32)>,
        polls: u32,
    }

    impl DmaSpiBus for Bus {
        type Error = ();
        fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
            assert!(self.running.is_none(), "write during a transfer");
            self.ops.push(Op::Write(bytes.to_vec()));
            Ok(())
        }
        fn start_dma(&mut self, buf: DrawBufferMem, len: usize) -> Result<(), ((), DrawBufferMem)> {
            assert!(self.running.is_none(), "two transfers at once");
            self.ops.push(Op::Dma(len));
            self.running = Some((buf, self.polls));
            Ok(())
        }
        fn finish_dma(&mut self) -> Option<DrawBufferMem> {
            match self.running.take()? {
                (buf, 0) => Some(buf),
                (buf, n) => {
                    self.running = Some((buf, n - 1));
                    None
                }
            }
        }
    }

    struct Pin;
    impl embedded_hal::digital::ErrorType for Pin {
        type Error = core::convert::Infallible;
    }
    impl OutputPin for Pin {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        fn set_high(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn iface(polls: u32) -> DmaSpiInterface<Bus, Pin, Pin> {
        DmaSpiInterface::new(
            Bus {
                ops: Vec::new(),
                running: None,
                polls,
            },
            Pin,
            Pin,
        )
    }

    fn buf(len: usize) -> DrawBufferMem {
        DrawBufferMem::new(Box::leak(std::vec![0u8; len].into_boxed_slice()))
    }

    #[test]
    fn pixels_run_in_the_background_and_come_back_in_order() {
        let mut i = iface(3);
        assert!(i.start_pixels(buf(8), 6).unwrap().is_none());
        assert!(i.poll_pixels().is_none(), "still running");
        // A command waits for the transfer, which then is handed back.
        i.command(0x2A, &[1, 2]).unwrap();
        let b = i.poll_pixels().expect("finished by the command's wait");
        assert_eq!(b.len(), 8);
        assert!(i.start_pixels(buf(4), 4).unwrap().is_none());
        assert!(
            i.start_pixels(buf(2), 9).unwrap().is_none(),
            "waits for the previous one"
        );
        assert_eq!(i.poll_pixels().map(|b| b.len()), Some(4));
        assert!(i.poll_pixels().is_none(), "the second transfer still runs");
        let last = (0..10).find_map(|_| i.poll_pixels());
        assert_eq!(last.map(|b| b.len()), Some(2));
        let (bus, _, _) = i.release();
        assert_eq!(
            bus.ops,
            [
                Op::Dma(6),
                Op::Write(std::vec![0x2A]),
                Op::Write(std::vec![1, 2]),
                Op::Dma(4),
                Op::Dma(2) // clamped to the buffer
            ]
        );
    }

    #[test]
    fn hung_transfer_times_out_instead_of_spinning() {
        let mut i = iface(u32::MAX).with_max_polls(10);
        assert!(i.start_pixels(buf(8), 8).unwrap().is_none());
        assert_eq!(i.command(0x2C, &[]), Err(DmaSpiError::Timeout));
        let (e, b) = i.start_pixels(buf(4), 4).unwrap_err();
        assert_eq!(e, DmaSpiError::Timeout);
        assert_eq!(b.len(), 4, "the refused buffer is returned");
    }
}
