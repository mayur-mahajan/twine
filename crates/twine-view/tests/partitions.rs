//! Freedom from interference between partitions (R5.S07, ADR-001 § 6.3): a critical `Ui` (A,
//! partition 0: a safety-critical label showing a value from a `Latest`) and a non-critical
//! `Ui` (B, partition 1) run on one thread, stepped in one loop, each on its own
//! `MemoryDisplay`. Faults of every kind the ADR lists are injected into B, in turn; after
//! each, A's panel shows exactly what it shows without B (XXH64 of its pixels against a
//! reference run of A alone, plus A's own off-screen `check_region`), A has no fault, its
//! timing and heartbeat advance, and a new value reaches A's display within one update. B
//! shows the expected fault.
//!
//! A **control run** puts both UIs on one partition and shows the interference the
//! partitioned runs exclude (a loop cut dropping A's effects, B's faults in A's stream, B's
//! limits in A's runtime), so these tests fail if partitions silently collapse into one.
//!
//! Built with `--features partitions,display-integrity` (and with `safety,partitions`, which
//! adds the `no-panic-runtime` cases); `cargo xtask ci` stage `test-features`.
//!
//! req: SAF-034
#![cfg(all(feature = "partitions", feature = "display-integrity"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_core::{ColorFormat, Partition, alloc_guard};
use twine_hal::DisplayInfo;
use twine_reactive::Latest;
use twine_testing::snapshot::xxh64;
use twine_testing::{HARNESS_LIMITS, MemoryDisplay, MockClock, profile_limits};
use twine_view::prelude::*;

const W: u16 = 96;
const H: u16 = 48;
/// The critical partition and the non-critical one.
const PART_A: Partition = Partition::ALL[0];
const PART_B: Partition = Partition::ALL[1];

/// One `Latest` per test (tests run in parallel; each test's A consumes its own).
static SPEED: [Latest<u16>; 10] = [const { Latest::new(0) }; 10];

thread_local! {
    /// The fake high-resolution time (µs): one µs per reading, plus what a slow binding adds.
    static HIRES_US: Cell<u64> = const { Cell::new(0) };
}

/// The engines' high-resolution timer.
fn hires() -> Instant {
    HIRES_US.with(|c| {
        c.set(c.get() + 1);
        Instant::from_micros(c.get())
    })
}

/// Simulates `us` µs of work in the calling update.
fn burn(us: u64) {
    HIRES_US.with(|c| c.set(c.get() + us));
}

fn config(engine: EngineConfig, limits: Limits) -> AppConfig {
    let mut c = AppConfig::new()
        .limits(profile_limits(limits))
        .theme(&lvgl::THEME);
    c.engine = EngineConfig {
        hires_timer: Some(hires),
        ..engine
    };
    c
}

fn build<V: View>(part: Partition, clock: &MockClock, cfg: AppConfig, app: impl FnOnce(Scope) -> V) -> Ui {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    Ui::builder(panel)
        .runtime(Runtime::current_thread_partition(part))
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 16 }))
        .app_config(cfg)
        .try_build(app)
        .unwrap()
}

/// XXH64 of the panel's pixels (as the golden-image manifests hash them).
fn panel_hash(ui: &Ui) -> u64 {
    let panel = ui.engine().driver::<MemoryDisplay>(ui.display()).unwrap();
    xxh64(&panel.to_rgb888(), 0)
}

fn advance(clock: &MockClock) {
    clock.advance(Duration::ms(20));
}

/// Updates until idle (bounded).
fn settle(ui: &mut Ui, clock: &MockClock) {
    for _ in 0..50 {
        advance(clock);
        if matches!(ui.update(), Wake::Idle | Wake::IdleFor(_)) {
            return;
        }
    }
}

/// The critical UI: one safety-critical label showing the speed.
struct Critical {
    ui: Ui,
    latest: &'static Latest<u16>,
    label: Rc<RefCell<Option<NodeRef<twine_widgets::label::Label>>>>,
    /// Reference pixels of A alone showing 10 and 20.
    ref_10: u64,
    ref_20: u64,
}

