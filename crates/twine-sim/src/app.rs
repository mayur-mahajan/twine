//! [`SimApp`]: the simulator application shared by the window and the headless runner.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration as StdDuration;

use std::time::Instant as StdInstant;

use twine_core::{ColorFormat, Duration, Instant};
use twine_engine::{BufferMode, DisplayId, Engine, EngineConfig, InputId, Wake};
use twine_hal::{BufferSpec, Clock, DisplayDriver, DrawBufferMem};

use crate::config::{RawKeyHook, SimConfig};
use crate::display::SimDisplay;
use crate::fb_display::SimFramebufferDisplay;
use crate::hotkeys::{self, Hotkey, HotkeyRegistry};
use crate::input::SimDevices;
use crate::script::{self, ScriptAction, ScriptError};
use crate::{paths, png_out};

/// Simulated time advanced per headless frame.
pub const HEADLESS_FRAME: Duration = Duration::ms(16);

/// Errors of the simulator runners.
#[derive(Debug)]
pub enum SimError {
    /// The headless script could not be parsed.
    Script {
        /// The script file.
        file: PathBuf,
        /// The error with its line.
        error: ScriptError,
    },
    /// Reading the script or writing PNGs failed.
    Io(std::io::Error),
    /// The window or event loop could not be created.
    Window(String),
    /// The engine or its display could not be created.
    Engine(twine_engine::EngineError),
}

impl fmt::Display for SimError {
    /// Script errors are formatted as `file:line: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Script { file, error } => {
                write!(f, "{}:{}: {}", file.display(), error.line, error.message)
            }
            SimError::Io(e) => write!(f, "I/O error: {e}"),
            SimError::Window(e) => write!(f, "window error: {e}"),
            SimError::Engine(e) => write!(f, "engine error: {e}"),
        }
    }
}

impl std::error::Error for SimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SimError::Script { error, .. } => Some(error),
            SimError::Io(e) => Some(e),
            SimError::Engine(e) => Some(e),
            SimError::Window(_) => None,
        }
    }
}

impl From<twine_engine::EngineError> for SimError {
    fn from(e: twine_engine::EngineError) -> Self {
        SimError::Engine(e)
    }
}

impl From<std::io::Error> for SimError {
    fn from(e: std::io::Error) -> Self {
        SimError::Io(e)
    }
}

/// What a headless run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessReport {
    /// Frames rendered.
    pub frames: u32,
    /// Script time at the end (the clock source; unaffected by pausing or slow motion).
    pub sim_time: Duration,
    /// Every PNG written (script shots, hotkey screenshots, `final.png` last).
    pub shots: Vec<PathBuf>,
    /// The output directory.
    pub out_dir: PathBuf,
}

/// The simulator clock: wall time since start in a window, exactly 16 ms per frame headless,
/// with time control for animations (hotkeys F5 slow motion ×0.25, F6 pause, F7 single step).
///
/// Cheap to clone; clones share the time and the time control.
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_hal::Clock;
/// use twine_sim::SimClock;
///
/// let c = SimClock::manual();
/// c.advance(Duration::ms(100));
/// c.set_slow_motion(true);
/// c.advance(Duration::ms(100)); // a quarter of it passes
/// assert_eq!(c.now(), Instant::from_millis(125));
/// c.set_paused(true);
/// c.advance(Duration::ms(100));
/// c.step(Duration::ms(16)); // single step while paused
/// assert_eq!(c.now(), Instant::from_millis(141));
/// ```
#[derive(Clone, Debug)]
pub struct SimClock {
    kind: ClockKind,
    ctl: Rc<Cell<TimeCtl>>,
}

#[derive(Clone, Debug)]
enum ClockKind {
    Wall(std::time::Instant),
    Manual(Rc<Cell<Instant>>),
}

/// Time control: simulator time = `base_sim + (source − base_src) × speed`, frozen while
/// paused.
#[derive(Clone, Copy, Debug, Default)]
struct TimeCtl {
    slow: bool,
    paused: bool,
    base_src: u64,
    base_sim: u64,
}

impl SimClock {
    fn with_kind(kind: ClockKind) -> Self {
        Self {
            kind,
            ctl: Rc::default(),
        }
    }

    /// A clock following the wall time from now on (starting at `Instant` 0).
    #[must_use]
    pub fn wall() -> Self {
        Self::with_kind(ClockKind::Wall(std::time::Instant::now()))
    }

    /// A deterministic clock at `Instant` 0 that only moves with [`advance`](Self::advance).
    #[must_use]
    pub fn manual() -> Self {
        Self::with_kind(ClockKind::Manual(Rc::default()))
    }

    /// Advances the source of a manual clock by `d` (no effect on a wall clock); the
    /// simulator time moves by `d`, `d / 4` in slow motion, or not at all while paused.
    pub fn advance(&self, d: Duration) {
        if let ClockKind::Manual(t) = &self.kind {
            t.set(t.get() + d);
        }
    }

