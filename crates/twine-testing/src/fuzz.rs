//! Operation-sequence fuzzing beyond the decoders (R5.S08): interpreters that turn arbitrary
//! bytes into sequences of API calls and check the library's invariants after every call.
//!
//! | Interpreter | Operations | Checked after every operation | Feature |
//! |-------------|------------|-------------------------------|---------|
//! | [`run_tree_ops`] | create, delete, move, position, size, flags, states, flex layout, scroll, render steps — on live **and stale** node ids | [`Tree::check_invariants`](twine_engine::Tree::check_invariants), the node and depth bounds of `Limits`, only `Capacity` faults | `engine` |
//! | [`run_input_ops`] | pointer press/move/release, keys, encoder turns and clicks, time, pointer health — on a screen of real widgets | tree invariants, the focused node is alive, faults limited to `InputDevice` (and only after a health change) | `ui` |
//! | [`run_reactive_ops`] | scopes (create, dispose), signals, memos, effects, writes, batches, reads — on live **and disposed** handles | live node and scope counts equal the model, no effect left pending, memo values and effect runs equal the model, disposed handles answer `Disposed`; disposing everything returns the runtime to its starting counts | `ui` |
//!
//! The same functions drive two harnesses: property tests on stable (`proptest` feeding random
//! bytes; they run with the workspace tests: `twine-testing/tests/fuzz_*.rs`) and the
//! cargo-fuzz targets `tree_ops`, `input_events` and `reactive_ops` of `fuzz/` (nightly,
//! coverage-guided; `cargo xtask fuzz <target>`). Decoding never fails: missing bytes read as
//! zero, so every byte string is a valid program.
//!
//! A violated invariant panics with the operation index and the operation list, which is what
//! both harnesses report and shrink.

/// A reader of fuzz input: each call takes the next bytes; past the end it returns zeros.
///
/// ```
/// use twine_testing::fuzz::Bytes;
/// let mut b = Bytes::new(&[7, 1, 2]);
/// assert_eq!(b.u8(), 7);
/// assert_eq!(b.u16(), 0x0201);
/// assert!(b.is_empty());
/// assert_eq!(b.u8(), 0);
/// ```
#[derive(Clone, Debug)]
pub struct Bytes<'a> {
    data: &'a [u8],
}

impl<'a> Bytes<'a> {
    /// A reader over `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Whether every byte was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The next byte (0 past the end).
    pub fn u8(&mut self) -> u8 {
        match self.data.split_first() {
            Some((b, rest)) => {
                self.data = rest;
                *b
            }
            None => 0,
        }
    }

    /// The next two bytes, little-endian.
    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    /// The next byte as a signed value.
    pub fn i8(&mut self) -> i8 {
        i8::from_le_bytes([self.u8()])
    }

    /// The next byte as a bool (lowest bit).
    pub fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }
}

