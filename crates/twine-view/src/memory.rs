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
/// **Shared parts:** the reactive runtime is per partition and context and shared by every
/// `Ui` on it (R3.S08 multi-UI; with the `partitions` feature each partition's `Ui`s report
/// their own runtime), and the waker pool is process-wide, so with several UIs on one runtime
/// those two fields are the same in every report; do not add them up across such UIs.
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
    /// [`Limits::engine_queue`](twine_core::Limits::engine_queue)).
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
        self.engine
            .heap_bytes()
            .saturating_add(self.reactive.bytes())
            .saturating_add(self.engine_queues)
            .saturating_add(self.waker_pool)
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
    /// let display = engine.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::Rgb565), 32 * 2 * 4, None).unwrap();
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

/// Seals Twine for a UI whose memory is `report` ([`UiCore::seal`], `Ui::seal`,
/// `AsyncUi::seal`): checks [`Limits::heap_budget`](twine_core::Limits::heap_budget) against
/// [`MemoryReport::heap_bytes`] (a `Capacity` fault with code
/// [`CapacityFault::HeapBudget`](twine_core::fault::CapacityFault::HeapBudget) when it is
/// exceeded), logs the heap in use, then [`alloc_guard::seal`](twine_core::alloc_guard::seal)s
/// the engine's partition (the UI's: other partitions keep their own seal state).
pub(crate) fn seal(engine: &mut Engine, report: &MemoryReport) {
    let heap = report.heap_bytes();
    if let Some(budget) = engine.limits().heap_budget.filter(|&b| heap > b) {
        twine_core::error!(
            target: "twine::view",
            "heap budget exceeded at seal: {} bytes in use, Limits::heap_budget = {}",
            heap,
            budget
        );
        engine.raise_fault(
            twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
                .code(twine_core::fault::CapacityFault::HeapBudget.code()),
        );
    }
    twine_core::alloc_guard::seal(engine.partition());
    twine_core::info!(
        target: "twine::view",
        "sealed {}: {} heap bytes, {} static bytes",
        engine.partition(),
        heap,
        report.static_bytes()
    );
}

impl UiCore {
    /// Ends start-up: from now on Twine is expected to allocate nothing, and every growth
    /// point that does allocate raises [`FaultKind::AllocAfterSeal`](twine_core::fault::FaultKind::AllocAfterSeal)
    /// at the next update (see [`twine_core::alloc_guard`] for how it is detected and what it
    /// covers). Call it once the UI is built and has run its first updates (every screen and
    /// buffer the product uses exists; with [`Limits::preallocate`](twine_core::Limits::preallocate)
    /// the bounded structures are reserved). Before sealing, it checks
    /// [`Limits::heap_budget`](twine_core::Limits::heap_budget) against the report's
    /// [`heap_bytes`](MemoryReport::heap_bytes) (a `Capacity` fault with code
    /// [`CapacityFault::HeapBudget`](twine_core::fault::CapacityFault::HeapBudget) when
    /// exceeded). Returns the memory report at sealing time (evidence of the fixed memory).
    ///
    /// The seal covers the UI's [`Partition`](twine_core::Partition) (its runtime's and
    /// engine's growth; other partitions seal on their own, ADR-001 § 5.3), process-wide
    /// without `std` (per thread with it); end it with
    /// [`alloc_guard::unseal`](twine_core::alloc_guard::unseal) before a planned
    /// reconfiguration. Never panics; O(nodes) once (the report).
    ///
    /// ```
    /// use twine_core::alloc_guard;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_core::ColorFormat;
    /// use twine_view::prelude::*;
    /// use twine_view::UiCore;
    ///
    /// let mut engine = Engine::new(EngineConfig::default()).unwrap();
    /// let display = engine.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::Rgb565), 32 * 2 * 4, None).unwrap();
    /// let core = UiCore::mount(Runtime::take().unwrap(), &mut engine, display, |_| label("hi")).unwrap();
    /// let report = core.seal(&mut engine);
    /// assert!(alloc_guard::is_sealed(core.partition()));
    /// assert!(report.heap_bytes() > 0);
    /// alloc_guard::unseal(core.partition());
    /// # core.dispose(&mut engine);
    /// ```
    pub fn seal(&self, engine: &mut Engine) -> MemoryReport {
        let report = self.memory_report(engine);
        seal(engine, &report);
        report
    }
}
