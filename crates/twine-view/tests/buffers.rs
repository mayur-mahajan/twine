//! Draw buffers of the `Ui` (R3.S03): `Ui::builder` needs explicit buffers (no implicit heap
//! buffer), `draw_buffers!` statics are taken once, and a refused display allocates nothing
//! (rework F10b).

use twine_core::{ColorFormat, Rect};
use twine_engine::EngineError;
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MockClock};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

draw_buffers! {
    /// Two 8-row buffers of a 64 px RGB565 panel.
    static BUFS: 2 x 8 rows x 64 px @ Rgb565;
}

fn panel() -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565))
}

/// A panel of any format that accepts every flush.
struct NullPanel(DisplayInfo, Option<DrawBufferMem>);

impl DisplayDriver for NullPanel {
    type Error = core::convert::Infallible;
    fn info(&self) -> DisplayInfo {
        self.0
    }
    fn begin_flush(&mut self, _area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        self.1 = Some(buf);
        Ok(())
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.1.take()
    }
}

// A `Ui::builder` without `.buffers(..)` does not compile (R3.S09): see the compile-fail
// doctests of `twine_view::typestate`.

#[test]
fn draw_buffers_statics_are_taken_once_and_render() {
    let bufs = BUFS.take().expect("first take");
    assert!(BUFS.take().is_none(), "second take");
    let mut ui = Ui::builder(panel())
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(BufferMode::partial_double_from(bufs))
        .build(|_| label("hi"));
    let _ = ui.update();
    let d = ui.display();
    assert_eq!(ui.engine().last_stats(d).chunks, 4, "32 rows in 8-row chunks");
}

/// F10b: the display is refused before any heap buffer is allocated, so nothing leaks.
#[test]
fn regression_refused_display_leaks_no_heap_buffers() {
    let attempt = |rows: u16| {
        let a8 = NullPanel(DisplayInfo::new(320, 240, ColorFormat::A8), None);
        let builder = Ui::builder(a8)
            .runtime(Runtime::current_thread())
            .clock(MockClock::new())
            .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows }));
        let (r, stats) = count_allocs(move || builder.try_build(|_| label("hi")).map(|_| ()));
        assert!(
            matches!(
                r,
                Err(UiError::Engine(EngineError::FormatDisabled(ColorFormat::A8)))
            ),
            "{r:?}"
        );
        assert!(stats.live <= 0, "a refused display leaked {stats:?}");
        stats
    };
    // The engine built (and dropped) meanwhile allocates the same whatever the buffer size:
    // no buffer was allocated.
    let (small, big) = (attempt(1), attempt(240));
    assert_eq!(small.bytes, big.bytes, "{small:?} vs {big:?}");
}
