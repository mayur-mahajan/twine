//! [`Msgbox`]: a message box with an optional header (title and buttons), a text content and
//! a footer of buttons, modal on the top layer when created without a parent (LVGL
//! `lv_msgbox`, 9.x structure).
//!
//! ```text
//! msgbox_backdrop (top layer, 100 % × 100 %; only for modal message boxes)
//! └── msgbox (flex column, 2 × LV_DPI_DEF wide, centered)
//!     ├── msgbox_header (title, msgbox_header_buttons)   — with a title or header button
//!     ├── msgbox_content (texts)
//!     └── msgbox_footer (msgbox_footer_buttons)          — with a footer button
//! ```

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::Duration;
use twine_engine::{
    Engine, EngineError, Event, EventCode, EventCx, EventFilter, EventResult, GroupDef, GroupId, InputId,
    NodeId, OBJ_FLAGS, ObjFlags, Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_image::ImageSource;
use twine_style::{Align, FlexAlign, FlexFlow, Length, Part, PropId};
use twine_text::symbols;
use twine_widgets::button::{BUTTON_CLASS, Button};
use twine_widgets::image::Image;
use twine_widgets::label::Label;

use crate::util::{self, ClassObj, log_set};

/// The class of [`Msgbox`]: `"msgbox"`.
pub static MSGBOX_CLASS: WidgetClass = WidgetClass::new("msgbox")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The modal backdrop (`"msgbox_backdrop"`): clickable (it takes the input meant for what
/// is below) but not scrollable (a focused button of the box never scrolls it away).
pub static MSGBOX_BACKDROP_CLASS: WidgetClass = WidgetClass::new("msgbox_backdrop")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS.difference(ObjFlags::SCROLLABLE));
/// The header (`"msgbox_header"`).
pub static MSGBOX_HEADER_CLASS: WidgetClass = WidgetClass::new("msgbox_header")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS.difference(ObjFlags::SCROLLABLE));
/// The content (`"msgbox_content"`).
pub static MSGBOX_CONTENT_CLASS: WidgetClass = WidgetClass::new("msgbox_content")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The footer (`"msgbox_footer"`).
pub static MSGBOX_FOOTER_CLASS: WidgetClass = WidgetClass::new("msgbox_footer")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS.difference(ObjFlags::SCROLLABLE));
/// A footer button (`"msgbox_footer_button"`): a [`Button`] in the default focus group.
pub static MSGBOX_FOOTER_BUTTON_CLASS: WidgetClass = WidgetClass::new("msgbox_footer_button")
    .parts(BUTTON_CLASS.parts)
    .default_flags(BUTTON_CLASS.default_flags)
    .group_def(GroupDef::True);
/// A header button (`"msgbox_header_button"`): a [`Button`] in the default focus group.
pub static MSGBOX_HEADER_BUTTON_CLASS: WidgetClass = WidgetClass::new("msgbox_header_button")
    .parts(BUTTON_CLASS.parts)
    .default_flags(BUTTON_CLASS.default_flags)
    .group_def(GroupDef::True);

/// LVGL `lv_msgbox_class.width_def`: `LV_DPI_DEF * 2`.
pub const MSGBOX_DEFAULT_WIDTH: i32 = util::DPI_DEF * 2;
/// LVGL `lv_msgbox_footer_class.height_def` and the header / header button size:
/// `LV_DPI_DEF / 3`.
pub const MSGBOX_BAR_HEIGHT: i32 = util::DPI_DEF / 3;

