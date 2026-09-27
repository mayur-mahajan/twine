//! [`Menu`]: hierarchical pages with a header, a back button, an optional sidebar and a page
//! history (LVGL `lv_menu`).
//!
//! The node structure is LVGL's:
//!
//! ```text
//! menu (flex row)
//! ├── storage (hidden: pages not shown)
//! ├── menu_sidebar_container (with a sidebar page; 30 % wide)
//! │   ├── menu_sidebar_header_container (back button, title)
//! │   └── the sidebar page
//! └── menu_main_container (flex column)
//!     ├── menu_main_header_container (back button, title)
//!     └── the main page
//! ```

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use twine_engine::{
    Engine, EngineError, EventCode, EventFilter, EventParam, EventResult, HandlerId, NodeId, OBJ_FLAGS,
    ObjFlags, State, Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_image::ImageSource;
use twine_style::{FlexAlign, FlexFlow, Length};
use twine_text::symbols;
use twine_widgets::button::Button;
use twine_widgets::image::Image;
use twine_widgets::label::{Label, LabelText};

use crate::util::{self, ClassObj, log_set};

/// The class of [`Menu`]: `"menu"`, the base object's parts and flags.
pub static MENU_CLASS: WidgetClass = WidgetClass::new("menu")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The class of [`MenuPage`]: `"menu_page"`.
pub static MENU_PAGE_CLASS: WidgetClass = WidgetClass::new("menu_page")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// A row of a page (`"menu_cont"`): not clickable unless it loads a page.
pub static MENU_CONT_CLASS: WidgetClass = WidgetClass::new("menu_cont")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS.difference(ObjFlags::CLICKABLE));
/// A group of rows drawn as a card (`"menu_section"`).
pub static MENU_SECTION_CLASS: WidgetClass = WidgetClass::new("menu_section")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS.difference(ObjFlags::CLICKABLE));
/// A gap between rows (`"menu_separator"`).
pub static MENU_SEPARATOR_CLASS: WidgetClass = WidgetClass::new("menu_separator")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The column holding the main header and page (`"menu_main_container"`).
pub static MENU_MAIN_CONTAINER_CLASS: WidgetClass = WidgetClass::new("menu_main_container")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(
        OBJ_FLAGS
            .union(ObjFlags::EVENT_BUBBLE)
            .difference(ObjFlags::CLICKABLE),
    );
/// The main header (`"menu_main_header_container"`).
pub static MENU_MAIN_HEADER_CONTAINER_CLASS: WidgetClass = WidgetClass::new("menu_main_header_container")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(
        OBJ_FLAGS
            .union(ObjFlags::EVENT_BUBBLE)
            .difference(ObjFlags::CLICKABLE),
    );
/// The sidebar column (`"menu_sidebar_container"`).
pub static MENU_SIDEBAR_CONTAINER_CLASS: WidgetClass = WidgetClass::new("menu_sidebar_container")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(
        OBJ_FLAGS
            .union(ObjFlags::EVENT_BUBBLE)
            .difference(ObjFlags::CLICKABLE),
    );
/// The sidebar header (`"menu_sidebar_header_container"`).
pub static MENU_SIDEBAR_HEADER_CONTAINER_CLASS: WidgetClass =
    WidgetClass::new("menu_sidebar_header_container")
        .parts(twine_engine::OBJ_CLASS.parts)
        .default_flags(
            OBJ_FLAGS
                .union(ObjFlags::EVENT_BUBBLE)
                .difference(ObjFlags::CLICKABLE),
        );

/// LVGL `lv_menu_class`: `LV_DPI_DEF * 3 / 2` by `LV_DPI_DEF * 2`.
pub const MENU_DEFAULT_SIZE: (i32, i32) = (util::DPI_DEF * 3 / 2, util::DPI_DEF * 2);

/// Where the menu's headers are (LVGL `lv_menu_mode_header_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MenuHeaderMode {
    /// Above the page, fixed (the page fills the rest and scrolls).
    #[default]
    TopFixed,
    /// Above the page, scrolling away with it.
    TopUnfixed,
    /// Below the page, fixed.
    BottomFixed,
}

/// A menu page (LVGL `lv_menu_page`): a flex column with an optional title shown in the
/// header while the page is loaded.
#[derive(Debug, Default)]
pub struct MenuPage {
    title: Option<LabelText>,
}

