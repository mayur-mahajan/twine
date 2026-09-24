//! [`TestUi`]: the declarative `Ui` on an in-memory display with a mock clock and mock
//! inputs (feature `ui`).

use std::cell::{Ref, RefCell, RefMut};
use std::fmt::Write as _;
use std::rc::Rc;

use twine_core::{ColorFormat, Duration, Point, Rect};
use twine_engine::{
    Engine, EngineConfig, InvalidateReason, NodeId, ObjFlags, RefreshStats, State, ThemeHook, Wake,
};
use twine_hal::{BufferSpec, Key};
use twine_reactive::Scope;
use twine_view::{UiCore, View};
use twine_widgets::button::BUTTON_CLASS;

use crate::{EngineHarness, FlushRecord, Query};

/// The high-level test harness: an [`EngineHarness`] (RGB565 [`MemoryDisplay`](crate::MemoryDisplay),
/// [`MockClock`](crate::MockClock), mock pointer, keypad and encoder) whose updates run the
/// declarative `Ui` cycle.
///
/// Defaults: LVGL's light default theme with Montserrat 14, two 40-row partial buffers, the
/// default engine configuration. Builder methods come before [`mount`](Self::mount).
///
/// ```
/// use twine_testing::{TestUi, by_id, by_text};
/// use twine_view::prelude::*;
///
/// fn counter(cx: Scope) -> impl View {
///     let count = cx.signal(0u32);
///     column((
///         label(text!("{}", count.get())).test_id("n"),
///         button(label("+1")).on_click(move || count.update(|c| *c += 1)),
///     ))
/// }
///
/// let mut t = TestUi::new(120, 80).mount(counter);
/// t.find(by_text("+1")).click();
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("n")).text(), "1");
/// t.assert_idle();
/// ```
pub struct TestUi {
    h: RefCell<EngineHarness>,
    core: Option<Rc<RefCell<UiCore>>>,
    /// The statistics of the application's last frame, kept while a snapshot re-renders the
    /// whole screen (cleared by the next update).
    pinned_frame: std::cell::Cell<Option<RefreshStats>>,
}

impl std::fmt::Debug for TestUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestUi")
            .field("mounted", &self.core.is_some())
            .finish_non_exhaustive()
    }
}

impl TestUi {
    /// A `w × h` RGB565 display with the default light theme.
    #[must_use]
    pub fn new(w: u16, h: u16) -> Self {
        Self {
            h: RefCell::new(EngineHarness::new(w, h)),
            core: None,
            pinned_frame: std::cell::Cell::new(None),
        }
    }

    fn map_harness(self, f: impl FnOnce(EngineHarness) -> EngineHarness) -> Self {
        let h = f(self.h.into_inner());
        Self {
            h: RefCell::new(h),
            core: self.core,
            pinned_frame: std::cell::Cell::new(None),
        }
    }

    /// Uses pixel format `f`.
    #[must_use]
    pub fn format(self, f: ColorFormat) -> Self {
        self.map_harness(|h| h.format(f))
    }

    /// Uses `t` as the display's theme.
    #[must_use]
    pub fn theme(self, t: Rc<dyn ThemeHook>) -> Self {
        self.map_harness(|h| h.theme(t))
    }

    /// Uses no theme.
    #[must_use]
    pub fn no_theme(self) -> Self {
        self.map_harness(EngineHarness::no_theme)
    }

    /// Uses the buffer layout `m`.
    #[must_use]
    pub fn buffers(self, m: BufferSpec) -> Self {
        self.map_harness(|h| h.buffers(m))
    }

    /// Uses the engine configuration `c`.
    #[must_use]
    pub fn config(self, c: EngineConfig) -> Self {
        self.map_harness(|h| h.config(c))
    }

    /// Builds `app` (once) on the display, like `Ui::build`: a default focus group for the
    /// keypad and the encoder is created first. Updates then run the `Ui` cycle.
    #[must_use]
    pub fn mount<V: View>(mut self, app: impl FnOnce(Scope) -> V) -> Self {
        let h = self.h.get_mut();
        let display = h.display();
        let e = h.engine_mut();
        if e.default_group().is_none() {
            if let Ok(g) = e.create_group() {
                e.set_default_group(Some(g));
            }
        }
        let core = Rc::new(RefCell::new(UiCore::mount(e, display, app)));
        let c = core.clone();
        h.set_step_fn(Box::new(move |e, now| c.borrow_mut().update(e, now)));
        self.core = Some(core);
        self
    }