/// A message box (LVGL `lv_msgbox`).
///
/// - [`create`] with `None` makes it **modal**: a backdrop on the top layer (grey at 50 % in
///   the default theme) blocks the input to everything below, and the box gets a focus group
///   of its own (the default group and the keypad / encoder inputs switch to it; closing
///   restores them). The declarative `msgbox` view is shown with `show_modal`, which does the
///   same.
/// - [`close`] deletes the box (with its backdrop); [`close_async`] does it at the next
///   update, so a handler of the box's own buttons can call it.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::msgbox::{self, Msgbox};
///
/// let mut h = EngineHarness::new(320, 240);
/// let m = msgbox::create(h.engine_mut(), None).unwrap();
/// h.engine_mut().with_widget_mut(m, |b: &mut Msgbox, cx| {
///     b.add_title(cx, "Delete?");
///     b.add_text(cx, "This cannot be undone.");
///     b.add_footer_button(cx, "Yes");
///     b.add_footer_button(cx, "No");
/// });
/// h.run_until_idle();
/// msgbox::close(h.engine_mut(), m);
/// assert!(!h.engine().tree().contains(m));
/// ```
#[derive(Debug)]
pub struct Msgbox {
    header: Option<NodeId>,
    title: Option<NodeId>,
    content: NodeId,
    footer: Option<NodeId>,
    /// The backdrop created for a modal box.
    backdrop: Option<NodeId>,
    /// `(group of the box, the default group before it)` of a modal box.
    groups: Option<(GroupId, Option<GroupId>)>,
}

