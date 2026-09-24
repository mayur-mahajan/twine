//! [`Button`]: a clickable container (LVGL `lv_button`).

use alloc::boxed::Box;

use twine_engine::{
    Engine, EngineError, GroupDef, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_style::Part;

use crate::log_set;

/// Default flags of [`BUTTON_CLASS`]: the container's, minus `SCROLLABLE` (LVGL
/// `lv_button_constructor`: `lv_obj_set_scrollable(obj, false)`,
/// `lv_obj_set_scroll_on_focus(obj, true)`).
const BUTTON_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .union(ObjFlags::SCROLL_ON_FOCUS);

/// The class of [`Button`]: `"button"`, parts `Main` and `Scrollbar`, always in the default
/// focus group (LVGL `LV_OBJ_CLASS_GROUP_DEF_TRUE`).
pub static BUTTON_CLASS: WidgetClass = WidgetClass::new("button")
    .parts(&[Part::Main, Part::Scrollbar])
    .default_flags(BUTTON_FLAGS)
    .group_def(GroupDef::True);

/// A button: a container that is clickable, focusable and not scrollable. With the default
/// theme it is a primary-colored rounded rectangle with a shadow (light mode) that darkens
/// while pressed, turns secondary when checked and shows an outline on keyboard focus.
///
/// **Children keep LVGL semantics**: a button has no layout; the theme only sets paddings and
/// centered text alignment. Center a label child with `engine.align(label, Align::Center, 0,
/// 0)` (LVGL `lv_obj_center`); the declarative `button(child)` does this for its child.
///
/// A checkable button toggles `State::CHECKED` when clicked (and with the arrow keys) and
/// sends `ValueChanged` (the engine's base object behaviour, LVGL `lv_obj_event`).
///
/// ```
/// use twine_engine::ObjFlags;
/// use twine_testing::EngineHarness;
/// use twine_widgets::button::{self, Button};
///
/// let mut h = EngineHarness::new(80, 40);
/// let screen = h.screen();
/// let b = button::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(b, |w: &mut Button, cx| w.set_checkable(cx, true));
/// assert!(h.engine().has_flag(b, ObjFlags::CHECKABLE));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Button;

impl Button {
    /// A button widget (insert it with `Engine::create` or use [`create`]).
    #[must_use]
    pub const fn new() -> Self {
        Button
    }

    /// Makes the button toggle `State::CHECKED` on click (the `CHECKABLE` flag). Idempotent.
    pub fn set_checkable(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        let id = cx.node();
        if cx.engine().has_flag(id, ObjFlags::CHECKABLE) == on {
            return;
        }
        log_set(BUTTON_CLASS.name, id, "checkable");
        cx.engine_mut().set_flag(id, ObjFlags::CHECKABLE, on);
    }

    /// Whether the button is checkable.
    #[must_use]
    pub fn is_checkable(&self, cx: &MeasureCx<'_>) -> bool {
        cx.flags().contains(ObjFlags::CHECKABLE)
    }
}

impl Widget for Button {
    fn class(&self) -> &'static WidgetClass {
        &BUTTON_CLASS
    }
}

/// Creates a button as the last child of `parent` (LVGL `lv_button_create`). Its size is
/// `Content` until set.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Button))
}
