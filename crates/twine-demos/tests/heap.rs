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