    /// Runs `f` on the engine (imperative scenes; can be combined with [`mount`](Self::mount)).
    #[must_use]
    pub fn mount_engine(mut self, f: impl FnOnce(&mut Engine)) -> Self {
        f(self.h.get_mut().engine_mut());
        self
    }

    /// The root scope of the mounted application.
    ///
    /// # Panics
    /// Before [`mount`](Self::mount).
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.core
            .as_ref()
            .expect("TestUi::mount first")
            .borrow()
            .root_scope()
    }

    /// The `Ui`'s waker (set by channel sends).
    ///
    /// # Panics
    /// Before [`mount`](Self::mount).
    #[must_use]
    pub fn waker(&self) -> &'static twine_reactive::UiWaker {
        self.core.as_ref().expect("TestUi::mount first").borrow().waker()
    }

    fn harness(&self) -> RefMut<'_, EngineHarness> {
        self.pinned_frame.set(None);
        self.h.borrow_mut()
    }

    /// The harness for an operation that updates (the snapshot's frame is forgotten).
    fn h_mut(&mut self) -> &mut EngineHarness {
        self.pinned_frame.set(None);
        self.h.get_mut()
    }

    // ---- time -----------------------------------------------------------------------------

    /// One update at the current mock time.
    pub fn update(&mut self) -> Wake {
        self.h_mut().update()
    }

    /// Advances the clock by `d` in steps of `refr_period`, updating after each step.
    pub fn advance(&mut self, d: Duration) {
        self.h_mut().advance(d);
    }

    /// Updates, jumping the clock to each requested wake-up, until the `Ui` is idle. Returns
    /// the simulated time it took.
    ///
    /// # Panics
    /// After 60 simulated seconds, with a report: running animations, timers, pressed inputs
    /// and the tree dump.
    pub fn run_until_idle(&mut self) -> Duration {
        let h = self.h_mut();
        let start = h.now();
        let limit = start + Duration::secs(60);
        let mut spins = 0u32;
        loop {
            match h.update() {
                Wake::Idle => return h.now().saturating_duration_since(start),
                Wake::At(t) => {
                    spins = 0;
                    if t > h.now() {
                        h.clock().set(t);
                    }
                }
                Wake::Now => {
                    spins += 1;
                    if spins > 1000 {
                        h.clock().advance(Duration::ms(1));
                    }
                }
            }
            assert!(h.now() < limit, "{}", Self::report(h));
        }
    }

    fn report(h: &mut EngineHarness) -> String {
        let pressed = h.pointer_input().1.state().pressed;
        let e = h.engine();
        let mut s = String::from("TestUi not idle after 60 s simulated\n");
        let _ = writeln!(s, "running anims: {}", e.anim_count());
        let _ = writeln!(s, "pending timers: {}", e.timer_count());
        let _ = writeln!(s, "pointer pressed: {pressed}");
        let _ = writeln!(s, "pending effects: {}", twine_reactive::has_pending_effects());
        let _ = writeln!(s, "{}", e.dump());
        s
    }

    // ---- input ----------------------------------------------------------------------------

    /// Presses the pointer at `p` and updates.
    pub fn press(&mut self, p: Point) {
        self.h_mut().press(p);
    }

    /// Releases the pointer and updates.
    pub fn release(&mut self) {
        self.h_mut().release();
    }

    /// Presses and releases at `p`.
    pub fn tap(&mut self, p: Point) {
        self.h_mut().tap(p);
    }

    /// Drags from `from` to `to` over `dur` (one pointer read per `read_period`).
    pub fn drag(&mut self, from: Point, to: Point, dur: Duration) {
        self.h_mut().drag(from, to, dur);
    }

    /// Presses and releases `k` on the keypad.
    pub fn key(&mut self, k: Key) {
        self.h_mut().key(k);
    }

    /// Types `s` on the keypad (one `Key::Char` press and release per character).
    pub fn type_text(&mut self, s: &str) {
        self.h_mut().type_text(s);
    }

    /// Rotates the encoder by `diff` steps.
    pub fn encoder(&mut self, diff: i16) {
        self.h_mut().encoder(diff);
    }

    /// Clicks the encoder button.
    pub fn encoder_click(&mut self) {
        self.h_mut().encoder_click();
    }

    // ---- queries --------------------------------------------------------------------------

    /// The single node matching `q`.
    ///
    /// # Panics
    /// If none or several match, listing the tree.
    #[track_caller]
    #[allow(clippy::needless_pass_by_value)] // `find(by_id("ok"))` reads better than `find(&…)`
    pub fn find(&self, q: Query) -> NodeHandle<'_> {
        let all = self.shown(&q);
        match all.as_slice() {
            [one] => NodeHandle { ui: self, id: *one },
            [] => panic!("no node matches {q:?}\n{}", self.tree_dump()),
            many => panic!("{} nodes match {q:?}\n{}", many.len(), self.tree_dump()),
        }
    }

    /// Every node matching `q`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn find_all(&self, q: Query) -> Vec<NodeHandle<'_>> {
        let all = self.shown(&q);
        all.into_iter().map(|id| NodeHandle { ui: self, id }).collect()
    }

    /// The nodes matching `q` on what the display shows: the layers, the active screen and,
    /// during a screen load animation, the previous screen (other screens are skipped).
    fn shown(&self, q: &Query) -> Vec<NodeId> {
        let h = self.h.borrow();
        let e = h.engine();
        let d = h.display();
        let shown = [e.active_screen(d), e.prev_screen(d)];
        h.find_all(q)
            .into_iter()
            .filter(|&id| {
                let root = e.tree().root_of(id);
                let is_screen = root.is_some_and(|r| e.screens(d).contains(&r));
                !is_screen || shown.contains(&root)
            })
            .collect()
    }

    /// A handle to node `id`.
    #[must_use]
    pub fn node(&self, id: NodeId) -> NodeHandle<'_> {
        NodeHandle { ui: self, id }
    }

    /// The whole engine as text.
    #[must_use]
    pub fn tree_dump(&self) -> String {
        self.h.borrow().tree_dump()
    }

    // ---- assertions -----------------------------------------------------------------------

    /// Redraws the whole screen and compares it with the snapshot `name`.
    #[track_caller]
    pub fn assert_snapshot(&mut self, name: &str) {
        let last = self.last_frame();
        self.h.get_mut().assert_snapshot(name);
        self.pinned_frame.set(Some(last));
    }

    /// Compares the panel as it is (no redraw, no time passing) with the snapshot `name`,
    /// e.g. in the middle of an animation.
    #[track_caller]
    pub fn assert_panel_snapshot(&self, name: &str) {
        self.h.borrow().assert_panel_snapshot(name);
    }

    /// Redraws the whole screen and compares region `r` with the snapshot `name`.
    #[track_caller]
    pub fn assert_region_snapshot(&mut self, name: &str, r: Rect) {
        let last = self.last_frame();
        self.h.get_mut().assert_region_snapshot(name, r);
        self.pinned_frame.set(Some(last));
    }

    /// Asserts that an update renders nothing and returns [`Wake::Idle`].
    #[track_caller]
    pub fn assert_idle(&mut self) {
        self.h_mut().assert_idle();
    }

    /// Statistics of the application's last frame (a snapshot's full redraw does not count).
    #[must_use]
    pub fn last_frame(&self) -> RefreshStats {
        self.pinned_frame
            .get()
            .unwrap_or_else(|| self.h.borrow().last_frame())
    }

    /// The invalidations rendered by the last update.
    #[must_use]
    pub fn invalidations(&self) -> Ref<'_, [(Rect, InvalidateReason)]> {
        Ref::map(self.h.borrow(), EngineHarness::invalidations)
    }

    /// The flushes of the last update.
    #[must_use]
    pub fn flushes(&self) -> Ref<'_, [FlushRecord]> {
        Ref::map(self.h.borrow(), EngineHarness::flushes)
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> Ref<'_, Engine> {
        Ref::map(self.h.borrow(), EngineHarness::engine)
    }

    /// The engine, mutably.
    pub fn engine_mut(&mut self) -> &mut Engine {
        self.h.get_mut().engine_mut()
    }

    /// The underlying engine harness.
    pub fn harness_mut(&mut self) -> &mut EngineHarness {
        self.h.get_mut()
    }
}

