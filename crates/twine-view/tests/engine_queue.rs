//! The per-`Ui` engine command queue (R1 review rework F1): engine side effects issued without
//! the engine — the cleanups of a scope disposed outside `Ui::update`, `AnimController` and
//! `ThemeHandle` calls between updates — are applied at the start of the next update instead
//! of being lost; bounded, allocation-free, per `Ui`.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::ColorFormat;
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{EngineHarness, MemoryDisplay, MockClock, TestUi};
use twine_view::prelude::*;
use twine_view::{CapacityFault, EngineAccess, UiCore};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Mounts `app` with a child scope of the root handed to it (to be disposed by the test, outside
/// any update) and runs one update; returns the UI and the child scope.
fn mount_with_child<V: View>(app: impl FnOnce(Scope) -> V + 'static) -> (TestUi, Scope) {
    let child: Rc<Cell<Option<Scope>>> = Rc::default();
    let c2 = child.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let sub = cx.child();
        c2.set(Some(sub));
        app(sub)
    });
    t.update();
    (t, child.get().unwrap())
}

fn forever() -> AnimSpec {
    AnimSpec::new(Duration::ms(200)).forever()
}

/// The animation of a scope disposed outside `Ui::update` used to keep running forever (its
/// cleanup's `anim_stop` found no engine): the UI never went idle.
#[test]
fn regression_repeating_animation_stops_when_its_scope_is_disposed_outside_update() {
    let (mut t, sub) = mount_with_child(|cx| {
        let (x, _) = cx.animation(0, 100, forever());
        container(()).width(x)
    });
    t.advance(Duration::ms(50));
    assert_eq!(t.engine().anim_count(), 1);
    sub.dispose(); // no engine lent: queued
    assert_eq!(
        t.engine().anim_count(),
        1,
        "applied at the next update, not before"
    );
    t.update();
    assert_eq!(t.engine().anim_count(), 0);
    t.run_until_idle();
    assert_eq!(t.update(), Wake::Idle);
}

#[test]
fn regression_tween_stops_when_its_scope_is_disposed_outside_update() {
    let open: Rc<Cell<Option<Signal<bool>>>> = Rc::default();
    let o2 = open.clone();
    let (mut t, sub) = mount_with_child(move |cx| {
        let open = cx.signal(false);
        o2.set(Some(open));
        let h = cx.tween(move || if open.get() { 90 } else { 10 }, Duration::secs(10));
        container(()).height(h)
    });
    open.get().unwrap().set(true);
    t.update();
    assert_eq!(t.engine().anim_count(), 1);
    sub.dispose();
    t.update();
    assert_eq!(t.engine().anim_count(), 0);
    assert_eq!(t.update(), Wake::Idle);
}

#[test]
fn regression_timers_removed_when_their_scope_is_disposed_outside_update() {
    let fired = Rc::new(Cell::new(0u32));
    let (f1, f2) = (fired.clone(), fired.clone());
    let (mut t, sub) = mount_with_child(move |cx| {
        cx.interval(Duration::ms(100), move || f1.set(f1.get() + 1));
        cx.timeout(Duration::secs(5), move || f2.set(f2.get() + 100));
        label("timers")
    });
    assert_eq!(t.engine().timer_count(), 2);
    t.advance(Duration::ms(250));
    let before = fired.get();
    assert!(before >= 2, "the interval ran: {before}");
    sub.dispose();
    assert_eq!(t.engine().timer_count(), 2);
    t.update();
    assert_eq!(t.engine().timer_count(), 0);
    t.run_until_idle();
    assert_eq!(fired.get(), before, "neither timer ran after the disposal");
    assert_eq!(t.update(), Wake::Idle);
}

#[test]
fn regression_modal_backdrop_removed_when_its_scope_is_disposed_outside_update() {
    let (mut t, sub) = mount_with_child(|cx| {
        let _ = cx.show_modal(|_, _| label("modal"));
        label("page")
    });
    t.run_until_idle();
    let top = {
        let e = t.engine();
        e.top_layer(e.default_display().unwrap()).unwrap()
    };
    assert_eq!(t.engine().tree().children(top).count(), 1, "backdrop shown");
    let group_before = t.engine().default_group();
    sub.dispose();
    assert_eq!(t.engine().tree().children(top).count(), 1);
    t.update();
    assert_eq!(t.engine().tree().children(top).count(), 0, "backdrop removed");
    assert_eq!(t.engine().default_group(), group_before, "focus group restored");
    t.run_until_idle();
}

