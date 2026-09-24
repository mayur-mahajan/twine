//! [`EngineHarness`]: a bare [`Engine`] on an in-memory display with a mock clock, for engine
//! tests (feature `engine`).

use std::rc::Rc;
use twine_core::{ColorFormat, Duration, Instant, Point, Rect, Rotation};

use twine_engine::{
    BufferMode, DisplayId, Engine, EngineConfig, InputId, InvalidateReason, NodeId, RefreshStats, ThemeHook,
    Wake,
};
use twine_hal::{BufferSpec, Clock, DisplayInfo, Key, PollHint};
use twine_theme::DefaultTheme;

use crate::mock_display::MockFramebufferDisplay;
use crate::snapshot::{SnapshotConfig, Tolerance, assert_rgb_snapshot};
use crate::{
    FlushRecord, MemoryDisplay, MockClock, MockDmaDisplay, MockEncoder, MockKeypad, MockPointer, convert,
    leak_buffer,
};

/// Framebuffer mode of [`EngineHarness::framebuffer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FbMode {
    /// Two framebuffers with area sync.
    Full,
    /// One framebuffer rendered in place.
    Direct,
}

/// How the harness display is driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// [`MemoryDisplay`] (blocking).
    Memory,
    /// [`MockDmaDisplay`] with this many polls per transfer.
    Dma(u32),
    /// [`MockFramebufferDisplay`] with a present delay in polls.
    Framebuffer(FbMode, u32),
}

