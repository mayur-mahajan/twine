//! [`MemoryReport`]: what a UI's memory is used for ([`Ui::memory_report`](crate::Ui::memory_report)).

use twine_engine::{Engine, EngineMemory};
use twine_reactive::{RuntimeMemory, WakerLease};

use crate::ui::UiCore;

/// The memory of a UI by part: the engine ([`EngineMemory`]), the reactive runtime
/// ([`RuntimeMemory`]), the engine command queues and the waker pool. From
/// [`Ui::memory_report`](crate::Ui::memory_report), `AsyncUi::memory_report` and
/// `twine_testing::TestUi::memory_report`.
///
/// **Units:** bytes, as asked of the allocator (capacities, box sizes; no allocator
/// overhead), except the counts documented as such in the parts. See [`EngineMemory`] and
/// [`RuntimeMemory`] for what each part includes and leaves out.
///
/// **Shared parts:** the reactive runtime is per context and shared by every `Ui` on it
/// (R3.S08 multi-UI), and the waker pool is process-wide, so with several UIs those two
/// fields are the same in every report; do not add them up across UIs.
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
/// let ui = Ui::builder(panel)
///     .runtime(Runtime::take().unwrap())
///     .clock(MockClock::new())
///     .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
///     .build(|cx| {
///         let n = cx.signal(0u32);
///         label(text!("{}", n.get()))
///     });
/// let m = ui.memory_report();
/// assert_eq!(m.engine.draw_buffers_heap, 64 * 2 * 8 + BufferMode::ALLOC_PADDING);
/// assert!(m.reactive.nodes >= 2); // the signal and the label's binding
/// assert!(m.engine_queues > 0);
/// assert_eq!(m.heap_bytes(), m.engine.heap_bytes() + m.reactive.bytes() + m.engine_queues + m.waker_pool);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MemoryReport {
    /// The engine: tree, render scratch and caches, glyph and image caches, layer buffer,
    /// draw buffers.
    pub engine: EngineMemory,
    /// The reactive runtime the UI runs on (shared by every UI on it).
    pub reactive: RuntimeMemory,
    /// The UI's engine command queues (one per display; see
    /// [`AppConfig::engine_queue_capacity`](crate::AppConfig::engine_queue_capacity)).
    pub engine_queues: usize,
    /// Heap wakers of the process-wide waker pool
    /// ([`WakerLease::heap_bytes`]; 0 while every UI uses a static slot or its own `static`
    /// waker).
    pub waker_pool: usize,
}

impl MemoryReport {
    /// Heap bytes of every part: the engine's heap, the reactive runtime, the command queues
    /// and the waker pool.
    ///
    /// # Panics
    /// Never for a report from `memory_report` (its parts are sizes of memory that exists); a
    /// hand-built value whose parts sum past `usize::MAX` overflows (a panic in debug builds).
    ///
    /// ```
    /// use twine_engine::EngineMemory;
    /// use twine_view::MemoryReport;
    ///
    /// let m = MemoryReport {
    ///     engine: EngineMemory { tree: 1000, ..EngineMemory::default() },
    ///     engine_queues: 200,
    ///     waker_pool: 64,
    ///     ..MemoryReport::default()
    /// };
    /// assert_eq!(m.heap_bytes(), 1264);
    /// ```
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.engine.heap_bytes() + self.reactive.bytes() + self.engine_queues + self.waker_pool
    }

    /// Caller memory (statics) the UI uses: a static layer buffer and static draw buffers
    /// ([`EngineMemory::static_bytes`]).
    ///
    /// # Panics
    /// As [`heap_bytes`](Self::heap_bytes): never for a report from `memory_report`.
    ///
    /// ```
    /// use twine_engine::EngineMemory;
    /// use twine_view::MemoryReport;
    ///
    /// let m = MemoryReport {
    ///     engine: EngineMemory { draw_buffers_static: 2048, ..EngineMemory::default() },
    ///     ..MemoryReport::default()
    /// };
    /// assert_eq!((m.static_bytes(), m.heap_bytes()), (2048, 0));
    /// ```
    #[must_use]
    pub fn static_bytes(&self) -> usize {
        self.engine.static_bytes()
    }
}

impl UiCore {
    /// The memory report of this UI on `engine` ([`MemoryReport`]). `Ui`, `AsyncUi` and
    /// `TestUi` call it with their engine. Allocates nothing; never panics; O(nodes).
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_core::ColorFormat;
    /// use twine_view::prelude::*;
    /// use twine_view::UiCore;
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let display = engine.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::Rgb565), 32 * 2 * 4).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, display, |_| label("hi")).unwrap();
    /// let m = core.memory_report(&engine);
    /// assert_eq!(m.engine, engine.memory_report());
    /// assert!(m.engine_queues > 0);
    /// ```
    #[must_use]
    pub fn memory_report(&self, engine: &Engine) -> MemoryReport {
        MemoryReport {
            engine: engine.memory_report(),
            reactive: self.runtime().memory(),
            engine_queues: self.engine_queue_bytes(),
            waker_pool: WakerLease::heap_bytes(),
        }
    }
}
