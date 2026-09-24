//! # twine-drivers
//!
//! Display, touch and input drivers of the Twine GUI library, implementing the `twine-hal`
//! traits on top of `embedded-hal` 1.0 buses. Everything is `no_std` and allocation-free and
//! works on any microcontroller HAL.
//!
//! | Module | Devices |
//! |--------|---------|
//! | [`interface`] | transports: [`SpiInterface`](interface::SpiInterface) (SPI + DC) and friends |
//! | [`mipi_dcs`] | generic MIPI DCS panel driver [`MipiDcs`](mipi_dcs::MipiDcs) |
//! | [`ili9341`] | ILI9341 240 × 320 |
//! | [`ili9342`] | ILI9342C 320 × 240 |
//! | [`ili9488`] | ILI9488 320 × 480 (RGB666 over SPI) |
//! | [`st7789`] | ST7789 240 × 320, 240 × 240, 135 × 240 |
//! | [`st7735`] | ST7735R/S 128 × 160 (green/red/black tab), 80 × 160 |
//! | [`st7796`] | ST7796S 320 × 480 |
//! | [`gc9a01`] | GC9A01 240 × 240 round |
//! | [`ssd1306`], [`sh1106`] | monochrome OLEDs (I2C/SPI, `I1` page conversion in [`mono`]) |
//! | [`touch`] | touch: XPT2046, `FT6x36`, `FT5x06`, GT911, CST816S, STMPE811 |
//! | [`encoder`] | rotary encoder on GPIOs (quadrature + button) |
//! | [`keypad`] | key matrix on GPIOs |
//!
//! Every display driver has a blocking flavour (`DisplayDriver`) and, with feature `async`, an
//! async flavour (`AsyncDisplayDriver`) that overlaps DMA transfers with rendering when the HAL
//! provides an async, DMA-backed `SpiDevice`.
//!
//! ## Features
//!
//! - `async`: async interfaces and drivers (`embedded-hal-async`), `AsyncInputWait` for IRQ pins.
//! - `testkit`: recording mock buses (module `testkit`) for host tests of code built on these drivers.
//! - `defmt` / `log`: logging backends (target `twine::driver`).
//!
//! ```
//! use twine_core::Rect;
//! use twine_drivers::ili9341;
//! use twine_drivers::testkit::{BusOp, Recorder};
//! use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};
//!
//! // On hardware: `spi` is an `embedded_hal::spi::SpiDevice`, `dc`/`rst` are output pins and
//! // `delay` implements `DelayNs`. Here the recording mocks stand in for them.
//! let rec = Recorder::new();
//! let mut lcd = ili9341::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg90, &mut rec.delay()).unwrap();
//! let buf = DrawBufferMem::new(Box::leak(Box::new([0u8; 320 * 2])));
//! lcd.begin_flush(Rect::from_xywh(0, 0, 320, 1), buf).unwrap();
//! assert_eq!(rec.ops().last(), Some(&BusOp::Pixels(640)));
//! ```
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(any(test, feature = "testkit"))]
extern crate alloc;

pub mod debounce;
pub mod encoder;
pub mod gc9a01;
pub mod ili9341;
pub mod ili9342;
pub mod ili9488;
pub mod interface;
pub mod keypad;
pub mod mipi_dcs;
pub mod mono;
mod no_pin;
mod panel_macros;
pub mod sh1106;
pub mod ssd1306;
pub mod st7735;
pub mod st7789;
pub mod st7796;
pub mod touch;

#[cfg(any(test, feature = "testkit"))]
mod mock;

#[cfg(feature = "testkit")]
pub mod testkit {
    //! Recording mock buses for host-side driver tests.
    //!
    //! Every mock created from one [`Recorder`] appends to the same operation log ([`BusOp`]), so
    //! pin transitions, SPI/I2C traffic and delays interleave exactly as the driver issued them.
    //!
    //! - [`RecordingSpi`]: blocking and async `SpiDevice`. Written bytes become [`BusOp::Cmd`] (one
    //!   per byte) while the pin named `"dc"` is low, otherwise [`BusOp::Data`]. Data following a
    //!   `RAMWR` (`0x2C`) / `RAMWRC` (`0x3C`) command is logged as [`BusOp::Pixels`] (length only);
    //!   the bytes are kept in [`Recorder::pixel_bytes`].
    //! - [`RecordingPin`]: output, input and async `Wait` pin; logs [`BusOp::Pin`] unless created
    //!   with [`Recorder::quiet_pin`]. A rising edge on the pin `"wr"` latches the levels of the pins
    //!   `"d0"`…`"d15"` into a [`BusOp::Strobe`] (i80 parallel bus).
    //! - [`RecordingDelay`]: blocking and async `DelayNs`, logs [`BusOp::DelayUs`].
    //! - [`RecordingI2c`]: blocking and async `I2c`, logs [`BusOp::I2cWrite`] / [`BusOp::I2cRead`].
    //!
    //! Read data comes from responders ([`Recorder::set_spi_responder`],
    //! [`Recorder::set_i2c_responder`]); without one, reads return zeros. Async operations complete
    //! immediately, except that [`Recorder::pending_once`] makes the next async bus operation
    //! return `Pending` once (to test that a driver starts work on the first poll).
    //! [`block_on`] is a tiny executor for the async mocks.
    //!
    //! ```
    //! use embedded_hal::digital::OutputPin;
    //! use embedded_hal::spi::SpiDevice;
    //! use twine_drivers::testkit::{BusOp, Recorder};
    //!
    //! let rec = Recorder::new();
    //! let mut spi = rec.spi();
    //! let mut dc = rec.quiet_pin("dc");
    //! dc.set_low().unwrap();
    //! spi.write(&[0x2A]).unwrap();
    //! dc.set_high().unwrap();
    //! spi.write(&[0, 0, 0, 9]).unwrap();
    //! assert_eq!(rec.ops(), [BusOp::Cmd(0x2A), BusOp::Data(vec![0, 0, 0, 9])]);
    //! ```

    pub use crate::mock::*;
}

pub use no_pin::NoPin;
pub use twine_hal::Calibration;