    /// The time source in µs (wall time or the manual time), unaffected by the time control.
    pub(crate) fn source_us(&self) -> u64 {
        match &self.kind {
            ClockKind::Wall(start) => u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX),
            ClockKind::Manual(t) => t.get().as_micros(),
        }
    }

    /// Changes the time control from now on (the current time stays continuous).
    fn rebase(&self, f: impl FnOnce(&mut TimeCtl)) {
        let now = self.now().as_micros();
        let mut c = self.ctl.get();
        c.base_src = self.source_us();
        c.base_sim = now;
        f(&mut c);
        self.ctl.set(c);
    }

    /// Slow motion: time runs at a quarter of the speed (F5).
    pub fn set_slow_motion(&self, on: bool) {
        self.rebase(|c| c.slow = on);
    }

    /// Whether slow motion is on.
    #[must_use]
    pub fn slow_motion(&self) -> bool {
        self.ctl.get().slow
    }

    /// Freezes (`true`) or continues the simulator time (F6). Input is still processed while
    /// paused.
    pub fn set_paused(&self, on: bool) {
        self.rebase(|c| c.paused = on);
    }

    /// Whether the time is paused.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.ctl.get().paused
    }

    /// Moves a paused clock forward by `d` (F7 steps 16 ms). No effect while running.
    pub fn step(&self, d: Duration) {
        let mut c = self.ctl.get();
        if c.paused {
            c.base_sim += d.as_micros();
            self.ctl.set(c);
        }
    }

    /// The wall time it takes the simulator time to advance by `d` (4 × `d` in slow motion;
    /// `None` while paused).
    #[must_use]
    pub fn wall_duration(&self, d: Duration) -> Option<Duration> {
        let c = self.ctl.get();
        if c.paused {
            None
        } else if c.slow {
            Some(Duration::us(d.as_micros().saturating_mul(4)))
        } else {
            Some(d)
        }
    }
}

impl Clock for SimClock {
    fn now(&self) -> Instant {
        let c = self.ctl.get();
        if c.paused {
            return Instant::from_micros(c.base_sim);
        }
        let elapsed = self.source_us().saturating_sub(c.base_src);
        let scaled = if c.slow { elapsed / 4 } else { elapsed };
        Instant::from_micros(c.base_sim + scaled)
    }
}

type DrawFn = Box<dyn FnMut(&mut [u8], ColorFormat, &SimFrame)>;

/// What the app runs: a raw framebuffer program or the engine.
enum Program {
    /// `show_framebuffer` programs: a full-screen buffer redrawn every frame.
    Framebuffer {
        display: Box<SimDisplay>,
        draw: DrawFn,
        buf: Option<DrawBufferMem>,
    },
    /// Engine apps (`run_engine`).
    Engine(Box<EngineProgram>),
}

/// An engine with one simulated display.
struct EngineProgram {
    engine: Engine,
    display: DisplayId,
    /// The display is a [`SimFramebufferDisplay`] (`Full` / `Direct` buffer modes).
    framebuffer: bool,
    /// `present` calls already shown.
    presents_seen: u64,
    raw_key: Option<RawKeyHook>,
    perf_overlay: bool,
    /// The registered simulated devices (per [`SimConfig::input`]).
    inputs: EngineInputs,
    /// Themes switched by F12.
    theme_toggle: Option<crate::ThemeToggle>,
    /// The dark theme of `theme_toggle` is installed.
    dark: bool,
    /// Runs instead of `Engine::step` (the declarative `Ui` cycle of [`crate::run`]).
    stepper: Option<StepFn>,
}

/// An update function replacing `Engine::step` (see [`SimApp::set_step_fn`]).
pub type StepFn = Box<dyn FnMut(&mut Engine, Instant) -> Wake>;

/// Input ids of the simulated devices registered with an engine app.
#[derive(Clone, Copy, Debug, Default)]
struct EngineInputs {
    pointer: Option<InputId>,
    keypad: Option<InputId>,
    encoder: Option<InputId>,
}

/// Registers the simulated devices selected by `cfg.input` with the engine, plus a default
/// focus group that the keypad and the encoder send their keys to.
fn register_inputs(
    engine: &mut Engine,
    display: DisplayId,
    cfg: &SimConfig,
    devices: &SimDevices,
) -> Result<EngineInputs, SimError> {
    let mut ids = EngineInputs::default();
    if cfg.input.pointer {
        ids.pointer = Some(engine.add_input(devices.pointer.clone(), display)?);
    }
    if cfg.input.keypad {
        ids.keypad = Some(engine.add_input(devices.keypad.clone(), display)?);
    }
    if cfg.input.encoder {
        ids.encoder = Some(engine.add_input(devices.encoder.clone(), display)?);
    }
    if ids.keypad.is_some() || ids.encoder.is_some() {
        let g = engine.create_group()?;
        engine.set_default_group(Some(g));
        for id in [ids.keypad, ids.encoder].into_iter().flatten() {
            engine.set_input_group(id, Some(g));
        }
    }
    Ok(ids)
}