impl MenuPage {
    /// The title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_ref().map(LabelText::as_str)
    }

    /// Sets the title (copied; LVGL `lv_menu_set_page_title`). It shows the next time the
    /// page is loaded. Idempotent.
    pub fn set_title(&mut self, cx: &mut WidgetCx<'_>, title: Option<&str>) {
        if self.title() == title {
            return;
        }
        log_set(MENU_PAGE_CLASS.name, cx.node(), "title");
        self.title = title.map(|t| LabelText::Owned(String::from(t)));
    }

    /// Sets a `'static` title (LVGL `lv_menu_set_page_title_static`). Idempotent.
    pub fn set_title_static(&mut self, cx: &mut WidgetCx<'_>, title: Option<&'static str>) {
        if matches!((&self.title, title), (Some(LabelText::Static(a)), Some(b)) if core::ptr::eq(*a, b))
            || (self.title.is_none() && title.is_none())
        {
            return;
        }
        log_set(MENU_PAGE_CLASS.name, cx.node(), "title_static");
        self.title = title.map(LabelText::Static);
    }
}

impl Widget for MenuPage {
    fn class(&self) -> &'static WidgetClass {
        &MENU_PAGE_CLASS
    }

    /// LVGL `lv_menu_page_constructor`: 100 % wide, content high, a flex column.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, Length::pct(100), Length::Content);
        util::set_flex(e, id, FlexFlow::Column);
        e.set_flex_align(id, FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);
        e.set_flag(id, ObjFlags::EVENT_BUBBLE, true);
    }
}

/// A menu (LVGL `lv_menu`). Build pages with [`page_create`], fill them with [`cont_create`]
/// rows (and [`section_create`], [`separator_create`]), link rows to pages with
/// [`set_load_page_event`] and show the root with [`Menu::set_page`]. Loading a page pushes
/// it on the history; the header's back button pops it. With a sidebar page
/// ([`Menu::set_sidebar_page`]) the menu has two columns, and the sidebar row whose page is
/// loaded is `CHECKED`. Page switches are instant (LVGL).
///
/// - **Events**: `ValueChanged` on the menu after each page change.
/// - Pages not shown live in a hidden storage node; they are deleted with the menu.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::menu::{self, Menu};
///
/// let mut h = EngineHarness::new(320, 240);
/// let screen = h.screen();
/// let e = h.engine_mut();
/// let m = menu::create(e, screen).unwrap();
/// let root = menu::page_create(e, m, None).unwrap();
/// let sub = menu::page_create(e, m, Some("Display")).unwrap();
/// let row = menu::cont_create(e, root).unwrap();
/// menu::set_load_page_event(e, m, row, sub);
/// e.with_widget_mut(m, |w: &mut Menu, cx| w.set_page(cx, Some(root)));
/// assert_eq!(h.engine().widget::<Menu>(m).unwrap().cur_main_page(), Some(root));
/// ```
#[derive(Debug)]
pub struct Menu {
    storage: NodeId,
    main: NodeId,
    main_header: NodeId,
    main_header_back_btn: NodeId,
    main_header_title: NodeId,
    sidebar: Option<Sidebar>,
    main_page: Option<NodeId>,
    sidebar_page: Option<NodeId>,
    /// Loaded pages, the current one last.
    history: Vec<NodeId>,
    prev_depth: usize,
    mode_header: MenuHeaderMode,
    mode_root_back_btn: bool,
    selected_tab: Option<NodeId>,
    /// The rows that load a page: `(row, page, click handler)`.
    load_page_map: Vec<(NodeId, NodeId, HandlerId)>,
}

/// The nodes of a sidebar.
#[derive(Clone, Copy, Debug)]
struct Sidebar {
    cont: NodeId,
    header: NodeId,
    back_btn: NodeId,
    title: NodeId,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    /// A menu (its child nodes are created by `init`).
    #[must_use]
    pub fn new() -> Self {
        let none = NodeId::DANGLING;
        Self {
            storage: none,
            main: none,
            main_header: none,
            main_header_back_btn: none,
            main_header_title: none,
            sidebar: None,
            main_page: None,
            sidebar_page: None,
            history: Vec::new(),
            prev_depth: 0,
            mode_header: MenuHeaderMode::TopFixed,
            mode_root_back_btn: false,
            selected_tab: None,
            load_page_map: Vec::new(),
        }
    }

