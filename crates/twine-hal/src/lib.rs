//! # twine-hal
//!
//! The hardware abstraction of the Twine GUI library: the traits a display, input device,
//! clock or platform (sleeping, waking, interrupt context) implements, and the plain data
//! types exchanged with the engine.
//!
//! `twine-hal` sits directly above `twine-core` in the crate layering. It depends only on
//! `twine-core` (plus, behind the opt-in `platform-*` features, the one crate each platform
//! implementation needs), is `no_std` and **allocation-free**, so drivers (`twine-drivers`,
//! firmware crates) can implement it on any microcontroller. The engine (`twine-engine`), the desktop
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
//! | [`Platform`] | bare metal, RTOS and OS ports ([`platform`]) | wait for a deadline or a wake-up; `notify`; `in_interrupt` |
//! | `AsyncPlatform` | async runtimes (feature `async`), `EmbassyPlatform` | the async UI's clock and timer (bounded flushes) |
//!
//! Touch drivers share [`Calibration`] (3-point affine mapping of raw resistive-touch readings)
//! and [`TouchTransform`] / [`TouchMount`] (raw capacitive-panel coordinates to the rotated
//! screen, derived from the display with [`TouchTransform::for_display`] when the engine calls
//! [`InputDevice::fit_to_display`]).
//!
//! Data types: [`DisplayInfo`], [`ControlError`], [`DrawBufferMem`], [`DrawBuffer`] (and [`buffer_bytes`]), [`Calibration`],
//! [`TouchTransform`], [`TouchMount`], [`Rotation`] (from
//! `twine-core`), [`InputKind`], [`InputData`], [`PointerData`], [`KeypadData`],
//! [`EncoderData`], [`ButtonData`], [`Key`], [`PollHint`], [`DeviceHealth`].
//!
//! ## Features
//!
//! - `async`: `AsyncDisplayDriver`, `AsyncInputWait` and `AsyncPlatform`.
//! - `platform-cortex-m`: `CortexMPlatform` (`WFE`/`SEV`, `SCB::vect_active`; adds `cortex-m`).
//! - `platform-riscv`: `RiscvPlatform` (`WFI`; adds `riscv` and `critical-section`).
//! - `platform-std`: `StdPlatform` (condition variable; needs `std`).
//! - `platform-embassy`: `EmbassyPlatform`, an `AsyncPlatform` over `embassy-time` (implies
//!   `async`).
//!
//! No platform implementation is enabled by default: the application chooses (or writes) its
//! own, so no OS, executor or chip is assumed (see [`platform`]).
//! - `defmt`: `defmt::Format` for every public type (and `defmt` logging in `twine-core`).
//! - `log`: `log` backend of the `twine-core` logging macros.
//!
//! ```
//! use twine_core::ColorFormat;
//! use twine_hal::{DisplayInfo, Key, buffer_bytes};
//!
//! let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565Swapped);
//! const BYTES: usize = buffer_bytes(320, 24, ColorFormat::Rgb565Swapped); // compile time
//! assert_eq!(BYTES, info.bytes_per_row() * 24);
//! assert_eq!(Key::from_name("Next"), Some(Key::Next));
//! ```
#![no_std]
#![forbid(unsafe_code)]

pub mod buffer;
pub mod calibration;
pub mod clock;
pub mod display;
pub mod input;
pub mod platform;
pub mod touch;

pub use buffer::{DrawBuffer, DrawBufferMem, buffer_bytes};
pub use calibration::Calibration;
pub use clock::Clock;
#[cfg(feature = "async")]
pub use display::AsyncDisplayDriver;
pub use display::{ControlError, DisplayDriver, DisplayInfo, FramebufferDisplay, Rotation};
#[cfg(feature = "async")]
pub use input::AsyncInputWait;
pub use input::{
    ButtonData, DeviceHealth, EncoderData, InputData, InputDevice, InputKind, Key, KeypadData, PointerData,
    PollHint,
};
#[cfg(feature = "platform-cortex-m")]
pub use platform::CortexMPlatform;
#[cfg(feature = "platform-embassy")]
pub use platform::EmbassyPlatform;
pub use platform::Platform;
#[cfg(feature = "platform-riscv")]
pub use platform::RiscvPlatform;
#[cfg(feature = "platform-std")]
pub use platform::StdPlatform;
#[cfg(feature = "async")]
pub use platform::{AsyncPlatform, WaitUntil};
pub use touch::{TouchMount, TouchTransform};
