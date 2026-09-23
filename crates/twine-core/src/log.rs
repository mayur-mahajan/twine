//! Logging facade: [`trace!`], [`debug!`], [`info!`], [`warn!`] and [`error!`].
//!
//! The same call syntax works with every backend:
//!
//! ```
//! use twine_core::log::{debug, warn};
//!
//! let id = 7;
//! warn!(target: "twine::engine", "node {} not found", id);
//! debug!("frame done");
//! ```
//!
//! # Backends
//!
//! The backend is chosen by cargo features of `twine-core` (the choice is made where the macros
//! are *defined*, so it applies uniformly to every crate that uses them):
//!
//! | Features            | Backend | Behaviour |
//! |---------------------|---------|-----------|
//! | none                | `none`  | arguments are type-checked but never evaluated; no code is emitted |
//! | `log`               | `log`   | forwards to [`log::log!`](https://docs.rs/log) with the given target (default `"twine"`) |
//! | `defmt` (± `log`)   | `defmt` | forwards to `defmt`; **`defmt` wins when both are enabled** |
//!
//! With `defmt` the `target:` is **dropped** (defmt needs literal format strings and records the
//! module path itself); messages that need an area start with a short `[area]` prefix instead.
//! Because defmt's macros expand to `::defmt::…` paths, every crate that enables its own `defmt`
//! feature must also depend on `defmt` directly:
//! `defmt = ["dep:defmt", "twine-core/defmt"]`.
//!
//! Only `{}` and `{:?}` placeholders are portable across backends; every logged type implements
//! `Display`/`Debug` and, with `defmt`, `defmt::Format`.
//!
//! # Level policy
//!
//! - `error!`: invariant violation recovered from, effect loop limit, driver error.
//! - `warn!`: invalid user input (unknown node id, bad style value, missing glyph first time per
//!   font/char, buffer too small, dropped channel messages).
//! - `info!`: lifecycle (Ui built with config summary, display added, screen loaded), perf summary
//!   every 5 s.
//! - `debug!`: per-frame summary (areas, px, render/flush µs), layout passes, scope dispose.
//! - `trace!`: per-invalidation with reason, per-event dispatch, per-effect run.
//!
//! Targets are `"twine::<area>"`; use exactly the strings in [`TARGETS`].

/// The allowed log target strings (`docs/design/10-simulator-testing.md` §4).
pub const TARGETS: &[&str] = &[
    "twine::core",
    "twine::reactive",
    "twine::render",
    "twine::text",
    "twine::image",
    "twine::style",
    "twine::layout",
    "twine::engine",
    "twine::input",
    "twine::event",
    "twine::refresh",
    "twine::anim",
    "twine::view",
    "twine::driver",
    "twine::perf",
    "twine::sim",
    "twine::fs",
    "twine::vector",
    "twine::lottie",
    "twine::extra",
    "twine::widgets",
    "twine::theme",
];

/// The compiled logging backend: `"defmt"`, `"log"` or `"none"`.
#[cfg(feature = "defmt")]
pub const BACKEND: &str = "defmt";
/// The compiled logging backend: `"defmt"`, `"log"` or `"none"`.
#[cfg(all(feature = "log", not(feature = "defmt")))]
pub const BACKEND: &str = "log";
/// The compiled logging backend: `"defmt"`, `"log"` or `"none"`.
#[cfg(not(any(feature = "log", feature = "defmt")))]
pub const BACKEND: &str = "none";

/// Whether a logging backend is compiled in (`BACKEND != "none"`). For tests.
#[doc(hidden)]
#[must_use]
pub const fn __log_enabled() -> bool {
    cfg!(any(feature = "log", feature = "defmt"))
}

pub use crate::{debug, error, info, trace, warn};

#[cfg(feature = "defmt")]
mod imp_defmt {
    /// Logs at trace level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! trace {
        (target: $target:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::trace!($fmt $(, $arg)*)
        };
        ($fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::trace!($fmt $(, $arg)*)
        };
    }
    /// Logs at debug level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! debug {
        (target: $target:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::debug!($fmt $(, $arg)*)
        };
        ($fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::debug!($fmt $(, $arg)*)
        };
    }
    /// Logs at info level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! info {
        (target: $target:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::info!($fmt $(, $arg)*)
        };
        ($fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::info!($fmt $(, $arg)*)
        };
    }
    /// Logs at warn level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! warn {
        (target: $target:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::warn!($fmt $(, $arg)*)
        };
        ($fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::warn!($fmt $(, $arg)*)
        };
    }
    /// Logs at error level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! error {
        (target: $target:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::error!($fmt $(, $arg)*)
        };
        ($fmt:literal $(, $arg:expr)* $(,)?) => {
            $crate::__private::defmt::error!($fmt $(, $arg)*)
        };
    }
}

