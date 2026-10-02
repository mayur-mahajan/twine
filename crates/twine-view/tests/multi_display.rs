//! Several displays and several UIs on one reactive runtime (R3.S08).
//!
//! - Two `Ui`s (two engines) sharing one runtime and a signal: an update of one never runs
//!   the other's bindings, channel handlers or engine commands, so nothing is written into the
//!   wrong engine; the other UI's work waits for its own update, which its waker requests.
//! - One `Ui` with two displays: independent dirty areas, inputs, theme modes and display
//!   commands; a shared signal updates both; one waker; the blocking run loop drives both;
//!   the steady state allocates nothing.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use twine_core::ColorFormat;
use twine_engine::{DisplayId, Engine, EngineConfig, EngineError, NodeId};
use twine_hal::{DisplayInfo, PollHint};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MockClock, MockPlatform, MockPointer};
use twine_view::UiCore;
use twine_view::prelude::*;
use twine_view::run::{self, LoopEvent};
use twine_widgets::label::Label;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn panel(w: u16, h: u16) -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565))
}

fn bufs() -> BufferMode {
    BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 })
}

fn leak<T>(v: T) -> &'static T {
    Box::leak(Box::new(v))
}

/// Every label text under `n`, depth first.
fn texts_under(e: &Engine, n: NodeId, out: &mut Vec<String>) {
    if let Some(l) = e.widget::<Label>(n) {
        out.push(l.text().to_owned());
    }
    for c in e.tree().children(n) {
        texts_under(e, c, out);
    }
}

/// The label texts on `display`'s active screen.
fn texts(e: &Engine, display: DisplayId) -> Vec<String> {
    let mut out = Vec::new();
    texts_under(e, e.active_screen(display).unwrap(), &mut out);
    out
}

/// The node with test id `id` under `n`.
fn find_under(e: &Engine, n: NodeId, id: &str) -> Option<NodeId> {
    if e.tree().node(n).and_then(twine_engine::Node::test_id) == Some(id) {
        return Some(n);
    }
    e.tree().children(n).find_map(|c| find_under(e, c, id))
}

/// The center of the node with test id `id` on `display`, in that display's coordinates.
fn center_of(e: &Engine, display: DisplayId, id: &str) -> Point {
    let n = find_under(e, e.active_screen(display).unwrap(), id).unwrap();
    e.coords(n).center()
}

/// Updates until idle, advancing the clock to every deadline.
fn settle(ui: &mut Ui, clock: &MockClock) {
    for _ in 0..50 {
        match ui.update() {
            Wake::At(t) => clock.set(t),
            Wake::Now => {}
            Wake::Idle | Wake::IdleFor(_) => return,
        }
    }
    panic!("not idle");
}

fn flushes(ui: &mut Ui, display: DisplayId) -> usize {
    ui.engine_mut()
        .driver_mut::<MemoryDisplay>(display)
        .unwrap()
        .take_flushes()
        .len()
}

// ---- two Uis, one runtime -------------------------------------------------------------------

/// Two UIs (two engines) on this thread's runtime. Their trees have the same shape, so a
/// binding of one applied to the other's engine would find a node with its id there — the
/// cross-talk this test catches: `a`'s bound label sits where `b`'s fixed label is and vice
/// versa.
struct TwoUis {
    a: Ui,
    b: Ui,
    clock: MockClock,
    shared: Signal<u32>,
    to_a: &'static Channel<u32, 4>,
    to_b: &'static Channel<u32, 4>,
}

fn two_uis() -> TwoUis {
    let rt = Runtime::current_thread();
    let clock = MockClock::new();
    // App state shared by both UIs, in a root of its own.
    let shared = rt.create_root().signal(0u32);
    let to_a: &'static Channel<u32, 4> = leak(Channel::new());
    let to_b: &'static Channel<u32, 4> = leak(Channel::new());
    let build = |name: &'static str, rx: &'static Channel<u32, 4>, bound_first: bool| {
        Ui::builder(panel(96, 48))
            .runtime(rt)
            .clock(clock.clone())
            .buffers(bufs())
            .build(move |cx| {
                cx.on_message(rx, move |v| shared.set(v));
                let bound = label(text!("{} {}", name, shared.get()));
                let fixed = label(format!("{name} fixed"));
                if bound_first {
                    column((bound, fixed)).into_any()
                } else {
                    column((fixed, bound)).into_any()
                }
            })
    };
    let a = build("a", to_a, false);
    let b = build("b", to_b, true);
    TwoUis {
        a,
        b,
        clock,
        shared,
        to_a,
        to_b,
    }
}

