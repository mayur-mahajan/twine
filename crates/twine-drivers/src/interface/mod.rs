//! Byte-level transports for command/data display controllers: [`DcsInterface`] (blocking),
//! `AsyncDcsInterface` (feature `async`) and their implementations.
//!
//! | Interface | Bus | Notes |
//! |-----------|-----|-------|
//! | [`SpiInterface`] | 4-wire SPI (`SpiDevice` + DC pin) | the usual MIPI DCS panel wiring; DMA happens inside the HAL's `SpiDevice` |
//! | [`I2cInterface`] | I2C (control byte `0x00` / `0x40`) | SSD1306 / SH1106 OLEDs |
//! | [`I80Interface8`], [`I80Interface16`] | Intel 8080 parallel via GPIO bit-banging | slow (~1–3 MB/s), for completeness |
//!
//! A panel driver ([`MipiDcs`](crate::mipi_dcs::MipiDcs), [`Ssd1306`](crate::ssd1306::Ssd1306),
//! …) is generic over the interface, so the same panel works over any of them.

mod i2c;
mod i80;
mod spi;

pub use i2c::I2cInterface;
pub use i80::{I80Interface8, I80Interface16};
pub use spi::SpiInterface;

/// Error of an interface that drives a bus plus GPIO pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InterfaceError<S, P> {
    /// The bus (SPI) failed.
    Spi(S),
    /// A GPIO pin (DC, WR, data line) failed.
    Pin(P),
}

/// A blocking command/data transport to a display controller.
///
/// For MIPI DCS panels a command is one byte sent with DC low, followed by its parameters sent
/// with DC high. [`write_pixels`](Self::write_pixels) sends pixel data (after `RAMWR`) with DC
/// high.
pub trait DcsInterface {
    /// The transport's error type.
    type Error: core::fmt::Debug;

    /// Sends command `cmd` followed by its parameter bytes.
    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error>;

    /// Sends pixel data (the controller must be in memory-write mode, e.g. after `RAMWR`).
    ///
    /// Implementations send the whole slice in one bus transaction where the bus allows it, so
    /// a DMA-capable HAL can transfer the chunk in one go.
    fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Sends every byte of `bytes` as a command byte (controllers such as the SSD1306 take
    /// their parameters as command bytes). Default: one [`command`](Self::command) per byte.
    fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        for &b in bytes {
            self.command(b, &[])?;
        }
        Ok(())
    }
}

/// An async command/data transport (feature `async`); same semantics as [`DcsInterface`].
///
/// The futures start the bus transfer on their first poll, so a DMA transfer of
/// [`write_pixels`](Self::write_pixels) overlaps with whatever is polled alongside it.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AsyncDcsInterface {
    /// The transport's error type.
    type Error: core::fmt::Debug;

    /// Sends command `cmd` followed by its parameter bytes.
    async fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), Self::Error>;

    /// Sends pixel data (after `RAMWR`), in one bus transaction where possible.
    async fn write_pixels(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Sends every byte of `bytes` as a command byte. Default: one command per byte.
    async fn command_bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        for &b in bytes {
            self.command(b, &[]).await?;
        }
        Ok(())
    }
}
