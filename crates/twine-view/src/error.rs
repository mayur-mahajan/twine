//! Errors of building a UI: [`BuildFailure`], [`BuildError`] and [`UiError`].
//!
//! Building never aliases a widget that could not be created to another node: it gets
//! [`DEAD_NODE`](twine_engine::DEAD_NODE), its build steps and children are skipped, and a
//! [`FaultKind::BuildFailed`](twine_core::fault::FaultKind::BuildFailed) fault is raised whose
//! [`code`](twine_engine::FaultRecord::code) is the [`BuildFailure`]. The constructors of the
//! runtime ([`UiBuilder::try_build`](crate::UiBuilder::try_build),
//! [`UiCore::mount`](crate::UiCore::mount), and the async builder) turn any such fault raised
//! while the application is built into an error. What the product does about it (fail-safe
//! screen, reset, retry with a smaller UI) is the application's decision.

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

/// Widgets could not be created while an application was built: each got
/// [`DEAD_NODE`](twine_engine::DEAD_NODE) and raised a `BuildFailed` fault (see
/// [`BuildCx::create`](crate::BuildCx::create)). The parts that were created are removed again
/// (see [`UiCore::mount`](crate::UiCore::mount)).
///
/// ```
/// use twine_view::{BuildError, BuildFailure};
/// let e = BuildError::new(3, BuildFailure::Capacity, None);
/// assert_eq!(e.failures, 3);
/// assert_eq!(e.to_string(), "3 widget(s) could not be built: the widget tree is full");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BuildError {
    /// How many widgets failed (each one's children were skipped and are not counted).
    pub failures: u32,
    /// Why the last one failed.
    pub cause: BuildFailure,
    /// The parent the last one was to be created under (`None` for a root).
    pub parent: Option<NodeId>,
}

impl BuildError {
    /// An error for `failures` failed widgets, the last one failing for `cause` under `parent`.
    #[must_use]
    pub const fn new(failures: u32, cause: BuildFailure, parent: Option<NodeId>) -> BuildError {
        BuildError {
            failures,
            cause,
            parent,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} widget(s) could not be built: {}",
            self.failures, self.cause
        )?;
        if let Some(p) = self.parent {
            write!(f, " (parent {})", fmt_node_id(p))?;
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
            "{=u32} widget(s) could not be built: {} (parent {})",
            self.failures,
            self.cause,
            self.parent.map(fmt_node_id)
        );
    }
}

/// Why a UI runtime could not be constructed: the engine rejected the configuration (display,
/// buffers, inputs, clock), or the application could not be built.
///
/// ```
/// use twine_view::{BuildError, BuildFailure, UiError};
/// use twine_engine::EngineError;
/// let e: UiError = EngineError::InvalidConfig("Ui needs a clock").into();
/// assert!(matches!(e, UiError::Engine(_)));
/// let e: UiError = BuildError::new(1, BuildFailure::Capacity, None).into();
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

impl core::error::Error for UiError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            UiError::Engine(e) => Some(e),
            UiError::Build(e) => Some(e),
        }
    }
}