/// A node query for [`EngineHarness::find`] (and `TestUi::find`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// The widget's text (`Widget::text`).
    Text(String),
    /// The node's test id (`Engine::set_test_id`).
    Id(&'static str),
    /// The widget class name.
    Class(&'static str),
    /// Both queries match.
    And(Box<Query>, Box<Query>),
}

impl Query {
    /// Whether node `id` matches.
    #[must_use]
    pub fn matches(&self, engine: &Engine, id: NodeId) -> bool {
        let Some(n) = engine.tree().node(id) else {
            return false;
        };
        match self {
            Query::Id(t) => n.test_id() == Some(*t),
            Query::Class(c) => n.class().name == *c,
            Query::Text(t) => n.widget().text() == Some(t.as_str()),
            Query::And(a, b) => a.matches(engine, id) && b.matches(engine, id),
        }
    }

    /// This query and `other`.
    #[must_use]
    pub fn and(self, other: Query) -> Query {
        Query::And(Box::new(self), Box::new(other))
    }
}

/// Nodes whose text is `s`.
#[must_use]
pub fn by_text(s: &str) -> Query {
    Query::Text(s.into())
}

/// Nodes with test id `s`.
#[must_use]
pub fn by_id(s: &'static str) -> Query {
    Query::Id(s)
}

/// Nodes of widget class `s`.
#[must_use]
pub fn by_class(s: &'static str) -> Query {
    Query::Class(s)
}

/// Replaces `Engine::step` in [`EngineHarness::update`] (e.g. the declarative `Ui` cycle).
pub type StepFn = Box<dyn FnMut(&mut Engine, Instant) -> Wake>;

/// An [`Engine`] with one display on an in-memory panel, a [`MockClock`] and helpers to step
/// time, inspect flushes and invalidations, find nodes and compare snapshots.
///
/// Defaults: RGB565, two 40-row partial buffers, a blocking [`MemoryDisplay`], the default
/// [`EngineConfig`]. Builder methods rebuild the engine, so call them before
/// [`mount_engine`](Self::mount_engine).
///
/// ```
/// use twine_core::{Color, Opa};
/// use twine_engine::{Obj, Wake};
/// use twine_style::{Selector, StyleProp};
/// use twine_testing::EngineHarness;
///
/// let mut h = EngineHarness::new(64, 48).no_theme().mount_engine(|e| {
///     let screen = e.active_screen(e.default_display().unwrap()).unwrap();
///     let b = e.create(screen, Box::new(Obj)).unwrap();
///     e.set_pos(b, 4, 4);
///     e.set_size(b, 10, 10);
///     e.set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::RED));
///     e.set_local_prop(b, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
/// });
/// h.run_until_idle();
/// assert_eq!(h.pixel(5, 5), Color::RED);
/// h.assert_idle();
/// ```
pub struct EngineHarness {
    engine: Engine,
    clock: MockClock,
    display: DisplayId,
    s: Settings,
    flushes: Vec<FlushRecord>,
    invalidations: Vec<(Rect, InvalidateReason)>,
    inputs: HarnessInputs,
    stepper: Option<StepFn>,
}

/// The mock devices the input helpers drive (registered on first use).
#[derive(Debug, Default)]
struct HarnessInputs {
    pointer: Option<(InputId, MockPointer)>,
    keypad: Option<(InputId, MockKeypad)>,
    encoder: Option<(InputId, MockEncoder)>,
}

/// What the harness engine is built from.
#[derive(Clone, Debug)]
struct Settings {
    w: u16,
    h: u16,
    format: ColorFormat,
    buffers: BufferSpec,
    config: EngineConfig,
    kind: Kind,
    rotation: Rotation,
    align: u8,
    /// Installed on the display after it is added.
    theme: Option<Rc<dyn ThemeHook>>,
}

impl Settings {
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(self.w, self.h, self.format)
            .with_rotation(self.rotation)
            .with_hw_rotation(false)
            .with_align(self.align)
    }

    /// A fresh engine with the harness display.
    fn build(&self) -> (Engine, DisplayId) {
        let mut engine = Engine::new(self.config).expect("valid engine config");
        let info = self.info();
        let display = match self.kind {
            Kind::Framebuffer(mode, delay) => {
                let full = mode == FbMode::Full;
                engine.add_framebuffer_display(
                    MockFramebufferDisplay::new(info, full, delay),
                    if full {
                        BufferMode::full()
                    } else {
                        BufferMode::direct()
                    },
                )
            }
            kind => {
                let bytes = self.buffers.bytes_per_buffer(&info);
                let mode = BufferMode::Partial {
                    a: leak_buffer(bytes),
                    b: (self.buffers.buffer_count() == 2).then(|| leak_buffer(bytes)),
                };
                if let Kind::Dma(polls) = kind {
                    #[cfg(feature = "debug-checks")]
                    engine.set_render_hook(Some(crate::mock_display::record_render_start));
                    engine.add_display(MockDmaDisplay::new(info, polls), mode)
                } else {
                    engine.add_display(MemoryDisplay::new(info), mode)
                }
            }
        };
        let display = display.expect("harness display");
        if let Some(t) = &self.theme {
            engine.set_theme(display, t.clone());
        }
        (engine, display)
    }
}

impl std::fmt::Debug for EngineHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineHarness")
            .field("settings", &self.s)
            .finish_non_exhaustive()
    }
}

impl EngineHarness {
    /// A `w × h` RGB565 display with two 40-row buffers and LVGL's default light theme
    /// ([`DefaultTheme::light`]).
    #[must_use]
    pub fn new(w: u16, h: u16) -> Self {
        let s = Settings {
            w,
            h,
            format: ColorFormat::Rgb565,
            buffers: BufferSpec::PartialDouble { rows: 40 },
            config: EngineConfig::default(),
            kind: Kind::Memory,
            rotation: Rotation::Deg0,
            align: 1,
            theme: Some(Rc::new(DefaultTheme::light())),
        };
        let (engine, display) = s.build();
        Self {
            engine,
            clock: MockClock::new(),
            display,
            s,
            flushes: Vec::new(),
            invalidations: Vec::new(),
            inputs: HarnessInputs::default(),
            stepper: None,
        }
    }

