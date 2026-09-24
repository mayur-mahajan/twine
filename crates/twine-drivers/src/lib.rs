//! # twine-drivers
//!
//! Display, touch and input drivers of the Twine GUI library, implementing the `twine-hal`
//! traits on top of `embedded-hal` 1.0 buses. Everything is `no_std` and allocation-free and
//! works on any microcontroller HAL.
//!
//! | Module | Devices |
//! |--------|---------|
//! | `interface` | transports: `SpiInterface` (SPI + DC), `QspiInterface` (quad SPI) and friends |
//! | `mipi_dcs` | generic MIPI DCS panel driver `MipiDcs` |
//! | `ili9341` | ILI9341 240 × 320 |
//! | `ili9342` | ILI9342C 320 × 240 |
//! | `ili9488` | ILI9488 320 × 480 (RGB666 over SPI) |
//! | `st7789` | ST7789 240 × 320, 240 × 240, 135 × 240 |
//! | `st7735` | ST7735R/S 128 × 160 (green/red/black tab), 80 × 160 |
//! | `st7796` | ST7796S 320 × 480 |
//! | `gc9a01` | GC9A01 240 × 240 round |
//! | `jd9853` | JD9853 172 × 320 IPS |
//! | `co5300`, `sh8601`, `rm67162` | QSPI AMOLED controllers (410 × 502, 368 × 448, 240 × 536) |
//! | `ssd1306`, `sh1106` | monochrome OLEDs (I2C/SPI, `I1` page conversion in `mono`) |
//! | `touch` | touch: XPT2046, `FT6x36`, `FT5x06`, GT911, CST816S, AXS5106L, STMPE811 |
//! | `encoder` | rotary encoder on GPIOs (quadrature + button) |
//! | `keypad` | key matrix on GPIOs |
//!
//! Every display driver has a blocking flavour (`DisplayDriver`) and, with feature `async`, an
//! async flavour (`AsyncDisplayDriver`) that overlaps DMA transfers with rendering when the HAL
//! provides an async, DMA-backed `SpiDevice`.
//!
//! ## Atomics on chips without compare-and-swap
//!
//! The crate uses `portable-atomic` and does not choose its fallback: the application does. On
//! targets without atomic read-modify-write instructions — `thumbv6m` (RP2040, Cortex-M0/M0+)
//! and single-core `riscv32imc` (ESP32-C3) — add one of:
//!
//! - `portable-atomic = { version = "1", features = ["critical-section"] }` plus a
//!   critical-section implementation (e.g. `embassy-rp`'s `critical-section-impl`,
//!   `cortex-m`'s `critical-section-single-core`), or
//! - `--cfg portable_atomic_unsafe_assume_single_core` in `RUSTFLAGS` (single-core chips only;
//!   esp-hal already enables `portable-atomic`'s `unsafe-assume-single-core` on the ESP32-C3).
//!
//! Not both: `portable-atomic` rejects the combination. Other targets need nothing.
//!
//! ## Features
//!
//! Every driver and bus interface is behind its own cargo feature and nothing is enabled by
//! default: enable exactly what your board has, e.g. `features = ["ili9341", "xpt2046",
//! "async"]` (a panel feature enables the interface and the MIPI DCS core it needs). `all`
//! enables everything. No driver knows about pins or boards: constructors take the HAL's bus,
//! pin and delay objects.
//!
//! - Interfaces: `spi`, `i2c`, `i80`, `qspi`; `mipi-dcs` (generic panel core for your own
//!   `PanelSpec`).
//! - Displays: `ili9341`, `ili9342`, `ili9488`, `st7789`, `st7735`, `st7796`, `gc9a01`,
//!   `jd9853`, `co5300`, `sh8601`, `rm67162`, `ssd1306`, `sh1106`.
//! - Touch and input: `xpt2046`, `ft6x36` (also `FT5x06`, FT3168), `gt911`, `cst816s`,
//!   `axs5106l`, `stmpe811`, `encoder`, `keypad`.
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

#[cfg(feature = "co5300")]
#[cfg_attr(docsrs, doc(cfg(feature = "co5300")))]
pub mod co5300;
#[cfg(any(feature = "encoder", feature = "keypad"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "encoder", feature = "keypad"))))]
pub mod debounce;
#[cfg(feature = "encoder")]
#[cfg_attr(docsrs, doc(cfg(feature = "encoder")))]
pub mod encoder;
#[cfg(feature = "gc9a01")]
#[cfg_attr(docsrs, doc(cfg(feature = "gc9a01")))]
pub mod gc9a01;
#[cfg(feature = "ili9341")]
#[cfg_attr(docsrs, doc(cfg(feature = "ili9341")))]
pub mod ili9341;
#[cfg(feature = "ili9342")]
#[cfg_attr(docsrs, doc(cfg(feature = "ili9342")))]
pub mod ili9342;
#[cfg(feature = "ili9488")]
#[cfg_attr(docsrs, doc(cfg(feature = "ili9488")))]
pub mod ili9488;
pub mod interface;
#[cfg(feature = "jd9853")]
#[cfg_attr(docsrs, doc(cfg(feature = "jd9853")))]
pub mod jd9853;
#[cfg(feature = "keypad")]
#[cfg_attr(docsrs, doc(cfg(feature = "keypad")))]
pub mod keypad;
#[cfg(feature = "mipi-dcs")]
#[cfg_attr(docsrs, doc(cfg(feature = "mipi-dcs")))]
pub mod mipi_dcs;
#[cfg(any(feature = "ssd1306", feature = "sh1106"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "ssd1306", feature = "sh1106"))))]
pub mod mono;
mod no_pin;
#[cfg(feature = "mipi-dcs")]
mod panel_macros;
#[cfg(feature = "rm67162")]
#[cfg_attr(docsrs, doc(cfg(feature = "rm67162")))]
pub mod rm67162;
#[cfg(feature = "sh1106")]
#[cfg_attr(docsrs, doc(cfg(feature = "sh1106")))]
pub mod sh1106;
#[cfg(feature = "sh8601")]
#[cfg_attr(docsrs, doc(cfg(feature = "sh8601")))]
pub mod sh8601;
#[cfg(feature = "ssd1306")]
#[cfg_attr(docsrs, doc(cfg(feature = "ssd1306")))]
pub mod ssd1306;
#[cfg(feature = "st7735")]
#[cfg_attr(docsrs, doc(cfg(feature = "st7735")))]
pub mod st7735;
#[cfg(feature = "st7789")]
#[cfg_attr(docsrs, doc(cfg(feature = "st7789")))]
pub mod st7789;
#[cfg(feature = "st7796")]
#[cfg_attr(docsrs, doc(cfg(feature = "st7796")))]
pub mod st7796;
#[cfg(any(
    feature = "xpt2046",
    feature = "ft6x36",
    feature = "gt911",
    feature = "cst816s",
    feature = "stmpe811",
    feature = "axs5106l"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        feature = "xpt2046",
        feature = "ft6x36",
        feature = "gt911",
        feature = "cst816s",
        feature = "stmpe811",
        feature = "axs5106l"
    )))
)]
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
