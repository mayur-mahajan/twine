//! [`SpiInterface`]: 4-wire SPI with a data/command (DC) pin.

use embedded_hal::digital::OutputPin;
use twine_core::log::trace;

use super::{DcsInterface, InterfaceError};

/// 4-wire SPI transport: an `SpiDevice` (which owns chip select) plus a DC pin.
///
/// DC low: command byte; DC high: parameters and pixel data. The blocking implementation needs
/// `embedded_hal::spi::SpiDevice`; the async one (feature `async`) needs
/// `embedded_hal_async::spi::SpiDevice` — use an async, DMA-backed `SpiDevice` (e.g.
/// `embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice`) to overlap pixel transfers with
/// rendering.
///
/// # Wiring
///
/// | Panel pin | MCU |
/// |-----------|-----|
/// | `SCK`, `SDI`/`MOSI` | SPI clock / MOSI (`MISO` is not needed) |
/// | `CS` | chip select owned by the `SpiDevice` |
/// | `DC` / `RS` | any GPIO → `dc` |
///
/// ```
/// use twine_drivers::interface::{DcsInterface, SpiInterface};
/// use twine_drivers::testkit::{BusOp, Recorder};
///
/// let rec = Recorder::new();
/// let mut iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
/// iface.command(0x2A, &[0x00, 0x00, 0x00, 0xEF]).unwrap();
/// assert_eq!(rec.ops(), [BusOp::Cmd(0x2A), BusOp::Data(vec![0, 0, 0, 0xEF])]);
/// ```
#[derive(Debug)]
pub struct SpiInterface<SPI, DC> {
    spi: SPI,
    dc: DC,
}

impl<SPI, DC> SpiInterface<SPI, DC> {
    /// Wraps an SPI device and the DC pin.
    #[must_use]
    pub const fn new(spi: SPI, dc: DC) -> Self {
        Self { spi, dc }
    }

    /// Returns the SPI device and the DC pin.
    #[must_use]
    pub fn release(self) -> (SPI, DC) {
        (self.spi, self.dc)
    }
}

impl<SPI: embedded_hal::spi::SpiDevice, DC: OutputPin> DcsInterface for SpiInterface<SPI, DC> {
    type Error = InterfaceError<SPI::Error, DC::Error>;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "dcs cmd {:x} len {}", cmd, params.len());
        self.dc.set_low().map_err(InterfaceError::Pin)?;
        self.spi.write(&[cmd]).map_err(InterfaceError::Spi)?;
        if !params.is_empty() {
            self.dc.set_high().map_err(InterfaceError::Pin)?;
            self.spi.write(params).map_err(InterfaceError::Spi)?;
        }
        Ok(())
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.dc.set_high().map_err(InterfaceError::Pin)?;
        self.spi.write(data).map_err(InterfaceError::Spi)
    }

    fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "cmd bytes len {}", bytes.len());
        self.dc.set_low().map_err(InterfaceError::Pin)?;
        self.spi.write(bytes).map_err(InterfaceError::Spi)
    }
}

#[cfg(feature = "async")]
impl<SPI: embedded_hal_async::spi::SpiDevice, DC: OutputPin> super::AsyncDcsInterface
    for SpiInterface<SPI, DC>
{
    type Error = InterfaceError<SPI::Error, DC::Error>;

    async fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "dcs cmd {:x} len {}", cmd, params.len());
        self.dc.set_low().map_err(InterfaceError::Pin)?;
        self.spi.write(&[cmd]).await.map_err(InterfaceError::Spi)?;
        if !params.is_empty() {
            self.dc.set_high().map_err(InterfaceError::Pin)?;
            self.spi.write(params).await.map_err(InterfaceError::Spi)?;
        }
        Ok(())
    }

    async fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.dc.set_high().map_err(InterfaceError::Pin)?;
        self.spi.write(data).await.map_err(InterfaceError::Spi)
    }

    async fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        trace!(target: "twine::driver", "cmd bytes len {}", bytes.len());
        self.dc.set_low().map_err(InterfaceError::Pin)?;
        self.spi.write(bytes).await.map_err(InterfaceError::Spi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn spi_interface_command_sets_dc_low_then_high() {
        let rec = Recorder::new();
        let mut iface = SpiInterface::new(rec.spi(), rec.pin("dc"));
        iface.command(0x36, &[0x48]).unwrap();
        iface.command(0x29, &[]).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::Pin("dc", false),
                BusOp::Cmd(0x36),
                BusOp::Pin("dc", true),
                BusOp::Data(vec![0x48]),
                BusOp::Pin("dc", false),
                BusOp::Cmd(0x29),
            ]
        );
    }

    #[test]
    fn write_pixels_single_transaction() {
        let rec = Recorder::new();
        let mut iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
        let data: Vec<u8> = (0..=255).cycle().take(4000).collect();
        iface.write_pixels(&data).unwrap();
        assert_eq!(rec.ops(), [BusOp::Data(data)]);
        assert_eq!(rec.spi_transactions(), 1);
    }

    #[test]
    fn command_bytes_one_transaction_dc_low() {
        let rec = Recorder::new();
        let mut iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
        iface.command_bytes(&[0xAE, 0xD5, 0x80]).unwrap();
        assert_eq!(rec.ops(), [BusOp::Cmd(0xAE), BusOp::Cmd(0xD5), BusOp::Cmd(0x80)]);
        assert_eq!(rec.spi_transactions(), 1);
    }

    #[test]
    fn errors_are_mapped() {
        let rec = Recorder::new();
        let mut iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
        rec.fail_next();
        assert!(matches!(iface.command(1, &[]), Err(InterfaceError::Pin(_))));
        let (_spi, _dc) = iface.release();
    }

    #[test]
    fn async_interface_same_sequence_as_blocking() {
        fn run_blocking(rec: &Recorder) {
            let mut i = SpiInterface::new(rec.spi(), rec.pin("dc"));
            i.command(0x2A, &[0, 0, 0, 0xEF]).unwrap();
            i.command(0x2C, &[]).unwrap();
            i.write_pixels(&[1, 2, 3, 4]).unwrap();
            i.command_bytes(&[0x11, 0x29]).unwrap();
        }
        let a = Recorder::new();
        run_blocking(&a);
        let b = Recorder::new();
        block_on(async {
            let mut i = SpiInterface::new(b.spi(), b.pin("dc"));
            crate::interface::AsyncDcsInterface::command(&mut i, 0x2A, &[0, 0, 0, 0xEF])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::command(&mut i, 0x2C, &[])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::write_pixels(&mut i, &[1, 2, 3, 4])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::command_bytes(&mut i, &[0x11, 0x29])
                .await
                .unwrap();
        });
        assert_eq!(a.ops(), b.ops());
        assert_eq!(a.pixel_bytes(), b.pixel_bytes());
    }
}