#[test]
fn controller_and_theme_calls_between_updates_take_effect_at_the_next_update() {
    let ctl: Rc<Cell<Option<AnimController>>> = Rc::default();
    let c2 = ctl.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let (x, ctl) = cx.animation(0, 100, forever());
        c2.set(Some(ctl));
        container(()).width(x)
    });
    t.update();
    let ctl = ctl.get().unwrap();
    let playing = |t: &mut TestUi| EngineAccess::provide(t.engine_mut(), || ctl.is_playing());
    assert!(playing(&mut t));

    ctl.pause();
    assert!(playing(&mut t), "not before the next update");
    t.update();
    assert!(!playing(&mut t));
    assert_eq!(t.update(), Wake::Idle, "a paused animation lets the UI idle");

    ctl.set_playing(true);
    t.update();
    assert!(playing(&mut t));
    ctl.stop();
    t.update();
    assert!(!playing(&mut t));
    assert_eq!(t.engine().anim_count(), 0);
    ctl.restart();
    t.update();
    assert!(playing(&mut t));

    let d = t.engine().default_display().unwrap();
    let mode = |t: &TestUi| t.engine().theme_mode(d);
    assert_eq!(mode(&t), ThemeMode::Light);
    use_theme(t.root_scope()).set(DefaultTheme::dark());
    assert_eq!(mode(&t), ThemeMode::Light);
    t.update();
    assert_eq!(mode(&t), ThemeMode::Dark);
}

/// An animation created and then paused between updates: the pause is applied after the
/// deferred start (call order is kept).
#[test]
fn animation_created_then_paused_between_updates_stays_paused() {
    let mut t = TestUi::new(200, 100).mount(|_| label("x"));
    t.run_until_idle();
    let (x, ctl) = t.root_scope().animation(0, 100, forever());
    ctl.pause();
    t.update();
    assert!(!EngineAccess::provide(t.engine_mut(), || ctl.is_playing()));
    assert_eq!(t.engine().anim_count(), 1, "started, then paused");
    t.advance(Duration::ms(100));
    assert_eq!(x.get_untracked(), 0);
}

/// A call made while an enclosing `EngineAccess::with` holds the engine (inside an update) is
/// queued too, and the update asks to run again at once.
#[test]
fn call_while_the_engine_is_borrowed_runs_at_the_next_update() {
    let ctl: Rc<Cell<Option<AnimController>>> = Rc::default();
    let c2 = ctl.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let (x, ctl) = cx.animation(0, 100, forever());
        c2.set(Some(ctl));
        column((
            container(()).width(x),
            button(label("p")).on_click(move || {
                EngineAccess::with(|_e| ctl.pause());
            }),
        ))
    });
    t.update();
    let ctl = ctl.get().unwrap();
    t.find(twine_testing::by_text("p")).click();
    t.run_until_idle(); // idles only once the queued pause was applied
    assert!(!EngineAccess::provide(t.engine_mut(), || ctl.is_playing()));
}

