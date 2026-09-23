//! [`SimApp`]: the simulator application shared by the window and the headless runner.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration as StdDuration;

use twine_core::{ColorFormat, Duration, Instant};
use twine_hal::{Clock, DisplayDriver, DrawBufferMem};

use crate::config::SimConfig;
use crate::display::SimDisplay;
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
        }
    }
}

impl std::error::Error for SimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SimError::Script { error, .. } => Some(error),
            SimError::Io(e) => Some(e),
            SimError::Window(_) => None,
        }
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
    /// Simulated time at the end.
    pub sim_time: Duration,
    /// Every PNG written (script shots, hotkey screenshots, `final.png` last).
    pub shots: Vec<PathBuf>,
    /// The output directory.
    pub out_dir: PathBuf,
}

/// The simulator clock: wall time since start in a window, exactly 16 ms per frame headless.
///
/// Cheap to clone; clones share the time.
#[derive(Clone, Debug)]
pub struct SimClock(ClockKind);

#[derive(Clone, Debug)]
enum ClockKind {
    Wall(std::time::Instant),
    Manual(Rc<Cell<Instant>>),
}

impl SimClock {
    /// A clock following the wall time from now on (starting at `Instant` 0).
    #[must_use]
    pub fn wall() -> Self {
        Self(ClockKind::Wall(std::time::Instant::now()))
    }

    /// A deterministic clock at `Instant` 0 that only moves with [`advance`](Self::advance).
    #[must_use]
    pub fn manual() -> Self {
        Self(ClockKind::Manual(Rc::default()))
    }

    /// Advances a manual clock by `d` (no effect on a wall clock).
    pub fn advance(&self, d: Duration) {
        if let ClockKind::Manual(t) = &self.0 {
            t.set(t.get() + d);
        }
    }
}

impl Clock for SimClock {
    fn now(&self) -> Instant {
        match &self.0 {
            ClockKind::Wall(start) => {
                Instant::from_micros(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX))
            }
            ClockKind::Manual(t) => t.get(),
        }
    }
}

type DrawFn = Box<dyn FnMut(&mut [u8], ColorFormat, u32)>;

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
    pub(crate) display: SimDisplay,
    devices: SimDevices,
    hotkeys: HotkeyRegistry,
    draw: DrawFn,
    buf: Option<DrawBufferMem>,
    frame: u32,
    clock: SimClock,
    output_dir: PathBuf,
    recording: Option<Recording>,
    shots: Vec<PathBuf>,
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
    pub fn framebuffer(cfg: SimConfig, draw: impl FnMut(&mut [u8], ColorFormat, u32) + 'static) -> Self {
        let display = SimDisplay::from_config(&cfg);
        let info = display.info();
        let len = info.bytes_per_row() * usize::from(info.height);
        let mem: &'static mut [u8] = Box::leak(vec![0u8; len].into_boxed_slice());
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
            display,
            devices: SimDevices::new(),
            hotkeys: HotkeyRegistry::default(),
            draw: Box::new(draw),
            buf: Some(DrawBufferMem::new(mem)),
            frame: 0,
            clock,
            output_dir,
            recording: None,
            shots: Vec::new(),
        }
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
        if !matches!(self.clock.0, ClockKind::Manual(_)) {
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
            let t = self.clock.now().as_millis();
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
            self.wait_for_buffer();
            self.render_frame();
            self.wait_for_buffer();
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
            sim_time: Duration::us(self.clock.now().as_micros()),
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

    /// Blocks until the display has returned the draw buffer (bus emulation may delay it).
    fn wait_for_buffer(&mut self) {
        while !self.reclaim() {
            let wait = self
                .display
                .busy_until()
                .map_or(StdDuration::from_millis(1), |t| {
                    t.saturating_duration_since(std::time::Instant::now())
                });
            std::thread::sleep(wait.max(StdDuration::from_micros(50)));
        }
    }

    /// Takes the draw buffer back from the display if its flush has finished. Returns whether
    /// the app now holds the buffer.
    pub(crate) fn reclaim(&mut self) -> bool {
        if self.buf.is_none() {
            self.buf = self.display.poll_flush();
        }
        self.buf.is_some()
    }

    /// Whether the app holds the draw buffer (the previous flush has finished).
    pub(crate) fn has_buffer(&self) -> bool {
        self.buf.is_some()
    }

    /// Draws the next frame and starts flushing it. No-op while the buffer is in flight.
    pub(crate) fn render_frame(&mut self) {
        let Some(mut buf) = self.buf.take() else {
            return;
        };
        let format = self.display.info().format;
        (self.draw)(buf.as_mut_slice(), format, self.frame);
        self.frame = self.frame.wrapping_add(1);
        let area = self.display.info().area();
        if let Err(e) = self.display.begin_flush(area, buf) {
            // The buffer comes back through `poll_flush`.
            log::error!(target: "twine::sim", "full-screen flush failed: {e}");
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
            _ => {}
        }
        let handled = self.hotkeys.dispatch(hk);
        if !handled && !matches!(hk, Hotkey::Help | Hotkey::Screenshot | Hotkey::Record) {
            log::info!(target: "twine::sim", "{hk}: available once the engine is running");
        }
    }

    fn write_panel_png(&self, path: &Path) -> std::io::Result<()> {
        let info = self.display.info();
        png_out::write_rgb_png(
            path,
            u32::from(info.width),
            u32::from(info.height),
            &self.display.panel_rgb888(),
        )
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
