//! [`Validity`]: a value together with how much it can be trusted, and its
//! [`ValidityStatus`].
//!
//! A display that shows measured data must never show a wrong value as if it were right: a
//! sensor that stopped reporting, a value that is too old, a failed computation must look
//! different from a valid reading. `Validity<T>` carries that information from the data source
//! to the widget: the view layer turns it into the standard look (the value, the value marked
//! as stale, or dashes with a cross-hatch) and the producer decides only what it knows (a
//! value, a failure, nothing yet). Freshness is added by the consumer
//! (`ScopeExt::watch_fresh` in `twine-view`), which knows the time.
//!
//! ```
//! use twine_core::validity::{Validity, ValidityStatus};
//! use twine_core::Duration;
//!
//! let speed: Validity<u16> = Validity::Valid(88);
//! assert_eq!(speed.value(), Some(&88));
//! assert_eq!(speed.status(), ValidityStatus::Valid);
//!
//! let old = Validity::Stale { value: 88u16, age: Duration::ms(1500) };
//! assert_eq!(old.value(), Some(&88)); // still shown, marked as stale
//! assert_eq!(old.status(), ValidityStatus::Stale);
//!
//! let lost: Validity<u16> = Validity::Failed;
//! assert_eq!(lost.value(), None); // shown as dashes
//! assert_eq!(lost.status(), ValidityStatus::Invalid);
//! assert_eq!(lost.value_or(0), 0);
//! ```

use crate::Duration;

/// A value and how far it can be trusted.
///
/// | Variant | Meaning | Standard look |
/// |---------|---------|---------------|
/// | [`Valid`](Self::Valid) | a current value | the value |
/// | [`Stale`](Self::Stale) | the last value, older than allowed | the value, marked stale (`State::STALE`) |
/// | [`Failed`](Self::Failed) | the source reported a failure | dashes and a cross-hatch (`State::INVALID`) |
/// | [`NoData`](Self::NoData) | no value yet | dashes and a cross-hatch (`State::INVALID`) |
///
/// The producer stores `Valid`, `Failed` or `NoData` (e.g. in a `Latest<Validity<T>>`); the
/// consumer turns an old `Valid` into `Stale`. `From<T>` gives `Valid`. The default is
/// `NoData`. Never panics; `Copy` when `T` is.
///
/// ```
/// use twine_core::validity::Validity;
///
/// let v: Validity<i32> = 21.into();
/// assert!(v.is_valid());
/// assert_eq!(v.map(|t| t * 2), Validity::Valid(42));
/// assert_eq!(Validity::<i32>::default(), Validity::NoData);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Validity<T> {
    /// A current value.
    Valid(T),
    /// The last value, older than the consumer allows.
    Stale {
        /// The last value.
        value: T,
        /// How old the value was when it was found stale (or last re-checked).
        age: Duration,
    },
    /// The source reported a failure (sensor fault, out of range, lost link).
    Failed,
    /// No value yet (nothing received since start-up).
    NoData,
}

/// How a [`Validity`] is shown: its variant without the value.
///
/// ```
/// use twine_core::validity::{Validity, ValidityStatus};
/// assert_eq!(ValidityStatus::from(Validity::<u8>::NoData), ValidityStatus::Invalid);
/// assert!(ValidityStatus::Valid < ValidityStatus::Stale);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ValidityStatus {
    /// The value is current.
    #[default]
    Valid,
    /// The value is shown but too old.
    Stale,
    /// There is no value to show (failed, or no data yet).
    Invalid,
}

impl<T> Validity<T> {
    /// The value to show: `Some` for `Valid` and `Stale`, `None` otherwise.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Validity::Valid(v) | Validity::Stale { value: v, .. } => Some(v),
            Validity::Failed | Validity::NoData => None,
        }
    }

    /// The value to show, or `fallback` (e.g. the position a bar shows under the cross-hatch).
    #[must_use]
    pub fn value_or(self, fallback: T) -> T {
        match self {
            Validity::Valid(v) | Validity::Stale { value: v, .. } => v,
            Validity::Failed | Validity::NoData => fallback,
        }
    }

    /// Whether this is [`Valid`](Self::Valid).
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Validity::Valid(_))
    }

    /// How this is shown ([`ValidityStatus`]).
    #[must_use]
    pub const fn status(&self) -> ValidityStatus {
        match self {
            Validity::Valid(_) => ValidityStatus::Valid,
            Validity::Stale { .. } => ValidityStatus::Stale,
            Validity::Failed | Validity::NoData => ValidityStatus::Invalid,
        }
    }

    /// Maps the value, keeping the variant (and the age of a stale value).
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Validity<U> {
        match self {
            Validity::Valid(v) => Validity::Valid(f(v)),
            Validity::Stale { value, age } => Validity::Stale { value: f(value), age },
            Validity::Failed => Validity::Failed,
            Validity::NoData => Validity::NoData,
        }
    }

    /// A reference to the value, keeping the variant.
    #[must_use]
    pub const fn as_ref(&self) -> Validity<&T> {
        match self {
            Validity::Valid(v) => Validity::Valid(v),
            Validity::Stale { value, age } => Validity::Stale { value, age: *age },
            Validity::Failed => Validity::Failed,
            Validity::NoData => Validity::NoData,
        }
    }
}

impl<T> Default for Validity<T> {
    /// [`Validity::NoData`].
    fn default() -> Self {
        Validity::NoData
    }
}

impl<T> From<T> for Validity<T> {
    /// [`Validity::Valid`].
    fn from(v: T) -> Self {
        Validity::Valid(v)
    }
}

impl<T> From<Validity<T>> for ValidityStatus {
    fn from(v: Validity<T>) -> Self {
        v.status()
    }
}

impl<T> From<&Validity<T>> for ValidityStatus {
    fn from(v: &Validity<T>) -> Self {
        v.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_map_to_statuses_and_values() {
        let stale = Validity::Stale {
            value: 3u8,
            age: Duration::ms(10),
        };
        assert_eq!(stale.status(), ValidityStatus::Stale);
        assert_eq!(
            stale.map(u16::from),
            Validity::Stale {
                value: 3u16,
                age: Duration::ms(10)
            }
        );
        assert_eq!(stale.as_ref().value(), Some(&&3));
        assert_eq!(Validity::<u8>::Failed.map(|v| v + 1), Validity::Failed);
        assert_eq!(Validity::<u8>::NoData.value_or(7), 7);
        assert_eq!(ValidityStatus::from(&Validity::Valid(1)), ValidityStatus::Valid);
    }
}