#[test]
fn regression_two_uis_on_one_runtime_never_write_into_each_other() {
    let mut t = two_uis();
    settle(&mut t.a, &t.clock);
    settle(&mut t.b, &t.clock);
    let (da, db) = (t.a.display(), t.b.display());
    assert_eq!(texts(t.a.engine(), da), ["a fixed", "a 0"]);
    assert_eq!(texts(t.b.engine(), db), ["b 0", "b fixed"]);

    // A's channel handler changes the shared signal during A's update.
    t.to_a.try_send(1).unwrap();
    t.a.update();
    assert_eq!(
        texts(t.a.engine(), da),
        ["a fixed", "a 1"],
        "B's binding wrote into A's engine"
    );
    assert_eq!(
        texts(t.b.engine(), db),
        ["b 0", "b fixed"],
        "B changes only in B's update"
    );
    assert!(
        t.b.root_scope().has_pending_effects(),
        "B's binding waits for B's update"
    );
    assert!(
        !t.a.root_scope().has_pending_effects(),
        "B's work does not keep A busy"
    );
    assert!(t.b.waker().take(), "B was woken to run it");
    t.b.update(); // B's own update runs it
    assert_eq!(texts(t.b.engine(), db), ["b 1", "b fixed"]);
    assert_eq!(texts(t.a.engine(), da), ["a fixed", "a 1"]);
    settle(&mut t.a, &t.clock);
    settle(&mut t.b, &t.clock);

    // And the other way round.
    t.to_b.try_send(2).unwrap();
    t.b.update();
    assert_eq!(texts(t.b.engine(), db), ["b 2", "b fixed"]);
    assert_eq!(
        texts(t.a.engine(), da),
        ["a fixed", "a 1"],
        "A's binding wrote into B's engine"
    );
    t.a.update();
    assert_eq!(texts(t.a.engine(), da), ["a fixed", "a 2"]);
    assert_eq!(texts(t.b.engine(), db), ["b 2", "b fixed"]);
}

#[test]
fn each_ui_drains_only_its_own_channels() {
    let mut t = two_uis();
    settle(&mut t.a, &t.clock);
    settle(&mut t.b, &t.clock);
    t.to_b.try_send(7).unwrap();
    assert!(t.b.waker().is_set(), "B's channel wakes B");
    assert!(!t.a.waker().is_set(), "… not A");
    t.a.update();
    assert_eq!(t.shared.get(), 0, "A's update did not run B's handler");
    assert!(t.to_b.len() == 1, "B's message still queued");
    t.b.update();
    assert_eq!(t.shared.get(), 7);
}

#[test]
fn write_between_updates_wakes_the_ui_whose_binding_waits() {
    let mut t = two_uis();
    settle(&mut t.a, &t.clock);
    settle(&mut t.b, &t.clock);
    // E.g. from a run-loop callback: no update is running, no engine is lent.
    t.shared.set(5);
    assert!(
        t.a.waker().is_set() && t.b.waker().is_set(),
        "both UIs have a binding to run"
    );
    settle(&mut t.a, &t.clock);
    settle(&mut t.b, &t.clock);
    assert_eq!(texts(t.a.engine(), t.a.display()), ["a fixed", "a 5"]);
    assert_eq!(texts(t.b.engine(), t.b.display()), ["b 5", "b fixed"]);
}

// ---- one Ui, two displays -------------------------------------------------------------------

struct Dashboard {
    ui: Ui,
    clock: MockClock,
    main: DisplayId,
    aux: DisplayId,
    speed: Signal<u32>,
    main_only: Signal<u32>,
    aux_only: Signal<u32>,
    main_taps: Rc<Cell<u32>>,
    aux_taps: Rc<Cell<u32>>,
    main_touch: MockPointer,
    aux_touch: MockPointer,
}