#[test]
fn queue_overflow_raises_a_fault_and_applies_the_rest() {
    let ctls: Rc<Cell<Option<[AnimController; 3]>>> = Rc::default();
    let c2 = ctls.clone();
    let clock = MockClock::new();
    let mut ui = Ui::builder(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
        .clock(clock.clone())
        .engine_queue_capacity(2)
        .try_build(move |cx| {
            let (_, a) = cx.animation(0, 100, forever());
            let (_, b) = cx.animation(0, 100, forever());
            let (_, c) = cx.animation(0, 100, forever());
            c2.set(Some([a, b, c]));
            label("x")
        })
        .unwrap();
    ui.update();
    let _ = ui.take_faults();
    let [a, b, c] = ctls.get().unwrap();
    a.pause();
    b.pause();
    c.pause(); // dropped: the queue holds two commands
    ui.update();
    assert!(ui.take_faults().contains(FaultKind::Capacity));
    let rec = ui.last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(
        CapacityFault::from_code(rec.code),
        Some(CapacityFault::EngineQueue)
    );
    assert_eq!(rec.occurrences, 1);
    assert_eq!(rec.display, Some(ui.display()));
    let playing = |ui: &mut Ui, k: AnimController| EngineAccess::provide(ui.engine_mut(), || k.is_playing());
    assert!(!playing(&mut ui, a));
    assert!(!playing(&mut ui, b));
    assert!(playing(&mut ui, c), "the dropped command was not applied");
    // Reported once.
    ui.update();
    assert!(!ui.take_faults().contains(FaultKind::Capacity));
}

#[test]
fn commands_go_to_the_ui_that_owns_the_scope() {
    let (mut a, sub_a) = mount_with_child(|cx| {
        let (x, _) = cx.animation(0, 100, forever());
        container(()).width(x)
    });
    let (mut b, _sub_b) = mount_with_child(|cx| {
        let (x, _) = cx.animation(0, 100, forever());
        container(()).width(x)
    });
    sub_a.dispose();
    use_theme(a.root_scope()).set(DefaultTheme::dark());
    b.update();
    assert_eq!(b.engine().anim_count(), 1, "B keeps its animation");
    assert_eq!(a.engine().anim_count(), 1, "A's command waits for A's update");
    a.update();
    assert_eq!(a.engine().anim_count(), 0);
    assert_eq!(b.engine().anim_count(), 1);
    let mode = |t: &TestUi| {
        let e = t.engine();
        e.theme_mode(e.default_display().unwrap())
    };
    assert_eq!(mode(&a), ThemeMode::Dark);
    assert_eq!(mode(&b), ThemeMode::Light);
}

#[test]
fn queuing_allocates_nothing() {
    let ctl: Rc<Cell<Option<AnimController>>> = Rc::default();
    let c2 = ctl.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let (x, ctl) = cx.animation(0, 100, forever());
        c2.set(Some(ctl));
        container(()).width(x)
    });
    t.update();
    let ctl = ctl.get().unwrap();
    let theme = use_theme(t.root_scope());
    let dark: Rc<dyn twine_engine::ThemeHook> = Rc::new(DefaultTheme::dark());
    let ((), stats) = count_allocs(|| {
        for _ in 0..4 {
            ctl.pause();
            ctl.resume();
            ctl.set_playing(false);
        }
        theme.set_rc(dark.clone());
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
    t.update();
    assert!(!EngineAccess::provide(t.engine_mut(), || ctl.is_playing()));
}

/// A queued stop of an animation that already ended (or a removal of a timer that already
/// fired) does not touch the animation or timer that reused its slot.
#[test]
fn stale_ids_are_ignored() {
    let (mut t, sub) = mount_with_child(|cx| {
        let (x, _) = cx.animation(0, 10, AnimSpec::new(Duration::ms(50)));
        cx.timeout(Duration::ms(10), || {});
        container(()).width(x)
    });
    t.run_until_idle();
    assert_eq!((t.engine().anim_count(), t.engine().timer_count()), (0, 0));
    // New ones, likely in the freed slots.
    let (_, other) = t.root_scope().animation(0, 100, forever());
    t.root_scope().interval(Duration::ms(100), || {});
    t.update();
    assert_eq!((t.engine().anim_count(), t.engine().timer_count()), (1, 1));
    sub.dispose(); // queues a stop and a removal with stale ids
    t.update();
    assert_eq!((t.engine().anim_count(), t.engine().timer_count()), (1, 1));
    assert!(EngineAccess::provide(t.engine_mut(), || other.is_playing()));
}

#[test]
fn ui_core_dispose_applies_queued_commands_and_cleanups() {
    let mut h = EngineHarness::new(100, 60);
    let d = h.display();
    let child: Rc<Cell<Option<Scope>>> = Rc::default();
    let c2 = child.clone();
    let core = UiCore::mount(h.engine_mut(), d, move |cx| {
        let sub = cx.child();
        c2.set(Some(sub));
        let (x, _) = sub.animation(0, 100, forever());
        let (y, _) = cx.animation(0, 100, forever());
        cx.interval(Duration::ms(10), || {});
        container(()).width(x).height(y)
    })
    .unwrap();
    assert_eq!(h.engine().anim_count(), 2);
    child.get().unwrap().dispose(); // queued
    core.dispose(h.engine_mut()); // applies it, then disposes the rest with the engine
    assert_eq!(h.engine().anim_count(), 0);
    assert_eq!(h.engine().timer_count(), 0);
}
