//! [`I2cInterface`]: command/data over I2C with a control byte (SSD1306, SH1106).

use embedded_hal::i2c::Operation;
use twine_core::log::trace;

use super::DcsInterface;

/// Control byte announcing a stream of command bytes (Co = 0, D/C# = 0).
const CONTROL_COMMANDS: u8 = 0x00;
/// Control byte announcing a stream of display RAM data (Co = 0, D/C# = 1).
const CONTROL_DATA: u8 = 0x40;

/// I2C transport for controllers that prefix every write with a control byte: `0x00` for
/// commands (parameters are sent as command bytes), `0x40` for display data (SSD1306 datasheet
/// §8.1.5, SH1106 datasheet "I2C-bus write data").
///
/// Each call is one I2C transaction (the control byte and the payload are written as one
/// transfer without a repeated start).
///
/// # Wiring
///
/// | Module pin | MCU |
/// |------------|-----|
/// | `SCL`, `SDA` | I2C bus (with pull-ups, usually on the module) |
/// | address | `0x3C` (SA0 low, most modules) or `0x3D` |
///
/// ```
/// use twine_drivers::interface::{DcsInterface, I2cInterface};
/// use twine_drivers::testkit::{BusOp, Recorder};
///
/// let rec = Recorder::new();
/// let mut iface = I2cInterface::new(rec.i2c(), 0x3C);
/// iface.command_bytes(&[0xAE, 0xD5, 0x80]).unwrap();
/// assert_eq!(rec.ops(), [BusOp::I2cWrite { addr: 0x3C, data: vec![0x00, 0xAE, 0xD5, 0x80] }]);
/// ```
#[derive(Debug)]
pub struct I2cInterface<I2C> {
    i2c: I2C,
    addr: u8,
}

impl<I2C> I2cInterface<I2C> {
    /// The usual address of SSD1306/SH1106 modules (SA0 low).
    pub const DEFAULT_ADDR: u8 = 0x3C;

    /// Wraps an I2C bus; `addr` is the 7-bit device address.
    #[must_use]
    pub const fn new(i2c: I2C, addr: u8) -> Self {
        Self { i2c, addr }
    }

    /// The device address.
    #[must_use]
    pub const fn addr(&self) -> u8 {
        self.addr
    }

    /// Returns the bus.
    #[must_use]
    pub fn release(self) -> I2C {
        self.i2c
    }
}

impl<I2C: embedded_hal::i2c::I2c> DcsInterface for I2cInterface<I2C> {
    type Error = I2C::Error;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "i2c cmd {:x} len {}", cmd, params.len());
        self.i2c.transaction(
            self.addr,
            &mut [
                Operation::Write(&[CONTROL_COMMANDS, cmd]),
                Operation::Write(params),
            ],
        )
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.i2c.transaction(
            self.addr,
            &mut [Operation::Write(&[CONTROL_DATA]), Operation::Write(data)],
        )
    }

    fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.i2c.transaction(
            self.addr,
            &mut [Operation::Write(&[CONTROL_COMMANDS]), Operation::Write(bytes)],
        )
    }
}

#[cfg(feature = "async")]
impl<I2C: embedded_hal_async::i2c::I2c> super::AsyncDcsInterface for I2cInterface<I2C> {
    type Error = I2C::Error;

    async fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "i2c cmd {:x} len {}", cmd, params.len());
        self.i2c
            .transaction(
                self.addr,
                &mut [
                    Operation::Write(&[CONTROL_COMMANDS, cmd]),
                    Operation::Write(params),
                ],
            )
            .await
    }

    async fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.i2c
            .transaction(
                self.addr,
                &mut [Operation::Write(&[CONTROL_DATA]), Operation::Write(data)],
            )
            .await
    }

    async fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.i2c
            .transaction(
                self.addr,
                &mut [Operation::Write(&[CONTROL_COMMANDS]), Operation::Write(bytes)],
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use alloc::vec;

    #[test]
    fn control_bytes() {
        let rec = Recorder::new();
        let mut i = I2cInterface::new(rec.i2c(), 0x3D);
        assert_eq!(i.addr(), 0x3D);
        i.command(0x81, &[0x5F]).unwrap();
        i.write_pixels(&[1, 2]).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::I2cWrite {
                    addr: 0x3D,
                    data: vec![0x00, 0x81, 0x5F]
                },
                BusOp::I2cWrite {
                    addr: 0x3D,
                    data: vec![0x40, 1, 2]
                },
            ]
        );
        assert_eq!(rec.i2c_transactions(), 2);
        let _ = i.release();
    }

    #[test]
    fn async_matches_blocking() {
        let a = Recorder::new();
        let mut i = I2cInterface::new(a.i2c(), 0x3C);
        i.command(0x81, &[0x5F]).unwrap();
        i.command_bytes(&[0xAF]).unwrap();
        i.write_pixels(&[9]).unwrap();
        let b = Recorder::new();
        block_on(async {
            let mut i = I2cInterface::new(b.i2c(), 0x3C);
            crate::interface::AsyncDcsInterface::command(&mut i, 0x81, &[0x5F])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::command_bytes(&mut i, &[0xAF])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::write_pixels(&mut i, &[9])
                .await
                .unwrap();
        });
        assert_eq!(a.ops(), b.ops());
    }
}