    fn rebuild(&mut self) {
        let (engine, display) = self.s.build();
        self.engine = engine;
        self.display = display;
        self.flushes.clear();
        self.invalidations.clear();
        self.inputs = HarnessInputs::default();
        self.stepper = None;
    }

    /// Runs `f` instead of `Engine::step` in every [`update`](Self::update) (and every helper
    /// that updates): the declarative `Ui` installs its update cycle here.
    pub fn set_step_fn(&mut self, f: StepFn) {
        self.stepper = Some(f);
    }

    /// Installs `theme` on the display instead of the default light theme (rebuilds the
    /// engine).
    #[must_use]
    pub fn theme(mut self, theme: Rc<dyn ThemeHook>) -> Self {
        self.s.theme = Some(theme);
        self.rebuild();
        self
    }

    /// Uses no theme: nodes have only their own styles and the style defaults (rebuilds the
    /// engine).
    #[must_use]
    pub fn no_theme(mut self) -> Self {
        self.s.theme = None;
        self.rebuild();
        self
    }

    /// Uses pixel format `f` for the display (rebuilds the engine).
    #[must_use]
    pub fn format(mut self, f: ColorFormat) -> Self {
        self.s.format = f;
        self.rebuild();
        self
    }

    /// Uses the buffer layout `b` (`Full` / `Direct` switch to a [`MockFramebufferDisplay`]).
    #[must_use]
    pub fn buffers(mut self, b: BufferSpec) -> Self {
        self.s.buffers = b;
        if matches!(b, BufferSpec::Full | BufferSpec::Direct) {
            let delay = if let Kind::Framebuffer(_, d) = self.s.kind {
                d
            } else {
                0
            };
            self.s.kind = Kind::Framebuffer(
                if b == BufferSpec::Full {
                    FbMode::Full
                } else {
                    FbMode::Direct
                },
                delay,
            );
        } else if matches!(self.s.kind, Kind::Framebuffer(..)) {
            self.s.kind = Kind::Memory;
        }
        self.rebuild();
        self
    }

    /// Uses the engine configuration `c` (rebuilds the engine).
    #[must_use]
    pub fn config(mut self, c: EngineConfig) -> Self {
        self.s.config = c;
        self.rebuild();
        self
    }

    /// Drives the display with a [`MockDmaDisplay`] whose transfers take `polls` polls, and
    /// logs render starts (see [`dma_log`](crate::dma_log)).
    #[must_use]
    pub fn dma(mut self, polls: u32) -> Self {
        self.s.kind = Kind::Dma(polls);
        self.rebuild();
        self
    }

    /// Uses a [`MockFramebufferDisplay`] in `mode`, whose swaps take `present_delay` polls.
    #[must_use]
    pub fn framebuffer(mut self, mode: FbMode, present_delay: u32) -> Self {
        self.s.kind = Kind::Framebuffer(mode, present_delay);
        self.s.buffers = match mode {
            FbMode::Full => BufferSpec::Full,
            FbMode::Direct => BufferSpec::Direct,
        };
        self.rebuild();
        self
    }

    /// Software rotation of the panel (rebuilds the engine).
    #[must_use]
    pub fn rotation(mut self, r: Rotation) -> Self {
        self.s.rotation = r;
        self.rebuild();
        self
    }

    /// Flush area alignment of the panel (rebuilds the engine).
    #[must_use]
    pub fn align(mut self, a: u8) -> Self {
        self.s.align = a;
        self.rebuild();
        self
    }

