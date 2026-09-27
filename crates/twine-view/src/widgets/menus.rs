//! Menus: [`menu`], [`menu_page`], [`menu_cont`], [`menu_section`], [`menu_separator`] and
//! [`MenuPageRef`].

use twine_engine::{EventCode, EventFilter, EventResult, NodeId, fmt_node_id};
use twine_reactive::{Scope, Signal};
use twine_widgets_ext::ClassObj;
use twine_widgets_ext::menu::{
    self, MENU_CONT_CLASS, MENU_SECTION_CLASS, MENU_SEPARATOR_CLASS, Menu, MenuHeaderMode, MenuPage,
};

use crate::build::{BuildCx, WidgetView, widget_view};
use crate::prop::IntoProp;
use crate::view::{View, ViewSeq};

/// A reference to a menu page, filled when the page is built (`.page_ref(r)`), so rows can
/// load pages defined later. `Copy`. Created with [`ScopeExt::menu_page_ref`](crate::ScopeExt::menu_page_ref).
#[derive(Clone, Copy, Debug)]
pub struct MenuPageRef {
    cell: Signal<Option<NodeId>>,
}

impl MenuPageRef {
    /// An empty reference owned by `cx`.
    #[must_use]
    pub fn new(cx: Scope) -> Self {
        Self {
            cell: cx.signal(None),
        }
    }

    /// The page, once built.
    #[must_use]
    pub fn get(&self) -> Option<NodeId> {
        if self.cell.is_alive() {
            self.cell.get_untracked()
        } else {
            None
        }
    }
}

/// A menu showing `root` (a [`menu_page`]) with a header and a back button; rows of its
/// pages load other pages (`menu_cont(..).loads(page_ref)`), which are given with
/// [`pages`](WidgetView::pages) (and the [`sidebar`](WidgetView::sidebar) page).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn settings(cx: Scope) -> impl View {
///     let display = cx.menu_page_ref();
///     menu(menu_page(None, (
///         menu_cont(label("Display")).loads(display),
///     )))
///     .pages(menu_page(Some("Display"), label("Brightness")).page_ref(display))
/// }
/// # let _ = settings;
/// ```
pub fn menu(root: impl View) -> WidgetView<Menu> {
    widget_view(Menu::new).op(move |cx, node| {
        let root = build_in_storage(cx, node, root);
        cx.engine()
            .with_widget_mut(node, |m: &mut Menu, wcx| m.set_page(wcx, root));
    })
}

/// Builds `v` in the menu's storage; returns its node when it is a page.
fn build_in_storage(cx: &mut BuildCx<'_>, menu: NodeId, v: impl View) -> Option<NodeId> {
    let storage = cx.engine().widget::<Menu>(menu).map(Menu::storage)?;
    let page = cx.with_parent(storage, |cx| v.build(cx));
    if cx.engine().widget::<MenuPage>(page).is_none() {
        twine_core::warn!(target: "twine::view", "menu: {} is not a menu_page", fmt_node_id(page));
        return None;
    }
    Some(page)
}

impl WidgetView<Menu> {
    /// More pages (loaded by rows), kept hidden until loaded.
    #[must_use]
    pub fn pages(self, pages: impl ViewSeq) -> Self {
        self.op(move |cx, node| {
            if let Some(storage) = cx.engine().widget::<Menu>(node).map(Menu::storage) {
                cx.with_parent(storage, |cx| pages.build_seq(cx));
            }
        })
    }

    /// A sidebar page shown left of the main page; its rows loading pages are marked
    /// `CHECKED` while their page shows.
    #[must_use]
    pub fn sidebar(self, page: impl View) -> Self {
        self.op(move |cx, node| {
            let p = build_in_storage(cx, node, page);
            cx.engine()
                .with_widget_mut(node, |m: &mut Menu, wcx| m.set_sidebar_page(wcx, p));
        })
    }

    /// Where the headers go.
    #[must_use]
    pub fn header_mode(self, mode: impl IntoProp<MenuHeaderMode>) -> Self {
        self.bind(mode, |m: &mut Menu, cx, mode| m.set_mode_header(cx, mode))
    }

    /// Whether the root page shows a back button.
    #[must_use]
    pub fn root_back_button(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |m: &mut Menu, cx, on| m.set_mode_root_back_button(cx, on))
    }
}

/// A page of a [`menu()`] with an optional title (shown in the header while loaded) and its
/// `content` (usually [`menu_cont`] rows and [`menu_section`]s).
pub type MenuPageView = WidgetView<MenuPage>;

/// A menu page (see [`menu()`]).
pub fn menu_page(title: Option<&'static str>, content: impl ViewSeq) -> MenuPageView {
    widget_view(MenuPage::default)
        .op(move |cx, node| {
            cx.engine()
                .with_widget_mut(node, |p: &mut MenuPage, wcx| p.set_title_static(wcx, title));
        })
        .children(content)
}

impl WidgetView<MenuPage> {
    /// Fills `r` with this page when it is built.
    #[must_use]
    pub fn page_ref(self, r: MenuPageRef) -> Self {
        self.op(move |_, node| {
            if r.cell.is_alive() {
                r.cell.set(Some(node));
            }
        })
    }
}

/// A row of a menu page (a flex row, e.g. an icon, a label and a switch).
pub fn menu_cont(content: impl ViewSeq) -> WidgetView<ClassObj> {
    widget_view(|| ClassObj(&MENU_CONT_CLASS))
        .op(|cx, node| menu::init_cont(cx.engine(), node))
        .children(content)
}

impl WidgetView<ClassObj> {
    /// Clicking this row loads `page` (a [`menu_cont`] only; the reference may be filled
    /// after this row is built).
    #[must_use]
    pub fn loads(self, page: MenuPageRef) -> Self {
        self.op(move |cx, row| {
            menu::make_row_clickable(cx.engine(), row);
            cx.engine()
                .add_event_handler(row, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                    if ev.current_target != row {
                        return EventResult::Continue;
                    }
                    let e = ecx.engine_mut();
                    let menu = e.tree().ancestors(row).find(|&a| e.widget::<Menu>(a).is_some());
                    match (menu, page.get()) {
                        (Some(m), Some(p)) => {
                            e.with_widget_mut(m, |w: &mut Menu, cx| w.load_page(cx, row, p));
                        }
                        _ => twine_core::warn!(
                            target: "twine::view",
                            "menu_cont {}: no menu or page to load",
                            fmt_node_id(row)
                        ),
                    }
                    EventResult::Continue
                });
        })
    }
}

/// A section of a menu page: its rows drawn as one card.
pub fn menu_section(content: impl ViewSeq) -> WidgetView<ClassObj> {
    widget_view(|| ClassObj(&MENU_SECTION_CLASS))
        .op(|cx, node| menu::init_section(cx.engine(), node))
        .children(content)
}

/// A gap between rows of a menu page.
pub fn menu_separator() -> WidgetView<ClassObj> {
    widget_view(|| ClassObj(&MENU_SEPARATOR_CLASS)).op(|cx, node| menu::init_separator(cx.engine(), node))
}