/// Microseconds since the first call: the engine's high-resolution timer in the simulator.
fn sim_hires_timer() -> Instant {
    static START: std::sync::OnceLock<StdInstant> = std::sync::OnceLock::new();
    let start = START.get_or_init(StdInstant::now);
    Instant::from_micros(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX))
}

/// Leaks a zeroed, 4-byte aligned draw buffer (allocated once, like a `'static` MCU buffer).
fn leak_draw_buffer(len: usize) -> DrawBufferMem {
    let v: &'static mut [u8] = Box::leak(vec![0u8; len + 3].into_boxed_slice());
    let off = v.as_ptr().align_offset(4).min(3);
    DrawBufferMem::new(&mut v[off..off + len])
}

/// When the event loop should run the app again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Deadline {
    /// At this instant.
    At(StdInstant),
    /// Only on the next event (idle).
    Wait,
    /// Continuously.
    Poll,
}

/// Per-frame information handed to [`SimApp::framebuffer_with_input`] programs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimFrame {
    /// Frame index (0 for the first frame).
    pub index: u32,
    /// Simulator time of the frame (wall time in a window, 16 ms per frame headless).
    pub time: Instant,
    /// Keys pressed since the previous frame.
    pub keys: Vec<twine_hal::Key>,
    /// The pointer state (`None` when the pointer device is disabled).
    pub pointer: Option<twine_hal::PointerData>,
    /// Where the pointer was released since the previous frame, if it was.
    pub clicked: Option<twine_core::Point>,
}

#[derive(Debug)]
struct Recording {
    dir: PathBuf,
    frames: u32,
}

/// A simulator application: the emulated display, the input devices, hotkeys and the program
/// that draws frames. Run it with [`run`](Self::run) (window or headless, never returns) or
/// [`run_headless`](Self::run_headless).
///
/// ```no_run
/// use twine_sim::{Hotkey, SimApp, SimConfig};
///
/// let mut app = SimApp::framebuffer(SimConfig::new(320, 240), |fb, _format, frame| {
///     fb.fill(frame as u8);
/// });
/// app.on_hotkey(Hotkey::DumpTree, Box::new(|| println!("no tree yet")));
/// let _pointer = app.inputs().pointer;
/// app.run();
/// ```
pub struct SimApp {
    pub(crate) cfg: SimConfig,
    program: Program,
    devices: SimDevices,
    hotkeys: HotkeyRegistry,
    frame: u32,
    clock: SimClock,
    output_dir: PathBuf,
    recording: Option<Recording>,
    shots: Vec<PathBuf>,
    /// Whether frames drain the keypad queue into [`SimFrame::keys`] (input-aware runner).
    frame_input: bool,
    /// Receives a waker that wakes the window's event loop from any thread.
    pub(crate) waker_sink: Option<Box<dyn FnOnce(std::task::Waker)>>,
}

impl fmt::Debug for SimApp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimApp")
            .field("cfg", &self.cfg)
            .field("frame", &self.frame)
            .field("hotkeys", &self.hotkeys)
            .finish_non_exhaustive()
    }
}

impl SimApp {
    /// An app that calls `draw(framebuffer, format, frame_index)` on a full-screen framebuffer
    /// in the emulated panel format for every frame and flushes it through the [`SimDisplay`].
    ///
    /// The framebuffer keeps its content between frames. It is allocated once (leaked, like a
    /// `'static` MCU buffer).
    #[must_use]
    pub fn framebuffer(cfg: SimConfig, mut draw: impl FnMut(&mut [u8], ColorFormat, u32) + 'static) -> Self {
        let mut app =
            Self::framebuffer_with_input(cfg, move |fb, format, frame| draw(fb, format, frame.index));
        // Keys stay queued for `SimKeypad` readers.
        app.frame_input = false;
        app
    }

    /// Like [`framebuffer`](Self::framebuffer), but `draw` also receives the frame's input
    /// ([`SimFrame`]: keys pressed and clicks since the previous frame, the pointer state).
    #[must_use]
    pub fn framebuffer_with_input(
        cfg: SimConfig,
        draw: impl FnMut(&mut [u8], ColorFormat, &SimFrame) + 'static,
    ) -> Self {
        let display = SimDisplay::from_config(&cfg);
        let (pw, ph) = display.panel_size();
        let len = display.info().format.stride(u32::from(pw)) as usize * usize::from(ph);
        let mem: &'static mut [u8] = Box::leak(vec![0u8; len].into_boxed_slice());
        let program = Program::Framebuffer {
            display: Box::new(display),
            draw: Box::new(draw),
            buf: Some(DrawBufferMem::new(mem)),
        };
        Self::with_program(cfg, program, SimDevices::new())
    }

    fn with_program(cfg: SimConfig, program: Program, devices: SimDevices) -> Self {
        let clock = if cfg.headless.is_some() {
            SimClock::manual()
        } else {
            SimClock::wall()
        };
        let output_dir = cfg
            .headless
            .as_ref()
            .map_or_else(paths::sim_dir, |h| h.out_dir.clone());
        Self {
            cfg,
            program,
            devices,
            hotkeys: HotkeyRegistry::default(),
            frame: 0,
            clock,
            output_dir,
            recording: None,
            shots: Vec::new(),
            frame_input: true,
            waker_sink: None,
        }
    }