impl Default for Msgbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Msgbox {
    /// A message box (its content is created by `init`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            header: None,
            title: None,
            content: NodeId::DANGLING,
            footer: None,
            backdrop: None,
            groups: None,
        }
    }

    /// The header (LVGL `lv_msgbox_get_header`), once it has a title or a header button.
    #[must_use]
    pub fn header(&self) -> Option<NodeId> {
        self.header
    }

    /// The title label (LVGL `lv_msgbox_get_title`).
    #[must_use]
    pub fn title(&self) -> Option<NodeId> {
        self.title
    }

    /// The content (LVGL `lv_msgbox_get_content`).
    #[must_use]
    pub fn content(&self) -> NodeId {
        self.content
    }

    /// The footer (LVGL `lv_msgbox_get_footer`), once it has a button.
    #[must_use]
    pub fn footer(&self) -> Option<NodeId> {
        self.footer
    }

    /// The modal backdrop, for a box created without a parent.
    #[must_use]
    pub fn backdrop(&self) -> Option<NodeId> {
        self.backdrop
    }

    /// The header, created on first use (LVGL `lv_msgbox_add_title`): a flex row
    /// `dpi / 3` high, first in the box.
    fn ensure_header(&mut self, cx: &mut WidgetCx<'_>) -> Option<NodeId> {
        if let Some(h) = self.header {
            return Some(h);
        }
        let mb = cx.node();
        let dpi = util::display_dpi(cx.engine(), mb);
        let e = cx.engine_mut();
        let h = e.create(mb, Box::new(ClassObj(&MSGBOX_HEADER_CLASS))).ok()?;
        e.set_size(h, Length::pct(100), i32::from(dpi) / 3);
        util::set_flex(e, h, FlexFlow::Row);
        e.set_flex_align(h, FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);
        let first = e.tree().children(mb).next();
        if first != Some(h) {
            let _ = e.move_node(h, mb, first);
        }
        self.header = Some(h);
        Some(h)
    }

    /// Sets the title (copied), creating the header (LVGL `lv_msgbox_add_title`). Returns the
    /// title label.
    pub fn add_title(&mut self, cx: &mut WidgetCx<'_>, title: &str) -> Option<NodeId> {
        log_set(MSGBOX_CLASS.name, cx.node(), "title");
        let h = self.ensure_header(cx)?;
        let e = cx.engine_mut();
        let t = if let Some(t) = self.title {
            t
        } else {
            let t = e.create(h, Box::new(Label::new(""))).ok()?;
            e.set_flex_grow(t, 1);
            self.title = Some(t);
            t
        };
        e.with_widget_mut(t, |l: &mut Label, cx| l.set_text(cx, title));
        Some(t)
    }

    /// Adds a button with an optional icon to the header (right of the title; LVGL
    /// `lv_msgbox_add_header_button`).
    pub fn add_header_button(&mut self, cx: &mut WidgetCx<'_>, icon: Option<ImageSource>) -> Option<NodeId> {
        log_set(MSGBOX_CLASS.name, cx.node(), "header_button");
        if self.header.is_none() {
            // An empty title pushes the buttons to the right (LVGL).
            self.add_title(cx, "")?;
        }
        let h = self.header?;
        let e = cx.engine_mut();
        let b = e
            .create(h, Box::new(Button::with_class(&MSGBOX_HEADER_BUTTON_CLASS)))
            .ok()?;
        e.set_size(b, MSGBOX_BAR_HEIGHT, Length::pct(100));
        if let Some(src) = icon {
            let img = e.create(b, Box::new(Image::new())).ok()?;
            e.with_widget_mut(img, |i: &mut Image, cx| i.set_src(cx, src));
            e.align(img, Align::Center, 0, 0);
        }
        Some(b)
    }

    /// Adds a text (copied) to the content (LVGL `lv_msgbox_add_text`).
    pub fn add_text(&mut self, cx: &mut WidgetCx<'_>, text: &str) -> Option<NodeId> {
        log_set(MSGBOX_CLASS.name, cx.node(), "text");
        let e = cx.engine_mut();
        let l = e.create(self.content, Box::new(Label::new(""))).ok()?;
        e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, text));
        e.set_width(l, Length::pct(100));
        Some(l)
    }

    /// The footer, created on first use: a flex row `LV_DPI_DEF / 3` high, buttons spread
    /// evenly (LVGL `lv_msgbox_add_footer_button`).
    fn ensure_footer(&mut self, cx: &mut WidgetCx<'_>) -> Option<NodeId> {
        if let Some(f) = self.footer {
            return Some(f);
        }
        let mb = cx.node();
        let e = cx.engine_mut();
        let f = e.create(mb, Box::new(ClassObj(&MSGBOX_FOOTER_CLASS))).ok()?;
        e.set_size(f, Length::pct(100), MSGBOX_BAR_HEIGHT);
        util::set_flex(e, f, FlexFlow::Row);
        e.set_flex_align(f, FlexAlign::SpaceEvenly, FlexAlign::Center, FlexAlign::Center);
        self.footer = Some(f);
        Some(f)
    }

    /// Adds a button with `text` (copied) to the footer (LVGL `lv_msgbox_add_footer_button`).
    pub fn add_footer_button(&mut self, cx: &mut WidgetCx<'_>, text: &str) -> Option<NodeId> {
        log_set(MSGBOX_CLASS.name, cx.node(), "footer_button");
        let f = self.ensure_footer(cx)?;
        let e = cx.engine_mut();
        let b = e
            .create(f, Box::new(Button::with_class(&MSGBOX_FOOTER_BUTTON_CLASS)))
            .ok()?;
        e.set_size(b, Length::Content, Length::pct(100));
        if !text.is_empty() {
            let l = e.create(b, Box::new(Label::new(""))).ok()?;
            e.with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, text));
            e.align(l, Align::Center, 0, 0);
        }
        Some(b)
    }

    /// Adds a header button with `SYMBOL_CLOSE` that closes the box (LVGL
    /// `lv_msgbox_add_close_button`).
    pub fn add_close_button(&mut self, cx: &mut WidgetCx<'_>) -> Option<NodeId> {
        let b = self.add_header_button(cx, Some(ImageSource::Symbol(symbols::CLOSE)))?;
        let mb = cx.node();
        cx.engine_mut()
            .add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                if ev.target == ev.current_target {
                    close(ecx.engine_mut(), mb);
                }
                EventResult::Continue
            });
        Some(b)
    }

    /// The footer buttons, in order.
    pub fn footer_buttons<'e>(&self, e: &'e Engine) -> impl Iterator<Item = NodeId> + 'e {
        let f = self.footer;
        f.into_iter().flat_map(move |f| e.tree().children(f))
    }
}

/// The inputs attached to group `from`, re-attached to `to`.
fn move_inputs(e: &mut Engine, from: Option<GroupId>, to: Option<GroupId>) {
    let ids: Vec<InputId> = e.inputs().filter(|&i| e.input_group(i) == from).collect();
    for i in ids {
        e.set_input_group(i, to);
    }
}

