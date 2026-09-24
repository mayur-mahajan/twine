//! The spinner animates without allocating (this binary installs the counting allocator).

mod common;

use common::{Mode, harness};
use twine_core::Duration;
use twine_style::Align;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_widgets::spinner;

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn spinner_no_alloc_per_frame() {
    let mut h = harness(160, 120, Mode::Light);
    let screen = h.screen();
    let e = h.engine_mut();
    let s = spinner::create(e, screen).unwrap();
    e.set_size(s, 60, 60);
    e.align(s, Align::Center, 0, 0);
    // Warm up (caches, dirty-area lists).
    h.advance(Duration::ms(1200));
    let ((), stats) = count_allocs(|| {
        for _ in 0..120 {
            h.advance(Duration::ms(16));
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
