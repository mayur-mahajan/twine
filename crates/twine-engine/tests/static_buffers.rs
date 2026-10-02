//! Draw buffers (R3.S03): type-aligned `DrawBuffer` statics are accepted, caller memory is
//! checked (alignment, size) with clear errors, heap buffers (`BufferMode::Alloc`) are sized
//! for the display, and a refused display allocates nothing (rework F10b).

use twine_core::{ColorFormat, Rect};
use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, EngineError};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBuffer, DrawBufferMem, buffer_bytes};
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{MemoryDisplay, MockFramebufferDisplay};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const W: u16 = 64;
const H: u16 = 32;
const BYTES: usize = buffer_bytes(W, 8, ColorFormat::Rgb565);

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).unwrap()
}

fn panel() -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565))
}

/// What `draw_buffers!` hands out (leaked here: the macro lives in `twine-view`).
fn statics<const N: usize>() -> &'static mut [DrawBuffer<BYTES>; N] {
    Box::leak(Box::new([DrawBuffer::ZEROED; N]))
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

#[test]
fn typed_statics_are_accepted_and_render_in_chunks() {
    let mut e = engine();
    let d = e
        .add_display(panel(), BufferMode::partial_double_from(statics::<2>()))
        .unwrap();
    e.step(twine_core::Instant::from_millis(0));
    assert_eq!(e.last_stats(d).chunks, 4, "32 rows in 8-row chunks");
    let mut e = engine();
    e.add_display(panel(), BufferMode::partial_single_from(statics::<1>()))
        .unwrap();
}

#[test]
fn misaligned_caller_memory_is_refused_with_a_clear_error() {
    let mem: &'static mut [u8] = Box::leak(vec![0u8; BYTES + 8].into_boxed_slice());
    let off = mem.as_ptr().align_offset(4) + 1;
    let mut e = engine();
    let err = e
        .add_display(panel(), BufferMode::partial_single(&mut mem[off..off + BYTES]))
        .unwrap_err();
    assert_eq!(err, EngineError::BufferMisaligned);
    let msg = err.to_string();
    assert!(
        msg.contains("4-byte aligned") && msg.contains("draw_buffers!"),
        "{msg}"
    );
    assert_eq!(e.default_display(), None, "nothing was added");
}

#[test]
fn too_small_caller_memory_is_refused_with_sizes() {
    let mut e = engine();
    let err = e
        .add_display(
            panel(),
            BufferMode::partial_single(&mut statics::<1>()[0].as_mut_slice()[..10]),
        )
        .unwrap_err();
    assert_eq!(err, EngineError::BufferTooSmall { needed: 128, got: 10 });
    assert!(err.to_string().contains("10 bytes, 128 needed"), "{err}");
}

#[test]
fn modes_must_match_the_display_kind() {
    let mut e = engine();
    assert_eq!(
        e.add_display(panel(), BufferMode::Full).unwrap_err(),
        EngineError::BufferModeMismatch
    );
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let fb = MockFramebufferDisplay::new(info, true, 0);
    assert_eq!(
        e.add_framebuffer_display(fb, BufferMode::alloc(BufferSpec::default()))
            .unwrap_err(),
        EngineError::BufferModeMismatch
    );
}

#[test]
fn alloc_sizes_the_buffers_for_the_display() {
    let mut e = engine();
    let (d, stats) = count_allocs(|| {
        e.add_display(panel(), BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 }))
            .unwrap()
    });
    assert!(stats.bytes >= 2 * BYTES as u64, "{stats:?}");
    e.step(twine_core::Instant::from_millis(0));
    assert_eq!(e.last_stats(d).chunks, 4);
    // Rows are clamped to the display: one buffer of the whole screen.
    let mut e = engine();
    let d = e
        .add_display(
            panel(),
            BufferMode::alloc(BufferSpec::PartialSingle { rows: 500 }),
        )
        .unwrap();
    e.step(twine_core::Instant::from_millis(0));
    assert_eq!(e.last_stats(d).chunks, 1);
}

/// F10b: the display is checked before the heap buffers are allocated.
#[test]
fn regression_refused_display_allocates_no_buffers() {
    let spec = BufferMode::alloc(BufferSpec::PartialDouble { rows: 40 });
    // A format whose renderer is never compiled in (`A8` is not a draw format).
    let mut e = engine();
    let a8 = NullPanel(DisplayInfo::new(320, 240, ColorFormat::A8), None);
    let (r, stats) = count_allocs(|| e.add_display(a8, spec));
    assert_eq!(r.unwrap_err(), EngineError::FormatDisabled(ColorFormat::A8));
    assert!(stats.bytes < 1024, "a refused display allocated {stats:?}");
    assert_eq!(stats.live, 0, "a refused display leaked {stats:?}");

    // A display whose row alignment exceeds its height.
    let tall_align = NullPanel(DisplayInfo::new(320, 4, ColorFormat::Rgb565).with_align(8), None);
    let (r, stats) = count_allocs(|| e.add_display(tall_align, BufferMode::alloc(BufferSpec::default())));
    assert!(matches!(r, Err(EngineError::BufferTooSmall { .. })), "{r:?}");
    assert_eq!((stats.allocs, stats.live), (0, 0), "{stats:?}");

    // Too many displays.
    let mut e = engine();
    for _ in 0..twine_engine::MAX_DISPLAYS {
        e.add_display(panel(), BufferMode::alloc(BufferSpec::PartialSingle { rows: 1 }))
            .unwrap();
    }
    let fifth = panel();
    let (r, stats) = count_allocs(|| e.add_display(fifth, BufferMode::alloc(BufferSpec::default())));
    assert_eq!(r.unwrap_err(), EngineError::TooManyDisplays);
    // (The refused driver itself is dropped: `deallocs` may be non-zero.)
    assert_eq!((stats.allocs, stats.bytes), (0, 0), "{stats:?}");
}

/// F10b: a node budget without room for the display's 4 layer/screen nodes refuses the display
/// before its heap buffers are allocated (they used to be allocated, then leaked when the
/// layer nodes could not be created).
#[test]
fn regression_node_budget_refuses_display_before_allocating() {
    let attempt = |rows: u16| {
        let mut e = Engine::new(EngineConfig {
            max_nodes: 3,
            ..EngineConfig::default()
        })
        .unwrap();
        let p = panel();
        let (r, stats) =
            count_allocs(|| e.add_display(p, BufferMode::alloc(BufferSpec::PartialDouble { rows })));
        assert_eq!(r.unwrap_err(), EngineError::TooManyNodes);
        assert!(e.take_faults().contains(twine_core::fault::FaultKind::Capacity));
        assert_eq!(e.default_display(), None, "nothing was added");
        assert!(stats.live <= 0, "a refused display leaked {stats:?}");
        stats
    };
    // Allocations do not depend on the requested buffer size: no buffer was allocated.
    let (small, big) = (attempt(1), attempt(32));
    assert_eq!(small.bytes, big.bytes, "{small:?} vs {big:?}");

    // Exactly 4 free nodes are enough.
    let mut e = Engine::new(EngineConfig {
        max_nodes: 4,
        ..EngineConfig::default()
    })
    .unwrap();
    e.add_display(panel(), BufferMode::alloc(BufferSpec::default()))
        .unwrap();
}
