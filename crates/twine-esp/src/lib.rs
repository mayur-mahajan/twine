//! # twine-esp
//!
//! Thin adapters between esp-hal and the Twine drivers, for what a generic `embedded-hal`
//! trait cannot express. Display SPI panels need nothing from here: esp-hal's `SpiDma` in
//! async mode is an `embedded_hal_async::spi::SpiBus` that writes from internal RAM without
//! copying, so `twine-drivers` + `twine-embassy` drive it directly.
//!
//! | Adapter | Implements | For |
//! |---------|------------|-----|
//! | [`EspQspi`] | [`AsyncQspiBus`] over `SpiDma<Async>` (DMA), [`QspiBus`] over `Spi<Blocking>` | QSPI AMOLED panels (`twine_drivers::co5300`, `sh8601`, `rm67162`) |
//!
//! Enable exactly one chip feature (`esp32`, `esp32s3`, `esp32c3`, `esp32c6`), matching your
//! esp-hal chip feature.
//!
//! ```ignore
//! use esp_hal::spi::master::{Config, Spi};
//! use esp_hal::time::Rate;
//! use twine_drivers::co5300::{self, CO5300_410X502};
//! use twine_esp::EspQspi;
//!
//! let spi = Spi::new(peripherals.SPI2, Config::default().with_frequency(Rate::from_mhz(40)))?
//!     .with_sck(peripherals.GPIO11)
//!     .with_cs(peripherals.GPIO12)
//!     .with_sio0(peripherals.GPIO4)
//!     .with_sio1(peripherals.GPIO5)
//!     .with_sio2(peripherals.GPIO6)
//!     .with_sio3(peripherals.GPIO7)
//!     .with_dma(peripherals.DMA_CH0)
//!     .into_async();
//! let panel = co5300::new_async(EspQspi::new(spi), Some(rst), &CO5300_410X502, Rotation::Deg0, &mut Delay).await?;
//! ```
#![no_std]
#![forbid(unsafe_code)]

use esp_hal::Async;
use esp_hal::Blocking;
use esp_hal::spi::master::{Address, Command, DataMode, Spi, SpiDma};
use twine_drivers::interface::qspi::split_write;
use twine_drivers::interface::{AsyncQspiBus, QspiBus, QspiLines};

pub use esp_hal::spi::Error;

/// Largest single DMA transfer of esp-hal's SPI master (bytes); longer pixel writes are
/// split, continuing with `RAMWRC`.
pub const MAX_TRANSFER: usize = 32_736;

/// A quad-SPI panel bus over an esp-hal SPI master (`SpiDma<Async>` for DMA transfers,
/// `Spi<Blocking>` for CPU-driven ones). Chip select is the SPI's hardware CS (`with_cs`).
///
/// Each [`write`](AsyncQspiBus::write) is one transaction: an 8-bit instruction and a 24-bit
/// address on one line, then the data on one or four lines. Pixel writes longer than
/// [`MAX_TRANSFER`] are split into several transactions; every one after the first carries
/// `RAMWRC` (`0x3C`) so the panel continues where the previous one stopped.
#[derive(Debug)]
pub struct EspQspi<S> {
    spi: S,
}

impl<S> EspQspi<S> {
    /// Wraps a configured SPI master (clock, mode, `SCK`, `CS`, `SIO0`–`SIO3` set).
    #[must_use]
    pub const fn new(spi: S) -> Self {
        Self { spi }
    }

    /// Returns the SPI master.
    #[must_use]
    pub fn release(self) -> S {
        self.spi
    }
}

const fn data_mode(lines: QspiLines) -> DataMode {
    match lines {
        QspiLines::Single => DataMode::Single,
        QspiLines::Quad => DataMode::Quad,
    }
}

impl AsyncQspiBus for EspQspi<SpiDma<'_, Async>> {
    type Error = Error;

    async fn write(&mut self, instr: u8, addr: u32, data: &[u8], lines: QspiLines) -> Result<(), Error> {
        for (a, piece) in split_write(instr, addr, data, MAX_TRANSFER) {
            self.spi
                .half_duplex_write_async(
                    data_mode(lines),
                    Command::_8Bit(u16::from(instr), DataMode::Single),
                    Address::_24Bit(a, DataMode::Single),
                    0,
                    piece,
                )
                .await?;
        }
        Ok(())
    }
}

impl QspiBus for EspQspi<Spi<'_, Blocking>> {
    type Error = Error;

    fn write(&mut self, instr: u8, addr: u32, data: &[u8], lines: QspiLines) -> Result<(), Error> {
        for (a, piece) in split_write(instr, addr, data, MAX_TRANSFER) {
            self.spi.half_duplex_write(
                data_mode(lines),
                Command::_8Bit(u16::from(instr), DataMode::Single),
                Address::_24Bit(a, DataMode::Single),
                0,
                piece,
            )?;
        }
        Ok(())
    }
}
