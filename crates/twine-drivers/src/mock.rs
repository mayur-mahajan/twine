//! Recording mock buses (public as `testkit` with the `testkit` feature; see its docs).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt::{self, Write as _};
use core::future::Future;
use core::pin::Pin as CorePin;
use core::task::{Context, Poll, Waker};

use embedded_hal::digital;
use embedded_hal::i2c;
use embedded_hal::spi::{self, Operation};

/// One recorded bus operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BusOp {
    /// A command byte (SPI byte written while `dc` is low).
    Cmd(u8),
    /// Data bytes of one SPI write (or the transmitted bytes of a transfer).
    Data(Vec<u8>),
    /// Pixel data after `RAMWR`/`RAMWRC`: byte count of one SPI write.
    Pixels(usize),
    /// A delay in microseconds.
    DelayUs(u32),
    /// A logged pin changed its level (`true` = high).
    Pin(&'static str, bool),
    /// An i80 write strobe (rising edge of `"wr"`): level of `"dc"` and the latched data pins.
    Strobe {
        /// Level of the `dc` pin (`false` = command).
        dc: bool,
        /// Value on `d0`…`d15`.
        value: u16,
    },
    /// An I2C write.
    I2cWrite {
        /// 7-bit address.
        addr: u8,
        /// Bytes written.
        data: Vec<u8>,
    },
    /// An I2C read.
    I2cRead {
        /// 7-bit address.
        addr: u8,
        /// Bytes read.
        len: usize,
    },
    /// An async wait for a pin level.
    Wait(&'static str, bool),
    /// A single-line QSPI transaction (register write).
    QspiCmd {
        /// Instruction byte.
        instr: u8,
        /// 24-bit address.
        addr: u32,
        /// Data bytes.
        data: Vec<u8>,
    },
    /// A quad QSPI transaction (pixel write); the bytes go to [`Recorder::pixel_bytes`].
    QspiPixels {
        /// Instruction byte.
        instr: u8,
        /// 24-bit address.
        addr: u32,
        /// Number of data bytes.
        len: usize,
    },
}

impl fmt::Display for BusOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
            for (i, b) in bytes.iter().enumerate() {
                if i > 0 {
                    f.write_char(' ')?;
                }
                write!(f, "{b:02X}")?;
            }
            Ok(())
        }
        match self {
            BusOp::Cmd(c) => write!(f, "cmd {c:02X}"),
            BusOp::Data(d) => {
                f.write_str("data ")?;
                hex(f, d)
            }
            BusOp::Pixels(n) => write!(f, "pixels {n}"),
            BusOp::DelayUs(us) => write!(f, "delay_us {us}"),
            BusOp::Pin(name, level) => write!(f, "pin {name} {}", u8::from(*level)),
            BusOp::Strobe { dc, value } => write!(f, "strobe dc={} {value:04X}", u8::from(*dc)),
            BusOp::I2cWrite { addr, data } => {
                write!(f, "i2c_write {addr:02X} ")?;
                hex(f, data)
            }
            BusOp::I2cRead { addr, len } => write!(f, "i2c_read {addr:02X} {len}"),
            BusOp::Wait(name, level) => write!(f, "wait {name} {}", u8::from(*level)),
            BusOp::QspiCmd { instr, addr, data } => {
                write!(f, "qspi {instr:02X} {addr:06X}")?;
                if !data.is_empty() {
                    f.write_char(' ')?;
                    hex(f, data)?;
                }
                Ok(())
            }
            BusOp::QspiPixels { instr, addr, len } => write!(f, "qspi4 {instr:02X} {addr:06X} pixels {len}"),
        }
    }
}

/// Formats a log one operation per line (the format of the text fixtures).
#[must_use]
pub fn format_ops(ops: &[BusOp]) -> String {
    let mut s = String::new();
    for op in ops {
        let _ = writeln!(s, "{op}");
    }
    s
}

/// The error of every mock bus (injected with [`Recorder::fail_next`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MockError;

impl spi::Error for MockError {
    fn kind(&self) -> spi::ErrorKind {
        spi::ErrorKind::Other
    }
}

impl i2c::Error for MockError {
    fn kind(&self) -> i2c::ErrorKind {
        i2c::ErrorKind::Other
    }
}

impl digital::Error for MockError {
    fn kind(&self) -> digital::ErrorKind {
        digital::ErrorKind::Other
    }
}

type SpiResponder = Box<dyn FnMut(&[u8], &mut [u8])>;
type I2cResponder = Box<dyn FnMut(u8, &[u8], &mut [u8])>;

