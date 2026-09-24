//! Widget views: constructor functions returning [`WidgetView`](crate::WidgetView)s with the
//! widget-specific builder methods.

mod core;

pub use self::core::{button, image, label};
