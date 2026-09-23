//! # twine-sim
//!
//! The desktop simulator of the Twine GUI library:
//! a native window (winit 0.30 + softbuffer 0.4, pure Rust) showing an **emulated panel** — a
//! framebuffer in the panel's native pixel format, optionally behind an emulated SPI bus — plus
//! mouse/keyboard input as twine pointer, keypad and encoder devices, utility hotkeys, and a
//! headless mode for scripted, deterministic runs in CI.
//!
//! `twine-sim` is a std-only crate at the top of the crate layering. It depends on `twine-hal` for the driver traits
//! (its [`SimDisplay`] is a real [`DisplayDriver`](twine_hal::DisplayDriver)) and on the
//! engine and view crates for the engine and declarative runners.
//!
//! | Item | Purpose |
//! |------|---------|
//! | [`show_framebuffer`], [`show_framebuffer_with_input`] | draw raw frames without the engine (with per-frame input: [`SimFrame`]); window or headless |
//! | [`run_headless_framebuffer`] | the same, headless, returning a [`HeadlessReport`] |
//! | [`SimConfig`], [`Headless`] | configuration, environment overrides (`TWINE_SIM_*`) |
//! | [`SimDisplay`] | emulated panel (formats, bus speed, `hw_rotation`) |
//! | [`SimPointer`], [`SimKeypad`], [`SimEncoder`] | input devices fed by the window or a script |
//! | [`Hotkey`] | F1 help, F9 screenshot, F10 recording, F2–F8/F12 for engine runners |
//! | [`script`] | the `.twinescript` language of headless runs |
//!
//! ## Environment
//!
//! `TWINE_SIM_SCALE=1..4`, `TWINE_SIM_HEADLESS=1`, `TWINE_SIM_SCRIPT=<path>`,
//! `TWINE_SIM_FRAMES=<n>`, `TWINE_SIM_BUS_HZ=<bits per second>`,
//! `TWINE_SIM_FORMAT=rgb565|rgb565swapped|rgb888|xrgb8888|argb8888|l8|i1`, and `RUST_LOG`
//! (default `twine=info`; `twine::sim=debug` logs input events and flush times).
//!
//! ```no_run
//! use twine_core::ColorFormat;
//! use twine_sim::{SimConfig, show_framebuffer};
//!
//! show_framebuffer(SimConfig::new(320, 240).scale(2), |fb, format, frame| {
//!     assert_eq!(format, ColorFormat::Rgb565);
//!     fb.fill((frame % 256) as u8);
//! });
//! ```
#![forbid(unsafe_code)]

mod app;
pub mod config;
pub mod convert;
pub mod display;
pub mod hotkeys;
pub mod input;
pub mod paths;
mod png_out;
pub mod script;
mod window;

pub use app::{HEADLESS_FRAME, HeadlessReport, SimApp, SimClock, SimError, SimFrame};
pub use config::{Headless, SimConfig, SimInputs, ThemeSlot};
pub use display::{FlushStat, SimDisplay, SimDisplayError};
pub use hotkeys::Hotkey;
pub use input::{SimDevices, SimEncoder, SimKeypad, SimPointer};

use twine_core::ColorFormat;

/// Initialises `env_logger` with the default filter `twine=info` (`RUST_LOG` overrides).
/// Idempotent.
pub fn init_logging() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("twine=info")).try_init();
}

/// Shows frames drawn by `draw(framebuffer, format, frame_index)` and never returns.
///
/// Every frame (at most [`SimConfig::fps_limit`] per second) `draw` receives the full-screen
/// framebuffer in the emulated format (`cfg.format`), which is then flushed through the
/// [`SimDisplay`] (waiting for the emulated bus) and presented. The environment
/// ([`SimConfig::from_env`]) is applied first, so `TWINE_SIM_HEADLESS=1` runs any example
/// headless. The process exits with code 0 when the window is closed or the headless run ends
/// (3 on script errors, see [`SimApp::run`]).
pub fn show_framebuffer(cfg: SimConfig, mut draw: impl FnMut(&mut [u8], ColorFormat, u32) + 'static) -> ! {
    show_framebuffer_with_input(cfg, move |fb, format, frame| draw(fb, format, frame.index))
}

/// Like [`show_framebuffer`], but `draw` also receives the frame's input ([`SimFrame`]: keys
/// pressed and clicks since the previous frame, the pointer state).
///
/// This runner redraws **every** frame at [`SimConfig::fps_limit`]: it is a development tool
/// for programs that draw without the engine. The engine runner only redraws what changed.
///
/// ```no_run
/// use twine_hal::Key;
/// use twine_sim::{SimConfig, show_framebuffer_with_input};
///
/// let mut level = 0u8;
/// show_framebuffer_with_input(SimConfig::new(64, 64), move |fb, _format, frame| {
///     if frame.keys.contains(&Key::Up) {
///         level = level.saturating_add(16);
///     }
///     fb.fill(level);
/// });
/// ```
pub fn show_framebuffer_with_input(
    cfg: SimConfig,
    draw: impl FnMut(&mut [u8], ColorFormat, &SimFrame) + 'static,
) -> ! {
    init_logging();
    SimApp::framebuffer_with_input(cfg.from_env(), draw).run()
}

/// Runs [`show_framebuffer`] headless (with `cfg.headless` or the defaults) and returns a
/// report instead of exiting. The environment is **not** applied.
///
/// ```
/// use twine_sim::{Headless, SimConfig, run_headless_framebuffer};
///
/// let out = std::env::temp_dir().join(format!("twine-sim-doc-{}", std::process::id()));
/// let cfg = SimConfig::new(8, 8).headless(Some(Headless { frames: 3, script: None, out_dir: out.clone() }));
/// let report = run_headless_framebuffer(cfg, |fb, _, frame| fb.fill(frame as u8)).unwrap();
/// assert_eq!(report.frames, 3);
/// assert!(out.join("final.png").is_file());
/// std::fs::remove_dir_all(out).unwrap();
/// ```
pub fn run_headless_framebuffer(
    cfg: SimConfig,
    draw: impl FnMut(&mut [u8], ColorFormat, u32) + 'static,
) -> Result<HeadlessReport, SimError> {
    let cfg = if cfg.headless.is_some() {
        cfg
    } else {
        cfg.headless(Some(Headless::default()))
    };
    SimApp::framebuffer(cfg, draw).run_headless()
}