    /// Runs `f` on the engine (build the scene).
    #[must_use]
    pub fn mount_engine(mut self, f: impl FnOnce(&mut Engine)) -> Self {
        f(&mut self.engine);
        self
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The engine, mutably.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The display.
    #[must_use]
    pub fn display(&self) -> DisplayId {
        self.display
    }

    /// The active screen.
    #[must_use]
    pub fn screen(&self) -> NodeId {
        self.engine.active_screen(self.display).expect("harness display")
    }

    /// The clock (shared with the harness).
    #[must_use]
    pub fn clock(&self) -> &MockClock {
        &self.clock
    }

    /// The current mock time.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// One `Engine::step` at the current time. [`flushes`](Self::flushes) and
    /// [`invalidations`](Self::invalidations) then describe this update only.
    pub fn update(&mut self) -> Wake {
        self.invalidations.clear();
        self.invalidations
            .extend_from_slice(self.engine.invalidation_log());
        self.flushes.clear();
        let frame_before = self.engine.last_stats(self.display).frame;
        let now = self.clock.now();
        let wake = match &mut self.stepper {
            Some(f) => f(&mut self.engine, now),
            None => self.engine.step(now),
        };
        let frame = self.engine.last_stats(self.display).frame;
        if frame != frame_before {
            // A frame started in this update: it rendered everything invalidated up to its
            // start (including what the update itself changed).
            self.invalidations.clear();
            self.invalidations
                .extend_from_slice(self.engine.frame_invalidation_log());
        }
        let d = self.display;
        let mem = match self.s.kind {
            Kind::Memory => self.engine.driver_mut::<MemoryDisplay>(d),
            Kind::Dma(_) => self
                .engine
                .driver_mut::<MockDmaDisplay>(d)
                .map(MockDmaDisplay::memory_mut),
            Kind::Framebuffer(..) => None,
        };
        if let Some(m) = mem {
            m.drain_flushes_into(&mut self.flushes);
        }
        for f in &mut self.flushes {
            f.frame = frame;
        }
        wake
    }

    /// Advances the clock by `d` in steps of `refr_period`, updating after each step.
    pub fn advance(&mut self, d: Duration) {
        let period = self.engine.config().refr_period.as_micros().max(1);
        let mut left = d.as_micros();
        while left > 0 {
            let step = left.min(period);
            self.clock.advance(Duration::us(step));
            left -= step;
            self.update();
        }
    }

    /// Updates (advancing the clock to each requested wake-up) until the engine is idle.
    /// Returns the simulated time it took.
    ///
    /// # Panics
    /// After 60 simulated seconds, with the tree dump and the pending wake reason.
    pub fn run_until_idle(&mut self) -> Duration {
        let start = self.clock.now();
        let limit = start + Duration::secs(60);
        let mut spins = 0u32;
        loop {
            match self.update() {
                Wake::Idle => return self.clock.now().saturating_duration_since(start),
                Wake::At(t) => {
                    spins = 0;
                    if t > self.clock.now() {
                        self.clock.set(t);
                    }
                }
                Wake::Now => {
                    spins += 1;
                    if spins > 1000 {
                        // A pending transfer that only completes with time (or polls).
                        self.clock.advance(Duration::ms(1));
                    }
                }
            }
            assert!(
                self.clock.now() < limit,
                "engine not idle after 60 s (last wake pending)\n{}",
                self.tree_dump()
            );
        }
    }

    /// Asserts that an update renders nothing and returns [`Wake::Idle`] (P1).
    #[track_caller]
    pub fn assert_idle(&mut self) {
        let w = self.update();
        assert_eq!(w, Wake::Idle, "engine is not idle");
        assert!(self.flushes.is_empty(), "idle update flushed {:?}", self.flushes);
    }

    /// The flushes of the last update (partial modes).
    #[must_use]
    pub fn flushes(&self) -> &[FlushRecord] {
        &self.flushes
    }

    /// The invalidations rendered by the last update's frame (or, when it rendered no frame,
    /// those made before it; feature `debug-checks` of the engine).
    #[must_use]
    pub fn invalidations(&self) -> &[(Rect, InvalidateReason)] {
        &self.invalidations
    }

    /// Statistics of the display's last frame.
    #[must_use]
    pub fn last_frame(&self) -> RefreshStats {
        self.engine.last_stats(self.display)
    }

    /// The whole engine as text (every root of the display).
    #[must_use]
    pub fn tree_dump(&self) -> String {
        self.engine.dump()
    }

    /// Every node matching `q` (screens and layers of the display).
    #[must_use]
    pub fn find_all(&self, q: &Query) -> Vec<NodeId> {
        let e = &self.engine;
        let d = self.display;
        let mut roots: Vec<NodeId> = e.bottom_layer(d).into_iter().collect();
        roots.extend_from_slice(e.screens(d));
        roots.extend(e.top_layer(d));
        roots.extend(e.sys_layer(d));
        roots
            .into_iter()
            .flat_map(|r| e.tree().descendants(r).collect::<Vec<_>>())
            .filter(|id| q.matches(e, *id))
            .collect()
    }

    /// The single node matching `q`.
    ///
    /// # Panics
    /// If none or several match (with the tree dump).
    #[must_use]
    #[track_caller]
    #[allow(clippy::needless_pass_by_value)] // `find(Query::Id("ok"))` reads better than `find(&…)`
    pub fn find(&self, q: Query) -> NodeId {
        let all = self.find_all(&q);
        match all.as_slice() {
            [one] => *one,
            [] => panic!("no node matches {q:?}\n{}", self.tree_dump()),
            many => panic!("{} nodes match {q:?}\n{}", many.len(), self.tree_dump()),
        }
    }

    /// The panel's physical size.
    fn panel_size(&self) -> (u16, u16) {
        if self.s.rotation.swaps_axes() {
            (self.s.h, self.s.w)
        } else {
            (self.s.w, self.s.h)
        }
    }

    /// The panel as 8-bit RGB (physical orientation; for framebuffer modes the presented
    /// buffer).
    #[must_use]
    pub fn panel_rgb888(&self) -> Vec<u8> {
        let d = self.display;
        match self.s.kind {
            Kind::Memory => self
                .engine
                .driver::<MemoryDisplay>(d)
                .expect("memory display")
                .to_rgb888(),
            Kind::Dma(_) => self
                .engine
                .driver::<MockDmaDisplay>(d)
                .expect("dma display")
                .memory()
                .to_rgb888(),
            Kind::Framebuffer(mode, _) => {
                let idx = match mode {
                    FbMode::Full => self
                        .engine
                        .driver::<MockFramebufferDisplay>(d)
                        .and_then(MockFramebufferDisplay::last_presented)
                        .unwrap_or(0),
                    FbMode::Direct => 0,
                };
                let fb = self.engine.framebuffer(d, idx).expect("framebuffer");
                convert::to_rgb888(
                    fb,
                    self.s.format,
                    u32::from(self.s.w),
                    u32::from(self.s.h),
                    self.s.format.stride(u32::from(self.s.w)),
                    (twine_core::Color::BLACK, twine_core::Color::WHITE),
                )
            }
        }
    }

    /// The panel color at physical pixel `(x, y)`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> twine_core::Color {
        let (w, _) = self.panel_size();
        let rgb = self.panel_rgb888();
        let i = (y as usize * usize::from(w) + x as usize) * 3;
        twine_core::Color::new(rgb[i], rgb[i + 1], rgb[i + 2])
    }

