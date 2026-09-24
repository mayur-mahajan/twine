//! "Work ∝ change": a change runs only its bindings, redraws only its pixels, allocates
//! nothing in steady state, and the UI is idle right after.

use twine_core::{Point, Rect};
use twine_engine::EngineConfig;
use twine_hal::Key;
use twine_reactive::debug_stats;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const IDS: [&str; 50] = [
    "l0", "l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9", "l10", "l11", "l12", "l13", "l14", "l15",
    "l16", "l17", "l18", "l19", "l20", "l21", "l22", "l23", "l24", "l25", "l26", "l27", "l28", "l29", "l30",
    "l31", "l32", "l33", "l34", "l35", "l36", "l37", "l38", "l39", "l40", "l41", "l42", "l43", "l44", "l45",
    "l46", "l47", "l48", "l49",
];

/// 50 labels bound to 50 signals (provided as `Vec<Signal<i32>>`).
fn fifty(cx: Scope) -> impl View {
    let signals: Vec<Signal<i32>> = (0..50).map(|i| cx.signal(i)).collect();
    cx.provide(signals.clone());
    let labels: Vec<_> = signals
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let s = *s;
            label(text!("{}", s.get())).test_id(IDS[i]).width(40)
        })
        .collect();
    flex(FlexFlow::RowWrap, labels)
        .size(Length::pct(100), Length::pct(100))
        .gap(2)
}

#[test]
fn signal_update_runs_one_binding() {
    let mut t = TestUi::new(320, 240).mount(fifty);
    t.run_until_idle();
    let signals = t.root_scope().expect_context::<Vec<Signal<i32>>>();
    signals[17].set(1000); // outside the Ui: deferred to the next update
    let runs = debug_stats().effect_runs;
    t.run_until_idle();
    assert_eq!(debug_stats().effect_runs - runs, 1);
    assert_eq!(t.find(by_id("l17")).text(), "1000");
}

#[test]
fn signal_update_invalidates_only_its_rect() {
    let mut t = TestUi::new(320, 240).mount(fifty);
    t.run_until_idle();
    let signals = t.root_scope().expect_context::<Vec<Signal<i32>>>();
    let id = t.find(by_id("l23")).id();
    let area = {
        let e = t.engine();
        let n = e.tree().node(id).unwrap();
        n.coords().expand(i32::from(n.ext_draw()))
    };
    signals[23].set(77);
    // One frame period later: this update runs the binding and renders the frame.
    let period = t.engine().config().refr_period;
    t.advance(period);
    let inv: Vec<Rect> = t.invalidations().iter().map(|(r, _)| *r).collect();
    assert!(!inv.is_empty());
    for r in &inv {
        assert!(
            area.contains_rect(r),
            "{r:?} is outside the label's area {area:?}"
        );
    }
    t.run_until_idle();
    assert!(t.last_frame().dirty_px <= area.area() as u32);
}

#[test]
fn idle_after_every_interaction() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let n = cx.signal(0);
        column((
            label(text!("{}", n.get())),
            button(label("inc")).on_click(move || n.update(|v| *v += 1)),
            scroll_view(
                Dir::VER,
                (0..20)
                    .map(|i| label(format!("row {i}")).height(20))
                    .collect::<Vec<_>>(),
            )
            .size(200, 100),
        ))
    });
    t.run_until_idle();
    t.find(by_text("inc")).click();
    t.run_until_idle();
    t.assert_idle();
    t.drag(Point::new(100, 200), Point::new(100, 120), Duration::ms(200));
    t.run_until_idle();
    t.assert_idle();
    t.key(Key::Next);
    t.run_until_idle();
    t.assert_idle();
    t.encoder(1);
    t.run_until_idle();
    t.assert_idle();
}

#[test]
fn no_alloc_steady_state_frames() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let (a, _ctl) = cx.animation(
            Anim::new(0, 3600)
                .duration(Duration::ms(1000))
                .repeat(Repeat::Infinite),
        );
        let v = cx.tween(move || a.get() / 36, Duration::ms(50), Easing::Linear);
        label(text!("{} %", v.get())).test_id("l")
    });
    let period = t.engine().config().refr_period;
    // Warm-up: the label's buffers, queues and caches.
    for _ in 0..30 {
        t.advance(period);
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..100 {
            t.advance(period);
        }
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
    let _ = EngineConfig::default();
}

#[test]
fn unchanged_value_no_work() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let n = cx.signal(5);
        cx.provide(n);
        label(text!("{}", n.get()))
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    let runs = debug_stats().effect_runs;
    n.set_if_changed(5);
    t.update();
    assert_eq!(debug_stats().effect_runs, runs, "no effect ran");
    assert!(t.invalidations().is_empty());
    t.assert_idle();
}

#[test]
fn counter_heap_usage() {
    let before = twine_testing::alloc::current();
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let count = cx.signal(0u32);
        column((
            label(text!("Clicked {} times", count.get())).test_id("count"),
            button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
            button(label("Reset"))
                .disabled(move || count.get() == 0)
                .on_click(move || count.set(0)),
        ))
        .gap(12)
        .padding(16)
    });
    let built = twine_testing::alloc::current();
    t.run_until_idle();
    // The app's own heap: widgets, nodes, styles, bindings (the engine's caches and the
    // harness's display buffers are measured separately by their crates).
    let app = built.live - before.live;
    println!("counter heap usage (engine + harness + app): {app} bytes");
    let empty = {
        let before = twine_testing::alloc::current();
        let t2 = TestUi::new(320, 240).mount(|_| column(()));
        let after = twine_testing::alloc::current();
        drop(t2);
        after.live - before.live
    };
    let counter_only = app - empty;
    println!("counter heap usage (app only): {counter_only} bytes");
    assert!(counter_only <= 8 * 1024, "{counter_only} bytes");
}