/// Decodes operations from `data` with `decode` until the input is used up (at most `max`).
#[cfg(feature = "engine")]
fn decode_all<T>(data: &[u8], max: usize, decode: impl Fn(&mut Bytes<'_>) -> T) -> Vec<T> {
    let mut b = Bytes::new(data);
    let mut out = Vec::new();
    while !b.is_empty() && out.len() < max {
        out.push(decode(&mut b));
    }
    out
}

#[cfg(feature = "engine")]
pub use tree::{TreeOp, TreeRun, decode_tree_ops, run_tree_ops};

#[cfg(feature = "engine")]
mod tree {
    use twine_core::{Duration, FaultKind, Limits};
    use twine_engine::{Engine, NodeId, Obj, ObjFlags};
    use twine_style::{FlexFlow, LayoutKind, State};

    use super::{Bytes, decode_all};
    use crate::EngineHarness;

    /// One mutation of the widget tree. Node operands are indices into the list of nodes the
    /// run created (the screen is index 0), taken modulo its length — so they also name nodes
    /// that were deleted since: stale ids, which must be ignored with a warning (P7).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[allow(missing_docs)] // the fields are the operation's operands
    pub enum TreeOp {
        /// Creates a plain object under `parent`.
        Create { parent: u8 },
        /// Deletes `node` (and its subtree).
        Delete { node: u8 },
        /// Moves `node` under `parent` (refused for cycles, depth and dead nodes).
        Move { node: u8, parent: u8 },
        /// Sets the position.
        Pos { node: u8, x: i8, y: i8 },
        /// Sets the size (negative values included).
        Size { node: u8, w: i8, h: i8 },
        /// Sets or clears one of a few flags (`HIDDEN`, `SCROLLABLE`, `FLOATING`, `CLICKABLE`).
        Flag { node: u8, flag: u8, on: bool },
        /// Sets or clears one of a few states.
        State { node: u8, state: u8, on: bool },
        /// Switches the layout: none, flex row, flex column, flex row wrap.
        Flex { node: u8, kind: u8 },
        /// Scrolls by `(dx, dy)`.
        Scroll { node: u8, dx: i8, dy: i8 },
        /// Lets `ms` milliseconds pass and runs an update (layout and rendering).
        Step { ms: u8 },
    }

    impl TreeOp {
        fn decode(b: &mut Bytes<'_>) -> Self {
            match b.u8() % 10 {
                0 | 1 => Self::Create { parent: b.u8() },
                2 => Self::Delete { node: b.u8() },
                3 => Self::Move {
                    node: b.u8(),
                    parent: b.u8(),
                },
                4 => Self::Pos {
                    node: b.u8(),
                    x: b.i8(),
                    y: b.i8(),
                },
                5 => Self::Size {
                    node: b.u8(),
                    w: b.i8(),
                    h: b.i8(),
                },
                6 => Self::Flag {
                    node: b.u8(),
                    flag: b.u8(),
                    on: b.bool(),
                },
                7 => Self::State {
                    node: b.u8(),
                    state: b.u8(),
                    on: b.bool(),
                },
                8 => {
                    if b.bool() {
                        Self::Flex {
                            node: b.u8(),
                            kind: b.u8(),
                        }
                    } else {
                        Self::Scroll {
                            node: b.u8(),
                            dx: b.i8(),
                            dy: b.i8(),
                        }
                    }
                }
                _ => Self::Step { ms: b.u8() },
            }
        }
    }

    /// Decodes up to 256 tree operations from fuzz input. Never fails.
    ///
    /// ```
    /// use twine_testing::fuzz::{TreeOp, decode_tree_ops};
    /// assert_eq!(decode_tree_ops(&[0, 0]), vec![TreeOp::Create { parent: 0 }]);
    /// ```
    #[must_use]
    pub fn decode_tree_ops(data: &[u8]) -> Vec<TreeOp> {
        decode_all(data, 256, TreeOp::decode)
    }

    /// What a [`run_tree_ops`] run did.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct TreeRun {
        /// Nodes alive at the end (screen included).
        pub alive: usize,
        /// `Capacity` faults raised (node or depth bound reached).
        pub capacity_faults: u32,
    }

    /// The bounds of the fuzzed engine: small, so that the node and depth limits are reached.
    const LIMITS: Limits = Limits::DEFAULT.max_nodes(48).max_tree_depth(6);

    const FLAGS: [ObjFlags; 4] = [
        ObjFlags::HIDDEN,
        ObjFlags::SCROLLABLE,
        ObjFlags::FLOATING,
        ObjFlags::CLICKABLE,
    ];
    const STATES: [State; 5] = [
        State::PRESSED,
        State::FOCUSED,
        State::CHECKED,
        State::DISABLED,
        State::INVALID,
    ];

    fn pick(nodes: &[NodeId], i: u8) -> NodeId {
        nodes[usize::from(i) % nodes.len()]
    }

    fn apply(e: &mut Engine, nodes: &mut Vec<NodeId>, op: TreeOp) {
        match op {
            TreeOp::Create { parent } => {
                if let Ok(n) = e.create(pick(nodes, parent), Box::new(Obj)) {
                    nodes.push(n);
                }
            }
            TreeOp::Delete { node } => {
                // The screen (index 0) stays: deleting it is legal but ends the interesting part.
                if usize::from(node) % nodes.len() != 0 {
                    let _ = e.delete(pick(nodes, node));
                }
            }
            TreeOp::Move { node, parent } => {
                let _ = e.move_node(pick(nodes, node), pick(nodes, parent), None);
            }
            TreeOp::Pos { node, x, y } => e.set_pos(pick(nodes, node), i32::from(x), i32::from(y)),
            TreeOp::Size { node, w, h } => e.set_size(pick(nodes, node), i32::from(w), i32::from(h)),
            TreeOp::Flag { node, flag, on } => {
                e.set_flag(pick(nodes, node), FLAGS[usize::from(flag) % FLAGS.len()], on);
            }
            TreeOp::State { node, state, on } => {
                e.set_state(pick(nodes, node), STATES[usize::from(state) % STATES.len()], on);
            }
            TreeOp::Flex { node, kind } => {
                let n = pick(nodes, node);
                let flow = match kind % 4 {
                    0 => {
                        e.set_layout(n, LayoutKind::None);
                        return;
                    }
                    1 => FlexFlow::ROW,
                    2 => FlexFlow::COLUMN,
                    _ => FlexFlow::ROW.wrap(true),
                };
                e.set_layout(n, LayoutKind::Flex);
                e.set_flex_flow(n, flow);
            }
            TreeOp::Scroll { node, dx, dy } => {
                e.scroll_by(pick(nodes, node), i32::from(dx), i32::from(dy), false);
            }
            TreeOp::Step { .. } => {}
        }
    }

    /// Runs `ops` on a fresh 96×64 engine with small [`Limits`] and checks the invariants after
    /// every operation (see the [module docs](super)). Returns what the run did.
    ///
    /// ```
    /// use twine_testing::fuzz::{TreeOp, run_tree_ops};
    /// let run = run_tree_ops(&[TreeOp::Create { parent: 0 }, TreeOp::Step { ms: 16 }]);
    /// assert_eq!(run.alive, 2);
    /// ```
    ///
    /// # Panics
    /// When an invariant is violated (the message names the operation), or when the library
    /// panics.
    #[must_use]
    pub fn run_tree_ops(ops: &[TreeOp]) -> TreeRun {
        let mut h = EngineHarness::new(96, 64).limits(LIMITS);
        let mut nodes = vec![h.screen()];
        let mut run = TreeRun::default();
        for (i, op) in ops.iter().enumerate() {
            match *op {
                TreeOp::Step { ms } => {
                    h.advance(Duration::from_millis(u64::from(ms)));
                    let _ = h.update();
                }
                other => apply(h.engine_mut(), &mut nodes, other),
            }
            let e = h.engine();
            if let Err(err) = e.tree().check_invariants() {
                panic!("tree invariant violated after operation {i} ({op:?}): {err:?}\nops: {ops:?}");
            }
            let alive = nodes.iter().filter(|n| e.tree().node(**n).is_some()).count();
            assert!(
                alive <= usize::from(LIMITS.max_nodes),
                "node bound exceeded after operation {i}: {alive}\nops: {ops:?}"
            );
            for n in nodes.iter().filter(|n| e.tree().node(**n).is_some()) {
                let depth = e.tree().ancestors(*n).count();
                assert!(
                    depth <= usize::from(LIMITS.max_tree_depth),
                    "depth bound exceeded after operation {i}: {depth}\nops: {ops:?}"
                );
            }
        }
        let _ = h.update();
        let faults = h.engine_mut().take_faults();
        for kind in FaultKind::ALL {
            if kind != FaultKind::Capacity {
                assert!(!faults.contains(kind), "unexpected {kind:?} fault\nops: {ops:?}");
            }
        }
        run.capacity_faults = h.engine().fault_counts().get(FaultKind::Capacity);
        run.alive = nodes
            .iter()
            .filter(|n| h.engine().tree().node(**n).is_some())
            .count();
        run
    }
}

