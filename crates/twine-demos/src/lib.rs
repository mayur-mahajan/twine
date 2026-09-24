//! # twine-demos
//!
//! Demo applications of the Twine GUI library, shared by the desktop simulator examples and
//! the firmware of the supported boards. Each demo is a module with
//! `pub fn app(cx: Scope) -> impl View`; all of them are `no_std` and build for every embedded
//! target.
//!
//! | Demo | Shows |
//! |------|-------|
//! | [`counter`] | signals, `text!`, dynamic properties (`disabled`), click handlers |
//! | [`thermostat`] | messages from another task or interrupt ([`thermostat::SENSOR`]), tweens, memos |
#![no_std]

pub mod counter;
pub mod thermostat;
