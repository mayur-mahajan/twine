//! [`Tabview`]: tabs with a button bar and swipeable, snap-scrolling content (LVGL
//! `lv_tabview`, 9.x structure).
//!
//! ```text
//! tabview (flex column; row / reversed for other bar positions)
//! ├── tabview_tab_bar (flex row of buttons, one per tab)
//! └── tabview_content (flex row, horizontal snap scrolling, one page per tab)
//! ```

use alloc::boxed::Box;

use twine_engine::{
    Engine, EngineError, Event, EventCode, EventCx, EventFilter, EventParam, EventResult, NodeId, OBJ_FLAGS,
    ObjFlags, State, Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_style::{Align, Dir, FlexFlow, Length, Part, PropId, ScrollSnap, ScrollbarMode};
use twine_widgets::button::{BUTTON_CLASS, Button};
use twine_widgets::label::Label;

use crate::util::{self, ClassObj, log_set};

/// The class of [`Tabview`]: `"tabview"`, the base object's parts and flags.
pub static TABVIEW_CLASS: WidgetClass = WidgetClass::new("tabview")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The button bar (`"tabview_tab_bar"`).
pub static TABVIEW_TAB_BAR_CLASS: WidgetClass = WidgetClass::new("tabview_tab_bar")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);
/// The scrolling page container (`"tabview_content"`).
pub static TABVIEW_CONTENT_CLASS: WidgetClass = WidgetClass::new("tabview_content")
    .parts(twine_engine::OBJ_CLASS.parts)
    .default_flags(OBJ_FLAGS);

/// A tabview (LVGL `lv_tabview`): a bar of buttons (one per tab) and a content area whose
/// pages scroll horizontally (vertically with the bar at the left or right) and snap one
/// page at a time.
///
/// - Clicking a tab's button switches to its page (animated unless
///   [`set_animated`](Self::set_animated)`(false)`) and marks the button `CHECKED`.
/// - Swiping the content switches too: at the end of the scroll the page in view becomes
///   active.
/// - **Events**: `ValueChanged` with the new index as [`EventParam::Value`] when the user
///   switches (click or swipe); [`set_active`](Self::set_active) is silent.
/// - Pages are ordinary containers (class `"obj"`); the default theme pads them and makes
///   them scrollable. The buttons join the default focus group (keypad and encoder).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::tabview::{self, Tabview};
///
/// let mut h = EngineHarness::new(320, 240);
/// let screen = h.screen();
/// let tv = tabview::create(h.engine_mut(), screen).unwrap();
/// let pages = h.engine_mut().with_widget_mut(tv, |t: &mut Tabview, cx| {
///     [t.add_tab(cx, "One"), t.add_tab(cx, "Two")]
/// }).unwrap();
/// h.run_until_idle();
/// h.engine_mut().with_widget_mut(tv, |t: &mut Tabview, cx| t.set_active(cx, 1, false));
/// assert_eq!(h.engine().widget::<Tabview>(tv).unwrap().active(), 1);
/// assert!(pages.iter().all(|p| p.is_some()));
/// ```
#[derive(Debug)]
pub struct Tabview {
    tab_bar: NodeId,
    content: NodeId,
    tab_cur: u32,
    tab_pos: Dir,
    tab_bar_size: i32,
    animated: bool,
}

impl Default for Tabview {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabview {
    /// A tabview (its bar and content are created by `init`; the bar is at the top).
    #[must_use]
    pub fn new() -> Self {
        Self {
            tab_bar: NodeId::DANGLING,
            content: NodeId::DANGLING,
            tab_cur: 0,
            tab_pos: Dir::NONE,
            tab_bar_size: 0,
            animated: true,
        }
    }

    // ---- Getters --------------------------------------------------------------------------

    /// The index of the active tab (LVGL `lv_tabview_get_tab_active`).
    #[must_use]
    pub fn active(&self) -> u32 {
        self.tab_cur
    }

    /// The button bar (LVGL `lv_tabview_get_tab_bar`).
    #[must_use]
    pub fn tab_bar(&self) -> NodeId {
        self.tab_bar
    }

    /// The page container (LVGL `lv_tabview_get_content`).
    #[must_use]
    pub fn content(&self) -> NodeId {
        self.content
    }