#[cfg(feature = "ui")]
pub use input::{InputOp, decode_input_ops, run_input_ops};

#[cfg(feature = "ui")]
mod input {
    use twine_core::{Duration, FaultKind, Point};
    use twine_engine::{Engine, NodeId};
    use twine_hal::{DeviceHealth, Key};
    use twine_style::Align;
    use twine_widgets::{arc, button, checkbox, label, slider, spinbox, switch, textarea};

    use super::{Bytes, decode_all};
    use crate::EngineHarness;

    /// One input event (or the passage of time) delivered through the mock input devices.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[allow(missing_docs)] // the fields are the event's operands
    pub enum InputOp {
        /// Pointer pressed at `(x, y)` (may lie outside the screen).
        Press { x: u8, y: u8 },
        /// Pointer moved to `(x, y)`.
        Move { x: u8, y: u8 },
        /// Pointer released.
        Release,
        /// One of the navigation and editing keys, or a character.
        Key { key: u8 },
        /// Encoder turned by `diff` steps.
        Encoder { diff: i8 },
        /// Encoder button clicked.
        EncoderClick,
        /// `ms` milliseconds pass (timers, long press, animations).
        Wait { ms: u8 },
        /// The pointer reports `Ok`, `Degraded` or `Failed`.
        PointerHealth { health: u8 },
    }

