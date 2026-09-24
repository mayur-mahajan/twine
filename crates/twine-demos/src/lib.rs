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
#![no_std]

extern crate alloc;

mod assets;
pub mod calibration;
pub mod controls;
pub mod counter;
pub mod text_input;
pub mod thermostat;