    /// The memory display (partial modes).
    #[must_use]
    pub fn memory_display(&self) -> Option<&MemoryDisplay> {
        match self.s.kind {
            Kind::Memory => self.engine.driver::<MemoryDisplay>(self.display),
            Kind::Dma(_) => self
                .engine
                .driver::<MockDmaDisplay>(self.display)
                .map(MockDmaDisplay::memory),
            Kind::Framebuffer(..) => None,
        }
    }

    /// The framebuffer mock (framebuffer modes).
    #[must_use]
    pub fn framebuffer_display(&self) -> Option<&MockFramebufferDisplay> {
        self.engine.driver::<MockFramebufferDisplay>(self.display)
    }

    /// The harness pointer (a [`MockPointer`] with [`PollHint::Interrupt`]), registered on
    /// first use.
    pub fn pointer_input(&mut self) -> (InputId, MockPointer) {
        if self.inputs.pointer.is_none() {
            let m = MockPointer::new();
            m.set_poll_hint(PollHint::Interrupt);
            let id = self
                .engine
                .add_input(m.clone(), self.display)
                .expect("harness pointer");
            self.inputs.pointer = Some((id, m));
        }
        self.inputs.pointer.clone().expect("pointer")
    }

    /// The harness keypad ([`MockKeypad`], interrupt driven), registered on first use and
    /// attached to the engine's default group (if any at that time).
    pub fn keypad_input(&mut self) -> (InputId, MockKeypad) {
        if self.inputs.keypad.is_none() {
            let m = MockKeypad::new();
            m.set_poll_hint(PollHint::Interrupt);
            let id = self
                .engine
                .add_input(m.clone(), self.display)
                .expect("harness keypad");
            let g = self.engine.default_group();
            self.engine.set_input_group(id, g);
            self.inputs.keypad = Some((id, m));
        }
        self.inputs.keypad.clone().expect("keypad")
    }