    impl InputOp {
        fn decode(b: &mut Bytes<'_>) -> Self {
            match b.u8() % 9 {
                0 | 1 => Self::Press { x: b.u8(), y: b.u8() },
                2 => Self::Move { x: b.u8(), y: b.u8() },
                3 => Self::Release,
                4 => Self::Key { key: b.u8() },
                5 => Self::Encoder { diff: b.i8() },
                6 => Self::EncoderClick,
                7 => Self::Wait { ms: b.u8() },
                _ => Self::PointerHealth { health: b.u8() },
            }
        }
    }

    /// Decodes up to 256 input events from fuzz input. Never fails.
    ///
    /// ```
    /// use twine_testing::fuzz::{InputOp, decode_input_ops};
    /// assert_eq!(decode_input_ops(&[3]), vec![InputOp::Release]);
    /// ```
    #[must_use]
    pub fn decode_input_ops(data: &[u8]) -> Vec<InputOp> {
        decode_all(data, 256, InputOp::decode)
    }

    const KEYS: [Key; 14] = [
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Esc,
        Key::Del,
        Key::Backspace,
        Key::Enter,
        Key::Next,
        Key::Prev,
        Key::Home,
        Key::End,
        Key::Char('a'),
        Key::Char('7'),
    ];

    /// Widgets reacting to every input kind, in one focus group.
    fn scene(e: &mut Engine, screen: NodeId) {
        let g = e.create_group().ok();
        e.set_default_group(g);
        let b = button::create(e, screen).ok();
        if let Some(b) = b {
            let _ = label::create_with(e, b, "OK");
            e.align(b, Align::TopLeft, 4, 4);
        }
        if let Ok(c) = checkbox::create_with(e, screen, "On") {
            e.align(c, Align::TopRight, -4, 4);
        }
        if let Ok(s) = slider::create(e, screen) {
            e.set_width(s, 100);
            e.align(s, Align::LeftMid, 10, 0);
        }
        if let Ok(s) = switch::create(e, screen) {
            e.align(s, Align::RightMid, -4, 0);
        }
        if let Ok(t) = textarea::create(e, screen) {
            e.set_size(t, 90, 40);
            e.align(t, Align::BottomLeft, 4, -4);
        }
        if let Ok(s) = spinbox::create(e, screen) {
            e.align(s, Align::BottomMid, 0, -4);
        }
        if let Ok(a) = arc::create(e, screen) {
            e.set_size(a, 50, 50);
            e.align(a, Align::BottomRight, -4, -4);
        }
    }

