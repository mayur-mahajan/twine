//! [`LottieError`].

use core::fmt;

/// Why a Lottie file could not be loaded.
///
/// ```
/// use twine_lottie::{LottieError, load};
/// assert!(matches!(load(b"{ not json"), Err(LottieError::Json { .. })));
/// assert_eq!(load(b"[1, 2]"), Err(LottieError::Invalid("root is not an object")));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LottieError {
    /// The data is not valid JSON (1-based position of the error).
    Json {
        /// Line of the error.
        line: usize,
        /// Column of the error.
        column: usize,
    },
    /// The JSON is valid but not a usable Lottie composition.
    Invalid(&'static str),
}

impl fmt::Display for LottieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LottieError::Json { line, column } => {
                write!(f, "invalid JSON at line {line}, column {column}")
            }
            LottieError::Invalid(why) => write!(f, "invalid Lottie composition: {why}"),
        }
    }
}

impl core::error::Error for LottieError {}
