//! Configuration evidence: a stable, machine-readable `key=value` dump of everything that bounds
//! or configures Twine ([`Audit`], [`AuditSink`], [`AuditValue`], [`KeyValues`]).
//!
//! A certification or audit file needs the configuration a product actually runs with — every
//! limit, cache budget, timeout, feature and profile flag — in a form a tool can diff against
//! the reviewed one. Each configuration type implements [`Audit`]: it **visits** its entries
//! (`section`, `key`, [`AuditValue`]) into an [`AuditSink`]. Nothing is formatted or allocated
//! unless a sink does so, and the formatting code is linked only when the application calls
//! it (pay for what you use).
//!
//! # Format
//!
//! [`KeyValues`] (`Display`, and `defmt::Format` with the `defmt` feature) writes one entry per
//! line:
//!
//! ```text
//! section.key=value
//! ```
//!
//! - `section` and `key` are `snake_case` ASCII; a key never changes meaning once released, new
//!   keys are appended, removed keys are not reused.
//! - Values: unsigned and signed integers in decimal, `true` / `false`, text verbatim (names
//!   only, never containing `=` or a line break), and `none` for an absent optional value.
//! - Units are part of the key (`_us` microseconds, `_bytes` bytes); counts have no suffix.
//! - The order of the entries is the visiting order and is stable for a given build.
//!
//! The view layer's `AppConfig::audit_report()` is the complete report of an application
//! (`audit.format`, `build.*` features and profile flags, `limits.*`, `engine.*`, `view.*`,
//! `theme.*`, and last `audit.digest_crc32`, the [`digest`] of the lines before it); [`Limits::summary`](crate::Limits::summary) is the limits part alone.
//!
//! ```
//! use twine_core::audit::{Audit, AuditSink, AuditValue, KeyValues};
//!
//! struct Budget { bytes: usize }
//! impl Audit for Budget {
//!     fn audit(&self, out: &mut dyn AuditSink) {
//!         out.entry("budget", "heap_bytes", AuditValue::from_usize(self.bytes));
//!     }
//! }
//! assert_eq!(KeyValues(&Budget { bytes: 4096 }).to_string(), "budget.heap_bytes=4096\n");
//! ```

use core::fmt;

/// One configuration value of an audit report.
///
/// ```
/// use twine_core::audit::AuditValue;
///
/// assert_eq!(AuditValue::Uint(3).to_string(), "3");
/// assert_eq!(AuditValue::Bool(true).to_string(), "true");
/// assert_eq!(AuditValue::Absent.to_string(), "none");
/// assert_eq!(AuditValue::Text("light").to_string(), "light");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditValue<'a> {
    /// An unsigned integer (counts, sizes, durations in the key's unit).
    Uint(u64),
    /// A signed integer.
    Int(i64),
    /// A flag.
    Bool(bool),
    /// A name (a theme, a mode, an enum variant). Never contains `=` or a line break.
    Text(&'a str),
    /// An optional value that is not set (written `none`).
    Absent,
}

impl AuditValue<'_> {
    /// A `usize` as [`Uint`](Self::Uint) (saturating on targets where `usize` is wider than
    /// 64 bits).
    ///
    /// ```
    /// use twine_core::audit::AuditValue;
    /// assert_eq!(AuditValue::from_usize(7), AuditValue::Uint(7));
    /// ```
    #[must_use]
    pub fn from_usize(v: usize) -> Self {
        AuditValue::Uint(u64::try_from(v).unwrap_or(u64::MAX))
    }

    /// An optional `usize`: [`Uint`](Self::Uint) or [`Absent`](Self::Absent).
    ///
    /// ```
    /// use twine_core::audit::AuditValue;
    /// assert_eq!(AuditValue::opt_usize(None), AuditValue::Absent);
    /// assert_eq!(AuditValue::opt_usize(Some(2)), AuditValue::Uint(2));
    /// ```
    #[must_use]
    pub fn opt_usize(v: Option<usize>) -> Self {
        v.map_or(AuditValue::Absent, AuditValue::from_usize)
    }
}

impl fmt::Display for AuditValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            AuditValue::Uint(v) => write!(f, "{v}"),
            AuditValue::Int(v) => write!(f, "{v}"),
            AuditValue::Bool(v) => f.write_str(if v { "true" } else { "false" }),
            AuditValue::Text(s) => f.write_str(s),
            AuditValue::Absent => f.write_str("none"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for AuditValue<'_> {
    fn format(&self, f: defmt::Formatter<'_>) {
        match *self {
            AuditValue::Uint(v) => defmt::write!(f, "{=u64}", v),
            AuditValue::Int(v) => defmt::write!(f, "{=i64}", v),
            AuditValue::Bool(v) => defmt::write!(f, "{=bool}", v),
            AuditValue::Text(s) => defmt::write!(f, "{=str}", s),
            AuditValue::Absent => defmt::write!(f, "none"),
        }
    }
}

