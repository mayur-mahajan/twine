//! # twine-sim
//!
//! The desktop simulator of the Twine GUI library (`docs/design/10-simulator-testing.md` §1):
//! a native window (winit 0.30 + softbuffer 0.4, pure Rust) showing an **emulated panel** — a
//! framebuffer in the panel's native pixel format, optionally behind an emulated SPI bus — plus
//! mouse/keyboard input as twine pointer, keypad and encoder devices, utility hotkeys, and a
//! headless mode for scripted, deterministic runs in CI.
//!
//! `twine-sim` is a std-only, layer-12 crate. It depends on `twine-hal` for the driver traits
//! (its [`SimDisplay`] is a real [`DisplayDriver`](twine_hal::DisplayDriver)) and, from later
//! phases, on the engine and view crates for `run_engine` / `run`.
//!
//! | Item | Purpose |
//! |------|---------|
//! | [`show_framebuffer`] | draw raw frames (pre-engine phases); window or headless |
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

pub use app::{HEADLESS_FRAME, HeadlessReport, SimApp, SimClock, SimError};
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
// NOTE(P03.S03): `show_framebuffer_with_input` with per-frame input; this becomes a wrapper.
pub fn show_framebuffer(cfg: SimConfig, draw: impl FnMut(&mut [u8], ColorFormat, u32) + 'static) -> ! {
    init_logging();
    SimApp::framebuffer(cfg.from_env(), draw).run()
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