    /// An engine app: creates an [`Engine`] (default configuration plus the simulator's
    /// high-resolution timer) with one simulated display — a [`SimDisplay`] for the partial
    /// buffer modes of [`SimConfig::buffer_mode`], a [`SimFramebufferDisplay`] for `Full` and
    /// `Direct` — and runs `setup` on it. The engine redraws only what changed and the event
    /// loop sleeps while it is idle.
    ///
    /// Before `setup`, the devices selected by [`SimConfig::input`] are registered with the
    /// engine (mouse → pointer, keyboard → keypad, wheel + middle button → encoder), and a
    /// default focus group is created and attached to the keypad and the encoder; add nodes to
    /// it with `Engine::group_add(engine.default_group().unwrap(), node)`. The engine is
    /// notified whenever a device changes, like an interrupt line.
    ///
    /// Hotkeys: F2 refresh-area debug overlay, F3 layout bounds, F4 performance overlay (needs
    /// `EngineConfig::default_font`), F8 tree dump. Keys pressed are also passed to
    /// [`SimConfig::on_raw_key`].
    pub fn engine(mut cfg: SimConfig, setup: impl FnOnce(&mut Engine)) -> Result<Self, SimError> {
        let mut engine = Engine::new(EngineConfig {
            hires_timer: Some(sim_hires_timer),
            ..cfg.engine_config
        })?;
        let framebuffer = matches!(cfg.buffer_mode, BufferSpec::Full | BufferSpec::Direct);
        let display = if framebuffer {
            let full = cfg.buffer_mode == BufferSpec::Full;
            // Framebuffer panels rotate in "hardware" (the engine cannot rotate them).
            cfg.hw_rotation = true;
            engine.add_framebuffer_display(
                SimFramebufferDisplay::new(cfg.width, cfg.height, cfg.format, cfg.rotation, full),
                if full {
                    BufferMode::full()
                } else {
                    BufferMode::direct()
                },
            )?
        } else {
            let d = SimDisplay::from_config(&cfg);
            let bytes = cfg.buffer_mode.bytes_per_buffer(&d.info());
            let mode = BufferMode::Partial {
                a: leak_draw_buffer(bytes),
                b: (cfg.buffer_mode.buffer_count() == 2).then(|| leak_draw_buffer(bytes)),
            };
            engine.add_display(d, mode)?
        };
        let devices = SimDevices::new();
        let inputs = register_inputs(&mut engine, display, &cfg, &devices)?;
        let toggle = cfg.theme_toggle.clone();
        if let Some(t) = cfg
            .theme
            .clone()
            .or_else(|| toggle.as_ref().map(|t| t.light.clone()))
        {
            engine.set_theme(display, t);
        }
        setup(&mut engine);
        let raw_key = cfg.on_raw_key.take();
        let program = Program::Engine(Box::new(EngineProgram {
            engine,
            display,
            framebuffer,
            presents_seen: 0,
            raw_key,
            perf_overlay: false,
            inputs,
            theme_toggle: toggle,
            dark: false,
            stepper: None,
        }));
        Ok(Self::with_program(cfg, program, devices))
    }

    /// The engine of an engine app.
    #[must_use]
    pub fn engine_ref(&self) -> Option<&Engine> {
        match &self.program {
            Program::Engine(p) => Some(&p.engine),
            Program::Framebuffer { .. } => None,
        }
    }

    /// The engine of an engine app, mutably.
    pub fn engine_mut(&mut self) -> Option<&mut Engine> {
        match &mut self.program {
            Program::Engine(p) => Some(&mut p.engine),
            Program::Framebuffer { .. } => None,
        }
    }

    /// Runs `f` instead of `Engine::step` for engine apps (e.g. the declarative `Ui` update
    /// cycle); the simulator keeps notifying input changes and waiting for the returned
    /// [`Wake`].
    pub fn set_step_fn(&mut self, f: StepFn) {
        if let Program::Engine(p) = &mut self.program {
            p.stepper = Some(f);
        }
    }

    /// `sink` receives a waker (usable from any thread) that wakes the window's event loop,
    /// e.g. to register it where background tasks signal new data.
    pub fn on_waker(&mut self, sink: Box<dyn FnOnce(std::task::Waker)>) {
        self.waker_sink = Some(sink);
    }

    /// Handles to the simulated input devices.
    #[must_use]
    pub fn inputs(&self) -> SimDevices {
        self.devices.clone()
    }

    /// The simulator clock (wall time, or 16 ms per frame when headless).
    #[must_use]
    pub fn clock(&self) -> SimClock {
        self.clock.clone()
    }

    /// Registers a callback for a hotkey (F2–F8, F12 are free for engine runners; F1, F9 and F10
    /// keep their built-in behaviour and additionally run registered callbacks).
    pub fn on_hotkey(&mut self, hotkey: Hotkey, f: Box<dyn FnMut()>) {
        self.hotkeys.on(hotkey, f);
    }