/// A node found by [`TestUi::find`].
#[derive(Clone, Copy)]
pub struct NodeHandle<'a> {
    ui: &'a TestUi,
    id: NodeId,
}

impl std::fmt::Debug for NodeHandle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeHandle({})", twine_engine::fmt_node_id(self.id))
    }
}

impl NodeHandle<'_> {
    /// The node.
    #[must_use]
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The node's coordinates.
    #[must_use]
    pub fn coords(&self) -> Rect {
        self.ui.engine().coords(self.id)
    }

    /// The node's state.
    #[must_use]
    pub fn state(&self) -> State {
        self.ui
            .engine()
            .tree()
            .node(self.id)
            .map_or(State::DEFAULT, twine_engine::Node::state)
    }

    /// The text: the widget's own (label, textarea…) or, for a button, its first child's that
    /// has one. Empty when there is none.
    #[must_use]
    pub fn text(&self) -> String {
        let e = self.ui.engine();
        let t = e.tree();
        let Some(n) = t.node(self.id) else {
            return String::new();
        };
        if let Some(s) = n.widget().text() {
            return s.to_owned();
        }
        if n.class().name == BUTTON_CLASS.name {
            for c in t.children(self.id) {
                if let Some(s) = t.node(c).and_then(|c| c.widget().text()) {
                    return s.to_owned();
                }
            }
        }
        String::new()
    }

    /// The part of the node that is on screen (its coordinates clipped by its ancestors),
    /// `None` when hidden or clipped away.
    #[must_use]
    pub fn visible_area(&self) -> Option<Rect> {
        let e = self.ui.engine();
        let t = e.tree();
        let n = t.node(self.id)?;
        if n.is_hidden() {
            return None;
        }
        let mut r = n.coords();
        for a in t.ancestors(self.id) {
            let an = t.node(a)?;
            if an.is_hidden() {
                return None;
            }
            if !an
                .flags()
                .intersects(ObjFlags::OVERFLOW_VISIBLE | ObjFlags::LAYOUT_PASSTHROUGH)
            {
                r = r.intersection(&an.coords())?;
            }
        }
        (!r.is_empty()).then_some(r)
    }

    /// Whether some part of the node is on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visible_area().is_some()
    }

    /// Taps the center of the node's visible area and updates.
    ///
    /// # Panics
    /// When the node is not visible, or the press would reach another node (covered): the
    /// hit must be the node, one of its descendants, or its nearest clickable ancestor (where
    /// the press of a non-clickable node goes).
    #[track_caller]
    pub fn click(&self) {
        let Some(area) = self.visible_area() else {
            panic!("click: node {:?} is not visible\n{}", self, self.ui.tree_dump());
        };
        let p = area.center();
        let mut h = self.ui.harness();
        let d = h.display();
        let hit = h.engine_mut().hit_test(d, p);
        let e = h.engine();
        let ok = hit.is_some_and(|x| {
            x == self.id
                || e.tree().ancestors(x).any(|a| a == self.id)
                || e.tree()
                    .ancestors(self.id)
                    .find(|a| e.has_flag(*a, ObjFlags::CLICKABLE))
                    .is_some_and(|a| a == x && !e.has_flag(self.id, ObjFlags::CLICKABLE))
        });
        if !ok {
            let dump = e.dump();
            drop(h);
            panic!("click: the press at {p:?} reaches {hit:?}, not {self:?} (covered?)\n{dump}");
        }
        h.tap(p);
    }

    /// Taps the center of the node's coordinates (no visibility check) and updates.
    pub fn tap(&self) {
        let p = self.coords().center();
        self.ui.harness().tap(p);
    }
}