    // ---- Getters --------------------------------------------------------------------------

    /// The page shown in the main area (LVGL `lv_menu_get_cur_main_page`).
    #[must_use]
    pub fn cur_main_page(&self) -> Option<NodeId> {
        self.main_page
    }

    /// The page shown in the sidebar (LVGL `lv_menu_get_cur_sidebar_page`).
    #[must_use]
    pub fn cur_sidebar_page(&self) -> Option<NodeId> {
        self.sidebar_page
    }

    /// The main header (LVGL `lv_menu_get_main_header`).
    #[must_use]
    pub fn main_header(&self) -> NodeId {
        self.main_header
    }

    /// The main header's back button (LVGL `lv_menu_get_main_header_back_button`).
    #[must_use]
    pub fn main_header_back_button(&self) -> NodeId {
        self.main_header_back_btn
    }

    /// The main header's title label.
    #[must_use]
    pub fn main_header_title(&self) -> NodeId {
        self.main_header_title
    }

    /// The sidebar header (LVGL `lv_menu_get_sidebar_header`), with a sidebar.
    #[must_use]
    pub fn sidebar_header(&self) -> Option<NodeId> {
        self.sidebar.map(|s| s.header)
    }

    /// The sidebar header's back button (LVGL `lv_menu_get_sidebar_header_back_button`).
    #[must_use]
    pub fn sidebar_header_back_button(&self) -> Option<NodeId> {
        self.sidebar.map(|s| s.back_btn)
    }

    /// The sidebar column, with a sidebar.
    #[must_use]
    pub fn sidebar_container(&self) -> Option<NodeId> {
        self.sidebar.map(|s| s.cont)
    }

    /// The main column (header and page).
    #[must_use]
    pub fn main_container(&self) -> NodeId {
        self.main
    }

    /// The hidden node holding the pages not shown.
    #[must_use]
    pub fn storage(&self) -> NodeId {
        self.storage
    }

    /// The loaded pages, the current one last.
    #[must_use]
    pub fn history(&self) -> &[NodeId] {
        &self.history
    }

    /// The header mode.
    #[must_use]
    pub fn mode_header(&self) -> MenuHeaderMode {
        self.mode_header
    }

    /// Whether the root page shows a back button.
    #[must_use]
    pub fn mode_root_back_button(&self) -> bool {
        self.mode_root_back_btn
    }

    /// The sidebar row whose page is loaded.
    #[must_use]
    pub fn selected_tab(&self) -> Option<NodeId> {
        self.selected_tab
    }

    /// Whether a click on `btn` goes nowhere back: the sidebar's back button, or the main
    /// back button on the root page (LVGL `lv_menu_back_button_is_root`; checked in a click
    /// handler of the back button, the depth before the click counts).
    #[must_use]
    pub fn back_button_is_root(&self, btn: NodeId) -> bool {
        if self.sidebar.is_some_and(|s| s.back_btn == btn) {
            return true;
        }
        btn == self.main_header_back_btn && self.prev_depth <= 1
    }

    // ---- Pages ----------------------------------------------------------------------------

    /// Shows `page` in the main area and pushes it on the history (`None`: shows nothing and
    /// clears the history; LVGL `lv_menu_set_page`). Updates the header title and the back
    /// buttons, marks the selected sidebar row and sends `ValueChanged`.
    pub fn set_page(&mut self, cx: &mut WidgetCx<'_>, page: Option<NodeId>) {
        log_set(MENU_CLASS.name, cx.node(), "page");
        let e = cx.engine_mut();
        if let Some(old) = self.main_page.filter(|p| e.tree().contains(*p)) {
            let _ = e.move_node(old, self.storage, None);
        }
        let page = page.filter(|p| {
            let ok = e.widget::<MenuPage>(*p).is_some();
            if !ok {
                twine_core::warn!(target: "twine::engine", "menu: {} is not a menu page", fmt_node_id(*p));
            }
            ok
        });
        if let Some(p) = page {
            self.history.push(p);
            let _ = e.move_node(p, self.main, None);
        } else {
            self.history.clear();
        }
        self.main_page = page;
        if let Some(t) = self.selected_tab.filter(|t| e.tree().contains(*t)) {
            e.set_state(t, State::CHECKED, self.sidebar_page.is_some());
        }
        let depth = self.history.len();
        let show = |e: &mut Engine, btn: NodeId, on: bool| {
            e.set_flag(btn, ObjFlags::HIDDEN, !on);
            e.set_flag(btn, ObjFlags::CLICKABLE, on);
        };
        if self.sidebar_page.is_some() {
            if let Some(s) = self.sidebar {
                show(e, s.back_btn, self.mode_root_back_btn);
            }
            show(e, self.main_header_back_btn, depth >= 2);
        } else {
            show(
                e,
                self.main_header_back_btn,
                depth >= 2 || self.mode_root_back_btn,
            );
        }
        self.refr_titles(e);
        cx.post_event(EventCode::ValueChanged, EventParam::None);
        self.refr_main_header_mode(cx.engine_mut());
        twine_core::debug!(
            target: "twine::engine",
            "menu#{}: page {:?}, depth {}",
            fmt_node_id(cx.node()),
            page.map(fmt_node_id),
            depth
        );
    }