#[derive(Default)]
struct State {
    ops: Vec<BusOp>,
    pins: BTreeMap<&'static str, bool>,
    pixel_mode: bool,
    pixel_bytes: Vec<u8>,
    spi_transactions: usize,
    i2c_transactions: usize,
    spi_responder: Option<SpiResponder>,
    i2c_responder: Option<I2cResponder>,
    pending_once: bool,
    fail_next: bool,
}

impl State {
    fn take_failure(&mut self) -> Result<(), MockError> {
        if core::mem::take(&mut self.fail_next) {
            Err(MockError)
        } else {
            Ok(())
        }
    }

    fn spi_write(&mut self, bytes: &[u8]) {
        if self.pins.get("dc") == Some(&false) {
            for &b in bytes {
                self.ops.push(BusOp::Cmd(b));
                self.pixel_mode = matches!(b, 0x2C | 0x3C);
            }
        } else if self.pixel_mode {
            self.ops.push(BusOp::Pixels(bytes.len()));
            self.pixel_bytes.extend_from_slice(bytes);
        } else {
            self.ops.push(BusOp::Data(bytes.to_vec()));
        }
    }

    fn spi_respond(&mut self, tx: &[u8], rx: &mut [u8]) {
        rx.fill(0);
        if let Some(r) = self.spi_responder.as_mut() {
            r(tx, rx);
        }
    }

    fn spi_transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), MockError> {
        self.take_failure()?;
        self.spi_transactions += 1;
        for op in ops {
            match op {
                Operation::Write(w) => self.spi_write(w),
                Operation::Read(r) => {
                    let len = r.len();
                    self.spi_respond(&alloc::vec![0; len], r);
                }
                Operation::Transfer(r, w) => {
                    self.spi_write(w);
                    let w = w.to_vec();
                    self.spi_respond(&w, r);
                }
                Operation::TransferInPlace(buf) => {
                    let tx = buf.to_vec();
                    self.spi_write(&tx);
                    self.spi_respond(&tx, buf);
                }
                Operation::DelayNs(ns) => self.ops.push(BusOp::DelayUs(ns.div_ceil(1000))),
            }
        }
        Ok(())
    }

    fn i2c_transaction(&mut self, addr: u8, ops: &mut [i2c::Operation<'_>]) -> Result<(), MockError> {
        self.take_failure()?;
        self.i2c_transactions += 1;
        let mut written = Vec::new();
        for op in ops {
            match op {
                i2c::Operation::Write(w) => {
                    // Consecutive writes of one transaction are one bus write (no restart).
                    match self.ops.last_mut() {
                        Some(BusOp::I2cWrite { addr: a, data }) if *a == addr && !written.is_empty() => {
                            data.extend_from_slice(w);
                        }
                        _ => self.ops.push(BusOp::I2cWrite {
                            addr,
                            data: w.to_vec(),
                        }),
                    }
                    written.extend_from_slice(w);
                }
                i2c::Operation::Read(r) => {
                    self.ops.push(BusOp::I2cRead { addr, len: r.len() });
                    r.fill(0);
                    if let Some(resp) = self.i2c_responder.as_mut() {
                        resp(addr, &written, r);
                    }
                }
            }
        }
        Ok(())
    }

    fn set_pin(&mut self, name: &'static str, level: bool, logged: bool) {
        let old = self.pins.insert(name, level);
        if logged {
            self.ops.push(BusOp::Pin(name, level));
        }
        if name == "wr" && old == Some(false) && level {
            let mut value = 0u16;
            for (i, n) in DATA_PINS.iter().enumerate() {
                if self.pins.get(n) == Some(&true) {
                    value |= 1 << i;
                }
            }
            let dc = self.pins.get("dc").copied().unwrap_or(true);
            self.ops.push(BusOp::Strobe { dc, value });
        }
    }
}

const DATA_PINS: [&str; 16] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "d10", "d11", "d12", "d13", "d14", "d15",
];

/// The shared log and the factory of recording mocks. Cheap to clone (all clones share state).
#[derive(Clone, Default)]
pub struct Recorder(Rc<RefCell<State>>);

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("ops", &self.0.borrow().ops)
            .finish()
    }
}

impl Recorder {
    /// An empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A recording SPI device.
    #[must_use]
    pub fn spi(&self) -> RecordingSpi {
        RecordingSpi(self.clone())
    }

    /// A recording quad-SPI bus.
    #[must_use]
    pub fn qspi(&self) -> RecordingQspi {
        RecordingQspi(self.clone())
    }