/// Receives the entries of an audit report (see the [module docs](self) for the format).
///
/// Implement it to store, transmit or digest the report without formatting text (e.g. send
/// each entry over a diagnostic link, or compare against expected values).
pub trait AuditSink {
    /// One entry: `section.key = value`.
    fn entry(&mut self, section: &str, key: &str, value: AuditValue<'_>);
}

/// A configuration that can describe itself to an [`AuditSink`].
pub trait Audit {
    /// Visits every entry, in a stable order. Never allocates; never panics.
    fn audit(&self, out: &mut dyn AuditSink);
}

/// The CRC-32 ([`Crc32`](crate::crc::Crc32)) of the text [`KeyValues`] writes for `report`
/// (every `section.key=value` line with its line break, in visiting order), computed without
/// building the text: a short configuration digest to log, display on a service screen or
/// compare with the reviewed configuration's. Never allocates; links the value formatting.
///
/// ```
/// use twine_core::Limits;
/// use twine_core::audit::{KeyValues, digest};
/// use twine_core::crc::Crc32;
///
/// let text = KeyValues(&Limits::DEFAULT).to_string();
/// assert_eq!(digest(&Limits::DEFAULT), Crc32::of(text.as_bytes()));
/// assert_ne!(digest(&Limits::DEFAULT), digest(&Limits::DEFAULT.max_nodes(10)));
/// ```
#[must_use]
pub fn digest<T: Audit + ?Sized>(report: &T) -> crate::crc::Crc32 {
    /// Feeds each line into the checksum.
    struct Lines(crate::crc::Crc32);
    impl AuditSink for Lines {
        fn entry(&mut self, section: &str, key: &str, value: AuditValue<'_>) {
            // Writing into a checksum never fails.
            let _ = fmt::Write::write_fmt(&mut self.0, format_args!("{section}.{key}={value}\n"));
        }
    }
    let mut lines = Lines(crate::crc::Crc32::new());
    report.audit(&mut lines);
    lines.0
}

/// The `section.key=value` text of an [`Audit`] value (one line per entry): `Display`, and
/// `defmt::Format` with the `defmt` feature.
///
/// ```
/// use twine_core::Limits;
/// use twine_core::audit::KeyValues;
///
/// let text = KeyValues(&Limits::DEFAULT).to_string();
/// assert!(text.lines().any(|l| l == "limits.max_nodes=65535"));
/// ```
pub struct KeyValues<'a, T: Audit + ?Sized>(pub &'a T);

impl<T: Audit + ?Sized> fmt::Display for KeyValues<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// Writes each entry as a line; keeps the first error.
        struct Lines<'a, 'b> {
            f: &'a mut fmt::Formatter<'b>,
            result: fmt::Result,
        }
        impl AuditSink for Lines<'_, '_> {
            fn entry(&mut self, section: &str, key: &str, value: AuditValue<'_>) {
                if self.result.is_ok() {
                    self.result = writeln!(self.f, "{section}.{key}={value}");
                }
            }
        }
        let mut lines = Lines { f, result: Ok(()) };
        self.0.audit(&mut lines);
        lines.result
    }
}

impl<T: Audit + ?Sized> fmt::Debug for KeyValues<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(feature = "defmt")]
impl<T: Audit + ?Sized> defmt::Format for KeyValues<'_, T> {
    fn format(&self, f: defmt::Formatter<'_>) {
        /// Writes each entry as a line through defmt.
        struct Lines<'a>(defmt::Formatter<'a>);
        impl AuditSink for Lines<'_> {
            fn entry(&mut self, section: &str, key: &str, value: AuditValue<'_>) {
                defmt::write!(self.0, "{=str}.{=str}={}\n", section, key, value);
            }
        }
        self.0.audit(&mut Lines(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    struct Sample;
    impl Audit for Sample {
        fn audit(&self, out: &mut dyn AuditSink) {
            out.entry("a", "n", AuditValue::Uint(1));
            out.entry("a", "i", AuditValue::Int(-2));
            out.entry("b", "flag", AuditValue::Bool(false));
            out.entry("b", "name", AuditValue::Text("x"));
            out.entry("b", "opt", AuditValue::Absent);
        }
    }

    #[test]
    fn key_value_lines() {
        assert_eq!(
            KeyValues(&Sample).to_string(),
            "a.n=1\na.i=-2\nb.flag=false\nb.name=x\nb.opt=none\n"
        );
    }

    #[test]
    fn custom_sink_receives_every_entry() {
        struct Collect(Vec<(alloc::string::String, alloc::string::String)>);
        impl AuditSink for Collect {
            fn entry(&mut self, section: &str, key: &str, value: AuditValue<'_>) {
                self.0
                    .push((alloc::format!("{section}.{key}"), value.to_string()));
            }
        }
        let mut c = Collect(Vec::new());
        Sample.audit(&mut c);
        assert_eq!(c.0.len(), 5);
        assert_eq!(c.0[3], ("b.name".into(), "x".into()));
    }
}
