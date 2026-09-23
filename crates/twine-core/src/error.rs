//! The crate error type.

/// Errors of `twine-core` operations.
///
/// ```
/// use twine_core::Error;
/// assert_eq!(Error::InvalidArgument("radius").to_string(), "invalid argument: radius");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// A fixed-capacity container is full (e.g. an [`Arena`](crate::Arena) with 65 535 live
    /// entries).
    #[error("capacity exceeded")]
    CapacityExceeded,
    /// An argument is invalid; the payload names it.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// An index or coordinate is out of bounds.
    #[error("out of bounds")]
    OutOfBounds,
    /// The requested feature or format is not supported; the payload names it.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn error_display() {
        assert_eq!(Error::CapacityExceeded.to_string(), "capacity exceeded");
        assert_eq!(Error::InvalidArgument("x").to_string(), "invalid argument: x");
        assert_eq!(Error::OutOfBounds.to_string(), "out of bounds");
        assert_eq!(Error::Unsupported("I2 blit").to_string(), "unsupported: I2 blit");
        let e: &dyn core::error::Error = &Error::OutOfBounds;
        assert!(e.source().is_none());
    }
}
