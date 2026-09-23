//! Shared helpers for Twine simulator examples. Each example is a binary in `src/bin/`.
//!
//! [`gallery`] holds the renderer gallery pages (also used by the renderer's snapshot tests).
//!
//! [`framebuffer`] writes pixels into a raw framebuffer in any panel format the simulator
//! emulates, for the pre-engine examples that use `twine_sim::show_framebuffer`.

pub mod framebuffer;
pub mod gallery;
