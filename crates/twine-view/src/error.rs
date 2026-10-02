//! Errors of building a UI: [`BuildFailure`], [`BuildFault`], [`BuildReport`], [`BuildError`]
//! and [`UiError`].
//!
//! Building never aliases a widget that could not be created to another node: it gets
//! [`DEAD_NODE`](twine_engine::DEAD_NODE), its build steps and children are skipped, and a
//! [`FaultKind::BuildFailed`](twine_core::fault::FaultKind::BuildFailed) fault is raised whose
//! [`code`](twine_engine::FaultRecord::code) is the [`BuildFailure`]. The constructors of the
//! runtime ([`UiBuilder::try_build`](crate::UiBuilder::try_build),
//! [`UiCore::mount`](crate::UiCore::mount), and the async builder) record every such failure
//! of the application's build in a [`BuildReport`] they own, and fail with a [`BuildError`]
//! when it is not empty (the faults are telemetry; the report decides). What the product does
//! about it (fail-safe screen, reset, retry with a smaller UI) is the application's decision.

use core::fmt;

use twine_engine::{EngineError, NodeId, fmt_node_id};

/// Why a widget could not be created while building: the stable
/// [`code`](twine_engine::FaultRecord::code) of its `BuildFailed` fault record.
///
/// ```
/// use twine_view::BuildFailure;
/// assert_eq!(BuildFailure::Capacity.code(), 1);
/// assert_eq!(BuildFailure::from_code(2), Some(BuildFailure::ParentGone));
/// assert_eq!(BuildFailure::from_code(0), None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
#[non_exhaustive]
pub enum BuildFailure {
    /// The widget tree is full ([`EngineConfig::max_nodes`](twine_engine::EngineConfig::max_nodes)
    /// or the 65 535 nodes of the arena); the engine raised a `Capacity` fault as well.
    Capacity = 1,
    /// The parent does not exist (deleted while the view was being built).
    ParentGone = 2,
    /// Any other engine error.
    Engine = 3,
}

impl BuildFailure {
    /// The failure of an [`EngineError`] returned by node creation.
    #[must_use]
    pub fn of(e: &EngineError) -> BuildFailure {
        match e {
            EngineError::TooManyNodes => BuildFailure::Capacity,
            EngineError::NodeNotFound(_) => BuildFailure::ParentGone,
            _ => BuildFailure::Engine,
        }
    }

    /// The stable numeric code (the fault record's `code`).
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The failure with code `code` (`None` for an unknown code).
    #[must_use]
    pub const fn from_code(code: u32) -> Option<BuildFailure> {
        match code {
            1 => Some(BuildFailure::Capacity),
            2 => Some(BuildFailure::ParentGone),
            3 => Some(BuildFailure::Engine),
            _ => None,
        }
    }
}

/// Which bound of the view layer a [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity)
/// record raised by the view layer is about: the stable
/// [`code`](twine_engine::FaultRecord::code) of the record (the engine's own capacity faults,
/// e.g. a full widget tree, have code `0`).
///
/// ```
/// use twine_view::CapacityFault;
/// assert_eq!(CapacityFault::EngineQueue.code(), 1);
/// assert_eq!(CapacityFault::from_code(1), Some(CapacityFault::EngineQueue));
/// assert_eq!(CapacityFault::from_code(2), Some(CapacityFault::WakerPool));
/// assert_eq!(CapacityFault::from_code(0), None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
#[non_exhaustive]
pub enum CapacityFault {
    /// The `Ui`'s engine command queue was full: engine side effects issued without the
    /// engine (a scope disposed or a handle called outside `Ui::update`) were dropped; the
    /// record's `occurrences` is their number (see
    /// [`UiBuilder::engine_queue_capacity`](crate::UiBuilder::engine_queue_capacity)).
    EngineQueue = 1,
    /// The process-wide waker pool was exhausted when the `Ui` was mounted: it got the shared
    /// fallback waker instead of its own (see [`WakerLease`](twine_reactive::WakerLease) §
    /// Exhausted pool and [`WakerLease::set_heap_limit`](twine_reactive::WakerLease::set_heap_limit)),
    /// so channel and interrupt wake-ups are no longer routed to this UI alone.
    WakerPool = 2,
}

impl CapacityFault {
    /// The stable numeric code (the fault record's `code`).
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The bound with code `code` (`None` for an unknown code).
    #[must_use]
    pub const fn from_code(code: u32) -> Option<CapacityFault> {
        match code {
            1 => Some(CapacityFault::EngineQueue),
            2 => Some(CapacityFault::WakerPool),
            _ => None,
        }
    }
}

