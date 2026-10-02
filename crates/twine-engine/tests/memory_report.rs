//! `Engine::memory_report` against the heap the engine really takes (counting allocator), and
//! the static layer buffer (`Engine::with_layer_buf`): no heap for it, same pixels.

mod common;

use common::{boxed, white_screen};
use twine_core::{Color, ColorFormat, Instant, Opa, Rect};
use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, InvalidateReason, Obj};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_style::StyleProp;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::scenes::styled_box;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

const W: u16 = 100;
const H: u16 = 72;
const CHUNK: usize = W as usize * 2 * H as usize;

/// An opacity group (drawn through the layer buffer) over a white screen.
fn layered_scene(e: &mut Engine) {
    let s = white_screen(e);
    let g = styled_box(
        e,
        s,
        Rect::from_xywh(10, 10, 70, 50),
        &[StyleProp::Opacity(Opa::P50.into())],
    );
    boxed(e, g, Rect::from_xywh(10, 10, 45, 35), Color::RED);
    boxed(e, g, Rect::from_xywh(35, 25, 45, 35), Color::BLUE);
}

/// Renders one frame of the chunked display into `buf` (one chunk holds the whole screen).
fn render(e: &mut Engine, buf: &mut [u8], ms: u64) {
    assert!(
        e.refresh_begin(Instant::from_millis(ms)).is_some(),
        "nothing to render"
    );
    while e.render_chunk(buf).is_some() {}
    e.refresh_end();
}

fn info() -> DisplayInfo {
    DisplayInfo::new(W, H, ColorFormat::Rgb565)
}

fn leaked(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

#[test]
fn tree_bytes_are_the_heap_the_nodes_take() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let before = e.memory_report();
    let ((), stats) = count_allocs(|| {
        for _ in 0..300 {
            e.create(root, Box::new(Obj)).unwrap();
        }
    });
    let after = e.memory_report();
    assert_eq!(after.nodes, before.nodes + 300);
    assert!(after.node_slots >= 301);
    assert_eq!(
        (after.tree - before.tree) as i64,
        stats.live,
        "{before:?} → {after:?}, {stats:?}"
    );
    // Nothing else moved.
    assert_eq!(
        (after.render_scratch, after.glyph_cache, after.layer_buf),
        (before.render_scratch, before.glyph_cache, before.layer_buf)
    );
}

#[test]
fn render_caches_and_layer_buffer_are_what_the_engine_allocates() {
    let cfg = EngineConfig {
        glyph_cache_bytes: 4096,
        layer_buf_bytes: 16 * 1024,
        ..EngineConfig::default()
    };
    let (heap_engine, heap) = count_allocs(|| Engine::new(cfg).unwrap());
    let (static_engine, statik) = count_allocs(|| Engine::with_layer_buf(cfg, leaked(16 * 1024)).unwrap());
    let (h, s) = (heap_engine.memory_report(), static_engine.memory_report());
    // The static engine's heap lacks exactly the layer buffer (the leaked test buffer itself is
    // counted in `statik`: subtract it).
    assert_eq!(heap.live - (statik.live - 16 * 1024), 16 * 1024);
    assert_eq!((h.layer_buf, h.layer_buf_static), (16 * 1024, false));
    assert_eq!((s.layer_buf, s.layer_buf_static), (16 * 1024, true));
    assert_eq!(h.heap_bytes() - s.heap_bytes(), 16 * 1024);
    assert_eq!((h.static_bytes(), s.static_bytes()), (0, 16 * 1024));
    // Everything the report itemises was allocated by `Engine::new`; the rest is bookkeeping
    // (here mostly the `debug-checks` invalidation logs, 2 × 64 entries).
    let itemised = h.heap_bytes() as i64;
    assert!(
        itemised <= heap.live && heap.live - itemised <= 6 * 1024,
        "{h:?}, {heap:?}"
    );
    assert!(h.glyph_cache >= 4096);
    assert_eq!(h.render_scratch, 8 * 480); // the default widest span
}

#[test]
fn draw_buffers_are_reported_by_where_they_live() {
    struct Panel;
    impl DisplayDriver for Panel {
        type Error = ();
        fn info(&self) -> DisplayInfo {
            info()
        }
        fn begin_flush(&mut self, _: Rect, _: DrawBufferMem) -> Result<(), ()> {
            Ok(())
        }
        fn poll_flush(&mut self) -> Option<DrawBufferMem> {
            None
        }
    }
    let mut heap_engine = Engine::new(EngineConfig::default()).unwrap();
    let (_, heap) = count_allocs(|| {
        heap_engine
            .add_display(Panel, BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 }))
            .unwrap()
    });
    let mut static_engine = Engine::new(EngineConfig::default()).unwrap();
    let (a, b) = (leaked(W as usize * 2 * 8 + 4), leaked(W as usize * 2 * 8 + 4));
    let (a, b) = (aligned(a, W as usize * 2 * 8), aligned(b, W as usize * 2 * 8));
    let (_, statik) = count_allocs(|| {
        static_engine
            .add_display(Panel, BufferMode::partial_double(a, b))
            .unwrap()
    });
    let (h, s) = (heap_engine.memory_report(), static_engine.memory_report());
    let one = W as usize * 2 * 8;
    assert_eq!(h.draw_buffers_heap, 2 * (one + BufferMode::ALLOC_PADDING));
    assert_eq!(
        (h.draw_buffers_static, s.draw_buffers_heap, s.draw_buffers_static),
        (0, 0, 2 * one)
    );
    // The heap engine took exactly its buffers more than the static one for the same display.
    assert_eq!(heap.live - statik.live, h.draw_buffers_heap as i64);
}

/// The 4-byte aligned `len` bytes inside `m` (caller buffers must be aligned).
fn aligned(m: &'static mut [u8], len: usize) -> &'static mut [u8] {
    let off = m.as_ptr().align_offset(4);
    &mut m[off..off + len]
}

#[test]
fn static_layer_buffer_draws_layers_without_the_heap() {
    // Same scene, heap layer buffer vs static layer buffer: identical pixels; with the static
    // buffer, rendering the layered frame again allocates nothing.
    let mut frames = Vec::new();
    for layer in [None, Some(leaked(24 * 1024))] {
        let mut e = match layer {
            None => Engine::new(EngineConfig::default()).unwrap(),
            Some(buf) => Engine::with_layer_buf(EngineConfig::default(), buf).unwrap(),
        };
        let d = e.add_chunked_display(info(), CHUNK).unwrap();
        layered_scene(&mut e);
        let mut buf = vec![0u8; CHUNK];
        render(&mut e, &mut buf, 0);
        let first = buf.clone();
        if e.memory_report().layer_buf_static {
            e.invalidate_area(
                d,
                Rect::from_xywh(0, 0, i32::from(W), i32::from(H)),
                InvalidateReason::Explicit,
            );
            let ((), stats) = count_allocs(|| render(&mut e, &mut buf, 100));
            assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
            assert_eq!(buf, first);
        }
        frames.push(first);
    }
    assert_eq!(
        frames[0], frames[1],
        "the static layer buffer draws the same pixels"
    );
    // The group is half transparent: its overlap is not darker than either box alone.
    assert!(frames[0].iter().any(|b| *b != 0xFF));
}