    /// Frames rendered so far.
    #[must_use]
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// Runs the app — headless when [`SimConfig::headless`] is set, else in a window — and exits
    /// the process: code 0 on window close or headless success, 3 on a script error (printed as
    /// `file:line: message`), 1 on other errors.
    pub fn run(self) -> ! {
        crate::init_logging();
        let code = if self.cfg.headless.is_some() {
            match self.run_headless() {
                Ok(r) => {
                    log::info!(
                        target: "twine::sim",
                        "headless: {} frames ({}), output in {}",
                        r.frames,
                        r.sim_time,
                        r.out_dir.display()
                    );
                    0
                }
                Err(e @ SimError::Script { .. }) => {
                    eprintln!("{e}");
                    3
                }
                Err(e) => {
                    eprintln!("twine-sim: {e}");
                    1
                }
            }
        } else {
            match crate::window::run(self) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("twine-sim: {e}");
                    1
                }
            }
        };
        std::process::exit(code)
    }

    /// Runs without a window (using [`SimConfig::headless`], or
    /// [`Headless::default`](crate::Headless::default)) and
    /// returns instead of exiting.
    ///
    /// A deterministic clock advances exactly 16 ms per frame. Script input actions due at a
    /// frame's time are applied before the frame is drawn; `shot` and `hotkey` actions after.
    /// The run ends after the frame at which the last script action is due (or after
    /// `frames` frames without a script); then `final.png` is written.
    pub fn run_headless(mut self) -> Result<HeadlessReport, SimError> {
        let h = self.cfg.headless.clone().unwrap_or_default();
        if !matches!(self.clock.kind, ClockKind::Manual(_)) {
            self.clock = SimClock::manual();
        }
        self.output_dir.clone_from(&h.out_dir);
        let actions = match &h.script {
            Some(file) => {
                let src = std::fs::read_to_string(file).map_err(|e| {
                    SimError::Io(std::io::Error::new(e.kind(), format!("{}: {e}", file.display())))
                })?;
                let cmds = script::parse(&src).map_err(|error| SimError::Script {
                    file: file.clone(),
                    error,
                })?;
                script::schedule(&cmds)
            }
            None => Vec::new(),
        };
        log::info!(
            target: "twine::sim",
            "headless: {}x{} {}, {}, output {}",
            self.cfg.width,
            self.cfg.height,
            self.cfg.format,
            h.script.as_ref().map_or_else(|| format!("{} frames", h.frames), |s| format!("script {}", s.display())),
            h.out_dir.display()
        );
        let mut next = 0;
        let mut rendered = 0u32;
        loop {
            // Script time follows the clock's source: it keeps going while the time is paused.
            let t = self.clock.source_us() / 1000;
            let mut outputs = Vec::new();
            while let Some((at, action)) = actions.get(next) {
                if *at > t {
                    break;
                }
                if action.is_output() {
                    outputs.push(action.clone());
                } else {
                    self.apply_input(action);
                }
                next += 1;
            }
            self.headless_frame();
            self.on_presented();
            rendered += 1;
            for action in outputs {
                match action {
                    ScriptAction::Shot(name) => {
                        let path = h.out_dir.join(format!("{name}.png"));
                        self.write_panel_png(&path)?;
                        log::info!(target: "twine::sim", "shot {}", path.display());
                        self.shots.push(path);
                    }
                    ScriptAction::Hotkey(hk) => self.hotkey(hk),
                    _ => {}
                }
            }
            let done = if h.script.is_some() {
                next >= actions.len()
            } else {
                rendered >= h.frames.max(1)
            };
            if done {
                break;
            }
            self.clock.advance(HEADLESS_FRAME);
        }
        if let Some(rec) = self.recording.take() {
            log::info!(target: "twine::sim", "recording stopped: {} frames in {}", rec.frames, rec.dir.display());
        }
        let final_path = h.out_dir.join("final.png");
        self.write_panel_png(&final_path)?;
        self.shots.push(final_path);
        Ok(HeadlessReport {
            frames: rendered,
            sim_time: Duration::us(self.clock.source_us()),
            shots: std::mem::take(&mut self.shots),
            out_dir: h.out_dir,
        })
    }

    fn apply_input(&self, action: &ScriptAction) {
        let mut s = self.devices.state.borrow_mut();
        match action {
            ScriptAction::Press(p) => s.pointer_press(*p),
            ScriptAction::Move(p) => s.pointer_move(*p),
            ScriptAction::Release => s.pointer_release(),
            ScriptAction::Key(k, pressed) => s.key(*k, *pressed),
            ScriptAction::Wheel(d) => s.encoder_rotate(*d),
            ScriptAction::EncButton(pressed) => s.encoder_button(*pressed),
            ScriptAction::Shot(_) | ScriptAction::Hotkey(_) => {}
        }
    }

    /// One deterministic headless frame: a framebuffer program draws and flushes a frame; an
    /// engine app steps until its frame is on the panel.
    fn headless_frame(&mut self) {
        if matches!(self.program, Program::Engine(_)) {
            let now = self.clock.now();
            for _ in 0..10_000 {
                let wake = self.engine_step(now);
                let busy = self.busy_until();
                if wake != Wake::Now && busy.is_none() {
                    break;
                }
                if let Some(t) = busy {
                    std::thread::sleep(t.saturating_duration_since(StdInstant::now()));
                }
            }
            self.frame = self.frame.wrapping_add(1);
        } else {
            self.wait_for_buffer();
            self.render_frame();
            self.wait_for_buffer();
        }
    }

    /// Notifies the engine about devices whose state changed (the simulator's "interrupt"),
    /// feeds pressed keys to the raw key hook and steps the engine at `now`.
    fn engine_step(&mut self, now: Instant) -> Wake {
        let (keys, changes) = {
            let mut s = self.devices.state.borrow_mut();
            (s.take_raw_keys(), s.take_changes())
        };
        let Program::Engine(p) = &mut self.program else {
            return Wake::Idle;
        };
        let ids = p.inputs;
        for (changed, id) in [
            (changes.pointer, ids.pointer),
            (changes.keypad, ids.keypad),
            (changes.encoder, ids.encoder),
        ] {
            if let Some(id) = id.filter(|_| changed) {
                p.engine.notify_input(id);
            }
        }
        if let Some(hook) = &mut p.raw_key {
            for k in keys {
                log::info!(target: "twine::sim", "key {k}");
                (hook.0)(&mut p.engine, k);
            }
        }
        match &mut p.stepper {
            Some(f) => f(&mut p.engine, now),
            None => p.engine.step(now),
        }
    }

    /// Advances the app for the window at wall time `now_std`; `next_frame` paces framebuffer
    /// programs. Returns when to run again.
    pub(crate) fn window_tick(&mut self, now_std: StdInstant, next_frame: &mut StdInstant) -> Deadline {
        if matches!(self.program, Program::Engine(_)) {
            let now = self.clock.now();
            let wake = self.engine_step(now);
            let mut deadline = match wake {
                Wake::Idle => Deadline::Wait,
                Wake::Now => Deadline::At(now_std),
                // Simulator time may run slower than the wall time or be paused (F5 / F6).
                Wake::At(t) => match self.clock.wall_duration(t.saturating_duration_since(now)) {
                    Some(d) => Deadline::At(now_std + StdDuration::from_micros(d.as_micros())),
                    None => Deadline::Wait,
                },
            };
            if let Some(b) = self.busy_until() {
                // A transfer is in flight on the emulated bus: step again when it completes so
                // the engine takes its buffer back (and the pixels appear).
                deadline = match deadline {
                    Deadline::At(t) => Deadline::At(t.min(b)),
                    _ => Deadline::At(b),
                };
            }
            return deadline;
        }
        self.reclaim();
        let interval = self.frame_interval();
        let due = interval.is_none_or(|_| now_std >= *next_frame);
        if self.has_buffer() && due {
            self.render_frame();
            if let Some(i) = interval {
                *next_frame += i;
                if *next_frame < now_std {
                    *next_frame = now_std + i;
                }
            }
            self.reclaim();
        }
        if self.has_buffer() {
            interval.map_or(Deadline::Poll, |_| Deadline::At(*next_frame))
        } else {
            Deadline::At(self.busy_until().unwrap_or(now_std))
        }
    }

    /// Blocks until the display has returned the draw buffer (bus emulation may delay it).
    fn wait_for_buffer(&mut self) {
        while !self.reclaim() {
            let wait = self.busy_until().map_or(StdDuration::from_millis(1), |t| {
                t.saturating_duration_since(std::time::Instant::now())
            });
            std::thread::sleep(wait.max(StdDuration::from_micros(50)));
        }
    }

    /// Takes the draw buffer back from the display if its flush has finished. Returns whether
    /// the app holds the buffer (always `true` for engine apps).
    pub(crate) fn reclaim(&mut self) -> bool {
        match &mut self.program {
            Program::Framebuffer { display, buf, .. } => {
                if buf.is_none() {
                    *buf = display.poll_flush();
                }
                buf.is_some()
            }
            Program::Engine(_) => true,
        }
    }

    /// Whether the app holds the draw buffer (the previous flush has finished).
    pub(crate) fn has_buffer(&self) -> bool {
        match &self.program {
            Program::Framebuffer { buf, .. } => buf.is_some(),
            Program::Engine(_) => true,
        }
    }

    /// Draws the next frame and starts flushing it (framebuffer programs). No-op while the
    /// buffer is in flight.
    pub(crate) fn render_frame(&mut self) {
        let frame = {
            let mut s = self.devices.state.borrow_mut();
            SimFrame {
                index: self.frame,
                time: self.clock.now(),
                keys: if self.frame_input {
                    s.take_pressed_keys()
                } else {
                    Vec::new()
                },
                pointer: self.cfg.input.pointer.then(|| s.pointer()),
                clicked: if self.frame_input { s.take_click() } else { None },
            }
        };
        let Program::Framebuffer { display, draw, buf } = &mut self.program else {
            return;
        };
        let Some(mut b) = buf.take() else {
            return;
        };
        let format = display.info().format;
        draw(b.as_mut_slice(), format, &frame);
        self.frame = self.frame.wrapping_add(1);
        let (w, h) = display.panel_size();
        let area = twine_core::Rect::new(0, 0, i32::from(w), i32::from(h));
        if let Err(e) = display.begin_flush(area, b) {
            // The buffer comes back through `poll_flush`.
            log::error!(target: "twine::sim", "full-screen flush failed: {e}");
        }
    }

    /// When the oldest emulated transfer completes (`None` when the bus is idle).
    pub(crate) fn busy_until(&self) -> Option<StdInstant> {
        match &self.program {
            Program::Framebuffer { display, .. } => display.busy_until(),
            Program::Engine(p) if !p.framebuffer => p.engine.driver::<SimDisplay>(p.display)?.busy_until(),
            Program::Engine(_) => None,
        }
    }

    /// Whether the panel changed since the last call.
    pub(crate) fn take_dirty(&mut self) -> bool {
        match &mut self.program {
            Program::Framebuffer { display, .. } => display.take_dirty().is_some(),
            Program::Engine(p) if p.framebuffer => {
                let n = p
                    .engine
                    .driver::<SimFramebufferDisplay>(p.display)
                    .map_or(0, SimFramebufferDisplay::present_count);
                let changed = n != p.presents_seen;
                p.presents_seen = n;
                changed
            }
            Program::Engine(p) => p
                .engine
                .driver_mut::<SimDisplay>(p.display)
                .is_some_and(|d| d.take_dirty().is_some()),
        }
    }

    /// The physical panel size (what the window and screenshots show).
    pub(crate) fn panel_size(&self) -> (u16, u16) {
        match &self.program {
            Program::Framebuffer { display, .. } => display.panel_size(),
            Program::Engine(p) if p.framebuffer => (self.cfg.width, self.cfg.height),
            Program::Engine(p) => p
                .engine
                .driver::<SimDisplay>(p.display)
                .map_or((self.cfg.width, self.cfg.height), SimDisplay::panel_size),
        }
    }

    /// The presented framebuffer of an engine app with a framebuffer display.
    fn presented_framebuffer(p: &EngineProgram) -> Option<&[u8]> {
        let idx = p.engine.driver::<SimFramebufferDisplay>(p.display)?.presented()?;
        p.engine.framebuffer(p.display, idx)
    }

    /// The panel as `0x00RRGGBB` words (allocation reused).
    pub(crate) fn panel_xrgb_into(&self, out: &mut Vec<u32>) {
        let (w, h) = self.panel_size();
        match &self.program {
            Program::Framebuffer { display, .. } => display.panel_xrgb_into(out),
            Program::Engine(p) if p.framebuffer => {
                let f = self.cfg.format;
                let stride = f.stride(u32::from(w)) as usize;
                if let Some(fb) = Self::presented_framebuffer(p) {
                    crate::convert::to_xrgb(
                        fb,
                        f,
                        usize::from(w),
                        usize::from(h),
                        stride,
                        self.cfg.mono_colors,
                        out,
                    );
                } else {
                    out.clear();
                    out.resize(usize::from(w) * usize::from(h), 0);
                }
            }
            Program::Engine(p) => {
                if let Some(d) = p.engine.driver::<SimDisplay>(p.display) {
                    d.panel_xrgb_into(out);
                }
            }
        }
    }

    /// The panel as 8-bit RGB.
    pub(crate) fn panel_rgb888(&self) -> Vec<u8> {
        let (w, h) = self.panel_size();
        match &self.program {
            Program::Framebuffer { display, .. } => display.panel_rgb888(),
            Program::Engine(p) if p.framebuffer => {
                let f = self.cfg.format;
                let stride = f.stride(u32::from(w)) as usize;
                Self::presented_framebuffer(p).map_or_else(
                    || vec![0; usize::from(w) * usize::from(h) * 3],
                    |fb| {
                        crate::convert::to_rgb888(
                            fb,
                            f,
                            usize::from(w),
                            usize::from(h),
                            stride,
                            self.cfg.mono_colors,
                        )
                    },
                )
            }
            Program::Engine(p) => p
                .engine
                .driver::<SimDisplay>(p.display)
                .map_or_else(Vec::new, SimDisplay::panel_rgb888),
        }
    }

    /// Called after a frame became visible: records it when recording.
    pub(crate) fn on_presented(&mut self) {
        let Some(rec) = &self.recording else {
            return;
        };
        let path = rec.dir.join(hotkeys::frame_file(rec.frames));
        match self.write_panel_png(&path) {
            Ok(()) => {
                if let Some(rec) = &mut self.recording {
                    rec.frames += 1;
                }
            }
            Err(e) => {
                log::error!(target: "twine::sim", "recording stopped, cannot write {}: {e}", path.display());
                self.recording = None;
            }
        }
    }

    /// Engine hotkeys (F2 refresh debug, F3 layout bounds, F4 performance overlay, F8 tree
    /// dump). Returns whether `hk` was handled.
    fn engine_hotkey(&mut self, hk: Hotkey) -> bool {
        let Program::Engine(p) = &mut self.program else {
            return false;
        };
        let e = &mut p.engine;
        match hk {
            Hotkey::RefreshDebug => {
                let on = !e.config().debug_refresh;
                e.config_mut().debug_refresh = on;
                log::info!(target: "twine::sim", "refresh debug overlay {}", if on { "on" } else { "off" });
            }
            Hotkey::LayoutBounds => {
                let on = !e.bounds_overlay();
                e.set_bounds_overlay(on);
                log::info!(target: "twine::sim", "layout bounds overlay {}", if on { "on" } else { "off" });
            }
            Hotkey::PerfMonitor => {
                p.perf_overlay = !p.perf_overlay;
                if e.theme(p.display).is_none() && e.config().default_font.is_none() {
                    log::warn!(target: "twine::sim", "performance overlay needs a theme or EngineConfig::default_font");
                }
                if let Err(err) = e.set_perf_overlay(p.display, p.perf_overlay) {
                    log::error!(target: "twine::sim", "performance overlay: {err}");
                }
                log::info!(target: "twine::sim", "performance overlay {}", if p.perf_overlay { "on" } else { "off" });
            }
            Hotkey::DumpTree => print!("{}", e.dump()),
            Hotkey::ThemeToggle => {
                let Some(t) = &p.theme_toggle else {
                    return false;
                };
                p.dark = !p.dark;
                let theme = if p.dark { t.dark.clone() } else { t.light.clone() };
                log::info!(target: "twine::sim", "theme: {}", theme.name());
                e.set_theme(p.display, theme);
            }
            _ => return false,
        }
        true
    }

    /// Handles a hotkey (built-in behaviour, then registered callbacks).
    pub(crate) fn hotkey(&mut self, hk: Hotkey) {
        match hk {
            Hotkey::Help => print!("{}", hotkeys::help_text()),
            Hotkey::Screenshot => {
                let path = hotkeys::screenshot_path(&self.output_dir, Path::exists);
                match self.write_panel_png(&path) {
                    Ok(()) => {
                        log::info!(target: "twine::sim", "screenshot {}", path.display());
                        self.shots.push(path);
                    }
                    Err(e) => log::error!(target: "twine::sim", "screenshot failed: {e}"),
                }
            }
            Hotkey::Record => {
                if let Some(rec) = self.recording.take() {
                    log::info!(
                        target: "twine::sim",
                        "recording stopped: {} frames in {}",
                        rec.frames,
                        rec.dir.display()
                    );
                } else {
                    let dir = hotkeys::recording_dir(&self.output_dir, Path::exists);
                    match std::fs::create_dir_all(&dir) {
                        Ok(()) => {
                            log::info!(target: "twine::sim", "recording to {}", dir.display());
                            self.recording = Some(Recording { dir, frames: 0 });
                        }
                        Err(e) => log::error!(target: "twine::sim", "cannot record: {e}"),
                    }
                }
            }
            Hotkey::SlowMotion => {
                let on = !self.clock.slow_motion();
                self.clock.set_slow_motion(on);
                log::info!(target: "twine::sim", "slow motion x0.25 {}", if on { "on" } else { "off" });
            }
            Hotkey::Pause => {
                let on = !self.clock.paused();
                self.clock.set_paused(on);
                log::info!(target: "twine::sim", "time {}", if on { "paused (F7 steps 16 ms)" } else { "running" });
            }
            Hotkey::Step => {
                if self.clock.paused() {
                    self.clock.step(HEADLESS_FRAME);
                    log::info!(target: "twine::sim", "step: time {}", self.clock.now());
                } else {
                    log::info!(target: "twine::sim", "F7 steps only while the time is paused (F6)");
                }
            }
            _ => {}
        }
        let engine_handled = self.engine_hotkey(hk);
        let handled = self.hotkeys.dispatch(hk) || engine_handled;
        if !handled
            && !matches!(
                hk,
                Hotkey::Help
                    | Hotkey::Screenshot
                    | Hotkey::Record
                    | Hotkey::SlowMotion
                    | Hotkey::Pause
                    | Hotkey::Step
            )
        {
            log::info!(target: "twine::sim", "{hk}: not available in this app");
        }
    }

    fn write_panel_png(&self, path: &Path) -> std::io::Result<()> {
        let (w, h) = self.panel_size();
        png_out::write_rgb_png(path, u32::from(w), u32::from(h), &self.panel_rgb888())
    }

    /// The configured frame interval (`None` = unlimited).
    pub(crate) fn frame_interval(&self) -> Option<StdDuration> {
        self.cfg
            .fps_limit
            .map(|fps| StdDuration::from_micros(1_000_000 / u64::from(fps.max(1))))
    }

    /// The shared input state (for the window's event handling).
    pub(crate) fn devices(&self) -> &SimDevices {
        &self.devices
    }
}