    /// A recording I2C bus.
    #[must_use]
    pub fn i2c(&self) -> RecordingI2c {
        RecordingI2c(self.clone())
    }

    /// A recording delay.
    #[must_use]
    pub fn delay(&self) -> RecordingDelay {
        RecordingDelay(self.clone())
    }

    /// A pin whose level changes are logged as [`BusOp::Pin`].
    #[must_use]
    pub fn pin(&self, name: &'static str) -> RecordingPin {
        RecordingPin {
            rec: self.clone(),
            name,
            logged: true,
        }
    }

    /// A pin whose level is tracked (e.g. `dc`, i80 data lines) but not logged.
    #[must_use]
    pub fn quiet_pin(&self, name: &'static str) -> RecordingPin {
        RecordingPin {
            rec: self.clone(),
            name,
            logged: false,
        }
    }

    /// Sets the level an input pin reads, without logging.
    pub fn set_level(&self, name: &'static str, level: bool) {
        self.0.borrow_mut().pins.insert(name, level);
    }

    /// The current level of a pin (`None` if never set).
    #[must_use]
    pub fn level(&self, name: &'static str) -> Option<bool> {
        self.0.borrow().pins.get(name).copied()
    }

    /// A copy of the log.
    #[must_use]
    pub fn ops(&self) -> Vec<BusOp> {
        self.0.borrow().ops.clone()
    }

    /// Takes the log, leaving it empty (pin levels and responders are kept).
    #[must_use]
    pub fn take_ops(&self) -> Vec<BusOp> {
        core::mem::take(&mut self.0.borrow_mut().ops)
    }

    /// All pixel bytes written after `RAMWR`/`RAMWRC` so far.
    #[must_use]
    pub fn pixel_bytes(&self) -> Vec<u8> {
        self.0.borrow().pixel_bytes.clone()
    }

    /// Number of SPI transactions (`SpiDevice::transaction` calls, i.e. chip-select cycles).
    #[must_use]
    pub fn spi_transactions(&self) -> usize {
        self.0.borrow().spi_transactions
    }

    /// Number of I2C transactions.
    #[must_use]
    pub fn i2c_transactions(&self) -> usize {
        self.0.borrow().i2c_transactions
    }

    /// Sets the function that fills SPI read buffers: `(transmitted bytes, rx buffer)`.
    pub fn set_spi_responder(&self, f: impl FnMut(&[u8], &mut [u8]) + 'static) {
        self.0.borrow_mut().spi_responder = Some(Box::new(f));
    }

    /// Sets the function that fills I2C read buffers:
    /// `(address, bytes written earlier in the same transaction, rx buffer)`.
    pub fn set_i2c_responder(&self, f: impl FnMut(u8, &[u8], &mut [u8]) + 'static) {
        self.0.borrow_mut().i2c_responder = Some(Box::new(f));
    }

    /// The next async bus operation returns `Pending` once before completing.
    pub fn pending_once(&self) {
        self.0.borrow_mut().pending_once = true;
    }

    /// Whether a [`pending_once`](Self::pending_once) is still armed.
    #[must_use]
    pub fn pending_armed(&self) -> bool {
        self.0.borrow().pending_once
    }

    /// The next SPI/I2C transaction or pin operation fails with [`MockError`].
    pub fn fail_next(&self) {
        self.0.borrow_mut().fail_next = true;
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    fn take_pending(&self) -> bool {
        core::mem::take(&mut self.0.borrow_mut().pending_once)
    }
}

/// Returns `Pending` once (waking itself), then `Ready`.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: CorePin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

async fn maybe_yield(rec: &Recorder) {
    if rec.take_pending() {
        YieldOnce(false).await;
    }
}

/// Runs a future to completion by polling it in a loop (the mocks never block).
///
/// Panics if the future is still pending after 1000 polls (a test bug).
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = core::pin::pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
    panic!("block_on: future still pending after 1000 polls");
}

/// Polls a future exactly once; returns its output if it completed.
pub fn poll_once<F: Future + Unpin>(fut: &mut F) -> Option<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    match CorePin::new(fut).poll(&mut cx) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    }
}

/// Recording SPI device (blocking and async).
#[derive(Clone, Debug)]
pub struct RecordingSpi(Recorder);

impl spi::ErrorType for RecordingSpi {
    type Error = MockError;
}

impl spi::SpiDevice for RecordingSpi {
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), MockError> {
        self.0.with(|s| s.spi_transaction(operations))
    }
}