    /// Runs `ops` on a 200×150 screen of widgets (button, checkbox, slider, switch, textarea,
    /// spinbox, arc in one focus group, default theme) and checks the invariants after every
    /// event (see the [module docs](super)).
    ///
    /// ```
    /// use twine_testing::fuzz::{InputOp, run_input_ops};
    /// run_input_ops(&[InputOp::Press { x: 20, y: 20 }, InputOp::Release, InputOp::Key { key: 8 }]);
    /// ```
    ///
    /// # Panics
    /// When an invariant is violated (the message names the event), or when the library panics.
    pub fn run_input_ops(ops: &[InputOp]) {
        let mut h = EngineHarness::new(200, 150);
        let screen = h.screen();
        scene(h.engine_mut(), screen);
        h.run_until_idle();
        let mut health_changed = false;
        let mut pressed = false;
        for (i, op) in ops.iter().enumerate() {
            match *op {
                InputOp::Press { x, y } => {
                    pressed = true;
                    let _ = h.press(Point::new(i32::from(x), i32::from(y)));
                }
                InputOp::Move { x, y } => {
                    let _ = h.move_to(Point::new(i32::from(x), i32::from(y)));
                }
                InputOp::Release => {
                    pressed = false;
                    let _ = h.release();
                }
                InputOp::Key { key } => {
                    let _ = h.key(KEYS[usize::from(key) % KEYS.len()]);
                }
                InputOp::Encoder { diff } => {
                    let _ = h.encoder(i16::from(diff));
                }
                InputOp::EncoderClick => {
                    let _ = h.encoder_click();
                }
                InputOp::Wait { ms } => {
                    h.advance(Duration::from_millis(u64::from(ms)));
                    let _ = h.update();
                }
                InputOp::PointerHealth { health } => {
                    health_changed = true;
                    let health = match health % 3 {
                        0 => DeviceHealth::Ok,
                        1 => DeviceHealth::Degraded { errors: 1 },
                        _ => DeviceHealth::Failed,
                    };
                    let (id, pointer) = h.pointer_input();
                    pointer.set_health(health);
                    h.engine_mut().notify_input(id);
                    let _ = h.update();
                }
            }
            let e = h.engine();
            if let Err(err) = e.tree().check_invariants() {
                panic!("tree invariant violated after event {i} ({op:?}): {err:?}\nops: {ops:?}");
            }
            if let Some(f) = e.default_group().and_then(|g| e.focused(g)) {
                assert!(
                    e.tree().node(f).is_some(),
                    "focus on a dead node after event {i} ({op:?})\nops: {ops:?}"
                );
            }
        }
        if pressed {
            let _ = h.release();
        }
        // Let gestures, scroll throws and animations end (not `run_until_idle`: a focused
        // textarea's cursor blinks for as long as it is focused).
        for _ in 0..20 {
            h.advance(Duration::from_millis(100));
            let _ = h.update();
        }
        let faults = h.engine_mut().take_faults();
        for kind in FaultKind::ALL {
            let allowed = kind == FaultKind::InputDevice && health_changed;
            assert!(
                allowed || !faults.contains(kind),
                "unexpected {kind:?} fault\nops: {ops:?}"
            );
        }
    }
}

#[cfg(feature = "ui")]
pub use reactive::{ReactiveOp, decode_reactive_ops, run_reactive_ops};

#[cfg(feature = "ui")]
mod reactive {
    use std::cell::Cell;
    use std::rc::Rc;

    use twine_reactive::{EffectId, Memo, ReactiveError, Runtime, Scope, Signal};

    use super::{Bytes, decode_all};

