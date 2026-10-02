//! `Ui::memory_report` / `TestUi::memory_report` (R3.S10): the parts add up, match the engine,
//! the runtime and the heap the queues and buffers really take (counting allocator), and the
//! static layer and draw buffers keep the heap out.

use twine_core::ColorFormat;
use twine_engine::EngineError;
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{EngineHarness, MemoryDisplay, MockClock, TestUi, by_id};
use twine_view::prelude::*;
use twine_view::{MemoryReport, UiCore};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn scene(cx: Scope) -> impl View {
    let n = cx.signal(0u32);
    column((
        label(text!("{}", n.get())),
        button(label("+"))
            .on_click(move || n.update(|v| *v += 1))
            .test_id("inc"),
        slider(cx.signal(30)),
    ))
}

fn assert_consistent(m: &MemoryReport) {
    assert_eq!(
        m.heap_bytes(),
        m.engine.heap_bytes() + m.reactive.bytes() + m.engine_queues + m.waker_pool
    );
    assert_eq!(m.static_bytes(), m.engine.static_bytes());
}

#[test]
fn test_ui_report_matches_engine_and_runtime() {
    let t = TestUi::new(160, 120).mount(scene);
    let m = t.memory_report();
    assert_consistent(&m);
    assert_eq!(m.engine, t.engine().memory_report());
    assert_eq!(m.engine.nodes, t.engine().tree().len());
    let stats = Runtime::current_thread().stats();
    assert_eq!((m.reactive.nodes, m.reactive.scopes), (stats.nodes, stats.scopes));
    assert_eq!(m.reactive, Runtime::current_thread().memory());
    assert!(m.engine.tree > 0 && m.reactive.values > 0 && m.engine_queues > 0);
    // Clicking changes values, not structure: the tree and the queues keep their size.
    t.find(by_id("inc")).click();
    let after = t.memory_report();
    assert_eq!(
        (after.engine.nodes, after.engine_queues),
        (m.engine.nodes, m.engine_queues)
    );
}

#[test]
fn engine_queue_bytes_are_the_heap_of_the_queues() {
    let mut h = EngineHarness::new(64, 32);
    let d = h.display();
    let mut core = UiCore::mount(Runtime::current_thread(), h.engine_mut(), d, |_| label("x")).unwrap();
    let before = core.memory_report(h.engine()).engine_queues;
    let ((), stats) = count_allocs(|| core.set_engine_queue_capacity(256));
    let after = core.memory_report(h.engine()).engine_queues;
    assert!(after > before);
    assert_eq!(after as i64 - before as i64, stats.live, "{stats:?}");
    core.dispose(h.engine_mut());
}

#[test]
fn static_layer_and_draw_buffers_stay_out_of_the_heap() {
    twine_view::draw_buffers!(static BUFS: 2 x 16 rows x 160 px @ Rgb565);
    static LAYER: LayerBuffer<{ 16 * 1024 }> = LayerBuffer::zeroed();
    let config = AppConfig::new().engine(EngineConfig {
        layer_buf_bytes: 16 * 1024,
        ..EngineConfig::default()
    });
    let panel = || MemoryDisplay::new(DisplayInfo::new(160, 120, ColorFormat::Rgb565));

    // The two UIs share the thread's runtime, whose arenas grow by doubling: measure each
    // build's runtime growth and leave it out of the comparison.
    let rt_bytes = || Runtime::current_thread().memory().bytes() as i64;
    let rt0 = rt_bytes();
    let (heap_ui, heap) = count_allocs(|| {
        Ui::builder(panel())
            .runtime(Runtime::current_thread())
            .clock(MockClock::new())
            .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 16 }))
            .app_config(config.clone())
            .build(|_| label("hi"))
    });
    let rt1 = rt_bytes();
    let (static_ui, statik) = count_allocs(|| {
        Ui::builder(panel())
            .runtime(Runtime::current_thread())
            .clock(MockClock::new())
            .buffers(BufferMode::partial_double_from(BUFS.take().unwrap()))
            .layer_buf(LAYER.take().unwrap())
            .app_config(config.clone())
            .build(|_| label("hi"))
    });
    let rt2 = rt_bytes();
    let (h, s) = (heap_ui.memory_report(), static_ui.memory_report());
    assert_consistent(&h);
    assert_consistent(&s);
    let bufs = 2 * 160 * 2 * 16;
    assert_eq!(h.engine.draw_buffers_heap, bufs + 2 * BufferMode::ALLOC_PADDING);
    assert_eq!(
        (
            s.engine.draw_buffers_static,
            s.engine.layer_buf,
            s.engine.layer_buf_static
        ),
        (bufs, 16 * 1024, true)
    );
    assert_eq!(s.static_bytes(), bufs + 16 * 1024);
    // The static UI's heap is smaller by exactly the layer and draw buffers.
    let saved = h.engine.heap_bytes() - s.engine.heap_bytes();
    assert_eq!(saved, 16 * 1024 + bufs + 2 * BufferMode::ALLOC_PADDING);
    let (heap_ui_live, static_ui_live) = (heap.live - (rt1 - rt0), statik.live - (rt2 - rt1));
    assert_eq!(
        heap_ui_live - static_ui_live,
        saved as i64,
        "{heap:?} vs {statik:?}"
    );
}

#[test]
fn a_layer_buffer_below_four_kib_fails_the_build() {
    static SMALL: LayerBuffer<1024> = LayerBuffer::zeroed();
    let r = Ui::builder(MemoryDisplay::new(DisplayInfo::new(32, 16, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .layer_buf(SMALL.take().unwrap())
        .try_build(|_| label("hi"));
    assert!(
        matches!(r, Err(UiError::Engine(EngineError::InvalidConfig(_)))),
        "{r:?}"
    );
}
