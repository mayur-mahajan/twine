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
//! | [`run`], [`run_headless`] | declarative apps (`fn app(cx: Scope) -> impl View`) with the `Ui` update cycle |
//! | [`run_engine`], [`run_engine_headless`] | engine apps: an `Engine` on a simulated display, stepped only when it asks to be woken |
//! | [`show_framebuffer`], [`show_framebuffer_with_input`] | draw raw frames without the engine (with per-frame input: [`SimFrame`]); window or headless |
//! | [`run_headless_framebuffer`] | the same, headless, returning a [`HeadlessReport`] |
//! | [`SimConfig`], [`Headless`] | configuration, environment overrides (`TWINE_SIM_*`) |
//! | [`set_title`] | change the window title at run time (e.g. to show an example's current settings) |
//! | [`SimDisplay`] | emulated panel (formats, bus speed, hardware or software rotation) |
//! | [`SimFramebufferDisplay`] | emulated memory-mapped panel for the `Full` / `Direct` buffer modes |
//! | [`SimPointer`], [`SimKeypad`], [`SimEncoder`] | input devices fed by the window or a script |
//! | [`Hotkey`] | F1 help, F9 screenshot, F10 recording; F2 refresh debug, F3 layout bounds, F4 performance overlay, F8 tree dump for engine apps |
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
mod fb_display;
pub mod hotkeys;
pub mod input;
pub mod paths;
mod png_out;
pub mod script;
mod title;
mod window;

pub use app::{HEADLESS_FRAME, HeadlessReport, SimApp, SimClock, SimError, SimFrame, StepFn};
pub use config::{Headless, RawKeyHook, SimConfig, SimInputs, ThemeToggle};
pub use display::{FlushStat, SimDisplay, SimDisplayError};
pub use fb_display::SimFramebufferDisplay;
pub use hotkeys::Hotkey;
pub use input::{SimDevices, SimEncoder, SimKeypad, SimPointer};
pub use title::set_title;

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

/// Runs an engine app and never returns: creates an [`Engine`](twine_engine::Engine) with a
/// simulated display (buffer mode, format, rotation and bus speed from `cfg`, after
/// [`SimConfig::from_env`]), calls `setup` to build the UI imperatively, then steps the engine
/// whenever it asks to be woken — an idle UI leaves the event loop waiting, using no CPU.
///
/// Hotkeys: F2 refresh-area debug overlay, F3 layout bounds, F4 performance overlay, F8 tree
/// dump (plus F1, F9, F10). Headless runs (`TWINE_SIM_HEADLESS=1`) execute the script.
///
/// ```no_run
/// use twine_core::{Color, Opa};
/// use twine_engine::Obj;
/// use twine_sim::{SimConfig, run_engine};
/// use twine_style::{Selector, StyleProp};
///
/// run_engine(SimConfig::new(320, 240), |engine| {
///     let screen = engine.active_screen(engine.default_display().unwrap()).unwrap();
///     let b = engine.create(screen, Box::new(Obj)).unwrap();
///     engine.set_pos(b, 20, 20);
///     engine.set_size(b, 100, 60);
///     engine.set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::RED));
///     engine.set_local_prop(b, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
/// });
/// ```
pub fn run_engine(cfg: SimConfig, setup: impl FnOnce(&mut twine_engine::Engine)) -> ! {
    init_logging();
    match SimApp::engine(cfg.from_env(), setup) {
        Ok(app) => app.run(),
        Err(e) => {
            eprintln!("twine-sim: {e}");
            std::process::exit(1)
        }
    }
}

/// Runs a declarative app and never returns: builds `app` (once) with a `Ui` update cycle on
/// a simulated display (like [`run_engine`], after [`SimConfig::from_env`]) with the pointer,
/// keypad and encoder of `cfg.input` and `cfg.theme` (default: LVGL's light default theme),
/// then updates whenever the `Ui` asks to be woken — `Wake::Idle` leaves the event loop
/// waiting, using no CPU, until an input or a channel message arrives (channel sends from
/// other threads wake the window).
///
/// ```no_run
/// use twine_sim::SimConfig;
/// use twine_view::prelude::*;
///
/// fn hello(_cx: Scope) -> impl View {
///     label("Hello, twine!")
/// }
///
/// twine_sim::run(SimConfig::new(320, 240).title("hello"), hello);
/// ```
pub fn run<V: twine_view::View>(cfg: SimConfig, app: fn(twine_reactive::Scope) -> V) -> ! {
    init_logging();
    match build_ui_app(cfg.from_env(), app) {
        Ok(sim) => sim.run(),
        Err(e) => {
            eprintln!("twine-sim: {e}");
            std::process::exit(1)
        }
    }
}

/// Runs a declarative app headless (with `cfg.headless` or the defaults) and returns a
/// report. The environment is **not** applied.
pub fn run_headless<V: twine_view::View>(
    cfg: SimConfig,
    app: fn(twine_reactive::Scope) -> V,
) -> Result<HeadlessReport, SimError> {
    let cfg = if cfg.headless.is_some() {
        cfg
    } else {
        cfg.headless(Some(Headless::default()))
    };
    build_ui_app(cfg, app)?.run_headless()
}

/// A [`SimApp`] running `app` through a `UiCore` (see [`run`]).
fn build_ui_app<V: twine_view::View>(
    mut cfg: SimConfig,
    app: fn(twine_reactive::Scope) -> V,
) -> Result<SimApp, SimError> {
    use std::cell::RefCell;
    use std::rc::Rc;

    if cfg.theme.is_none() && cfg.theme_toggle.is_none() {
        cfg.theme = Some(Rc::new(twine_theme::DefaultTheme::light()));
    }
    let core: Rc<RefCell<Option<twine_view::UiCore>>> = Rc::default();
    let c = core.clone();
    let mut sim = SimApp::engine(cfg, move |engine| {
        if let Some(d) = engine.default_display() {
            *c.borrow_mut() = Some(twine_view::UiCore::mount(engine, d, app));
        }
    })?;
    let waker = core.borrow().as_ref().map(twine_view::UiCore::waker);
    if let Some(w) = waker {
        sim.on_waker(Box::new(move |std_waker| w.register(&std_waker)));
    }
    sim.set_step_fn(Box::new(move |engine, now| match core.borrow_mut().as_mut() {
        Some(ui) => ui.update(engine, now),
        None => engine.step(now),
    }));
    Ok(sim)
}

/// Runs an engine app headless (with `cfg.headless` or the defaults) and returns a report
/// instead of exiting. The environment is **not** applied.
pub fn run_engine_headless(
    cfg: SimConfig,
    setup: impl FnOnce(&mut twine_engine::Engine),
) -> Result<HeadlessReport, SimError> {
    let cfg = if cfg.headless.is_some() {
        cfg
    } else {
        cfg.headless(Some(Headless::default()))
    };
    SimApp::engine(cfg, setup)?.run_headless()
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