    /// The harness encoder ([`MockEncoder`], interrupt driven), registered on first use and
    /// attached to the engine's default group (if any at that time).
    pub fn encoder_input(&mut self) -> (InputId, MockEncoder) {
        if self.inputs.encoder.is_none() {
            let m = MockEncoder::new();
            m.set_poll_hint(PollHint::Interrupt);
            let id = self
                .engine
                .add_input(m.clone(), self.display)
                .expect("harness encoder");
            let g = self.engine.default_group();
            self.engine.set_input_group(id, g);
            self.inputs.encoder = Some((id, m));
        }
        self.inputs.encoder.clone().expect("encoder")
    }

    /// Presses the pointer at `p` (notifies the engine) and updates.
    pub fn press(&mut self, p: Point) -> Wake {
        let (id, m) = self.pointer_input();
        m.press(p);
        self.engine.notify_input(id);
        self.update()
    }

    /// Moves the (pressed or hovering) pointer to `p` and updates.
    pub fn move_to(&mut self, p: Point) -> Wake {
        let (id, m) = self.pointer_input();
        m.move_to(p);
        self.engine.notify_input(id);
        self.update()
    }

    /// Releases the pointer and updates.
    pub fn release(&mut self) -> Wake {
        let (id, m) = self.pointer_input();
        m.release();
        self.engine.notify_input(id);
        self.update()
    }

    /// Presses and releases at `p` (two updates at the same instant).
    pub fn tap(&mut self, p: Point) -> Wake {
        self.press(p);
        self.release()
    }

    /// Presses at `from`, moves linearly to `to` in steps of `read_period` over `dur`
    /// (advancing the clock, one update per step) and releases.
    pub fn drag(&mut self, from: Point, to: Point, dur: Duration) -> Wake {
        let period = self.engine.config().read_period.as_micros().max(1);
        let steps = (dur.as_micros() / period).max(1);
        self.press(from);
        for k in 1..=steps {
            self.clock.advance(Duration::us(period));
            let lerp = |a: i32, b: i32| a + ((i64::from(b - a) * k as i64) / steps as i64) as i32;
            self.move_to(Point::new(lerp(from.x, to.x), lerp(from.y, to.y)));
        }
        self.release()
    }

    /// Presses and releases `k` on the keypad and updates.
    pub fn key(&mut self, k: Key) -> Wake {
        let (id, m) = self.keypad_input();
        m.tap(k);
        self.engine.notify_input(id);
        self.update()
    }

    /// Queues a press (`pressed = true`) or release of `k` and updates.
    pub fn key_state(&mut self, k: Key, pressed: bool) -> Wake {
        let (id, m) = self.keypad_input();
        m.push(k, pressed);
        self.engine.notify_input(id);
        self.update()
    }

