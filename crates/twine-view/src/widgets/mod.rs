//! Widget views: constructor functions returning [`WidgetView`](crate::WidgetView)s with the
//! widget-specific builder methods.

mod controls;
mod core;
mod images;
mod lists;
mod menus;
mod selection;
mod span;
mod tabs;
mod text_input;
#[cfg(feature = "vector")]
mod vector;
mod windows;

pub use self::controls::{arc, bar, checkbox, led, line, line_static, slider, spinner, switch};
pub use self::core::{button, image, label};
pub use self::images::{animimg, image_button};
pub use self::lists::{list, list_button, list_text};
pub use self::menus::{MenuPageRef, MenuPageView, menu, menu_cont, menu_page, menu_section, menu_separator};
pub use self::selection::{dropdown, roller};
pub use self::span::{SpanView, span, spangroup};
pub use self::tabs::{TabView, TilePos, TileView, tab, tabview, tile, tileview};
pub use self::text_input::{Btn, btn, buttonmatrix, keyboard, spinbox, textarea};
#[cfg(feature = "vector")]
pub use self::vector::{VectorCanvas, vector_canvas};
pub use self::windows::{msgbox, window, window_button};

/// An `i32` count of the view API (`what` = the setter) as the widget's `u16`: an out-of-range
/// value is warned about and clamped into `0..=u16::MAX` (P7). In range: one comparison.
#[inline]
pub(crate) fn count_u16(what: &'static str, v: i32) -> u16 {
    match u16::try_from(v) {
        Ok(n) => n,
        Err(_) => u16::try_from(out_of_range(what, v, 0, i32::from(u16::MAX))).unwrap_or(0),
    }
}

/// An `i32` count of the view API as the widget's `u32`, at least `min`: a smaller value is
/// warned about and clamped to `min` (P7). In range: one comparison.
#[inline]
pub(crate) fn count_u32(what: &'static str, v: i32, min: u32) -> u32 {
    match u32::try_from(v) {
        Ok(n) if n >= min => n,
        _ => {
            let lo = i32::try_from(min).unwrap_or(i32::MAX);
            u32::try_from(out_of_range(what, v, lo, i32::MAX)).unwrap_or(min)
        }
    }
}

/// The out-of-range path of the count conversions: warns (no allocation) and clamps.
#[cold]
#[inline(never)]
fn out_of_range(what: &'static str, v: i32, lo: i32, hi: i32) -> i32 {
    let c = v.clamp(lo, hi);
    twine_core::warn!(target: "twine::view", "{} was {}, setting it to {}", what, v, c);
    c
}
