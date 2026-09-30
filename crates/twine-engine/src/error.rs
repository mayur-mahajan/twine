//! [`EngineError`] and [`InvariantError`].

use core::fmt::{self, Write};

use twine_core::ColorFormat;

use crate::{DisplayId, NodeId};

/// Errors of the engine API.
///
/// Callers that receive an id error usually log it at `warn!` and carry on (P7): invalid ids
/// never panic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The node does not exist (never created, or deleted).
    #[error("node {0} not found")]
    NodeNotFound(NodeId),
    /// The display does not exist.
    #[error("display {0} not found")]
    DisplayNotFound(DisplayId),
    /// More than [`MAX_DISPLAYS`](crate::MAX_DISPLAYS) displays.
    #[error("too many displays")]
    TooManyDisplays,
    /// More than [`MAX_INPUTS`](crate::MAX_INPUTS) input devices.
    #[error("too many input devices")]
    TooManyInputs,
    /// More than [`MAX_GROUPS`](crate::MAX_GROUPS) focus groups.
    #[error("too many focus groups")]
    TooManyGroups,
    /// More than 65 535 live nodes.
    #[error("too many nodes")]
    TooManyNodes,
    /// A draw buffer or framebuffer is smaller than needed.
    #[error("buffer too small: {got} bytes, {needed} needed")]
    BufferTooSmall {
        /// Bytes needed.
        needed: usize,
        /// Bytes provided.
        got: usize,
    },
    /// A draw buffer does not start at a 4-byte aligned address.
    #[error("draw buffer is not 4-byte aligned")]
    BufferMisaligned,
    /// Partial buffers were given for a framebuffer display, `Full`/`Direct` for a flush
    /// display, or a framebuffer display could not provide the buffers the mode needs.
    #[error("buffer mode does not match the display kind")]
    BufferModeMismatch,
    /// A configuration value or argument is invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),
    /// A display's pixels would have to be drawn in a colour format whose renderer is not
    /// compiled in: its `color-*` feature (of `twine`, `twine-engine` or `twine-render`) is
    /// disabled, or the format can never be a draw format (e.g. `A8`). Returned by
    /// [`Engine::add_display`](crate::Engine::add_display),
    /// [`add_framebuffer_display`](crate::Engine::add_framebuffer_display) and
    /// [`add_chunked_display`](crate::Engine::add_chunked_display) instead of adding a display
    /// that would stay blank; the display is not added, and
    /// [`FaultKind::FormatDisabled`](twine_core::fault::FaultKind::FormatDisabled) is raised.
    ///
    /// The format is the one the engine draws in: the display's format, except for `I1`
    /// panels behind partial or chunked buffers, which are drawn in `L8` and converted (the
    /// engine's `color-i1` feature enables both).
    #[error("drawing into {0} is not compiled in (enable its `color-*` feature)")]
    FormatDisabled(ColorFormat),
    /// A display driver reported an error.
    #[error("driver error {code}: {message}")]
    Driver {
        /// The driver's code for the error (`DisplayDriver::error_code`, `0` = none).
        code: DriverErrorCode,
        /// The error's `Debug` output, truncated to 64 bytes.
        message: heapless::String<64>,
    },
}

/// A display driver's numeric error code (from `DisplayDriver::error_code` and the matching
/// methods of the other driver traits; `0` means the driver gives none). Carried by
/// [`EngineError::Driver`] and, for flush faults, by
/// [`FaultRecord::code`](crate::FaultRecord::code).
///
/// ```
/// use twine_engine::DriverErrorCode;
/// let c = DriverErrorCode::new(0x2A);
/// assert_eq!(c.get(), 42);
/// assert_eq!(DriverErrorCode::default(), DriverErrorCode::NONE);
/// assert_eq!(format!("{c}"), "0x0000002a");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DriverErrorCode(u32);

impl DriverErrorCode {
    /// No code (the driver does not classify its errors).
    pub const NONE: DriverErrorCode = DriverErrorCode(0);

    /// The code `code`.
    #[must_use]
    pub const fn new(code: u32) -> DriverErrorCode {
        DriverErrorCode(code)
    }

    /// The numeric value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for DriverErrorCode {
    fn from(code: u32) -> Self {
        DriverErrorCode(code)
    }
}

impl fmt::Display for DriverErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#010x}", self.0)
    }
}

impl EngineError {
    /// The driver's error code of an [`EngineError::Driver`] (`None` for other errors).
    #[must_use]
    pub fn driver_code(&self) -> Option<DriverErrorCode> {
        match self {
            EngineError::Driver { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// An [`EngineError::Driver`] with `code` and the `Debug` output of the driver error `e`
    /// (truncated; formatting allocates nothing).
    pub fn driver(code: DriverErrorCode, e: &dyn fmt::Debug) -> Self {
        struct Trunc(heapless::String<64>);
        impl Write for Trunc {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                for c in s.chars() {
                    if self.0.push(c).is_err() {
                        break;
                    }
                }
                Ok(())
            }
        }
        let mut t = Trunc(heapless::String::new());
        let _ = write!(t, "{e:?}");
        EngineError::Driver { code, message: t.0 }
    }
}

/// A violated tree invariant, found by [`Tree::check_invariants`](crate::Tree::check_invariants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("tree invariant violated at {node}: {what} (related node: {other:?})")]
pub struct InvariantError {
    /// The node where the violation was found.
    pub node: NodeId,
    /// What is wrong.
    pub what: &'static str,
    /// Another node involved (e.g. the parent), if any.
    pub other: Option<NodeId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn driver_error_truncates() {
        let long = "x".repeat(200);
        let EngineError::Driver { code, message } = EngineError::driver(DriverErrorCode::new(3), &long)
        else {
            panic!("variant");
        };
        assert_eq!(message.len(), 64);
        assert_eq!(code.get(), 3);
        assert!(EngineError::TooManyDisplays.to_string().contains("displays"));
    }
}
