//! Allocation checks of the text input widgets (this binary installs the counting allocator):
//! editing a textarea within its capacity, stepping a spinbox, redrawing a span group.

mod common;

use common::{Mode, harness, with};
use twine_core::Duration;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_widgets::textarea::{self, Textarea};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn editing_reuses_capacity() {
    let mut h = harness(240, 120, Mode::Light);
    let screen = h.screen();
    let ta = textarea::create(h.engine_mut(), screen).unwrap();
    with(&mut h, ta, |t: &mut Textarea, cx| {
        t.add_text(cx, "hello world, some text");
    });
    h.run_until_idle();
    // Warm up with the same edits (first use of every buffer, the engine's logs).
    for _ in 0..2 {
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.delete_char(cx);
            t.delete_char(cx);
            t.add_char(cx, 'x');
            t.add_char(cx, 't');
        });
        h.advance(Duration::ms(50));
    }
    let ((), stats) = count_allocs(|| {
        with(&mut h, ta, |t: &mut Textarea, cx| {
            t.delete_char(cx);
            t.delete_char(cx);
            t.add_char(cx, 'x');
            t.add_char(cx, 't');
        });
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert_eq!(textarea::text_of(h.engine(), ta), Some("hello world, some text"));
    // Drawing the result allocates nothing either.
    let ((), stats) = count_allocs(|| h.advance(Duration::ms(50)));
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn spinbox_no_alloc_on_increment() {
    use twine_widgets::spinbox::{self, Spinbox};
    let mut h = harness(240, 80, Mode::Light);
    let screen = h.screen();
    let s = spinbox::create(h.engine_mut(), screen).unwrap();
    // Warm up: the label's buffer, the engine's logs.
    for _ in 0..3 {
        with(&mut h, s, |w: &mut Spinbox, cx| w.increment(cx));
        h.advance(Duration::ms(50));
    }
    let ((), stats) = count_allocs(|| {
        with(&mut h, s, |w: &mut Spinbox, cx| {
            w.increment(cx);
            w.decrement(cx);
            w.increment(cx);
        });
        h.advance(Duration::ms(50));
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert_eq!(h.engine().widget::<Spinbox>(s).unwrap().value(), 4);
}

#[test]
fn span_redraw_no_alloc() {
    use twine_widgets::spangroup::{self, SpanGroup, SpanMode};
    let mut h = harness(240, 120, Mode::Light);
    let screen = h.screen();
    let g = spangroup::create(h.engine_mut(), screen).unwrap();
    with(&mut h, g, |w: &mut SpanGroup, cx| {
        for t in ["Hello, ", "rich ", "text that wraps over a few lines"] {
            let id = w.add_span(cx);
            w.set_span_text(cx, id, t);
        }
        w.set_mode(cx, SpanMode::Break);
    });
    h.engine_mut().set_width(g, 120);
    h.run_until_idle();
    h.render_full();
    // Redrawing with the cached layout allocates nothing.
    let ((), stats) = count_allocs(|| h.render_full());
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}
