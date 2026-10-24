//! Switching the language allocates nothing once the labels' text buffers have grown (a test
//! binary of its own: it installs the counting allocator).

mod i18n {
    include!("i18n/catalog.rs");
}

use std::cell::Cell;
use std::rc::Rc;

use i18n::{Key, Lang};
use twine_extra::i18n::{Decimal, I18n, provide_i18n};
use twine_extra::tr;
use twine_testing::TestUi;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_view::prelude::*;

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn switching_language_allocates_nothing_after_warm_up() {
    type Handles = (I18n<Lang>, Signal<u32>);
    let slot: Rc<Cell<Option<Handles>>> = Rc::default();
    let s = slot.clone();
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let i18n = provide_i18n(cx, Lang::En);
        let n = cx.signal(3u32);
        s.set(Some((i18n, n)));
        column((
            label(tr!(Key::Hello)),
            label(tr!(Key::Greeting, name = "Ada")),
            label(tr!(Key::Items, count = n.get())),
            label(tr!(Key::Moved, "a.txt", "/tmp")),
            label(tr!(Key::Total, amount = Decimal::new(-123_456_789, 2))),
        ))
    });
    t.run_until_idle();
    let (i18n, n) = slot.get().unwrap();
    // Warm-up: the switches below once, so each label's text buffer holds its longest text and
    // the engine's per-frame lists (invalidated areas of old and new texts) have grown.
    let switches = [
        Lang::De,
        Lang::Ar,
        Lang::Ru,
        Lang::Fa,
        Lang::Hi,
        Lang::Fr,
        Lang::En,
        Lang::Zh,
        Lang::Ar,
    ];
    for lang in switches {
        i18n.set_language(lang);
        t.run_until_idle();
    }
    // Only the switch is measured (the re-formatted texts; drawing right-to-left text with
    // `bidi` allocates the algorithm's level tables, see `twine_text::bidi`). Each switch is
    // then drawn outside the measurement: without frames, the engine's invalidation log
    // (feature `debug-checks`, which other crates' dev-dependencies enable in
    // `cargo test --workspace`) would keep growing across the switches.
    for lang in switches {
        let ((), stats) = count_allocs(|| {
            i18n.set_language(lang);
            t.update();
        });
        t.run_until_idle();
        assert_eq!(
            (stats.allocs, stats.reallocs),
            (0, 0),
            "switch to {lang:?}: {stats:?}"
        );
    }
    // A count change re-formats in place too.
    let ((), stats) = count_allocs(|| {
        n.set(4);
        t.update();
    });
    assert_eq!((stats.allocs, stats.reallocs), (0, 0), "count change: {stats:?}");
}
