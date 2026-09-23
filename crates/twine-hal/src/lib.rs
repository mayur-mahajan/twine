//! # twine-hal
//!
//! The hardware abstraction of the Twine GUI library: the traits a display, input device or
//! clock driver implements, and the plain data types exchanged with the engine.
//!
//! `twine-hal` sits directly above `twine-core` in the crate layering. It depends only on
//! `twine-core`, is `no_std` and **allocation-free**, so drivers (`twine-drivers`, firmware
//! crates) can implement it on any microcontroller. The engine (`twine-engine`), the desktop
//! simulator (`twine-sim`) and the test harness (`twine-testing`) consume it.
//!
//! | Trait | Implemented by | Purpose |
//! |-------|----------------|---------|
//! | [`DisplayDriver`] | SPI/i80 panel drivers, `SimDisplay`, `MemoryDisplay` | blocking or DMA flush of rendered chunks (`begin_flush` / `poll_flush`) |
//! | `AsyncDisplayDriver` | async panel drivers (feature `async`) | `async fn flush` for embassy |
//! | [`FramebufferDisplay`] | memory-mapped panels (LTDC, RGB, DSI) | hands out framebuffers, `present` at vsync |
//! | [`InputDevice`] | touch, keypad, encoder, button drivers; mocks | non-blocking `read` of [`InputData`] |
//! | `AsyncInputWait` | IRQ-driven input drivers (feature `async`) | `wait_for_interrupt` |
//! | [`Clock`] | platform timers, `MockClock` | monotonic [`Instant`](twine_core::Instant) |
//!
//! Data types: [`DisplayInfo`], [`BufferSpec`], [`DrawBufferMem`], [`Rotation`] (from
//! `twine-core`), [`InputKind`], [`InputData`], [`PointerData`], [`KeypadData`],
//! [`EncoderData`], [`ButtonData`], [`Key`], [`PollHint`].
//!
//! ## Features
//!
//! - `async`: `AsyncDisplayDriver` and `AsyncInputWait`.
//! - `defmt`: `defmt::Format` for every public type (and `defmt` logging in `twine-core`).
//! - `log`: `log` backend of the `twine-core` logging macros.
//!
//! ```
//! use twine_core::ColorFormat;
//! use twine_hal::{BufferSpec, DisplayInfo, Key};
//!
//! let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped);
//! let bytes = BufferSpec::PartialDouble { rows: 24 }.bytes_per_buffer(&info);
//! assert_eq!(bytes, 320 * 2 * 24);
//! assert_eq!(Key::from_name("Next"), Some(Key::Next));
//! ```
#![no_std]
#![forbid(unsafe_code)]

pub mod buffer;
pub mod clock;
pub mod display;
pub mod input;

pub use buffer::DrawBufferMem;
pub use clock::Clock;
#[cfg(feature = "async")]
pub use display::AsyncDisplayDriver;
pub use display::{BufferSpec, DisplayDriver, DisplayInfo, FramebufferDisplay, Rotation};
#[cfg(feature = "async")]
pub use input::AsyncInputWait;
pub use input::{
    ButtonData, EncoderData, InputData, InputDevice, InputKind, Key, KeypadData, PointerData, PollHint,
};