    /// One reactive operation. Operands are indices into the run's scopes, signals, memos or
    /// effects (modulo their count), so they also name disposed ones.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[allow(missing_docs)] // the fields are the operation's operands
    pub enum ReactiveOp {
        /// A child scope of `parent`.
        Scope { parent: u8 },
        /// Disposes `scope` (never the root) and everything in it.
        Dispose { scope: u8 },
        /// A signal in `scope` holding `value`.
        Signal { scope: u8, value: i8 },
        /// A memo in `scope` summing two signals of `scope` or its ancestors.
        Memo { scope: u8, a: u8, b: u8 },
        /// An effect in `scope` reading one signal of `scope` or its ancestors.
        Effect { scope: u8, signal: u8 },
        /// Writes `value` to `signal`.
        Set { signal: u8, value: i8 },
        /// Writes several signals in one batch (`count` writes from the following operands).
        Batch { first: u8, count: u8, value: i8 },
        /// Reads `memo` (or a signal when there is none).
        Read { memo: u8 },
    }

    impl ReactiveOp {
        fn decode(b: &mut Bytes<'_>) -> Self {
            match b.u8() % 9 {
                0 => Self::Scope { parent: b.u8() },
                1 => Self::Dispose { scope: b.u8() },
                2 | 3 => Self::Signal {
                    scope: b.u8(),
                    value: b.i8(),
                },
                4 => Self::Memo {
                    scope: b.u8(),
                    a: b.u8(),
                    b: b.u8(),
                },
                5 => Self::Effect {
                    scope: b.u8(),
                    signal: b.u8(),
                },
                6 => Self::Set {
                    signal: b.u8(),
                    value: b.i8(),
                },
                7 => Self::Batch {
                    first: b.u8(),
                    count: b.u8() % 4,
                    value: b.i8(),
                },
                _ => Self::Read { memo: b.u8() },
            }
        }
    }

    /// Decodes up to 256 reactive operations from fuzz input. Never fails.
    ///
    /// ```
    /// use twine_testing::fuzz::{ReactiveOp, decode_reactive_ops};
    /// assert_eq!(decode_reactive_ops(&[0, 0]), vec![ReactiveOp::Scope { parent: 0 }]);
    /// ```
    #[must_use]
    pub fn decode_reactive_ops(data: &[u8]) -> Vec<ReactiveOp> {
        decode_all(data, 256, ReactiveOp::decode)
    }

    struct ScopeM {
        scope: Scope,
        parent: Option<usize>,
        alive: bool,
    }

    struct SignalM {
        signal: Signal<i64>,
        scope: usize,
        value: i64,
    }

    struct MemoM {
        memo: Memo<i64>,
        scope: usize,
        deps: [usize; 2],
    }

    struct EffectM {
        _id: EffectId,
        scope: usize,
        signal: usize,
        runs: Rc<Cell<u32>>,
        expected: u32,
    }

    /// The model: what should be alive and what values things should have.
    struct Model {
        scopes: Vec<ScopeM>,
        signals: Vec<SignalM>,
        memos: Vec<MemoM>,
        effects: Vec<EffectM>,
    }

    impl Model {
        fn alive(&self, scope: usize) -> bool {
            self.scopes[scope].alive
        }

        /// `scope` and its ancestors.
        fn lineage(&self, scope: usize) -> Vec<usize> {
            let mut out = vec![scope];
            let mut cur = self.scopes[scope].parent;
            while let Some(p) = cur {
                out.push(p);
                cur = self.scopes[p].parent;
            }
            out
        }

        /// Signals created in `scope` or an ancestor (alive with it).
        fn visible_signals(&self, scope: usize) -> Vec<usize> {
            let lineage = self.lineage(scope);
            (0..self.signals.len())
                .filter(|&i| lineage.contains(&self.signals[i].scope) && self.alive(self.signals[i].scope))
                .collect()
        }

        fn dispose(&mut self, scope: usize) {
            for i in 0..self.scopes.len() {
                if self.lineage(i).contains(&scope) {
                    self.scopes[i].alive = false;
                }
            }
        }

        fn live_nodes(&self) -> usize {
            self.signals.iter().filter(|s| self.alive(s.scope)).count()
                + self.memos.iter().filter(|m| self.alive(m.scope)).count()
                + self.effects.iter().filter(|e| self.alive(e.scope)).count()
        }

        fn live_scopes(&self) -> usize {
            self.scopes.iter().filter(|s| s.alive).count()
        }

