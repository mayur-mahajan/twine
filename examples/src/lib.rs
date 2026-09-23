//! Shared helpers for Twine simulator examples. Each example is a binary in `src/bin/`.
//!
//! [`framebuffer`] writes pixels into a raw framebuffer in any panel format the simulator
//! emulates, for the pre-engine examples that use `twine_sim::show_framebuffer`.

pub mod framebuffer;
