//! Heap used by the demos (counting allocator): the bytes still allocated after building the
//! app and rendering its first frame, minus an empty app's.

use twine::prelude::*;
use twine_testing::TestUi;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Live heap bytes after mounting `app` and running until idle (the `TestUi` included).
fn live<V: View>(app: impl FnOnce(Scope) -> V) -> i64 {
    let (t, stats) = count_allocs(|| {
        let mut t = TestUi::new(320, 240).mount(app);
        t.run_until_idle();
        t
    });
    drop(t);
    stats.live
}

#[test]
fn controls_heap() {
    let base = live(|_| container(()));
    let controls = live(twine_demos::controls::app_static) - base;
    let text_input = live(twine_demos::text_input::app) - base;
    eprintln!("heap: controls (static) {controls} B, text_input {text_input} B");
    assert!(controls < 64 * 1024, "{controls}");
    assert!(text_input < 64 * 1024, "{text_input}");
}

/// The heap high-water mark of the selection demo while every tab is visited (the pages of
/// the Lists and Tiles tabs are built while shown), above an empty app. `docs/perf.md`
/// records the number; the firmware sizes its heap from it (RP2040: 160 KiB with
/// `demo-selection`, the engine's own caches come on top).
#[test]
fn selection_heap_usage() {
    let (empty, base) = count_allocs(|| {
        let mut t = TestUi::new(320, 240).mount(|_| container(()));
        t.run_until_idle();
        t
    });
    drop(empty);
    let (t, stats) = count_allocs(|| {
        let mut t = TestUi::new(320, 240).mount(twine_demos::selection::app);
        t.run_until_idle();
        let s = t
            .root_scope()
            .expect_context::<twine_demos::selection::Selection>();
        for tab in [1, 2, 0, 1] {
            s.tab.set(tab);
            t.run_until_idle();
        }
        t
    });
    let nodes = t.engine().tree().len();
    drop(t);
    let peak = stats.peak - base.live;
    eprintln!("heap: selection high-water {peak} B above an empty app ({nodes} nodes on the Lists tab)");
    assert!(peak < 112 * 1024, "{peak}");
}