    /// The bar position.
    #[must_use]
    pub fn tab_bar_position(&self) -> Dir {
        self.tab_pos
    }

    /// The bar size (height at the top or bottom, width at the side).
    #[must_use]
    pub fn tab_bar_size(&self) -> i32 {
        self.tab_bar_size
    }

    /// Whether clicking a tab scrolls to it with an animation.
    #[must_use]
    pub fn animated(&self) -> bool {
        self.animated
    }

    /// The number of tabs (LVGL `lv_tabview_get_tab_count`).
    #[must_use]
    pub fn tab_count(&self, e: &Engine) -> u32 {
        tab_buttons(e, self.tab_bar).count() as u32
    }

    /// The button of tab `idx` (LVGL `lv_tabview_get_tab_button`).
    #[must_use]
    pub fn tab_button(&self, e: &Engine, idx: u32) -> Option<NodeId> {
        tab_buttons(e, self.tab_bar).nth(idx as usize)
    }

    /// The page of tab `idx`.
    #[must_use]
    pub fn tab_page(&self, e: &Engine, idx: u32) -> Option<NodeId> {
        e.tree().children(self.content).nth(idx as usize)
    }

    // ---- Setters --------------------------------------------------------------------------

    /// Adds a tab titled `name` (copied): a button in the bar and a page (returned) in the
    /// content (LVGL `lv_tabview_add_tab`). The first tab becomes active.
    pub fn add_tab(&mut self, cx: &mut WidgetCx<'_>, name: &str) -> Option<NodeId> {
        log_set(TABVIEW_CLASS.name, cx.node(), "add_tab");
        let tv = cx.node();
        let e = cx.engine_mut();
        let page = e.create(self.content, Box::new(twine_engine::Obj)).ok()?;
        e.set_size(page, Length::pct(100), Length::pct(100));
        let btn = e.create(self.tab_bar, Box::new(Button::new())).ok()?;
        e.set_flex_grow(btn, 1);
        e.set_size(btn, Length::pct(100), Length::pct(100));
        e.add_event_handler(btn, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
            if ev.target == ev.current_target {
                let b = ecx.node();
                ecx.engine_mut()
                    .with_widget_mut(tv, |t: &mut Tabview, cx| t.button_clicked(cx, b));
            }
            EventResult::Continue
        });
        let label = e.create(btn, Box::new(Label::new(""))).ok()?;
        e.with_widget_mut(label, |l: &mut Label, cx| l.set_text(cx, name));
        e.align(label, Align::Center, 0, 0);
        let idx = tab_buttons(e, self.tab_bar).count() as u32 - 1;
        if idx == self.tab_cur {
            self.set_active(cx, idx, false);
        }
        Some(page)
    }

    /// Renames tab `idx` (LVGL `lv_tabview_set_tab_text`). Unknown tabs log `warn!`.
    pub fn rename_tab(&mut self, cx: &mut WidgetCx<'_>, idx: u32, name: &str) {
        let e = cx.engine_mut();
        let Some(label) = self
            .tab_button(e, idx)
            .and_then(|b| e.tree().children(b).find(|&c| e.widget::<Label>(c).is_some()))
        else {
            twine_core::warn!(target: "twine::engine", "tabview: no tab {}", idx);
            return;
        };
        e.with_widget_mut(label, |l: &mut Label, cx| l.set_text(cx, name));
    }

    /// Switches to tab `idx`, scrolling the content to its page (animated when `anim`), and
    /// marks its button `CHECKED` (LVGL `lv_tabview_set_active`; no `ValueChanged`). An index
    /// past the last tab is remembered (it activates when that tab is added) but shows
    /// nothing.
    pub fn set_active(&mut self, cx: &mut WidgetCx<'_>, idx: u32, anim: bool) {
        self.tab_cur = idx;
        let e = cx.engine_mut();
        if idx >= self.tab_count(e) || !e.tree().contains(self.content) {
            return;
        }
        e.update_layout();
        let c = self.content;
        let ca = e.content_area(c);
        if self.tab_pos.intersects(Dir::VER) {
            let gap = e.style_i32(c, Part::Main, PropId::PadColumn);
            let x = idx as i32 * (gap + ca.width());
            let rtl = util::is_rtl(&cx.measure());
            let e = cx.engine_mut();
            e.scroll_to_x(c, if rtl { -x } else { x }, anim);
        } else {
            let gap = e.style_i32(c, Part::Main, PropId::PadRow);
            e.scroll_to_y(c, idx as i32 * (gap + ca.height()), anim);
        }
        let e = cx.engine_mut();
        let mut i = 0;
        let mut child = e
            .tree()
            .node(self.tab_bar)
            .and_then(twine_engine::Node::first_child);
        while let Some(b) = child {
            if is_tab_button(e, b) {
                e.set_state(b, State::CHECKED, i == idx);
                i += 1;
            }
            child = e.tree().node(b).and_then(twine_engine::Node::next_sibling);
        }
    }

