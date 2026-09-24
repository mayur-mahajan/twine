//! Label text storage and drawing allocations (this binary installs the counting allocator).

mod common;

use common::{Mode, harness, with};
use twine_core::Duration;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_widgets::label::{self, Label};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn label_set_text_reuses_capacity() {
    let mut h = harness(200, 60, Mode::Light);
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    with(&mut h, l, |w: &mut Label, cx| {
        w.set_text(cx, "a first, rather long text");
    });
    let ((), stats) = count_allocs(|| {
        with(&mut h, l, |w: &mut Label, cx| w.set_text(cx, "shorter text"));
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn label_static_text_no_alloc() {
    let mut h = harness(200, 60, Mode::Light);
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let ((), stats) = count_allocs(|| {
        with(&mut h, l, |w: &mut Label, cx| {
            w.set_text_static(cx, "a static text");
        });
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn write_text_swaps_buffers_without_allocating() {
    use core::fmt::Write;
    let mut h = harness(200, 60, Mode::Light);
    let screen = h.screen();
    let l = label::create(h.engine_mut(), screen).unwrap();
    for i in 0..3 {
        with(&mut h, l, |w: &mut Label, cx| {
            w.write_text(cx, |s| {
                let _ = write!(s, "value {i:04}");
            })
        });
    }
    let (changed, stats) = count_allocs(|| {
        with(&mut h, l, |w: &mut Label, cx| {
            w.write_text(cx, |s| {
                let _ = write!(s, "value {:04}", 7);
            })
        })
    });
    assert!(changed);
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    let (changed, _) = count_allocs(|| {
        with(&mut h, l, |w: &mut Label, cx| {
            w.write_text(cx, |s| {
                let _ = write!(s, "value {:04}", 7);
            })
        })
    });
    assert!(!changed, "same text: no change");
}

#[test]
fn rendering_a_long_label_allocates_nothing_after_the_first_frame() {
    let mut h = harness(240, 120, Mode::Light);
    let screen = h.screen();
    let l = label::create_with(
        h.engine_mut(),
        screen,
        "0123456789 abcdefghij klmnopqrst uvwxyzABCD EFGHIJKLMN OPQRSTUVWX YZ01234567 89abcdefgh ijklmnopqr stuvwxyzAB",
    )
    .unwrap();
    h.engine_mut().set_width(l, 200);
    h.run_until_idle();
    let ((), stats) = count_allocs(|| {
        for _ in 0..5 {
            h.engine_mut().invalidate_all();
            h.advance(Duration::ms(16));
        }
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