/// Creates a message box in `parent`, or — with `None` — modal: on a backdrop on the default
/// display's top layer, with a focus group of its own (LVGL `lv_msgbox_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist or there is no display (logged).
pub fn create(e: &mut Engine, parent: Option<NodeId>) -> Result<NodeId, EngineError> {
    let (parent, backdrop) = if let Some(p) = parent {
        (p, None)
    } else {
        let Some(top) = e.default_display().and_then(|d| e.top_layer(d)) else {
            twine_core::warn!(target: "twine::engine", "msgbox: no display for a modal message box");
            return Err(EngineError::InvalidConfig("no display"));
        };
        let b = e.create(top, Box::new(ClassObj(&MSGBOX_BACKDROP_CLASS)))?;
        e.set_size(b, Length::pct(100), Length::pct(100));
        (b, Some(b))
    };
    // A modal box's buttons join a group of their own.
    let groups = if backdrop.is_some() {
        let prev = e.default_group();
        e.create_group().ok().map(|g| {
            e.set_default_group(Some(g));
            move_inputs(e, prev, Some(g));
            (g, prev)
        })
    } else {
        None
    };
    let mb = e.create(parent, Box::new(Msgbox::new()))?;
    e.with_widget_mut(mb, |m: &mut Msgbox, _| {
        m.backdrop = backdrop;
        m.groups = groups;
    });
    twine_core::debug!(
        target: "twine::engine",
        "msgbox {} created{}",
        fmt_node_id(mb),
        if backdrop.is_some() { " (modal)" } else { "" }
    );
    Ok(mb)
}

/// Deletes the message box and its backdrop (LVGL `lv_msgbox_close`).
pub fn close(e: &mut Engine, mbox: NodeId) {
    let Some(target) = e
        .widget::<Msgbox>(mbox)
        .map(|m| m.backdrop.filter(|b| e.tree().contains(*b)).unwrap_or(mbox))
    else {
        twine_core::warn!(target: "twine::engine", "msgbox: {} is not a message box", fmt_node_id(mbox));
        return;
    };
    let _ = e.delete(target);
}

/// Closes the message box at the next update (LVGL `lv_msgbox_close_async`): safe inside its
/// own event handlers.
pub fn close_async(e: &mut Engine, mbox: NodeId) {
    e.timer_add(Duration::ZERO, move |e, id| {
        e.timer_remove(id);
        if e.tree().contains(mbox) {
            close(e, mbox);
        }
    });
}

impl Widget for Msgbox {
    fn class(&self) -> &'static WidgetClass {
        &MSGBOX_CLASS
    }

    /// LVGL `lv_msgbox_create`: `LV_DPI_DEF × 2` wide, content high, a flex column with the
    /// content area, centered in its parent.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let mb = cx.node();
        let e = cx.engine_mut();
        e.set_size(mb, MSGBOX_DEFAULT_WIDTH, Length::Content);
        util::set_flex(e, mb, FlexFlow::Column);
        let Ok(content) = e.create(mb, Box::new(ClassObj(&MSGBOX_CONTENT_CLASS))) else {
            return;
        };
        e.set_size(content, Length::pct(100), Length::Content);
        util::set_flex(e, content, FlexFlow::Column);
        e.align(mb, Align::Center, 0, 0);
        self.content = content;
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match ev.code {
            // LVGL `msgbox_size_changed_event_cb`: with a fixed height the content takes the
            // rest (and scrolls).
            EventCode::SizeChanged | EventCode::StyleChanged => {
                let node = cx.node();
                let e = cx.engine_mut();
                let content_h =
                    e.style_prop(node, Part::Main, PropId::Height).as_length() == Some(Length::Content);
                if e.tree().contains(self.content) {
                    e.set_flex_grow(self.content, u8::from(!content_h));
                }
            }
            EventCode::Delete => {
                if let Some((g, prev)) = self.groups.take() {
                    let e = cx.engine_mut();
                    move_inputs(e, Some(g), prev);
                    if e.default_group() == Some(g) {
                        e.set_default_group(prev);
                    }
                    e.delete_group(g);
                }
            }
            _ => {}
        }
        EventResult::Continue
    }
}