        /// Records that `signal` was written (each live effect reading it runs once more).
        fn written(&mut self, signals: &[usize]) {
            for e in &mut self.effects {
                if self.scopes[e.scope].alive && signals.contains(&e.signal) {
                    e.expected += 1;
                }
            }
        }
    }

    fn pick(len: usize, i: u8) -> Option<usize> {
        (len > 0).then(|| usize::from(i) % len)
    }

    fn apply(m: &mut Model, rt: Runtime, op: ReactiveOp) {
        match op {
            ReactiveOp::Scope { parent } => {
                let p = usize::from(parent) % m.scopes.len();
                match m.scopes[p].scope.try_child() {
                    Ok(scope) => {
                        assert!(m.alive(p), "a child scope was created in a disposed scope");
                        m.scopes.push(ScopeM {
                            scope,
                            parent: Some(p),
                            alive: true,
                        });
                    }
                    Err(e) => assert!(!m.alive(p), "try_child failed in a live scope: {e:?}"),
                }
            }
            ReactiveOp::Dispose { scope } => {
                let s = usize::from(scope) % m.scopes.len();
                if s != 0 {
                    m.scopes[s].scope.dispose();
                    m.dispose(s);
                }
            }
            ReactiveOp::Signal { scope, value } => {
                let s = usize::from(scope) % m.scopes.len();
                let value = i64::from(value);
                match m.scopes[s].scope.try_signal(value) {
                    Ok(signal) => {
                        assert!(m.alive(s), "a signal was created in a disposed scope");
                        m.signals.push(SignalM {
                            signal,
                            scope: s,
                            value,
                        });
                    }
                    Err(e) => assert!(!m.alive(s), "try_signal failed in a live scope: {e:?}"),
                }
            }
            ReactiveOp::Memo { scope, a, b } => {
                let s = usize::from(scope) % m.scopes.len();
                let visible = m.visible_signals(s);
                let (Some(ia), Some(ib)) = (pick(visible.len(), a), pick(visible.len(), b)) else {
                    return;
                };
                let deps = [visible[ia], visible[ib]];
                let (sa, sb) = (m.signals[deps[0]].signal, m.signals[deps[1]].signal);
                match m.scopes[s]
                    .scope
                    .try_memo(move || sa.try_get().unwrap_or(0) + sb.try_get().unwrap_or(0))
                {
                    Ok(memo) => m.memos.push(MemoM { memo, scope: s, deps }),
                    Err(e) => assert!(!m.alive(s), "try_memo failed in a live scope: {e:?}"),
                }
            }
            ReactiveOp::Effect { scope, signal } => {
                let s = usize::from(scope) % m.scopes.len();
                let visible = m.visible_signals(s);
                let Some(i) = pick(visible.len(), signal) else {
                    return;
                };
                let dep = visible[i];
                let sig = m.signals[dep].signal;
                let runs = Rc::new(Cell::new(0u32));
                let r = runs.clone();
                match m.scopes[s].scope.try_effect(move || {
                    let _ = sig.try_get();
                    r.set(r.get() + 1);
                }) {
                    Ok(id) => {
                        m.effects.push(EffectM {
                            _id: id,
                            scope: s,
                            signal: dep,
                            runs,
                            expected: 1,
                        });
                    }
                    Err(e) => assert!(!m.alive(s), "try_effect failed in a live scope: {e:?}"),
                }
            }
            ReactiveOp::Set { signal, value } => {
                let Some(i) = pick(m.signals.len(), signal) else {
                    return;
                };
                let value = i64::from(value);
                let alive = m.alive(m.signals[i].scope);
                match m.signals[i].signal.try_set(value) {
                    Ok(()) => {
                        assert!(alive, "a disposed signal accepted a write");
                        m.signals[i].value = value;
                        m.written(&[i]);
                    }
                    Err(e) => {
                        assert!(!alive, "a live signal refused a write: {e:?}");
                        assert!(matches!(e, ReactiveError::Disposed(_)), "{e:?}");
                    }
                }
            }
            ReactiveOp::Batch { first, count, value } => {
                let mut written = Vec::new();
                rt.batch(|| {
                    for k in 0..count {
                        let Some(i) = pick(m.signals.len(), first.wrapping_add(k)) else {
                            return;
                        };
                        let v = i64::from(value) + i64::from(k);
                        if m.signals[i].signal.try_set(v).is_ok() {
                            m.signals[i].value = v;
                            written.push(i);
                        }
                    }
                });
                written.sort_unstable();
                written.dedup();
                m.written(&written);
            }
            ReactiveOp::Read { memo } => {
                if let Some(i) = pick(m.memos.len(), memo) {
                    let alive = m.alive(m.memos[i].scope);
                    match m.memos[i].memo.try_get() {
                        Ok(v) => {
                            assert!(alive, "a disposed memo answered a read");
                            let expected: i64 = m.memos[i].deps.iter().map(|&d| m.signals[d].value).sum();
                            assert_eq!(v, expected, "memo value differs from the model");
                        }
                        Err(e) => {
                            assert!(!alive, "a live memo refused a read: {e:?}");
                            assert!(matches!(e, ReactiveError::Disposed(_)), "{e:?}");
                        }
                    }
                } else if let Some(i) = pick(m.signals.len(), memo) {
                    let alive = m.alive(m.signals[i].scope);
                    assert_eq!(
                        m.signals[i].signal.try_get().ok(),
                        alive.then_some(m.signals[i].value)
                    );
                }
            }
        }
    }