    /// LVGL `lv_menu_value_changed_event_cb`: the headers show the pages' titles.
    fn refr_titles(&self, e: &mut Engine) {
        let title_of = |e: &Engine, p: NodeId| e.widget::<MenuPage>(p).and_then(|w| w.title.clone());
        if let Some(p) = self.main_page {
            let t = title_of(e, p);
            set_title_label(e, self.main_header_title, t);
        }
        if let (Some(p), Some(s)) = (self.sidebar_page, self.sidebar) {
            let t = title_of(e, p);
            set_title_label(e, s.title, t);
        }
    }

    /// Shows `page` in a sidebar (created on first use) and reloads the main page (`None`:
    /// removes the sidebar; LVGL `lv_menu_set_sidebar_page`).
    pub fn set_sidebar_page(&mut self, cx: &mut WidgetCx<'_>, page: Option<NodeId>) {
        log_set(MENU_CLASS.name, cx.node(), "sidebar_page");
        let menu = cx.node();
        let e = cx.engine_mut();
        if let Some(p) = page {
            if self.sidebar.is_none() {
                match create_sidebar(e, menu, self.main) {
                    Ok(s) => self.sidebar = Some(s),
                    Err(_) => return,
                }
            }
            if let Some(s) = self.sidebar {
                let _ = e.move_node(p, s.cont, None);
            }
            self.sidebar_page = Some(p);
            self.refr_sidebar_header_mode(e);
        } else {
            if let Some(s) = self.sidebar.take() {
                if let Some(p) = self.sidebar_page.filter(|p| e.tree().contains(*p)) {
                    let _ = e.move_node(p, self.storage, None);
                }
                let _ = e.delete(s.cont);
            }
            self.sidebar_page = None;
        }
        self.refr(cx);
    }

    /// LVGL `lv_menu_refr`: reloads the current page.
    fn refr(&mut self, cx: &mut WidgetCx<'_>) {
        let page = self.history.pop();
        self.set_page(cx, page);
    }

    /// Where the headers go (LVGL `lv_menu_set_mode_header`). Idempotent.
    pub fn set_mode_header(&mut self, cx: &mut WidgetCx<'_>, mode: MenuHeaderMode) {
        if self.mode_header == mode {
            return;
        }
        log_set(MENU_CLASS.name, cx.node(), "mode_header");
        self.mode_header = mode;
        self.refr_main_header_mode(cx.engine_mut());
        self.refr_sidebar_header_mode(cx.engine_mut());
    }

    /// Whether the root page shows a back button (LVGL `lv_menu_set_mode_root_back_button`).
    /// Idempotent.
    pub fn set_mode_root_back_button(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.mode_root_back_btn == on {
            return;
        }
        log_set(MENU_CLASS.name, cx.node(), "mode_root_back_button");
        self.mode_root_back_btn = on;
        self.refr(cx);
    }

    /// Empties the history (LVGL `lv_menu_clear_history`): the next page loaded is a root.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Places `header` before or after `page` in `cont` per the header mode, `page` growing
    /// or not, and hides a header with nothing visible in it (LVGL
    /// `lv_menu_refr_*_header_mode`).
    fn refr_header(&self, e: &mut Engine, cont: NodeId, header: NodeId, page: NodeId) {
        let first = e.tree().children(cont).next();
        match self.mode_header {
            MenuHeaderMode::TopFixed | MenuHeaderMode::TopUnfixed => {
                if first != Some(header) {
                    let _ = e.move_node(header, cont, first);
                }
            }
            MenuHeaderMode::BottomFixed => {
                let _ = e.move_node(header, cont, None);
            }
        }
        let grow = u8::from(self.mode_header != MenuHeaderMode::TopUnfixed);
        e.set_flex_grow(page, grow);
        let visible = e
            .tree()
            .children(header)
            .any(|c| !e.has_flag(c, ObjFlags::HIDDEN));
        e.set_flag(header, ObjFlags::HIDDEN, !visible);
    }

