//! Widget views: constructor functions returning [`WidgetView`](crate::WidgetView)s with the
//! widget-specific builder methods.

mod controls;
mod core;
mod images;
mod span;
mod text_input;

pub use self::controls::{arc, bar, checkbox, led, line, line_static, slider, spinner, switch};
pub use self::core::{button, image, label};
pub use self::images::{animimg, image_button};
pub use self::span::{SpanView, span, spangroup};
pub use self::text_input::{buttonmatrix, keyboard, spinbox, textarea};
