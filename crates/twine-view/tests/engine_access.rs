//! `EngineAccess` diagnostics (R1.S09): calls that must return what the engine holds and run
//! without it return `None` / `false`, and in debug builds warn once per call site — without
//! allocating. Calls that are queued instead (F1, see `engine_queue.rs`) never warn.

use std::cell::Cell;
use std::rc::Rc;

use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{CapturedLog, TestUi, by_id, by_text, capture_logs};
use twine_view::prelude::*;
use twine_widgets::label::Label;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// The handles `setup` hands out.
type Handles = (NodeRef<Label>, AnimController);

/// A mounted UI with a filled `NodeRef` and an animation controller, handed out for use
/// outside `Ui::update`.
fn setup() -> (TestUi, NodeRef<Label>, AnimController) {
    let out: Rc<Cell<Option<Handles>>> = Rc::default();
    let o2 = out.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let r = cx.node_ref::<Label>();
        let (_, ctl) = cx.animation(0, 100, AnimSpec::new(Duration::ms(100)));
        o2.set(Some((r, ctl)));
        label("x").node_ref(r).test_id("l")
    });
    t.run_until_idle();
    let (r, ctl) = out.get().unwrap();
    (t, r, ctl)
}

fn engine_warnings(logs: &[CapturedLog]) -> Vec<&CapturedLog> {
    logs.iter()
        .filter(|l| l.level == log::Level::Warn && l.message.contains("without the engine"))
        .collect()
}

#[cfg(debug_assertions)]
#[test]
fn node_ref_warns_once_per_call_site() {
    let (t, r, _) = setup();
    let (results, logs) = capture_logs(|| {
        let mut results = Vec::new();
        for _ in 0..3 {
            // One call site, called three times.
            results.push(r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no")));
        }
        // A second call site.
        results.push(r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no")));
        results
    });
    // Behaviour unchanged: nothing ran.
    assert!(results.iter().all(Option::is_none));
    assert_eq!(t.find(by_id("l")).text(), "x");
    let warns = engine_warnings(&logs);
    assert_eq!(warns.len(), 2, "{logs:?}");
    for w in &warns {
        assert_eq!(w.target, "twine::view");
        assert!(w.message.contains("NodeRef::with_mut"), "{}", w.message);
        // The caller's location, not the library's.
        assert!(w.message.contains("engine_access.rs:"), "{}", w.message);
    }
    assert_ne!(warns[0].message, warns[1].message, "different lines");
}

#[cfg(debug_assertions)]
#[test]
fn handles_warn_once_per_call_site() {
    let (t, _, ctl) = setup();
    let theme = use_theme(t.root_scope());
    let ((), logs) = capture_logs(|| {
        for _ in 0..2 {
            // Queued for the next update (F1): no warning.
            ctl.pause();
            ctl.set_playing(true);
            theme.set(DefaultTheme::dark());
            // Needs the engine to answer.
            assert!(!ctl.is_playing());
        }
    });
    let warns = engine_warnings(&logs);
    assert_eq!(warns.len(), 1, "{logs:?}");
    assert!(
        warns[0].message.contains("AnimController::is_playing"),
        "{}",
        warns[0].message
    );
    assert!(
        warns[0].message.contains("engine_access.rs:"),
        "{}",
        warns[0].message
    );
}

#[test]
fn inside_the_ui_no_warning() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(200, 100).mount(|cx| {
            let r: NodeRef<Label> = cx.node_ref();
            column((
                label("before").node_ref(r).test_id("l"),
                button(label("poke")).on_click(move || {
                    assert_eq!(
                        r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "after")),
                        Some(())
                    );
                }),
            ))
        });
        t.run_until_idle();
        t.find(by_text("poke")).click();
        t.run_until_idle();
        assert_eq!(t.find(by_id("l")).text(), "after");
    });
    assert!(engine_warnings(&logs).is_empty(), "{logs:?}");
}

#[test]
fn repeated_diagnostic_allocates_nothing() {
    let (_t, r, ctl) = setup();
    let call = || {
        let n = r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no"));
        assert!(!ctl.is_playing());
        n
    };
    let _ = call(); // first time at these call sites: may log (and format)
    let (n, stats) = count_allocs(|| {
        let mut n = None;
        for _ in 0..100 {
            n = call();
        }
        n
    });
    assert!(n.is_none());
    assert_eq!(stats.allocs, 0, "{stats:?}");
}

#[cfg(not(debug_assertions))]
#[test]
fn release_builds_do_not_warn() {
    let (t, r, ctl) = setup();
    let ((), logs) = capture_logs(|| {
        assert!(r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no")).is_none());
        ctl.pause();
        assert!(!ctl.is_playing());
        use_theme(t.root_scope()).set(DefaultTheme::dark());
    });
    assert!(engine_warnings(&logs).is_empty(), "{logs:?}");
}

/// One call site shared by the threads of `regression_concurrent_uis_warn_once`.
#[cfg(debug_assertions)]
fn poke(r: NodeRef<Label>) -> Option<()> {
    r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no"))
}

/// Several `Ui`s on several threads hitting the same call site at once warn once in total
/// (the first version's load/store table lost entries under this race).
#[cfg(debug_assertions)]
#[test]
fn regression_concurrent_uis_warn_once() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let (_t, r, _) = setup();
                barrier.wait();
                let ((), logs) = capture_logs(|| {
                    for _ in 0..50 {
                        assert!(poke(r).is_none());
                    }
                });
                engine_warnings(&logs).len()
            })
        })
        .collect();
    let total: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
    assert_eq!(total, 1);
}