#[cfg(feature = "async")]
impl embedded_hal_async::spi::SpiDevice for RecordingSpi {
    async fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), MockError> {
        // The transfer "starts" (is logged) on the first poll, then optionally stays pending once.
        let r = self.0.with(|s| s.spi_transaction(operations));
        maybe_yield(&self.0).await;
        r
    }
}

/// Recording quad-SPI bus (blocking and async): one [`BusOp::QspiCmd`] or
/// [`BusOp::QspiPixels`] per transaction.
#[derive(Clone, Debug)]
pub struct RecordingQspi(Recorder);

impl State {
    fn qspi_write(
        &mut self,
        instr: u8,
        addr: u32,
        data: &[u8],
        lines: crate::interface::QspiLines,
    ) -> Result<(), MockError> {
        self.take_failure()?;
        self.spi_transactions += 1;
        match lines {
            crate::interface::QspiLines::Single => self.ops.push(BusOp::QspiCmd {
                instr,
                addr,
                data: data.to_vec(),
            }),
            crate::interface::QspiLines::Quad => {
                self.ops.push(BusOp::QspiPixels {
                    instr,
                    addr,
                    len: data.len(),
                });
                self.pixel_bytes.extend_from_slice(data);
            }
        }
        Ok(())
    }
}

impl crate::interface::QspiBus for RecordingQspi {
    type Error = MockError;
    fn write(
        &mut self,
        instr: u8,
        addr: u32,
        data: &[u8],
        lines: crate::interface::QspiLines,
    ) -> Result<(), MockError> {
        self.0.with(|s| s.qspi_write(instr, addr, data, lines))
    }
}

#[cfg(feature = "async")]
impl crate::interface::AsyncQspiBus for RecordingQspi {
    type Error = MockError;
    async fn write(
        &mut self,
        instr: u8,
        addr: u32,
        data: &[u8],
        lines: crate::interface::QspiLines,
    ) -> Result<(), MockError> {
        let r = self.0.with(|s| s.qspi_write(instr, addr, data, lines));
        maybe_yield(&self.0).await;
        r
    }
}

/// Recording I2C bus (blocking and async).
#[derive(Clone, Debug)]
pub struct RecordingI2c(Recorder);

impl i2c::ErrorType for RecordingI2c {
    type Error = MockError;
}

impl i2c::I2c for RecordingI2c {
    fn transaction(&mut self, address: u8, operations: &mut [i2c::Operation<'_>]) -> Result<(), MockError> {
        self.0.with(|s| s.i2c_transaction(address, operations))
    }
}

#[cfg(feature = "async")]
impl embedded_hal_async::i2c::I2c for RecordingI2c {
    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [i2c::Operation<'_>],
    ) -> Result<(), MockError> {
        let r = self.0.with(|s| s.i2c_transaction(address, operations));
        maybe_yield(&self.0).await;
        r
    }
}

/// Recording delay (blocking and async); logs microseconds.
#[derive(Clone, Debug)]
pub struct RecordingDelay(Recorder);

impl RecordingDelay {
    fn log(&self, us: u32) {
        self.0.with(|s| s.ops.push(BusOp::DelayUs(us)));
    }
}

impl embedded_hal::delay::DelayNs for RecordingDelay {
    fn delay_ns(&mut self, ns: u32) {
        self.log(ns.div_ceil(1000));
    }
    fn delay_us(&mut self, us: u32) {
        self.log(us);
    }
    fn delay_ms(&mut self, ms: u32) {
        self.log(ms.saturating_mul(1000));
    }
}

#[cfg(feature = "async")]
impl embedded_hal_async::delay::DelayNs for RecordingDelay {
    async fn delay_ns(&mut self, ns: u32) {
        self.log(ns.div_ceil(1000));
    }
    async fn delay_us(&mut self, us: u32) {
        self.log(us);
    }
    async fn delay_ms(&mut self, ms: u32) {
        self.log(ms.saturating_mul(1000));
    }
}

/// Recording GPIO pin: output, input and async wait.
#[derive(Clone, Debug)]
pub struct RecordingPin {
    rec: Recorder,
    name: &'static str,
    logged: bool,
}

impl RecordingPin {
    /// The pin's name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    fn set(&mut self, level: bool) -> Result<(), MockError> {
        let (name, logged) = (self.name, self.logged);
        self.rec.with(|s| {
            s.take_failure()?;
            s.set_pin(name, level, logged);
            Ok(())
        })
    }

