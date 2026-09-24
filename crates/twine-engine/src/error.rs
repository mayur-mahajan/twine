//! [`EngineError`] and [`InvariantError`].

use core::fmt::{self, Write};

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
    /// A display driver reported an error (its `Debug` output, truncated to 64 bytes).
    #[error("driver error: {0}")]
    Driver(heapless::String<64>),
}

impl EngineError {
    /// A [`EngineError::Driver`] from a driver error's `Debug` output (truncated).
    pub fn driver(e: &dyn fmt::Debug) -> Self {
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
        EngineError::Driver(t.0)
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
        let EngineError::Driver(s) = EngineError::driver(&long) else {
            panic!("variant");
        };
        assert_eq!(s.len(), 64);
        assert!(EngineError::TooManyDisplays.to_string().contains("displays"));
    }
}