#[cfg(all(feature = "log", not(feature = "defmt")))]
mod imp_log {
    /// Logs at trace level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! trace {
        (target: $target:expr, $($arg:tt)+) => {
            $crate::__private::log::log!(target: $target, $crate::__private::log::Level::Trace, $($arg)+)
        };
        ($($arg:tt)+) => {
            $crate::__private::log::log!(target: "twine", $crate::__private::log::Level::Trace, $($arg)+)
        };
    }
    /// Logs at debug level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! debug {
        (target: $target:expr, $($arg:tt)+) => {
            $crate::__private::log::log!(target: $target, $crate::__private::log::Level::Debug, $($arg)+)
        };
        ($($arg:tt)+) => {
            $crate::__private::log::log!(target: "twine", $crate::__private::log::Level::Debug, $($arg)+)
        };
    }
    /// Logs at info level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! info {
        (target: $target:expr, $($arg:tt)+) => {
            $crate::__private::log::log!(target: $target, $crate::__private::log::Level::Info, $($arg)+)
        };
        ($($arg:tt)+) => {
            $crate::__private::log::log!(target: "twine", $crate::__private::log::Level::Info, $($arg)+)
        };
    }
    /// Logs at warn level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! warn {
        (target: $target:expr, $($arg:tt)+) => {
            $crate::__private::log::log!(target: $target, $crate::__private::log::Level::Warn, $($arg)+)
        };
        ($($arg:tt)+) => {
            $crate::__private::log::log!(target: "twine", $crate::__private::log::Level::Warn, $($arg)+)
        };
    }
    /// Logs at error level (see the [module docs](crate::log)).
    #[macro_export]
    macro_rules! error {
        (target: $target:expr, $($arg:tt)+) => {
            $crate::__private::log::log!(target: $target, $crate::__private::log::Level::Error, $($arg)+)
        };
        ($($arg:tt)+) => {
            $crate::__private::log::log!(target: "twine", $crate::__private::log::Level::Error, $($arg)+)
        };
    }
}

#[cfg(not(any(feature = "log", feature = "defmt")))]
mod imp_none {
    /// Logs at trace level (see the [module docs](crate::log)). Compiled out: arguments are
    /// type-checked but not evaluated.
    #[macro_export]
    macro_rules! trace {
        (target: $target:expr, $($arg:tt)+) => {{
            if false {
                let _ = $target;
                let _ = ::core::format_args!($($arg)+);
            }
        }};
        ($($arg:tt)+) => {{
            if false {
                let _ = ::core::format_args!($($arg)+);
            }
        }};
    }
    /// Logs at debug level (see the [module docs](crate::log)). Compiled out: arguments are
    /// type-checked but not evaluated.
    #[macro_export]
    macro_rules! debug {
        (target: $target:expr, $($arg:tt)+) => {{
            if false {
                let _ = $target;
                let _ = ::core::format_args!($($arg)+);
            }
        }};
        ($($arg:tt)+) => {{
            if false {
                let _ = ::core::format_args!($($arg)+);
            }
        }};
    }
    /// Logs at info level (see the [module docs](crate::log)). Compiled out: arguments are
    /// type-checked but not evaluated.
    #[macro_export]
    macro_rules! info {
        (target: $target:expr, $($arg:tt)+) => {{
            if false {
                let _ = $target;
                let _ = ::core::format_args!($($arg)+);
            }
        }};
        ($($arg:tt)+) => {{
            if false {
                let _ = ::core::format_args!($($arg)+);
            }
        }};
    }
    /// Logs at warn level (see the [module docs](crate::log)). Compiled out: arguments are
    /// type-checked but not evaluated.
    #[macro_export]
    macro_rules! warn {
        (target: $target:expr, $($arg:tt)+) => {{
            if false {
                let _ = $target;
                let _ = ::core::format_args!($($arg)+);
            }
        }};
        ($($arg:tt)+) => {{
            if false {
                let _ = ::core::format_args!($($arg)+);
            }
        }};
    }
    /// Logs at error level (see the [module docs](crate::log)). Compiled out: arguments are
    /// type-checked but not evaluated.
    #[macro_export]
    macro_rules! error {
        (target: $target:expr, $($arg:tt)+) => {{
            if false {
                let _ = $target;
                let _ = ::core::format_args!($($arg)+);
            }
        }};
        ($($arg:tt)+) => {{
            if false {
                let _ = ::core::format_args!($($arg)+);
            }
        }};
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_follow_naming_convention() {
        for t in TARGETS {
            assert!(t.starts_with("twine::"), "{t}");
        }
        assert_eq!(__log_enabled(), BACKEND != "none");
    }

    #[test]
    fn macros_usable_inside_crate() {
        let x = 3;
        trace!(target: "twine::core", "x = {}", x);
        debug!("x = {:?}", x);
        info!(target: "twine::core", "plain");
        warn!("x = {} {}", x, x);
        error!(target: "twine::core", "x = {}", x);
    }
}
