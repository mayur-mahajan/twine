//! The single place where a runtime [`ColorFormat`] selects the monomorphized drawing code.
//!
//! [`dispatch_format!`] expands to a `match` with one arm per enabled `color-*` feature
//! (`Argb8888` is always compiled because layers need it). A format whose feature is disabled
//! draws nothing: [`report_format_disabled`] logs a warning and records the format once, and
//! [`take_format_disabled`] hands the record to whoever reports faults (the engine raises
//! `FaultKind::FormatDisabled` from it).
//!
//! The engine refuses displays whose draw format is not compiled in
//! (`EngineError::FormatDisabled`), so for engine-driven rendering this is a defensive
//! fallback; it stays reachable for code that draws into its own buffers (canvases,
//! embedded-graphics targets, custom renderers). It costs nothing while drawing works: the
//! enabled formats are `match` arms, and the fallback is a `#[cold]` call.

use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::ColorFormat;

/// One "already warned" flag per LVGL format discriminant (all are `< 0x20`).
static WARNED: [AtomicBool; 32] = [const { AtomicBool::new(false) }; 32];
/// One "drawn into while disabled, not yet taken" flag per format discriminant.
static UNREPORTED: [AtomicBool; 32] = [const { AtomicBool::new(false) }; 32];
/// Whether any [`UNREPORTED`] flag may be set (keeps [`take_format_disabled`] to one load).
static ANY_UNREPORTED: AtomicBool = AtomicBool::new(false);

/// Records that drawing into `format` was skipped because its renderer is not compiled in:
/// the first time for each format, logs a warning and keeps the format for
/// [`take_format_disabled`]; later calls do nothing (one relaxed load).
///
/// The software renderer calls it itself; custom renderers that dispatch on the format may
/// call it for the same effect. Plain loads and stores, no CAS (like the rest of the renderer,
/// meant for one drawing context).
///
/// ```
/// use twine_core::ColorFormat;
/// twine_render::report_format_disabled(ColorFormat::A2);
/// twine_render::report_format_disabled(ColorFormat::A2); // reported once
/// let mut taken = Vec::new();
/// while let Some(f) = twine_render::take_format_disabled() {
///     taken.push(f);
/// }
/// assert_eq!(taken.iter().filter(|&&f| f == ColorFormat::A2).count(), 1);
/// ```
#[cold]
pub fn report_format_disabled(format: ColorFormat) {
    let i = usize::from(format as u8) & 31;
    if !WARNED[i].load(Ordering::Relaxed) {
        WARNED[i].store(true, Ordering::Relaxed);
        UNREPORTED[i].store(true, Ordering::Relaxed);
        ANY_UNREPORTED.store(true, Ordering::Relaxed);
        twine_core::warn!(
            target: "twine::render",
            "drawing into {} is disabled (enable the matching `color-*` feature of twine-render); nothing drawn",
            format
        );
    }
}

/// Takes one format recorded by [`report_format_disabled`] and not taken yet, or `None`.
/// Each format is handed out at most once per process (with the warning), so a caller that
/// raises a fault for it raises it once. One relaxed load when nothing was recorded.
#[must_use]
pub fn take_format_disabled() -> Option<ColorFormat> {
    if !ANY_UNREPORTED.load(Ordering::Relaxed) {
        return None;
    }
    for (i, flag) in UNREPORTED.iter().enumerate() {
        if flag.load(Ordering::Relaxed) {
            flag.store(false, Ordering::Relaxed);
            // `i < 32`; every recorded index came from a real format.
            if let Some(f) = ColorFormat::from_u8(i as u8) {
                return Some(f);
            }
        }
    }
    ANY_UNREPORTED.store(false, Ordering::Relaxed);
    None
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
/// [`report_format_disabled`] (the expression must then have type `()`), or evaluate the explicit
/// `else => fallback` expression.
macro_rules! dispatch_format {
    ($format:expr, $F:ident => $body:expr) => {
        $crate::dispatch::dispatch_format!($format, $F => $body, else => |f| $crate::dispatch::report_format_disabled(f))
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
