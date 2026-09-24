//! Building and dropping themes frees everything (the transition descriptors are `static`,
//! nothing is leaked). This test binary installs the counting allocator.

use twine_core::Size;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_theme::{DefaultTheme, MonoTheme, SimpleTheme};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn theme_construction_memory_bounded() {
    let ((), stats) = count_allocs(|| {
        for i in 0..100u16 {
            let t = DefaultTheme::light().with_dpi(100 + i);
            // Build the style sets of two display classes.
            let _ = t.styles(130, Size::new(240, 160));
            let _ = t.styles(130, Size::new(800, 480));
            drop(t);
            drop(SimpleTheme::new());
            drop(MonoTheme::new(i % 2 == 0, &twine_assets::fonts::MONTSERRAT_14));
        }
    });
    assert!(stats.allocs > 0);
    // Every allocation was freed again: 0 live bytes (≤ 100 × 0 leaked descriptors).
    assert_eq!(stats.allocs, stats.deallocs, "{stats:?}");
}
