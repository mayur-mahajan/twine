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