    /// Whether clicking a tab animates the switch (default on).
    pub fn set_animated(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.animated != on {
            log_set(TABVIEW_CLASS.name, cx.node(), "animated");
            self.animated = on;
        }
    }

    /// Puts the bar at the top, bottom, left or right (LVGL `lv_tabview_set_tab_bar_position`).
    /// Idempotent.
    pub fn set_tab_bar_position(&mut self, cx: &mut WidgetCx<'_>, dir: Dir) {
        if dir == self.tab_pos {
            return;
        }
        let flow = match dir {
            Dir::TOP => FlexFlow::Column,
            Dir::BOTTOM => FlexFlow::ColumnReverse,
            Dir::LEFT => FlexFlow::Row,
            Dir::RIGHT => FlexFlow::RowReverse,
            _ => {
                twine_core::warn!(target: "twine::engine", "tabview: bar position {:?} is not a side", dir);
                return;
            }
        };
        log_set(TABVIEW_CLASS.name, cx.node(), "tab_bar_position");
        let tv = cx.node();
        let (bar, c) = (self.tab_bar, self.content);
        let dpi = i32::from(util::display_dpi(cx.engine(), tv));
        let e = cx.engine_mut();
        util::set_flex(e, tv, flow);
        let now_ver = dir.intersects(Dir::VER);
        if now_ver {
            e.set_width(c, Length::pct(100));
            util::set_flex(e, bar, FlexFlow::Row);
            util::set_flex(e, c, FlexFlow::Row);
            e.set_scroll_snap_x(c, ScrollSnap::Center);
            e.set_scroll_snap_y(c, ScrollSnap::None);
        } else {
            e.set_height(c, Length::pct(100));
            util::set_flex(e, bar, FlexFlow::Column);
            util::set_flex(e, c, FlexFlow::Column);
            e.set_scroll_snap_x(c, ScrollSnap::None);
            e.set_scroll_snap_y(c, ScrollSnap::Center);
        }
        e.set_flex_grow(c, 1);
        let was_ver = self.tab_pos.intersects(Dir::VER);
        if was_ver != now_ver || self.tab_pos == Dir::NONE {
            if now_ver {
                e.set_size(bar, Length::pct(100), dpi / 2);
            } else {
                e.set_size(bar, dpi, Length::pct(100));
            }
        }
        self.tab_pos = dir;
        let size = self.tab_bar_size;
        self.tab_bar_size = i32::MIN;
        self.set_tab_bar_size(cx, size);
    }

    /// The bar's height (top / bottom) or width (left / right) (LVGL
    /// `lv_tabview_set_tab_bar_size`). Idempotent.
    pub fn set_tab_bar_size(&mut self, cx: &mut WidgetCx<'_>, size: i32) {
        if self.tab_bar_size == size {
            return;
        }
        log_set(TABVIEW_CLASS.name, cx.node(), "tab_bar_size");
        let bar = self.tab_bar;
        let e = cx.engine_mut();
        if self.tab_pos.intersects(Dir::VER) {
            e.set_height(bar, size);
        } else {
            e.set_width(bar, size);
        }
        self.tab_bar_size = size;
    }

    /// LVGL `button_clicked_event_cb`.
    fn button_clicked(&mut self, cx: &mut WidgetCx<'_>, btn: NodeId) {
        let prev = self.tab_cur;
        let Some(idx) = tab_buttons(cx.engine(), self.tab_bar).position(|b| b == btn) else {
            return;
        };
        let idx = idx as u32;
        let anim = self.animated;
        self.set_active(cx, idx, anim);
        if prev != idx {
            util::log_value(TABVIEW_CLASS.name, cx.node(), idx as i32);
            cx.post_event(EventCode::ValueChanged, EventParam::Value(idx as i32));
        }
    }