    fn refr_main_header_mode(&self, e: &mut Engine) {
        if let Some(p) = self.main_page.filter(|p| e.tree().contains(*p)) {
            self.refr_header(e, self.main, self.main_header, p);
        }
    }

    fn refr_sidebar_header_mode(&self, e: &mut Engine) {
        if let (Some(s), Some(p)) = (self.sidebar, self.sidebar_page) {
            if e.tree().contains(p) {
                self.refr_header(e, s.cont, s.header, p);
            }
        }
    }

    /// LVGL `lv_menu_back_event_cb`: back to the previous page.
    fn back(&mut self, cx: &mut WidgetCx<'_>, btn: NodeId) {
        self.prev_depth = self.history.len();
        if self.back_button_is_root(btn) {
            return;
        }
        if self.history.len() >= 2 {
            self.history.pop();
            let prev = self.history.pop();
            self.set_page(cx, prev);
        }
    }

    /// Loads `page` because `row` was clicked (LVGL `lv_menu_load_page_event_cb`): a row of
    /// the sidebar becomes the selected one (`CHECKED`) and starts a new history; without a
    /// sidebar the focus moves on to the page.
    pub fn load_page(&mut self, cx: &mut WidgetCx<'_>, row: NodeId, page: NodeId) {
        let e = cx.engine_mut();
        if self.sidebar_page.is_some() {
            let in_sidebar = self
                .sidebar
                .is_some_and(|s| e.tree().ancestors(row).any(|a| a == s.cont));
            if in_sidebar {
                if let Some(t) = self.selected_tab.filter(|t| *t != row && e.tree().contains(*t)) {
                    e.clear_state(t, State::CHECKED);
                }
                self.clear_history();
                self.selected_tab = Some(row);
            }
        }
        self.set_page(cx, Some(page));
        let e = cx.engine_mut();
        if self.sidebar_page.is_none() {
            if let Some(g) = e.default_group() {
                e.focus_next(g);
            }
        }
    }
}

/// Sets `label`'s text to `title` and shows it (hides it without a title).
fn set_title_label(e: &mut Engine, label: NodeId, title: Option<LabelText>) {
    match title {
        Some(t) => {
            e.with_widget_mut(label, |l: &mut Label, cx| match &t {
                LabelText::Static(s) => l.set_text_static(cx, s),
                LabelText::Owned(s) => l.set_text(cx, s),
            });
            e.set_flag(label, ObjFlags::HIDDEN, false);
        }
        None => e.set_flag(label, ObjFlags::HIDDEN, true),
    }
}

/// Creates a header (back button with `SYMBOL_LEFT`, hidden title) of `class` in `parent`.
fn create_header(
    e: &mut Engine,
    menu: NodeId,
    parent: NodeId,
    class: &'static WidgetClass,
) -> Result<(NodeId, NodeId, NodeId), EngineError> {
    let header = e.create(parent, Box::new(ClassObj(class)))?;
    e.set_size(header, Length::pct(100), Length::Content);
    util::set_flex(e, header, FlexFlow::Row);
    e.set_flex_align(header, FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);
    let back = e.create(header, Box::new(Button::new()))?;
    e.set_flag(back, ObjFlags::EVENT_BUBBLE, true);
    util::set_flex(e, back, FlexFlow::Row);
    let icon = e.create(back, Box::new(Image::new()))?;
    e.with_widget_mut(icon, |i: &mut Image, cx| {
        i.set_src(cx, ImageSource::Symbol(symbols::LEFT));
    });
    e.add_event_handler(back, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
        if ev.target == ev.current_target {
            let btn = ecx.node();
            ecx.engine_mut()
                .with_widget_mut(menu, |m: &mut Menu, cx| m.back(cx, btn));
        }
        EventResult::Continue
    });
    let title = e.create(header, Box::new(Label::new("")))?;
    e.set_flag(title, ObjFlags::HIDDEN, true);
    Ok((header, back, title))
}

