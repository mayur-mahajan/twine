//! # twine-demos
//!
//! Demo applications of the Twine GUI library, shared by the desktop simulator examples and
//! the firmware of the supported boards. Each demo is a module with
//! `pub fn app(cx: Scope) -> impl View`; all of them are `no_std` and build for every embedded
//! target.
//!
//! | Demo | Shows |
//! |------|-------|
//! | [`calibration`] | touch calibration for resistive touch (raw readings through a channel) |
//! | [`counter`] | signals, `text!`, dynamic properties (`disabled`), click handlers |
//! | [`thermostat`] | messages from another task or interrupt ([`thermostat::SENSOR`]), tweens, memos |
//! | [`controls`] | every basic control; value widgets sharing signals (two-way bindings) |
//! | [`text_input`] | text fields, the on-screen keyboard, a spinbox and a rich-text preview |
//! | [`selection`] | dropdown, roller, list, menu, tabview, tileview, window and message box |
//! | `lottie` | Lottie animations with play/pause, loop and a two-way frame scrubber (feature `lottie`) |
//! | `vector` | paths, gradients, strokes, SVG icons, an animated path (feature `vector`) |
//! | `multilang` | translations (`tr!`), right-to-left and Arabic text, font fallback, a file image (feature `multilang`) |
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod assets;
pub mod calibration;
pub mod controls;
pub mod counter;
#[cfg(feature = "lottie")]
pub mod lottie;
#[cfg(feature = "multilang")]
pub mod multilang;
pub mod selection;
pub mod text_input;
pub mod thermostat;
#[cfg(feature = "vector")]
pub mod vector;