    /// LVGL `cont_scroll_end_event_cb`: the page in view after a scroll becomes active.
    fn content_scroll_end(&mut self, cx: &mut WidgetCx<'_>) {
        let e = cx.engine();
        if e.active_input().is_some_and(|i| e.input_pressed(i)) {
            return;
        }
        let c = self.content;
        let p = e.scroll_end(c);
        let ca = e.content_area(c);
        let t = if self.tab_pos.intersects(Dir::VER) {
            let w = ca.width().max(1);
            if util::is_rtl(&cx.measure()) {
                -(p.x - w / 2) / w
            } else {
                (p.x + w / 2) / w
            }
        } else {
            let h = ca.height().max(1);
            (p.y + h / 2) / h
        };
        let t = t.max(0) as u32;
        let new_tab = t != self.tab_cur;
        let by_input = cx.engine().active_input().is_some();
        self.set_active(cx, t, by_input);
        if new_tab && t < self.tab_count(cx.engine()) {
            util::log_value(TABVIEW_CLASS.name, cx.node(), t as i32);
            cx.post_event(EventCode::ValueChanged, EventParam::Value(t as i32));
        }
    }
}

/// Whether `b` is a tab button (a plain button in the bar).
fn is_tab_button(e: &Engine, b: NodeId) -> bool {
    e.tree()
        .node(b)
        .is_some_and(|n| core::ptr::eq(n.class(), &raw const BUTTON_CLASS))
}

/// The tab buttons of `bar`, in order (LVGL `lv_obj_get_child_by_type(.., &lv_button_class)`).
fn tab_buttons(e: &Engine, bar: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    e.tree().children(bar).filter(move |&b| is_tab_button(e, b))
}

/// Creates a tabview, 100 % × 100 % of its parent, as the last child of `parent` (LVGL
/// `lv_tabview_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Tabview::new()))
}

impl Widget for Tabview {
    fn class(&self) -> &'static WidgetClass {
        &TABVIEW_CLASS
    }

    /// LVGL `lv_tabview_constructor`: the bar and the content (a flex row without scrollbar,
    /// scrolling one page at a time), the bar at the top, `dpi / 2` high.
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let tv = cx.node();
        let e = cx.engine_mut();
        e.set_size(tv, Length::pct(100), Length::pct(100));
        let (Ok(bar), Ok(content)) = (
            e.create(tv, Box::new(ClassObj(&TABVIEW_TAB_BAR_CLASS))),
            e.create(tv, Box::new(ClassObj(&TABVIEW_CONTENT_CLASS))),
        ) else {
            twine_core::warn!(target: "twine::engine", "tabview {}: cannot create its parts", fmt_node_id(tv));
            return;
        };
        self.tab_bar = bar;
        self.content = content;
        util::set_flex(e, content, FlexFlow::Row);
        e.set_scrollbar_mode(content, ScrollbarMode::Off);
        for code in [EventCode::LayoutChanged, EventCode::ScrollEnd] {
            e.add_event_handler(content, EventFilter::Code(code), move |ecx, ev| {
                if ev.target == ev.current_target {
                    let layout = ev.code == EventCode::LayoutChanged;
                    util::with_ready(ecx.engine_mut(), tv, |t: &mut Tabview, cx| {
                        if layout {
                            let cur = t.tab_cur;
                            t.set_active(cx, cur, false);
                        } else {
                            t.content_scroll_end(cx);
                        }
                    });
                }
                EventResult::Continue
            });
        }
        let dpi = i32::from(util::display_dpi(cx.engine(), tv));
        self.tab_bar_size = dpi / 2;
        self.set_tab_bar_position(cx, Dir::TOP);
        let e = cx.engine_mut();
        e.set_flag(content, ObjFlags::SCROLL_ONE, true);
        e.set_flag(content, ObjFlags::SCROLL_ON_FOCUS, false);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && ev.code == EventCode::SizeChanged {
            let cur = self.tab_cur;
            self.set_active(&mut cx.widget_cx(), cur, false);
        }
        EventResult::Continue
    }
}