fn dashboard() -> Dashboard {
    let rt = Runtime::current_thread();
    let clock = MockClock::new();
    let state = rt.create_root();
    let (speed, main_only, aux_only) = (state.signal(0u32), state.signal(0u32), state.signal(0u32));
    let (main_taps, aux_taps) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (main_touch, aux_touch) = (MockPointer::new(), MockPointer::new());
    // Interrupt-driven panels: read after a wake-up, so an idle dashboard sleeps.
    main_touch.set_poll_hint(PollHint::Interrupt);
    aux_touch.set_poll_hint(PollHint::Interrupt);
    let (mt, at) = (main_taps.clone(), aux_taps.clone());
    let ui = Ui::builder(panel(128, 128))
        .runtime(rt)
        .clock(clock.clone())
        .buffers(bufs())
        .theme(DefaultTheme::light())
        .input(main_touch.clone())
        .display(
            DisplayBuilder::new(panel(96, 112))
                .buffers(bufs())
                .theme(DefaultTheme::light())
                .input(aux_touch.clone()),
            move |_| {
                column((
                    label(text!("aux {}", speed.get())),
                    label(text!("only {}", aux_only.get())),
                    button(label("tap"))
                        .on_click(move || at.set(at.get() + 1))
                        .test_id("aux-tap"),
                ))
            },
        )
        .build(move |_| {
            column((
                label(text!("main {}", speed.get())),
                label(text!("only {}", main_only.get())),
                button(label("tap"))
                    .on_click(move || mt.set(mt.get() + 1))
                    .test_id("main-tap"),
            ))
        });
    let mut displays = ui.displays();
    let (main, aux) = (displays.next().unwrap(), displays.next().unwrap());
    drop(displays);
    Dashboard {
        ui,
        clock,
        main,
        aux,
        speed,
        main_only,
        aux_only,
        main_taps,
        aux_taps,
        main_touch,
        aux_touch,
    }
}

impl Dashboard {
    fn settle(&mut self) {
        settle(&mut self.ui, &self.clock);
    }

    /// Taps at `p` on one display's touch panel: press, wait, release.
    fn tap(&mut self, touch: &MockPointer, p: Point) {
        touch.press(p);
        self.ui.notify_input();
        // A pressed panel is read continuously: step a while instead of settling.
        for _ in 0..4 {
            self.clock.advance(Duration::ms(20));
            self.ui.update();
        }
        touch.release();
        self.ui.notify_input();
        self.clock.advance(Duration::ms(40));
        self.settle();
    }
}

#[test]
fn two_displays_build_and_shared_signal_updates_both() {
    let mut d = dashboard();
    d.settle();
    assert_eq!(d.ui.displays().count(), 2);
    assert_eq!(texts(d.ui.engine(), d.main), ["main 0", "only 0", "tap"]);
    assert_eq!(texts(d.ui.engine(), d.aux), ["aux 0", "only 0", "tap"]);
    d.speed.set(88);
    d.settle();
    assert_eq!(texts(d.ui.engine(), d.main)[0], "main 88");
    assert_eq!(texts(d.ui.engine(), d.aux)[0], "aux 88");
}

#[test]
fn two_displays_have_independent_dirty_areas() {
    let mut d = dashboard();
    d.settle();
    let (main, aux) = (d.main, d.aux);
    flushes(&mut d.ui, main);
    flushes(&mut d.ui, aux);

    d.main_only.set(1);
    d.settle();
    assert!(
        flushes(&mut d.ui, main) > 0,
        "the main display redraws its change"
    );
    assert_eq!(
        flushes(&mut d.ui, aux),
        0,
        "the auxiliary display has nothing to redraw"
    );

    d.aux_only.set(1);
    d.settle();
    assert_eq!(flushes(&mut d.ui, main), 0);
    assert!(flushes(&mut d.ui, aux) > 0);

    d.speed.set(1);
    d.settle();
    assert!(
        flushes(&mut d.ui, main) > 0 && flushes(&mut d.ui, aux) > 0,
        "both show the shared value"
    );
}