impl Critical {
    fn new(slot: usize, clock: &MockClock) -> Critical {
        let latest = &SPEED[slot];
        latest.set(10);
        let cell = Rc::new(RefCell::new(None));
        let c = cell.clone();
        let mut ui = build(
            PART_A,
            clock,
            config(EngineConfig::default(), HARNESS_LIMITS),
            move |cx| {
                let speed = cx.watch(latest);
                let r = cx.node_ref();
                *c.borrow_mut() = Some(r);
                container(
                    label(text!("{} km/h", speed.get()))
                        .safety_critical(true)
                        .node_ref(r),
                )
                .size(96, 48)
            },
        );
        settle(&mut ui, clock);
        let mut a = Critical {
            ui,
            latest,
            label: cell,
            ref_10: 0,
            ref_20: 0,
        };
        // The reference run: A alone (B does not exist yet).
        a.ref_10 = panel_hash(&a.ui);
        a.show(20, clock);
        a.ref_20 = panel_hash(&a.ui);
        a.show(10, clock);
        assert_eq!(panel_hash(&a.ui), a.ref_10, "rendering is deterministic");
        assert_ne!(a.ref_10, a.ref_20);
        assert!(a.ui.take_faults().is_empty());
        a
    }

    /// Sets the speed and runs one update: the new value must be on the panel.
    fn show(&mut self, v: u16, clock: &MockClock) {
        self.latest.set(v);
        advance(clock);
        self.ui.update();
        assert_eq!(self.text(), format!("{v} km/h"), "shown within one update");
    }

    fn text(&self) -> String {
        let n = self.node();
        self.ui
            .engine()
            .tree()
            .node(n)
            .and_then(|n| n.widget().text())
            .unwrap_or_default()
            .to_string()
    }

    fn node(&self) -> NodeId {
        self.label.borrow().unwrap().get_untracked().unwrap()
    }

    /// Everything the ADR requires of A after a fault in B.
    fn assert_intact(&mut self, clock: &MockClock, heartbeat_before: u32) {
        let faults = self.ui.take_faults();
        assert!(faults.is_empty(), "A has no fault: {faults:?}");
        assert_eq!(
            panel_hash(&self.ui),
            self.ref_10,
            "A's pixels equal the run without B"
        );
        let n = self.node();
        let e = self.ui.engine_mut();
        let mut buf = vec![0u8; e.critical_report(n).unwrap().region.unwrap().buffer_bytes];
        assert!(
            e.check_region(n, &mut buf).unwrap().ok(),
            "A's own integrity check"
        );
        assert!(self.ui.heartbeat() > heartbeat_before, "A's heartbeat advances");
        let t = self.ui.timing();
        assert!(t.worst_update_us > 0, "A's timing is measured");
        assert!(
            t.worst_update_us < 1_000,
            "A's updates never contain B's work: {t:?}"
        );
        assert!(self.ui.fail_safe().is_none());
        self.show(20, clock);
        assert_eq!(panel_hash(&self.ui), self.ref_20);
        self.show(10, clock);
        assert_eq!(panel_hash(&self.ui), self.ref_10);
        assert!(self.ui.take_faults().is_empty());
    }
}

/// Steps A and B in one loop (A first, as a product's super-loop would).
fn step_both(a: &mut Critical, b: &mut Ui, clock: &MockClock, n: usize) {
    for _ in 0..n {
        advance(clock);
        a.ui.update();
        b.update();
    }
}

fn plain_b(clock: &MockClock, engine: EngineConfig, limits: Limits) -> Ui {
    build(PART_B, clock, config(engine, limits), |_| {
        container(label("media")).size(96, 48).bg(Color::BLUE)
    })
}

/// (1) An effect loop in B: cut by B's limit, B's fault only; A's flush never sees it.
///
/// fault: `EffectLoopCut`
#[test]
fn effect_loop_in_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(0, &clock);
    let hb = a.ui.heartbeat();
    let mut b = plain_b(&clock, EngineConfig::default(), HARNESS_LIMITS);
    let rt_b = b.runtime();
    rt_b.set_flush_iterations_limit(3);
    let cx = b.root_scope();
    let spin = cx.signal(0u32);
    cx.effect(move || spin.set(spin.get() + 1));
    step_both(&mut a, &mut b, &clock, 4);
    assert!(b.take_faults().contains(FaultKind::EffectLoopCut));
    a.assert_intact(&clock, hb);
}

