//! [`Container`]: the base object (LVGL `lv_obj`), a styled rectangle holding children.
//!
//! The engine's [`Obj`](twine_engine::Obj) is the container: class `"obj"`, parts `Main`
//! and `Scrollbar`, and LVGL's `lv_obj` default flags (`CLICKABLE`, `CLICK_FOCUSABLE`,
//! `SCROLLABLE`, `SCROLL_ELASTIC`, `SCROLL_MOMENTUM`, `SCROLL_WITH_ARROW`, `SCROLL_CHAIN`,
//! `SCROLL_ON_FOCUS`, `SNAPPABLE`, `PRESS_LOCK`, `GESTURE_BUBBLE`). The default theme draws it
//! as a card (white or dark background, grey border, rounded corners, padding).

use alloc::boxed::Box;

use twine_engine::{Engine, EngineError, NodeId};

pub use twine_engine::{OBJ_CLASS as CONTAINER_CLASS, Obj as Container};

/// Creates a container as the last child of `parent` (LVGL `lv_obj_create`). Its size is
/// `Content` (it wraps its children) until set.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::container;
/// let mut h = EngineHarness::new(64, 48);
/// let screen = h.screen();
/// let c = container::create(h.engine_mut(), screen).unwrap();
/// assert_eq!(h.engine().tree().node(c).unwrap().class().name, "obj");
/// ```
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Container))
}