#[test]
fn two_displays_have_independent_inputs() {
    let mut d = dashboard();
    d.settle();
    // Each panel reports its own display's coordinates.
    let (aux_touch, main_touch) = (d.aux_touch.clone(), d.main_touch.clone());
    let aux_button = center_of(d.ui.engine(), d.aux, "aux-tap");
    let main_button = center_of(d.ui.engine(), d.main, "main-tap");
    d.tap(&aux_touch, aux_button);
    assert_eq!(
        (d.main_taps.get(), d.aux_taps.get()),
        (0, 1),
        "the aux panel taps the aux button"
    );
    d.tap(&main_touch, main_button);
    assert_eq!(
        (d.main_taps.get(), d.aux_taps.get()),
        (1, 1),
        "the main panel taps the main button"
    );
    // The aux button's position on the main panel hits nothing of the aux display.
    d.tap(&main_touch, aux_button);
    assert_eq!(
        d.aux_taps.get(),
        1,
        "the main panel never reaches the aux display"
    );
}

#[test]
fn per_display_theme_mode_and_commands() {
    static AUX_CMDS: Channel<DisplayCmd, 4> = Channel::new();
    let clock = MockClock::new();
    let mut ui = Ui::builder(panel(96, 48).with_power_control())
        .runtime(Runtime::current_thread())
        .clock(clock.clone())
        .buffers(bufs())
        .theme(DefaultTheme::light())
        .display(
            DisplayBuilder::new(panel(64, 32).with_power_control())
                .buffers(bufs())
                .theme(DefaultTheme::light())
                .display_commands(&AUX_CMDS),
            |_| label("aux"),
        )
        .build(|_| label("main"));
    settle(&mut ui, &clock);
    let main = ui.display();
    let aux = ui.displays().nth(1).unwrap();
    let aux_scope = ui.display_mut(aux).unwrap().scope();
    assert_ne!(aux_scope, ui.root_scope());
    assert_eq!(
        ui.root_scope().runtime().batch(|| use_theme(aux_scope).mode()),
        ThemeMode::Light
    );

    // Theme mode on the aux display only; its application sees it, the main one does not.
    ui.display_mut(aux).unwrap().set_theme_mode(ThemeMode::Dark);
    settle(&mut ui, &clock);
    assert_eq!(ui.engine().theme_mode(aux), ThemeMode::Dark);
    assert_eq!(ui.engine().theme_mode(main), ThemeMode::Light);
    assert_eq!(use_theme(aux_scope).mode(), ThemeMode::Dark);
    assert_eq!(use_theme(ui.root_scope()).mode(), ThemeMode::Light);

    // The aux display's handle switches only its own display.
    use_theme(aux_scope).set_mode(ThemeMode::Light);
    use_theme(ui.root_scope()).set_mode(ThemeMode::Dark);
    settle(&mut ui, &clock);
    assert_eq!(ui.engine().theme_mode(aux), ThemeMode::Light);
    assert_eq!(ui.engine().theme_mode(main), ThemeMode::Dark);

    // Display commands on the aux channel reach only the aux display.
    AUX_CMDS.try_send(DisplayCmd::Sleep).unwrap();
    assert!(ui.waker().is_set(), "the aux display's channel wakes the Ui");
    settle(&mut ui, &clock);
    assert!(ui.display_mut(aux).unwrap().asleep());
    assert!(!ui.display_asleep());
    ui.display_mut(aux).unwrap().set_brightness(Fraction::pct(30));
    settle(&mut ui, &clock);
    assert_eq!(ui.engine().display_brightness(aux), Some(Fraction::pct(30)));
    assert_eq!(ui.engine().display_brightness(main), None);
}

#[test]
fn one_waker_for_every_display_of_a_ui() {
    let ch: &'static Channel<u8, 2> = leak(Channel::new());
    let got = Rc::new(Cell::new(0u8));
    let g = got.clone();
    let clock = MockClock::new();
    let mut ui = Ui::builder(panel(96, 48))
        .runtime(Runtime::current_thread())
        .clock(clock.clone())
        .buffers(bufs())
        .build(|_| label("main"));
    ui.mount_on(DisplayBuilder::new(panel(64, 32)).buffers(bufs()), move |cx| {
        cx.on_message(ch, move |v| g.set(v));
        label("aux")
    })
    .unwrap();
    settle(&mut ui, &clock);
    assert_eq!(
        ui.root_scope().ui_waker().map(std::ptr::from_ref),
        Some(std::ptr::from_ref(ui.waker()))
    );
    ch.try_send(3).unwrap();
    assert!(
        ui.waker().is_set(),
        "a handler of the second display's application wakes the Ui"
    );
    settle(&mut ui, &clock);
    assert_eq!(got.get(), 3);
}