/// (2) B exhausts its reactive nodes, scopes and effect queue: B's bounds, B's faults.
///
/// fault: `Capacity`
#[test]
fn exhausted_bounds_in_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(1, &clock);
    let hb = a.ui.heartbeat();
    let tiny = Limits {
        max_reactive_nodes: 64,
        max_scopes: 8,
        max_pending_effects: 2,
        ..HARNESS_LIMITS
    };
    let mut b = plain_b(&clock, EngineConfig::default(), tiny);
    let cx = b.root_scope();
    let trigger = cx.signal(0u32);
    for _ in 0..8 {
        cx.effect(move || {
            trigger.get();
        });
    }
    trigger.set(1); // 8 effects, a queue of 2: 6 not queued
    while cx.try_signal(0u8).is_ok() {}
    while cx.try_child().is_ok() {}
    step_both(&mut a, &mut b, &clock, 3);
    assert!(b.take_faults().contains(FaultKind::Capacity));
    // A's bounds are its own: it keeps creating.
    let ca = a.ui.root_scope();
    let many: Vec<_> = (0..100).map(|i| ca.signal(i)).collect();
    assert_eq!(many[99].get(), 99);
    let _child = ca.child();
    a.assert_intact(&clock, hb);
}

/// (3) Reactive misuse in B under `no-panic-runtime` (the safety profile): a write to a
/// disposed signal and a memo cycle are B's faults, skipped in B.
///
/// fault: `DisposedHandle`, `MemoCycle`
#[cfg(feature = "safety")]
#[test]
fn reactive_misuse_in_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(2, &clock);
    let hb = a.ui.heartbeat();
    let mut b = plain_b(&clock, EngineConfig::default(), HARNESS_LIMITS);
    let cx = b.root_scope().child();
    let gone = cx.signal(1u32);
    cx.dispose();
    gone.set(2); // skipped: DisposedHandle in B
    // A memo reading itself once its source changes: the cycle keeps its previous value.
    let root = b.root_scope();
    let me: Rc<Cell<Option<Memo<u32>>>> = Rc::new(Cell::new(None));
    let m2 = me.clone();
    let src = root.signal(0u32);
    let m = root.memo(move || {
        let v = src.get();
        m2.get().map_or(v, |m| m.get() + v)
    });
    assert_eq!(m.get(), 0);
    me.set(Some(m));
    src.set(1);
    let _ = m.get(); // MemoCycle in B
    step_both(&mut a, &mut b, &clock, 3);
    let faults = b.take_faults();
    assert!(faults.contains(FaultKind::DisposedHandle), "{faults:?}");
    assert!(faults.contains(FaultKind::MemoCycle), "{faults:?}");
    a.assert_intact(&clock, hb);
}

/// (4) B allocates after A sealed (and after its own seal): only B's evidence.
///
/// fault: `AllocAfterSeal`
#[test]
fn allocation_after_a_seal_is_not_a_fault() {
    let clock = MockClock::new();
    let mut a = Critical::new(3, &clock);
    let hb = a.ui.heartbeat();
    let mut b = plain_b(&clock, EngineConfig::default(), HARNESS_LIMITS);
    step_both(&mut a, &mut b, &clock, 2);
    let _ = a.ui.seal();
    // B grows after A's seal: B is not sealed, nothing is noted anywhere.
    let cx = b.root_scope();
    let _many: Vec<_> = (0..64).map(|i| cx.signal(i)).collect();
    step_both(&mut a, &mut b, &clock, 2);
    assert!(!a.ui.take_faults().contains(FaultKind::AllocAfterSeal));
    // B seals, then grows: B's fault.
    let _ = b.seal();
    let _more: Vec<_> = (0..64).map(|i| cx.signal(i)).collect();
    step_both(&mut a, &mut b, &clock, 2);
    assert!(b.take_faults().contains(FaultKind::AllocAfterSeal));
    alloc_guard::unseal(PART_B);
    alloc_guard::unseal(PART_A);
    a.assert_intact(&clock, hb);
}

