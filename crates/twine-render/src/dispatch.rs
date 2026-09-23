//! The single place where a runtime [`ColorFormat`] selects the monomorphized drawing code.
//!
//! [`dispatch_format!`] expands to a `match` with one arm per enabled `color-*` feature
//! (`Argb8888` is always compiled because layers need it). A format whose feature is disabled
//! logs a warning once and draws nothing.

use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::ColorFormat;

/// One "already warned" flag per LVGL format discriminant (all are `< 0x20`).
static WARNED: [AtomicBool; 32] = [const { AtomicBool::new(false) }; 32];

/// Logs (once per format) that drawing into `format` is disabled, and does nothing else.
#[cold]
pub(crate) fn format_disabled(format: ColorFormat) {
    let flag = &WARNED[usize::from(format as u8) & 31];
    if !flag.load(Ordering::Relaxed) {
        flag.store(true, Ordering::Relaxed);
        twine_core::warn!(
            target: "twine::render",
            "drawing into {} is disabled (enable the matching `color-*` feature of twine-render); nothing drawn",
            format
        );
    }
}

/// Logs a warning only the first time `flag` is seen (no CAS; fine for single-context use).
pub(crate) fn warn_once(flag: &AtomicBool) -> bool {
    if flag.load(Ordering::Relaxed) {
        false
    } else {
        flag.store(true, Ordering::Relaxed);
        true
    }
}

/// `dispatch_format!(format, F => expr)`: evaluates `expr` with the type alias `F` bound to the
/// [`PixelFormat`](twine_core::PixelFormat) of `format`. Formats that are not byte-aligned
/// pixel formats (e.g. `I1`) or whose feature is disabled call
/// [`format_disabled`] (the expression must then have type `()`), or evaluate the explicit
/// `else => fallback` expression.
macro_rules! dispatch_format {
    ($format:expr, $F:ident => $body:expr) => {
        $crate::dispatch::dispatch_format!($format, $F => $body, else => |f| $crate::dispatch::format_disabled(f))
    };
    ($format:expr, $F:ident => $body:expr, else => |$f:ident| $fallback:expr) => {
        match $format {
            #[cfg(feature = "color-rgb565")]
            twine_core::ColorFormat::Rgb565 => {
                type $F = twine_core::color::Rgb565;
                $body
            }
            #[cfg(feature = "color-rgb565-swapped")]
            twine_core::ColorFormat::Rgb565Swapped => {
                type $F = twine_core::color::Rgb565Swapped;
                $body
            }
            #[cfg(feature = "color-rgb888")]
            twine_core::ColorFormat::Rgb888 => {
                type $F = twine_core::color::Rgb888;
                $body
            }
            #[cfg(feature = "color-xrgb8888")]
            twine_core::ColorFormat::Xrgb8888 => {
                type $F = twine_core::color::Xrgb8888;
                $body
            }
            twine_core::ColorFormat::Argb8888 => {
                type $F = twine_core::color::Argb8888;
                $body
            }
            #[cfg(feature = "color-l8")]
            twine_core::ColorFormat::L8 => {
                type $F = twine_core::color::L8;
                $body
            }
            $f => $fallback,
        }
    };
}
pub(crate) use dispatch_format;

/// Whether drawing into `format` is compiled in.
///
/// ```
/// use twine_core::ColorFormat;
/// assert!(twine_render::is_format_enabled(ColorFormat::Argb8888)); // always
/// assert!(!twine_render::is_format_enabled(ColorFormat::A8)); // never a draw format
/// ```
#[must_use]
#[allow(clippy::match_like_matches_macro)] // one arm per feature reads better
pub const fn is_format_enabled(format: ColorFormat) -> bool {
    match format {
        ColorFormat::Rgb565 => cfg!(feature = "color-rgb565"),
        ColorFormat::Rgb565Swapped => cfg!(feature = "color-rgb565-swapped"),
        ColorFormat::Rgb888 => cfg!(feature = "color-rgb888"),
        ColorFormat::Xrgb8888 => cfg!(feature = "color-xrgb8888"),
        ColorFormat::Argb8888 => true,
        ColorFormat::L8 => cfg!(feature = "color-l8"),
        ColorFormat::I1 => cfg!(feature = "color-i1"),
        _ => false,
    }
}
