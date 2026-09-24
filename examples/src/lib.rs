//! Shared helpers for Twine simulator examples. Each example is a binary in `src/bin/`.
//!
//! [`gallery`] holds the renderer gallery pages (also used by the renderer's snapshot tests).
//!
//! [`assets`] holds the images converted by `cargo xtask images` (used by `image_gallery`).
//!
//! [`framebuffer`] writes pixels into a raw framebuffer in any panel format the simulator
//! emulates, for the pre-engine examples that use `twine_sim::show_framebuffer`.
//!
//! [`scenes`] builds engine scenes imperatively (the `engine_boxes` scene; the same source is
//! the reference scene of the engine's tests and benchmarks in `twine-testing`).
//!
//! [`tile`] is a labelled box sized by its text, used by the layout examples.
//!
//! [`core_widgets`] builds the `core_widgets` showcase (also run headless by its test).

pub mod assets;
pub mod core_widgets;
pub mod framebuffer;
pub mod gallery;
#[path = "../../crates/twine-testing/src/scenes.rs"]
pub mod scenes;
pub mod tile;
