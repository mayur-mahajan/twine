//! Helpers shared by the complex widgets: DPI scaling, input-device queries, class-only
//! containers and logging.

use twine_engine::{Engine, InputKind, MeasureCx, NodeId, Widget, WidgetClass, fmt_node_id};
use twine_style::{BaseDir, FlexFlow, LayoutKind, Part, PropId};

/// LVGL `LV_DPI_DEF`: the DPI class default sizes are written for.
pub(crate) const DPI_DEF: i32 = 130;

/// The DPI of the display showing `id` (the default display's, else [`DPI_DEF`]).
pub(crate) fn display_dpi(e: &Engine, id: NodeId) -> u16 {
    e.display_of(id)
        .or_else(|| e.default_display())
        .and_then(|d| e.display_info(d))
        .map_or(DPI_DEF as u16, |i| i.dpi)
}

/// LVGL `lv_obj_set_flex_flow`: a flex layout with `flow`.
pub(crate) fn set_flex(e: &mut Engine, id: NodeId, flow: FlexFlow) {
    e.set_layout(id, LayoutKind::Flex);
    e.set_flex_flow(id, flow);
}

/// The resolution of the display showing `id` (LVGL `LV_HOR_RES` / `LV_VER_RES`; 320 × 240
/// without a display).
pub(crate) fn display_res(e: &Engine, id: NodeId) -> (i32, i32) {
    e.display_of(id)
        .or_else(|| e.default_display())
        .and_then(|d| e.display_info(d))
        .map_or((320, 240), |i| (i32::from(i.width), i32::from(i.height)))
}

/// Whether the node's `Main` part has a right-to-left base direction.
pub(crate) fn is_rtl(cx: &MeasureCx<'_>) -> bool {
    cx.style(Part::Main, PropId::BaseDir).get::<BaseDir>() == Some(BaseDir::Rtl)
}

/// The kind of the input device being processed, if any.
pub(crate) fn active_input_kind(e: &Engine) -> Option<InputKind> {
    e.active_input().and_then(|i| e.input_kind(i))
}

/// Whether the press being released turned into a scroll (LVGL
/// `lv_indev_get_scroll_obj(indev) != NULL`).
pub(crate) fn press_scrolled(e: &Engine) -> bool {
    e.active_input().and_then(|i| e.input_scroll_obj(i)).is_some()
}

/// Whether the group of `id` is in edit mode.
pub(crate) fn editing(e: &Engine, id: NodeId) -> bool {
    e.group_of(id).is_some_and(|g| e.group_editing(g))
}

/// Calls `f` with the widget `W` of `id` unless that widget is busy (its own `event` or a
/// setter is running, e.g. a child's event raised by that setter: the setter is doing the
/// work then). Returns `None` when skipped.
pub(crate) fn with_ready<W: Widget, R>(
    e: &mut Engine,
    id: NodeId,
    f: impl FnOnce(&mut W, &mut twine_engine::WidgetCx<'_>) -> R,
) -> Option<R> {
    e.widget::<W>(id)?;
    e.with_widget_mut(id, f)
}

/// A plain container of its own class: the building block of composite widgets (LVGL
/// `lv_obj` subclasses without data, e.g. a window's header or a menu's section).
#[derive(Clone, Copy, Debug)]
pub struct ClassObj(pub &'static WidgetClass);

impl Widget for ClassObj {
    fn class(&self) -> &'static WidgetClass {
        self.0
    }
}

/// Logs a setter change (`trace!` at `twine::engine`).
#[inline]
pub(crate) fn log_set(class: &str, id: NodeId, name: &str) {
    twine_core::trace!(target: "twine::engine", "{}#{} set_{}", class, fmt_node_id(id), name);
    let _ = (class, id, name);
}

/// Logs a value change (`debug!` at `twine::engine`).
#[inline]
pub(crate) fn log_value(class: &str, id: NodeId, v: i32) {
    twine_core::debug!(target: "twine::engine", "{}#{} value {}", class, fmt_node_id(id), v);
    let _ = (class, id, v);
}