impl fmt::Display for BuildFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BuildFailure::Capacity => "the widget tree is full",
            BuildFailure::ParentGone => "the parent node does not exist",
            BuildFailure::Engine => "the engine rejected the node",
        })
    }
}

/// One widget that could not be created while building: why, and under which parent.
///
/// ```
/// use twine_view::{BuildFailure, BuildFault};
/// let f = BuildFault::new(BuildFailure::ParentGone, None);
/// assert_eq!(f.cause, BuildFailure::ParentGone);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildFault {
    /// Why the widget was not created.
    pub cause: BuildFailure,
    /// The parent it was to be created under (`None` for a root).
    pub parent: Option<NodeId>,
}

impl BuildFault {
    /// A failure for `cause` under `parent`.
    #[must_use]
    pub const fn new(cause: BuildFailure, parent: Option<NodeId>) -> BuildFault {
        BuildFault { cause, parent }
    }
}

impl fmt::Display for BuildFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cause)?;
        if let Some(p) = self.parent {
            write!(f, " (parent {})", fmt_node_id(p))?;
        }
        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for BuildFault {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "{} (parent {})", self.cause, self.parent.map(fmt_node_id));
    }
}

/// The record of a build: how many widgets could not be created, and the first and the last
/// failure. The mount of a `Ui` / [`UiCore`](crate::UiCore) owns one for the duration of the
/// build, and every [`BuildCx::create`](crate::BuildCx::create) failure under the
/// application's root scope fills it — also the failures of nested builds that run while the
/// application is mounted (e.g. the rows a [`for_each`](crate::for_each) builds in its first
/// run). A non-empty report fails the mount with a [`BuildError`]
/// ([`into_result`](Self::into_result)).
///
/// The report is the explicit, per-mount account of what failed; the
/// [`FaultKind::BuildFailed`](twine_core::fault::FaultKind::BuildFailed) faults raised for the
/// same failures are telemetry (counted engine-wide, seen by the fault hook) and play no part
/// in the decision. `Copy`, 24 bytes on 32-bit targets; recording allocates nothing.
///
/// ```
/// use twine_view::{BuildFailure, BuildFault, BuildReport};
///
/// let mut report = BuildReport::new();
/// assert!(report.is_ok());
/// report.record(BuildFault::new(BuildFailure::Capacity, None));
/// report.record(BuildFault::new(BuildFailure::ParentGone, None));
/// assert_eq!(report.failures(), 2);
/// assert_eq!(report.first().map(|f| f.cause), Some(BuildFailure::Capacity));
/// assert_eq!(report.last().map(|f| f.cause), Some(BuildFailure::ParentGone));
/// let err = report.into_result().unwrap_err();
/// assert_eq!(err.failures, 2);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildReport {
    failures: u32,
    first: Option<BuildFault>,
    last: Option<BuildFault>,
}

impl BuildReport {
    /// An empty report. `const`; never panics.
    #[must_use]
    pub const fn new() -> BuildReport {
        BuildReport {
            failures: 0,
            first: None,
            last: None,
        }
    }

    /// Records one failed widget (the count saturates at `u32::MAX`). Never panics.
    pub fn record(&mut self, fault: BuildFault) {
        self.failures = self.failures.saturating_add(1);
        if self.first.is_none() {
            self.first = Some(fault);
        }
        self.last = Some(fault);
    }

    /// How many widgets failed (each one's children were skipped and are not counted;
    /// saturates at `u32::MAX`). Never panics.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// The first failure, if any. Never panics.
    #[must_use]
    pub const fn first(&self) -> Option<BuildFault> {
        self.first
    }

    /// The last failure, if any. Never panics.
    #[must_use]
    pub const fn last(&self) -> Option<BuildFault> {
        self.last
    }

    /// Whether nothing failed. Never panics.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.failures == 0
    }

    /// `Ok(())` when nothing failed, the [`BuildError`] otherwise. Never panics.
    ///
    /// # Errors
    ///
    /// [`BuildError`] with the count and the first and last failure when any widget failed.
    pub fn into_result(self) -> Result<(), BuildError> {
        match (self.first, self.last) {
            (Some(first), Some(last)) => Err(BuildError {
                failures: self.failures,
                first,
                last,
            }),
            _ => Ok(()),
        }
    }
}