    /// Runs `ops` in a fresh root scope of the calling thread's runtime and checks the runtime
    /// against a model after every operation; at the end the root is disposed and the runtime
    /// must be back at its starting node, scope, queue and registration counts (see the
    /// [module docs](super)).
    ///
    /// ```
    /// use twine_testing::fuzz::{ReactiveOp, run_reactive_ops};
    /// run_reactive_ops(&[
    ///     ReactiveOp::Signal { scope: 0, value: 1 },
    ///     ReactiveOp::Effect { scope: 0, signal: 0 },
    ///     ReactiveOp::Set { signal: 0, value: 2 },
    /// ]);
    /// ```
    ///
    /// # Panics
    /// When the runtime differs from the model (the message names the operation), or when the
    /// library panics.
    pub fn run_reactive_ops(ops: &[ReactiveOp]) {
        let rt = Runtime::current_thread();
        let base = rt.stats();
        let root = rt.create_root();
        let mut m = Model {
            scopes: vec![ScopeM {
                scope: root,
                parent: None,
                alive: true,
            }],
            signals: Vec::new(),
            memos: Vec::new(),
            effects: Vec::new(),
        };
        for (i, op) in ops.iter().enumerate() {
            apply(&mut m, rt, *op);
            let st = rt.stats();
            let ctx = || format!("after operation {i} ({op:?})\nops: {ops:?}");
            assert_eq!(st.nodes, base.nodes + m.live_nodes(), "live nodes {}", ctx());
            assert_eq!(st.scopes, base.scopes + m.live_scopes(), "live scopes {}", ctx());
            assert_eq!(st.pending, base.pending, "effects left pending {}", ctx());
            for (k, e) in m.effects.iter().enumerate() {
                if m.scopes[e.scope].alive {
                    assert_eq!(e.runs.get(), e.expected, "runs of effect {k} {}", ctx());
                }
            }
        }
        root.dispose();
        let end = rt.stats();
        assert_eq!(
            (end.nodes, end.scopes, end.pending, end.deferred, end.channels),
            (
                base.nodes,
                base.scopes,
                base.pending,
                base.deferred,
                base.channels
            ),
            "the runtime did not return to its starting counts\nops: {ops:?}"
        );
        for e in &m.effects {
            let before = e.runs.get();
            if let Some(s) = m.signals.get(e.signal) {
                let _ = s.signal.try_set(0);
            }
            assert_eq!(e.runs.get(), before, "a disposed effect ran\nops: {ops:?}");
        }
    }
}