/// LVGL `lv_menu_set_sidebar_page`'s sidebar: 30 % wide, full height, right after the
/// storage (before the main column).
fn create_sidebar(e: &mut Engine, menu: NodeId, main: NodeId) -> Result<Sidebar, EngineError> {
    let cont = e.create(menu, Box::new(ClassObj(&MENU_SIDEBAR_CONTAINER_CLASS)))?;
    e.move_node(cont, menu, Some(main))?;
    e.set_size(cont, Length::pct(30), Length::pct(100));
    util::set_flex(e, cont, FlexFlow::Column);
    let (header, back_btn, title) = create_header(e, menu, cont, &MENU_SIDEBAR_HEADER_CONTAINER_CLASS)?;
    Ok(Sidebar {
        cont,
        header,
        back_btn,
        title,
    })
}

impl Widget for Menu {
    fn class(&self) -> &'static WidgetClass {
        &MENU_CLASS
    }

    /// LVGL `lv_menu_constructor`: a flex row with the hidden storage and the main column
    /// (header with a back button and a title).
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let menu = cx.node();
        let e = cx.engine_mut();
        e.set_size(menu, MENU_DEFAULT_SIZE.0, MENU_DEFAULT_SIZE.1);
        util::set_flex(e, menu, FlexFlow::Row);
        let Ok(storage) = e.create(menu, Box::new(twine_engine::Obj)) else {
            return;
        };
        e.set_flag(storage, ObjFlags::HIDDEN, true);
        let Ok(main) = e.create(menu, Box::new(ClassObj(&MENU_MAIN_CONTAINER_CLASS))) else {
            return;
        };
        e.set_height(main, Length::pct(100));
        e.set_flex_grow(main, 1);
        util::set_flex(e, main, FlexFlow::Column);
        let Ok((header, back, title)) = create_header(e, menu, main, &MENU_MAIN_HEADER_CONTAINER_CLASS)
        else {
            return;
        };
        self.storage = storage;
        self.main = main;
        self.main_header = header;
        self.main_header_back_btn = back;
        self.main_header_title = title;
    }

    fn event(&mut self, cx: &mut twine_engine::EventCx<'_>, ev: &twine_engine::Event) -> EventResult {
        // Forget pages and rows deleted by the application.
        if ev.code == EventCode::ChildDeleted || (ev.code == EventCode::Delete && ev.target != cx.node()) {
            let e = cx.engine();
            let alive = |n: &NodeId| e.tree().contains(*n) && *n != ev.target;
            self.history.retain(alive);
            self.load_page_map.retain(|(r, p, _)| alive(r) && alive(p));
            if self.main_page.is_some_and(|p| !alive(&p)) {
                self.main_page = None;
            }
            if self.sidebar_page.is_some_and(|p| !alive(&p)) {
                self.sidebar_page = None;
            }
            if self.selected_tab.is_some_and(|t| !alive(&t)) {
                self.selected_tab = None;
            }
        }
        EventResult::Continue
    }
}

/// Creates a menu as the last child of `parent` (LVGL `lv_menu_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Menu::new()))
}

/// Creates a page of `menu` with an optional title (copied), kept in the menu's storage until
/// loaded (LVGL `lv_menu_page_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `menu` is not a menu (logged).
pub fn page_create(e: &mut Engine, menu: NodeId, title: Option<&str>) -> Result<NodeId, EngineError> {
    let Some(storage) = e.widget::<Menu>(menu).map(Menu::storage) else {
        twine_core::warn!(target: "twine::engine", "menu: {} is not a menu", fmt_node_id(menu));
        return Err(EngineError::NodeNotFound(menu));
    };
    let page = e.create(storage, Box::new(MenuPage::default()))?;
    e.with_widget_mut(page, |p: &mut MenuPage, cx| p.set_title(cx, title));
    Ok(page)
}

/// Creates a row in a page or a section: a flex row, 100 % wide (LVGL `lv_menu_cont_create`).
///
/// # Errors
/// [`EngineError::InvalidConfig`] when `parent` is neither a page nor a section (logged).
pub fn cont_create(e: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    let ok = e
        .tree()
        .node(parent)
        .is_some_and(|n| matches!(n.class().name, "menu_page" | "menu_section"));
    if !ok {
        twine_core::warn!(target: "twine::engine", "menu: a row needs a page or a section, not {}", fmt_node_id(parent));
        return Err(EngineError::InvalidConfig("menu row outside a page or section"));
    }
    let c = e.create(parent, Box::new(ClassObj(&MENU_CONT_CLASS)))?;
    init_cont(e, c);
    Ok(c)
}