    /// Types each character of `s` as a `Key::Char` press and release.
    pub fn type_text(&mut self, s: &str) -> Wake {
        let (id, m) = self.keypad_input();
        for c in s.chars() {
            m.tap(Key::Char(c));
        }
        self.engine.notify_input(id);
        self.update()
    }

    /// Rotates the encoder by `diff` steps and updates.
    pub fn encoder(&mut self, diff: i16) -> Wake {
        let (id, m) = self.encoder_input();
        m.rotate(diff);
        self.engine.notify_input(id);
        self.update()
    }

    /// Presses (`true`) or releases the encoder button and updates.
    pub fn encoder_button(&mut self, pressed: bool) -> Wake {
        let (id, m) = self.encoder_input();
        if pressed {
            m.press();
        } else {
            m.release();
        }
        self.engine.notify_input(id);
        self.update()
    }

    /// Presses and releases the encoder button (two updates at the same instant).
    pub fn encoder_click(&mut self) -> Wake {
        self.encoder_button(true);
        self.encoder_button(false)
    }

    /// Redraws the whole screen and renders until idle.
    pub fn render_full(&mut self) {
        let area = Rect::new(0, 0, i32::from(self.s.w), i32::from(self.s.h));
        self.engine
            .invalidate_area(self.display, area, InvalidateReason::Explicit);
        self.clock.advance(self.engine.config().refr_period);
        self.run_until_idle();
    }

    /// Redraws the whole screen and compares the panel with the snapshot `name` of the crate
    /// under test (`tests/snapshots/<name>.png`, located through the `CARGO_MANIFEST_DIR` and
    /// `CARGO_PKG_NAME` that cargo sets when running tests).
    #[track_caller]
    pub fn assert_snapshot(&mut self, name: &str) {
        let cfg = runtime_snapshot_config();
        self.assert_snapshot_with(&cfg, name);
    }

    /// Like [`assert_snapshot`](Self::assert_snapshot) with an explicit configuration.
    #[track_caller]
    pub fn assert_snapshot_with(&mut self, cfg: &SnapshotConfig, name: &str) {
        self.render_full();
        let (w, h) = self.panel_size();
        assert_rgb_snapshot(
            cfg,
            name,
            u32::from(w),
            u32::from(h),
            &self.panel_rgb888(),
            Tolerance::EXACT,
        );
    }

    /// Compares the panel as it is now (no redraw) with the snapshot `name`, e.g. to capture
    /// the effect of one partial frame.
    #[track_caller]
    pub fn assert_panel_snapshot(&self, name: &str) {
        let (w, h) = self.panel_size();
        assert_rgb_snapshot(
            &runtime_snapshot_config(),
            name,
            u32::from(w),
            u32::from(h),
            &self.panel_rgb888(),
            Tolerance::EXACT,
        );
    }

    /// Redraws the whole screen and compares the physical region `r` of the panel.
    #[track_caller]
    pub fn assert_region_snapshot(&mut self, name: &str, r: Rect) {
        self.render_full();
        let (w, _) = self.panel_size();
        let rgb = self.panel_rgb888();
        let mut out = Vec::with_capacity(r.area() as usize * 3);
        for y in r.y0..r.y1 {
            let o = (y as usize * usize::from(w) + r.x0 as usize) * 3;
            out.extend_from_slice(&rgb[o..o + r.width() as usize * 3]);
        }
        assert_rgb_snapshot(
            &runtime_snapshot_config(),
            name,
            r.width() as u32,
            r.height() as u32,
            &out,
            Tolerance::EXACT,
        );
    }
}

/// The snapshot configuration of the crate whose tests are running.
fn runtime_snapshot_config() -> SnapshotConfig {
    let dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is not set: run the test through cargo, or use assert_snapshot_with");
    let name = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "unknown".into());
    SnapshotConfig::for_crate(&dir, &name)
}