/// (5) A slow binding in B and a tiny step budget: B's updates overrun, B's fault; A's
/// updates contain none of B's work and its timing stays its own.
///
/// fault: `FrameOverrun`
#[test]
fn slow_update_in_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(4, &clock);
    let hb = a.ui.heartbeat();
    let engine = EngineConfig {
        step_budget: StepBudget::time(Duration::ms(1)),
        ..EngineConfig::default()
    };
    let tick_cell: Rc<Cell<Option<Signal<u32>>>> = Rc::new(Cell::new(None));
    let t = tick_cell.clone();
    let mut b = build(PART_B, &clock, config(engine, HARNESS_LIMITS), move |cx| {
        let tick = cx.signal(0u32);
        t.set(Some(tick));
        container(label(text!("{}", {
            burn(50_000); // 50 ms of work in B's binding
            tick.get()
        })))
        .size(96, 48)
    });
    let tick = tick_cell.get().unwrap();
    a.ui.reset_worst();
    for i in 1..=3 {
        tick.set(i);
        step_both(&mut a, &mut b, &clock, 2);
    }
    assert!(b.fault_counts().get(FaultKind::FrameOverrun) >= 1);
    assert!(b.timing().worst_update_us >= 50_000);
    a.assert_intact(&clock, hb);
}

/// (6) B enters its fail-safe screen and its panel fails an integrity check: B's state and
/// faults; A's display, observer and fail-safe state are its own.
///
/// fault: `FailSafe`, `DisplayIntegrity`
#[test]
fn fail_safe_and_integrity_mismatch_in_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(5, &clock);
    let hb = a.ui.heartbeat();
    let cell = Rc::new(RefCell::new(None));
    let c = cell.clone();
    let mut b = build(
        PART_B,
        &clock,
        config(EngineConfig::default(), HARNESS_LIMITS),
        move |cx| {
            let r = cx.node_ref();
            *c.borrow_mut() = Some(r);
            container(label("track 7").safety_critical(true).node_ref(r)).size(96, 48)
        },
    );
    step_both(&mut a, &mut b, &clock, 3);
    // A corrupted panel in B.
    let d = b.display();
    for byte in b
        .engine_mut()
        .driver_mut::<MemoryDisplay>(d)
        .unwrap()
        .framebuffer_mut()
    {
        *byte = !*byte;
    }
    let n = cell.borrow().unwrap().get_untracked().unwrap();
    let e = b.engine_mut();
    let mut buf = vec![0u8; e.critical_report(n).unwrap().region.unwrap().buffer_bytes];
    assert!(!e.check_region(n, &mut buf).unwrap().ok());
    b.set_fail_safe(|_| container(label("SERVICE")).size(96, 48).bg(Color::RED))
        .unwrap();
    b.enter_fail_safe(7);
    step_both(&mut a, &mut b, &clock, 3);
    let faults = b.take_faults();
    assert!(faults.contains(FaultKind::DisplayIntegrity) && faults.contains(FaultKind::FailSafe));
    assert_eq!(b.fail_safe(), Some(7));
    a.assert_intact(&clock, hb);
}

/// (7) B stalls (its context never runs again): A keeps updating and its heartbeat counts,
/// B's does not — the external watchdog sees which partition stalled.
#[test]
fn stalled_b() {
    let clock = MockClock::new();
    let mut a = Critical::new(6, &clock);
    let mut b = plain_b(&clock, EngineConfig::default(), HARNESS_LIMITS);
    step_both(&mut a, &mut b, &clock, 2);
    let (hb_a, hb_b) = (a.ui.heartbeat(), b.heartbeat());
    for _ in 0..10 {
        advance(&clock);
        a.ui.update(); // B is not stepped
    }
    assert_eq!(a.ui.heartbeat(), hb_a.wrapping_add(10));
    assert_eq!(b.heartbeat(), hb_b);
    a.assert_intact(&clock, hb_a);
}

/// The loop-cut scenario of the control run, on partitioned runtimes: B's loop (with B's
/// flush limit of 3) and a four-step effect chain of A, each started in its own runtime.
#[test]
fn partitioned_loop_cut_keeps_a_effect_chain() {
    let clock = MockClock::new();
    let mut a = Critical::new(7, &clock);
    let mut b = plain_b(&clock, EngineConfig::default(), HARNESS_LIMITS);
    let (kick, done) = loop_and_chain(&a.ui, &b);
    b.runtime().batch(|| kick.set(1));
    a.ui.runtime().batch(|| chain_start(&a.ui).set(1));
    assert!(done.get(), "A's chain completed");
    step_both(&mut a, &mut b, &clock, 2);
    assert!(b.take_faults().contains(FaultKind::EffectLoopCut));
    assert!(a.ui.take_faults().is_empty());
}

thread_local! {
    static CHAIN_START: Cell<Option<Signal<u32>>> = const { Cell::new(None) };
}

fn chain_start(_a: &Ui) -> Signal<u32> {
    CHAIN_START.with(Cell::get).unwrap()
}

