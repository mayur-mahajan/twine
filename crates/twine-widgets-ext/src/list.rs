//! [`List`]: a scrollable column of text headers ([`add_text`]) and buttons with an icon and a
//! text ([`add_button`]) (LVGL `lv_list`).

use alloc::boxed::Box;

use twine_engine::{Engine, EngineError, GroupDef, NodeId, OBJ_FLAGS, Widget, WidgetClass, WidgetCx};
use twine_image::ImageSource;
use twine_style::{FlexFlow, Length};
use twine_text::LongMode;
use twine_widgets::button::{BUTTON_CLASS, Button};
use twine_widgets::image::Image;
use twine_widgets::label::{LABEL_CLASS, Label};

use crate::util;

/// The class of [`List`]: `"list"`, the base object's parts and flags (LVGL `lv_list_class`).
pub static LIST_CLASS: WidgetClass = WidgetClass::new("list")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);

/// The class of a list's text headers: `"list_text"`, a [`Label`] (LVGL
/// `lv_list_text_class`).
pub static LIST_TEXT_CLASS: WidgetClass = WidgetClass::new("list_text")
    .parts(LABEL_CLASS.parts)
    .default_flags(LABEL_CLASS.default_flags);

/// The class of a list's buttons: `"list_button"`, a [`Button`] (LVGL `lv_list_button_class`).
pub static LIST_BUTTON_CLASS: WidgetClass = WidgetClass::new("list_button")
    .parts(BUTTON_CLASS.parts)
    .default_flags(BUTTON_CLASS.default_flags)
    .group_def(GroupDef::True);

/// LVGL `lv_list_class.width_def`: `LV_DPI_DEF * 3 / 2`.
pub const LIST_DEFAULT_WIDTH: i32 = util::DPI_DEF * 3 / 2;
/// LVGL `lv_list_class.height_def`: `LV_DPI_DEF * 2`.
pub const LIST_DEFAULT_HEIGHT: i32 = util::DPI_DEF * 2;

/// A list (LVGL `lv_list`): a container laid out as a flex column that scrolls vertically.
/// Its items are ordinary nodes: text headers are [`Label`]s of class `"list_text"`, buttons
/// are [`Button`]s of class `"list_button"` holding an optional [`Image`] and a [`Label`]
/// (both scroll circularly when too long). The default theme draws a card without vertical
/// padding, buttons with a bottom border, and headers on grey.
///
/// ```
/// use twine_image::ImageSource;
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::list;
///
/// let mut h = EngineHarness::new(240, 320);
/// let screen = h.screen();
/// let e = h.engine_mut();
/// let l = list::create(e, screen).unwrap();
/// list::add_text(e, l, "File").unwrap();
/// let b = list::add_button(e, l, Some(ImageSource::symbol(twine_text::Symbol::Save)), "Save").unwrap();
/// assert_eq!(list::button_text(h.engine(), b), Some("Save"));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct List;

impl Widget for List {
    fn class(&self) -> &'static WidgetClass {
        &LIST_CLASS
    }

    /// LVGL `lv_list_create`: `LV_DPI_DEF × 1.5` by `LV_DPI_DEF × 2`, a flex column.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, LIST_DEFAULT_WIDTH, LIST_DEFAULT_HEIGHT);
        util::set_flex(e, id, FlexFlow::COLUMN);
    }
}

/// Creates a list as the last child of `parent` (LVGL `lv_list_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(List))
}

/// The label of a list text: 100 % wide, content high, scrolling circularly when too long.
#[must_use]
pub fn text_label(text: &'static str) -> Label {
    Label::new(text).with_class(&LIST_TEXT_CLASS)
}

/// Sets up a new list text node (size and long mode).
pub fn init_text(e: &mut Engine, id: NodeId) {
    e.set_size(id, Length::pct(100), Length::Content);
    e.with_widget_mut(id, |l: &mut Label, cx| {
        l.set_long_mode(cx, LongMode::ScrollCircular);
    });
}

/// Sets up a new list button node: 100 % wide, content high, a flex row (LVGL
/// `lv_list_add_button`).
pub fn init_button(e: &mut Engine, id: NodeId) {
    e.set_size(id, Length::pct(100), Length::Content);
    util::set_flex(e, id, FlexFlow::ROW);
}

/// Sets up the label of a list button: scrolling circularly, taking the rest of the row.
pub fn init_button_label(e: &mut Engine, id: NodeId) {
    e.with_widget_mut(id, |l: &mut Label, cx| {
        l.set_long_mode(cx, LongMode::ScrollCircular);
    });
    e.set_flex_grow(id, 1);
}

/// Adds a text header to `list` (LVGL `lv_list_add_text`; the text is copied).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `list` does not exist (logged).
pub fn add_text(e: &mut Engine, list: NodeId, text: &str) -> Result<NodeId, EngineError> {
    let id = e.create(list, Box::new(text_label("")))?;
    init_text(e, id);
    e.with_widget_mut(id, |l: &mut Label, cx| l.set_text(cx, text));
    Ok(id)
}

/// Adds a button with an optional icon (a symbol or an image) and a text to `list` (LVGL
/// `lv_list_add_button`; the text is copied).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `list` does not exist (logged).
pub fn add_button(
    e: &mut Engine,
    list: NodeId,
    icon: Option<ImageSource>,
    text: &str,
) -> Result<NodeId, EngineError> {
    let btn = e.create(list, Box::new(Button::with_class(&LIST_BUTTON_CLASS)))?;
    init_button(e, btn);
    if let Some(src) = icon {
        let img = e.create(btn, Box::new(Image::new()))?;
        e.with_widget_mut(img, |i: &mut Image, cx| i.set_src(cx, src));
    }
    let lbl = e.create(btn, Box::new(Label::new("")))?;
    e.with_widget_mut(lbl, |l: &mut Label, cx| l.set_text(cx, text));
    init_button_label(e, lbl);
    Ok(btn)
}

/// The label of a list button: its first `"label"` child.
#[must_use]
pub fn button_label(e: &Engine, btn: NodeId) -> Option<NodeId> {
    e.tree().children(btn).find(|&c| e.widget::<Label>(c).is_some())
}

/// The text of a list button (LVGL `lv_list_get_button_text`).
#[must_use]
pub fn button_text(e: &Engine, btn: NodeId) -> Option<&str> {
    button_label(e, btn)
        .and_then(|l| e.widget::<Label>(l))
        .map(Label::text)
}

/// Replaces the text of a list button (LVGL `lv_list_set_button_text`). Returns `false`
/// (logged) when `btn` has no label.
pub fn set_button_text(e: &mut Engine, btn: NodeId, text: &str) -> bool {
    let Some(l) = button_label(e, btn) else {
        twine_core::warn!(target: "twine::engine", "list: {} has no label", twine_engine::fmt_node_id(btn));
        return false;
    };
    e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, text));
    true
}