/// Widgets could not be created while an application was built: each got
/// [`DEAD_NODE`](twine_engine::DEAD_NODE) (see [`BuildCx::create`](crate::BuildCx::create)),
/// and the mount's [`BuildReport`] recorded it. The parts that were created are removed again
/// (see [`UiCore::mount`](crate::UiCore::mount)).
///
/// ```
/// use twine_view::{BuildError, BuildFailure, BuildFault};
/// let e = BuildError::new(
///     3,
///     BuildFault::new(BuildFailure::Capacity, None),
///     BuildFault::new(BuildFailure::Capacity, None),
/// );
/// assert_eq!(e.failures, 3);
/// assert_eq!(e.to_string(), "3 widget(s) could not be built: the widget tree is full");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildError {
    /// How many widgets failed (each one's children were skipped and are not counted).
    pub failures: u32,
    /// The first failure (usually the root cause).
    pub first: BuildFault,
    /// The last failure.
    pub last: BuildFault,
}

impl BuildError {
    /// An error for `failures` failed widgets, `first` and `last` among them.
    #[must_use]
    pub const fn new(failures: u32, first: BuildFault, last: BuildFault) -> BuildError {
        BuildError {
            failures,
            first,
            last,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} widget(s) could not be built: {}",
            self.failures, self.first
        )?;
        if self.last != self.first {
            write!(f, "; last: {}", self.last)?;
        }
        Ok(())
    }
}

impl core::error::Error for BuildError {}

#[cfg(feature = "defmt")]
impl defmt::Format for BuildError {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(
            f,
            "{=u32} widget(s) could not be built: {} (last: {})",
            self.failures,
            self.first,
            self.last
        );
    }
}

/// Why a UI runtime could not be constructed: the engine rejected the configuration (display,
/// buffers, inputs, rotation), or the application could not be built.
///
/// ```
/// use twine_view::{BuildError, BuildFailure, BuildFault, UiError};
/// use twine_engine::EngineError;
/// let e: UiError = EngineError::InvalidConfig("the display already runs an application of this Ui").into();
/// assert!(matches!(e, UiError::Engine(_)));
/// let f = BuildFault::new(BuildFailure::Capacity, None);
/// let e: UiError = BuildError::new(1, f, f).into();
/// assert!(e.build_error().is_some());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UiError {
    /// The engine rejected the configuration.
    Engine(EngineError),
    /// The application could not be built.
    Build(BuildError),
}

impl UiError {
    /// The engine error, if this is one.
    #[must_use]
    pub fn engine_error(&self) -> Option<&EngineError> {
        match self {
            UiError::Engine(e) => Some(e),
            UiError::Build(_) => None,
        }
    }

    /// The build error, if this is one.
    #[must_use]
    pub fn build_error(&self) -> Option<&BuildError> {
        match self {
            UiError::Build(e) => Some(e),
            UiError::Engine(_) => None,
        }
    }
}

impl From<EngineError> for UiError {
    fn from(e: EngineError) -> Self {
        UiError::Engine(e)
    }
}

impl From<BuildError> for UiError {
    fn from(e: BuildError) -> Self {
        UiError::Build(e)
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::Engine(e) => write!(f, "engine: {e}"),
            UiError::Build(e) => write!(f, "build: {e}"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for UiError {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            UiError::Engine(e) => defmt::write!(f, "engine: {}", e),
            UiError::Build(e) => defmt::write!(f, "build: {}", e),
        }
    }
}

/// The failure path of the panicking `build` methods ([`UiBuilder::build`](crate::UiBuilder::build)
/// and the async builder's): panics with `e`, naming the `runtime` (`"Ui"`, `"AsyncUi"`).
///
/// With the `defmt` feature the message is a `defmt` panic (the error's `defmt::Format`), so
/// firmware does not link `core::fmt` and the `Debug` code of every error type it contains;
/// otherwise it is a `core` panic with the error's `Display`. Cold and out of line: the
/// generic `build` methods keep only the call.
#[cold]
#[inline(never)]
#[track_caller]
pub(crate) fn build_failed(runtime: &'static str, e: &UiError) -> ! {
    #[cfg(feature = "defmt")]
    defmt::panic!("twine: cannot build the {=str}: {}", runtime, e);
    #[cfg(not(feature = "defmt"))]
    panic!("twine: cannot build the {runtime}: {e}");
}

impl core::error::Error for UiError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            UiError::Engine(e) => Some(e),
            UiError::Build(e) => Some(e),
        }
    }
}