/// B: a self-feeding effect on `kick` with B's flush limit 3. A: a chain of four effects
/// (each queues the next in the following flush round); `done` is set by the last one.
fn loop_and_chain(a: &Ui, b: &Ui) -> (Signal<u32>, Rc<Cell<bool>>) {
    b.runtime().set_flush_iterations_limit(3);
    let cb = b.root_scope();
    let kick = cb.signal(0u32);
    cb.effect(move || {
        if kick.get() > 0 {
            kick.set(kick.get_untracked() + 1);
        }
    });
    let ca = a.root_scope();
    let s: Vec<Signal<u32>> = (0..4).map(|_| ca.signal(0u32)).collect();
    let done = Rc::new(Cell::new(false));
    for i in 0..3 {
        let (from, to) = (s[i], s[i + 1]);
        ca.effect(move || {
            let v = from.get();
            if v > 0 {
                to.set(v);
            }
        });
    }
    let (last, d) = (s[3], done.clone());
    ca.effect(move || {
        if last.get() > 0 {
            d.set(true);
        }
    });
    CHAIN_START.with(|c| c.set(Some(s[0])));
    (kick, done)
}

/// The control run: A and B on **one** partition (one runtime) — the interference the
/// partitioned tests exclude. If partitions collapsed into one runtime, the tests above would
/// fail the way this one passes.
#[test]
fn control_run_on_one_partition_shows_the_interference() {
    let clock = MockClock::new();
    let latest = &SPEED[8];
    latest.set(10);
    let mut a = build(
        PART_A,
        &clock,
        config(EngineConfig::default(), HARNESS_LIMITS),
        move |cx| {
            let speed = cx.watch(latest);
            container(label(text!("{} km/h", speed.get())).safety_critical(true)).size(96, 48)
        },
    );
    // B on A's partition, with tighter reactive bounds: they become A's too.
    let tight = Limits {
        max_reactive_nodes: 512,
        ..HARNESS_LIMITS
    };
    let mut b = build(PART_A, &clock, config(EngineConfig::default(), tight), |_| {
        container(label("media")).size(96, 48).bg(Color::BLUE)
    });
    assert_eq!(a.runtime(), b.runtime(), "one runtime");
    settle(&mut a, &clock);
    settle(&mut b, &clock);
    let _ = a.take_faults();
    let _ = b.take_faults();
    // (G1/G9) B's flush limit is A's: B's loop cut, in the same flush, drops A's effects.
    let (kick, done) = loop_and_chain(&a, &b);
    a.runtime().batch(|| {
        kick.set(1);
        chain_start(&a).set(1);
    });
    assert!(!done.get(), "the loop cut dropped A's pending effects");
    // (G4) The runtime's faults go to whichever UI updates first: B's cut lands in A.
    advance(&clock);
    a.update();
    assert!(
        a.take_faults().contains(FaultKind::EffectLoopCut),
        "B's fault in A's stream"
    );
    // (G2) B's bound is A's bound.
    let ca = a.root_scope();
    let mut created = 0;
    while ca.try_signal(0u8).is_ok() {
        created += 1;
        assert!(created < 10_000);
    }
    assert!(a.runtime().stats().nodes <= 512, "A is bounded by B's limit");
}

/// The partitioned counterpart of the control's G2/G9 checks: B's limits never become A's.
#[test]
fn b_limits_are_not_a_limits() {
    let clock = MockClock::new();
    let a = Critical::new(9, &clock);
    let tight = Limits {
        max_reactive_nodes: 512,
        ..HARNESS_LIMITS
    };
    let b = plain_b(&clock, EngineConfig::default(), tight);
    assert_ne!(a.ui.runtime(), b.runtime());
    assert_eq!(a.ui.partition(), PART_A);
    assert_eq!(b.partition(), PART_B);
    assert_eq!(a.ui.engine().partition(), PART_A);
    assert_eq!(b.engine().partition(), PART_B);
    let ca = a.ui.root_scope();
    let many: Vec<_> = (0..600).map(|i| ca.try_signal(i)).collect();
    assert!(many.iter().all(Result::is_ok), "A keeps its own bound");
    let audit_b = AppConfig::new().partition(PART_B).audit_report().to_string();
    assert!(audit_b.contains("partition.index=1\n"));
    assert!(audit_b.contains(&format!("build.partitions={}\n", Partition::COUNT)));
}