#[test]
fn ui_core_mount_on_refuses_unknown_and_taken_displays() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();
    let main = engine.add_display(panel(64, 32), bufs()).unwrap();
    let mut core = UiCore::mount(Runtime::current_thread(), &mut engine, main, |_| label("main")).unwrap();
    let taken = core.mount_on(&mut engine, main, |_| label("again"));
    assert!(matches!(
        taken,
        Err(UiError::Engine(EngineError::InvalidConfig(_)))
    ));
    // A display id this engine does not have (the second display of another engine).
    let mut other = Engine::new(EngineConfig::default()).unwrap();
    other.add_display(panel(8, 8), bufs()).unwrap();
    let unknown = other.add_display(panel(8, 8), bufs()).unwrap();
    assert!(matches!(
        core.mount_on(&mut engine, unknown, |_| label("x")),
        Err(UiError::Engine(EngineError::DisplayNotFound(_)))
    ));
    let aux = engine.add_display(panel(32, 16), bufs()).unwrap();
    let scope = core.mount_on(&mut engine, aux, |_| label("aux")).unwrap();
    assert_eq!(core.display_scope(aux), Some(scope));
    assert_eq!(core.display_scope(main), Some(core.root_scope()));
    core.dispose(&mut engine);
    assert!(
        !scope.is_alive(),
        "the further display's application goes with the UI"
    );
}

#[test]
fn blocking_run_loop_drives_both_displays() {
    struct Stop;
    let mut platform = MockPlatform::new();
    let rt = Runtime::current_thread();
    let n = rt.create_root().signal(0u32);
    let ui = Ui::builder(panel(96, 48))
        .runtime(rt)
        .platform(&platform)
        .buffers(bufs())
        .display(DisplayBuilder::new(panel(64, 32)).buffers(bufs()), move |_| {
            label(text!("aux {}", n.get()))
        })
        .build(move |_| label(text!("main {}", n.get())));
    let mut seen = Vec::new();
    let result = catch_unwind(AssertUnwindSafe(|| -> () {
        run::blocking_with(ui, &mut platform, |ui, event| {
            if let LoopEvent::Idle(_) = event {
                let displays: Vec<DisplayId> = ui.displays().collect();
                seen.push(
                    displays
                        .iter()
                        .map(|&d| texts(ui.engine(), d)[0].clone())
                        .collect::<Vec<_>>(),
                );
                if seen.len() == 2 {
                    resume_unwind(Box::new(Stop));
                }
                // A change made between updates wakes the Ui by itself (its bindings defer).
                n.set(1);
            }
        })
    }));
    match result {
        Err(p) if p.is::<Stop>() => {}
        Err(p) => resume_unwind(p),
        Ok(()) => unreachable!(),
    }
    assert_eq!(seen, [vec!["main 0", "aux 0"], vec!["main 1", "aux 1"]]);
}

#[test]
fn two_displays_steady_state_allocates_nothing() {
    let mut d = dashboard();
    d.settle();
    // Warm-up: size the label texts' buffers.
    for v in [100_000, 100_001] {
        d.speed.set(v);
        d.main_only.set(v);
        d.aux_only.set(v);
        d.settle();
    }
    // The test panels record every flush: drain the records into a buffer sized up front, so
    // only the Ui's own allocations are counted.
    let (main, aux) = (d.main, d.aux);
    let mut records = Vec::with_capacity(4096);
    let ((), stats) = count_allocs(|| {
        for v in 100_002..100_012 {
            d.speed.set(v);
            d.aux_only.set(v);
            d.settle();
            for display in [main, aux] {
                records.clear();
                d.ui.engine_mut()
                    .driver_mut::<MemoryDisplay>(display)
                    .unwrap()
                    .drain_flushes_into(&mut records);
            }
        }
    });
    assert_eq!((stats.allocs, stats.reallocs), (0, 0));
    assert_eq!(texts(d.ui.engine(), d.aux)[0], "aux 100011");
}