/// Sets up a row: 100 % wide, content high, a flex row with items centered vertically
/// (LVGL `lv_menu_cont_constructor`).
pub fn init_cont(e: &mut Engine, c: NodeId) {
    e.set_size(c, Length::pct(100), Length::Content);
    util::set_flex(e, c, FlexFlow::Row);
    e.set_flex_align(c, FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);
}

/// Sets up a section: 100 % wide, content high, a flex column.
pub fn init_section(e: &mut Engine, s: NodeId) {
    e.set_size(s, Length::pct(100), Length::Content);
    util::set_flex(e, s, FlexFlow::Column);
}

/// Sets up a separator: content sized.
pub fn init_separator(e: &mut Engine, s: NodeId) {
    e.set_size(s, Length::Content, Length::Content);
}

/// Makes `row` a row that loads a page: clickable, not scrollable, scrolled into view when
/// focused (LVGL `lv_menu_set_load_page_event`).
pub fn make_row_clickable(e: &mut Engine, row: NodeId) {
    e.set_flag(row, ObjFlags::CLICKABLE, true);
    e.set_flag(row, ObjFlags::SCROLLABLE, false);
    e.set_flag(row, ObjFlags::SCROLL_ON_FOCUS, true);
}

/// Creates a section (a card of rows) in a page (LVGL `lv_menu_section_create`).
///
/// # Errors
/// [`EngineError::InvalidConfig`] when `page` is not a page (logged).
pub fn section_create(e: &mut Engine, page: NodeId) -> Result<NodeId, EngineError> {
    if e.widget::<MenuPage>(page).is_none() {
        twine_core::warn!(target: "twine::engine", "menu: a section needs a page, not {}", fmt_node_id(page));
        return Err(EngineError::InvalidConfig("menu section outside a page"));
    }
    let s = e.create(page, Box::new(ClassObj(&MENU_SECTION_CLASS)))?;
    init_section(e, s);
    Ok(s)
}

/// Creates a separator in a page (LVGL `lv_menu_separator_create`).
///
/// # Errors
/// [`EngineError::InvalidConfig`] when `page` is not a page (logged).
pub fn separator_create(e: &mut Engine, page: NodeId) -> Result<NodeId, EngineError> {
    if e.widget::<MenuPage>(page).is_none() {
        twine_core::warn!(target: "twine::engine", "menu: a separator needs a page, not {}", fmt_node_id(page));
        return Err(EngineError::InvalidConfig("menu separator outside a page"));
    }
    let s = e.create(page, Box::new(ClassObj(&MENU_SEPARATOR_CLASS)))?;
    init_separator(e, s);
    Ok(s)
}

/// Makes a click on `row` load `page` (LVGL `lv_menu_set_load_page_event`): the row becomes
/// clickable, not scrollable and scrolls into view when focused; a previous link of the row is
/// replaced.
pub fn set_load_page_event(e: &mut Engine, menu: NodeId, row: NodeId, page: NodeId) {
    let old = e
        .widget::<Menu>(menu)
        .and_then(|m| m.load_page_map.iter().find(|(r, _, _)| *r == row).map(|x| x.2));
    if let Some(h) = old {
        e.remove_event_handler(row, h);
    }
    make_row_clickable(e, row);
    let h = e.add_event_handler(row, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
        if ev.current_target == row {
            ecx.engine_mut()
                .with_widget_mut(menu, |m: &mut Menu, cx| m.load_page(cx, row, page));
        }
        EventResult::Continue
    });
    e.with_widget_mut(menu, |m: &mut Menu, _| {
        m.load_page_map.retain(|(r, _, _)| *r != row);
        m.load_page_map.push((row, page, h));
    });
}

/// The menu's rows that load a page.
#[must_use]
pub fn load_page_links(e: &Engine, menu: NodeId) -> Vec<(NodeId, NodeId)> {
    e.widget::<Menu>(menu)
        .map(|m| m.load_page_map.iter().map(|&(r, p, _)| (r, p)).collect())
        .unwrap_or_default()
}
