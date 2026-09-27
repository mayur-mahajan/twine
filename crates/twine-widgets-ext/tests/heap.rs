//! Heap stability of the complex widgets (this binary installs the counting allocator):
//! opening and closing popups, switching pages and tabs many times leaves the heap and the
//! tree as they were.

mod common;

use common::{Mode, center, harness_with_group, with};
use twine_style::Align;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_widgets_ext::dropdown::{self, Dropdown};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn dropdown_open_close_100_heap_stable() {
    let mut h = harness_with_group(320, 240, Mode::Light);
    let screen = h.screen();
    let d = dropdown::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(d, Align::TopMid, 0, 10);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_options(cx, "One\nTwo\nThree\nFour");
    });
    h.run_until_idle();
    // Warm up: the engine's pools, the logs, the first list.
    for _ in 0..3 {
        h.tap(center(&h, d));
        h.run_until_idle();
        h.tap(center(&h, d));
        h.run_until_idle();
    }
    let nodes = h.engine().tree().len();
    let ((), stats) = count_allocs(|| {
        for _ in 0..100 {
            h.tap(center(&h, d));
            h.run_until_idle();
            h.tap(center(&h, d));
            h.run_until_idle();
        }
    });
    assert_eq!(h.engine().tree().len(), nodes);
    assert_eq!(stats.live, 0, "heap grew: {stats:?}");
}