    fn get(&self) -> Result<bool, MockError> {
        let name = self.name;
        self.rec.with(|s| {
            s.take_failure()?;
            Ok(s.pins.get(name).copied().unwrap_or(false))
        })
    }
}

impl digital::ErrorType for RecordingPin {
    type Error = MockError;
}

impl digital::OutputPin for RecordingPin {
    fn set_low(&mut self) -> Result<(), MockError> {
        self.set(false)
    }
    fn set_high(&mut self) -> Result<(), MockError> {
        self.set(true)
    }
}

impl digital::InputPin for RecordingPin {
    fn is_high(&mut self) -> Result<bool, MockError> {
        self.get()
    }
    fn is_low(&mut self) -> Result<bool, MockError> {
        self.get().map(|h| !h)
    }
}

#[cfg(feature = "async")]
impl RecordingPin {
    async fn wait_level(&mut self, level: bool) -> Result<(), MockError> {
        let name = self.name;
        self.rec.with(|s| s.ops.push(BusOp::Wait(name, level)));
        maybe_yield(&self.rec).await;
        self.rec.with(|s| {
            s.pins.insert(name, level);
        });
        Ok(())
    }
}

#[cfg(feature = "async")]
impl embedded_hal_async::digital::Wait for RecordingPin {
    async fn wait_for_high(&mut self) -> Result<(), MockError> {
        self.wait_level(true).await
    }
    async fn wait_for_low(&mut self) -> Result<(), MockError> {
        self.wait_level(false).await
    }
    async fn wait_for_rising_edge(&mut self) -> Result<(), MockError> {
        self.wait_level(true).await
    }
    async fn wait_for_falling_edge(&mut self) -> Result<(), MockError> {
        self.wait_level(false).await
    }
    async fn wait_for_any_edge(&mut self) -> Result<(), MockError> {
        let level = !self.get()?;
        self.wait_level(level).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::digital::OutputPin;
    use embedded_hal::i2c::I2c;
    use embedded_hal::spi::SpiDevice;

    #[test]
    fn pixel_mode_after_ramwr() {
        let rec = Recorder::new();
        let mut spi = rec.spi();
        let mut dc = rec.quiet_pin("dc");
        dc.set_low().unwrap();
        spi.write(&[0x2C]).unwrap();
        dc.set_high().unwrap();
        spi.write(&[1, 2, 3, 4]).unwrap();
        assert_eq!(rec.ops(), [BusOp::Cmd(0x2C), BusOp::Pixels(4)]);
        assert_eq!(rec.pixel_bytes(), [1, 2, 3, 4]);
        assert_eq!(rec.spi_transactions(), 2);
    }

    #[test]
    fn strobe_latches_data_pins() {
        let rec = Recorder::new();
        let mut wr = rec.quiet_pin("wr");
        let mut d1 = rec.quiet_pin("d1");
        let mut dc = rec.pin("dc");
        wr.set_high().unwrap();
        dc.set_low().unwrap();
        d1.set_high().unwrap();
        wr.set_low().unwrap();
        wr.set_high().unwrap();
        assert_eq!(
            rec.ops(),
            [BusOp::Pin("dc", false), BusOp::Strobe { dc: false, value: 2 }]
        );
    }

    #[test]
    fn i2c_write_read_and_format() {
        let rec = Recorder::new();
        rec.set_i2c_responder(|addr, w, r| {
            assert_eq!((addr, w), (0x38, &[0x02][..]));
            r.copy_from_slice(&[7, 8]);
        });
        let mut i2c = rec.i2c();
        let mut buf = [0; 2];
        i2c.write_read(0x38, &[0x02], &mut buf).unwrap();
        assert_eq!(buf, [7, 8]);
        assert_eq!(format_ops(&rec.ops()), "i2c_write 38 02\ni2c_read 38 2\n");
    }

    #[test]
    fn fail_next_fails_once() {
        let rec = Recorder::new();
        let mut spi = rec.spi();
        rec.fail_next();
        assert_eq!(spi.write(&[1]), Err(MockError));
        assert!(spi.write(&[1]).is_ok());
    }

    #[test]
    fn pending_once_yields_once() {
        let rec = Recorder::new();
        let mut spi = rec.spi();
        rec.pending_once();
        let mut fut = core::pin::pin!(embedded_hal_async::spi::SpiDevice::write(&mut spi, &[5]));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        // Started on the first poll:
        assert_eq!(rec.ops(), [BusOp::Data(alloc::vec![5])]);
        assert!(fut.as_mut().poll(&mut cx).is_ready());
    }
}
