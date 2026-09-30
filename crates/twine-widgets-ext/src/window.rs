//! [`Window`]: a header (title and buttons) above a scrollable content area (LVGL `lv_win`).

use alloc::boxed::Box;

use twine_engine::{Engine, EngineError, NodeId, OBJ_FLAGS, Widget, WidgetClass, WidgetCx, fmt_node_id};
use twine_image::ImageSource;
use twine_style::{Align, CrossAlign, FlexFlow, Length, MainAlign};
use twine_text::LongMode;
use twine_widgets::button::Button;
use twine_widgets::image::Image;
use twine_widgets::label::Label;

use crate::util::{self, ClassObj, log_set};

/// The class of [`Window`]: `"win"`, the base object's parts and flags.
pub static WIN_CLASS: WidgetClass = WidgetClass::new("win")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The window's header (`"win_header"`).
pub static WIN_HEADER_CLASS: WidgetClass = WidgetClass::new("win_header")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The window's content (`"win_content"`).
pub static WIN_CONTENT_CLASS: WidgetClass = WidgetClass::new("win_content")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);

/// The default header height: half the display's DPI (LVGL `lv_win_constructor`:
/// `lv_display_get_dpi(disp) / 2`).
#[must_use]
pub fn default_header_height(dpi: u16) -> i32 {
    i32::from(dpi) / 2
}

/// A window (LVGL `lv_win`): a flex column of a header (a flex row: [`add_title`] and
/// [`add_button`] fill it) and a content area that takes the rest and scrolls. The default
/// theme draws the header grey with a small padding and the content like a screen.
///
/// [`add_title`]: Window::add_title
/// [`add_button`]: Window::add_button
///
/// ```
/// use twine_image::ImageSource;
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::window::{self, Window};
///
/// let mut h = EngineHarness::new(320, 240);
/// let screen = h.screen();
/// let w = window::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(w, |win: &mut Window, cx| {
///     win.add_title(cx, "Settings");
///     win.add_button(cx, ImageSource::symbol(twine_text::Symbol::Close), 40);
/// });
/// h.run_until_idle();
/// let win = h.engine().widget::<Window>(w).unwrap();
/// assert_eq!(h.engine().coords(win.header()).height(), 65); // 130 dpi / 2
/// ```
#[derive(Debug)]
pub struct Window {
    header: NodeId,
    content: NodeId,
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Window {
    /// A window (its header and content are created by `init`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            header: NodeId::DANGLING,
            content: NodeId::DANGLING,
        }
    }

    /// The header (LVGL `lv_win_get_header`).
    #[must_use]
    pub fn header(&self) -> NodeId {
        self.header
    }

    /// The content area (LVGL `lv_win_get_content`).
    #[must_use]
    pub fn content(&self) -> NodeId {
        self.content
    }

    /// Adds a title to the header (copied): a label taking the free width, cut with dots
    /// when too long (LVGL `lv_win_add_title`).
    pub fn add_title(&mut self, cx: &mut WidgetCx<'_>, text: &str) -> Option<NodeId> {
        log_set(WIN_CLASS.name, cx.node(), "add_title");
        let e = cx.engine_mut();
        let t = e.create(self.header, Box::new(Label::new(""))).ok()?;
        init_title(e, t);
        e.with_widget_mut(t, |l: &mut Label, cx| l.set_text(cx, text));
        Some(t)
    }

    /// Adds a button `width` px wide and as high as the header with `icon` centered (LVGL
    /// `lv_win_add_button`).
    pub fn add_button(&mut self, cx: &mut WidgetCx<'_>, icon: ImageSource, width: i32) -> Option<NodeId> {
        log_set(WIN_CLASS.name, cx.node(), "add_button");
        let e = cx.engine_mut();
        let b = e.create(self.header, Box::new(Button::new())).ok()?;
        e.set_size(b, width, Length::pct(100));
        let img = e.create(b, Box::new(Image::new())).ok()?;
        e.with_widget_mut(img, |i: &mut Image, cx| i.set_src(cx, icon));
        e.align(img, Align::Center, 0, 0);
        Some(b)
    }

    /// The header height (default `dpi / 2`). Idempotent.
    pub fn set_header_height(&mut self, cx: &mut WidgetCx<'_>, h: i32) {
        let header = self.header;
        let e = cx.engine_mut();
        if e.style_prop(header, twine_style::Part::Main, twine_style::PropId::Height)
            .as_length()
            == Some(Length::Px(h))
        {
            return;
        }
        log_set(WIN_CLASS.name, cx.node(), "header_height");
        cx.engine_mut().set_height(header, h);
    }
}

/// Sets up a window title label: one line cut with dots when too long, taking the free
/// width (LVGL sets only the `Dots` long mode, so a content-high title wraps instead).
pub fn init_title(e: &mut Engine, label: NodeId) {
    e.with_widget_mut(label, |l: &mut Label, cx| {
        l.set_long_mode(cx, LongMode::Dots);
        l.set_max_lines(cx, 1);
    });
    e.set_flex_grow(label, 1);
}

/// Creates a window, 100 % × 100 % of its parent, as the last child of `parent` (LVGL
/// `lv_win_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Window::new()))
}

impl Widget for Window {
    fn class(&self) -> &'static WidgetClass {
        &WIN_CLASS
    }

    /// LVGL `lv_win_constructor`: a flex column, the header `dpi / 2` high (a flex row,
    /// items centered vertically) and the content taking the rest.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let win = cx.node();
        let dpi = util::display_dpi(cx.engine(), win);
        let e = cx.engine_mut();
        e.set_size(win, Length::pct(100), Length::pct(100));
        util::set_flex(e, win, FlexFlow::COLUMN);
        let (Ok(header), Ok(content)) = (
            e.create(win, Box::new(ClassObj(&WIN_HEADER_CLASS))),
            e.create(win, Box::new(ClassObj(&WIN_CONTENT_CLASS))),
        ) else {
            twine_core::warn!(target: "twine::engine", "win {}: cannot create its parts", fmt_node_id(win));
            return;
        };
        e.set_size(header, Length::pct(100), default_header_height(dpi));
        util::set_flex(e, header, FlexFlow::ROW);
        e.set_flex_align(header, MainAlign::Start, CrossAlign::Center, MainAlign::Center);
        e.set_flex_grow(content, 1);
        e.set_width(content, Length::pct(100));
        self.header = header;
        self.content = content;
    }
}
