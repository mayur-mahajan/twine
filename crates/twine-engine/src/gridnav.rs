//! Grid navigation (LVGL `lv_gridnav`): arrow keys move the focus among the children of a
//! container by their position on screen.
//!
//! The container is the member of the focus group; its children are not. The container keeps
//! track of a "focused child" that gets `FOCUSED | FOCUS_KEY` while the container is focused.
//! `Left`/`Right`/`Up`/`Down` keys pick the nearest child in that direction; other keys and the
//! press / click events of keypads and encoders are forwarded to the focused child.
//!
//! ```
//! use twine_core::Rect;
//! use twine_engine::{Engine, EngineConfig, EventCode, EventParam, Obj, State, gridnav};
//! use twine_hal::Key;
//!
//! let mut e = Engine::new(EngineConfig::default()).unwrap();
//! let cont = e.create_root(Box::new(Obj)).unwrap();
//! e.place(cont, Rect::from_xywh(0, 0, 100, 40)); // a root: placed by hand
//! let a = e.create(cont, Box::new(Obj)).unwrap();
//! e.set_size(a, 40, 40);
//! let b = e.create(cont, Box::new(Obj)).unwrap();
//! e.set_size(b, 40, 40);
//! e.set_pos(b, 50, 0);
//! e.update_layout();
//! gridnav::gridnav_add(&mut e, cont, gridnav::GridnavCtrl::empty());
//! let g = e.create_group().unwrap();
//! e.group_add(g, cont);
//! assert_eq!(gridnav::gridnav_focused(&e, cont), Some(a));
//! e.send_event(cont, EventCode::Key, EventParam::Key(Key::Right));
//! assert_eq!(gridnav::gridnav_focused(&e, cont), Some(b));
//! assert!(e.tree().node(b).unwrap().state().contains(State::FOCUSED));
//! ```

use twine_hal::Key;
use twine_style::State;

use crate::{
    Engine, Event, EventCode, EventCx, EventFilter, EventParam, EventResult, HandlerId, InputKind, NodeId,
    ObjFlags, fmt_node_id,
};

bitflags::bitflags! {
    /// Gridnav options (LVGL `lv_gridnav_ctrl_t`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct GridnavCtrl: u8 {
        /// At the end of a row / column continue on the next / previous one (and wrap around
        /// at the ends); without it, the group focus moves on instead.
        const ROLLOVER = 1 << 0;
        /// If the focused child can be scrolled in the key's direction, scroll it by a quarter
        /// of its size instead of moving the focus.
        const SCROLL_FIRST = 1 << 1;
        /// Only `Left`/`Right` move the focus.
        const HORIZONTAL_MOVE_ONLY = 1 << 2;
        /// Only `Up`/`Down` move the focus.
        const VERTICAL_MOVE_ONLY = 1 << 3;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for GridnavCtrl {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "GridnavCtrl({=u8:#x})", self.bits());
    }
}

/// Per-container gridnav state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GridnavDsc {
    container: NodeId,
    ctrl: GridnavCtrl,
    focused: Option<NodeId>,
    handler: HandlerId,
}

/// Which child [`find_child`] looks for (LVGL `find_mode_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Find {
    Left,
    Right,
    Top,
    Bottom,
    NextRowFirst,
    PrevRowLast,
    FirstRow,
    LastRow,
}

/// Enables grid navigation on `container` (replacing an earlier gridnav on it). Removes the
/// container's `SCROLL_WITH_ARROW` flag (the arrows now move the focus).
pub fn gridnav_add(engine: &mut Engine, container: NodeId, ctrl: GridnavCtrl) {
    if !engine.tree.contains(container) {
        twine_core::warn!(target: "twine::input", "gridnav_add: node {} not found", fmt_node_id(container));
        return;
    }
    gridnav_remove(engine, container);
    let handler = engine.add_event_handler(container, EventFilter::All, gridnav_event);
    engine.gridnavs.push(GridnavDsc {
        container,
        ctrl,
        focused: None,
        handler,
    });
    engine.set_flag(container, ObjFlags::SCROLL_WITH_ARROW, false);
}

/// Disables grid navigation on `container`.
pub fn gridnav_remove(engine: &mut Engine, container: NodeId) {
    if let Some(i) = engine.gridnavs.iter().position(|d| d.container == container) {
        let d = engine.gridnavs.swap_remove(i);
        engine.remove_event_handler(container, d.handler);
    }
}

/// Makes `child` the focused child of the gridnav `container` (it gets `FOCUSED | FOCUS_KEY`)
/// and scrolls it into view (animated with `anim`).
pub fn gridnav_set_focused(engine: &mut Engine, container: NodeId, child: Option<NodeId>, anim: bool) {
    let Some(i) = engine.gridnavs.iter().position(|d| d.container == container) else {
        twine_core::warn!(target: "twine::input", "gridnav_set_focused: {} is not a gridnav container", fmt_node_id(container));
        return;
    };
    if let Some(c) = child {
        if !focusable(engine, c) {
            twine_core::warn!(target: "twine::input", "gridnav_set_focused: {} is not focusable", fmt_node_id(c));
            return;
        }
    }
    if let Some(old) = engine.gridnavs[i].focused {
        engine.clear_state(old, State::FOCUSED | State::FOCUS_KEY);
    }
    if let Some(c) = child {
        engine.add_state(c, State::FOCUSED | State::FOCUS_KEY);
        engine.scroll_to_view(c, anim);
    }
    if let Some(d) = engine.gridnavs.iter_mut().find(|d| d.container == container) {
        d.focused = child;
    }
}

/// The focused child of the gridnav `container`.
#[must_use]
pub fn gridnav_focused(engine: &Engine, container: NodeId) -> Option<NodeId> {
    engine
        .gridnavs
        .iter()
        .find(|d| d.container == container)
        .and_then(|d| d.focused)
        .filter(|n| engine.tree.contains(*n))
}

fn dsc(e: &Engine, container: NodeId) -> Option<GridnavDsc> {
    e.gridnavs.iter().find(|d| d.container == container).copied()
}

fn set_dsc_focused(e: &mut Engine, container: NodeId, f: Option<NodeId>) {
    if let Some(d) = e.gridnavs.iter_mut().find(|d| d.container == container) {
        d.focused = f;
    }
}

/// The gridnav event handler (LVGL `gridnav_event_cb`).
fn gridnav_event(cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
    let obj = cx.node();
    let e = cx.engine_mut();
    let Some(d) = dsc(e, obj) else {
        return EventResult::Continue;
    };
    let focused = d
        .focused
        .filter(|f| e.tree.contains(*f) && e.tree.parent(*f) == Some(obj));
    match ev.code {
        EventCode::Key => {
            let Some(key) = ev.key() else {
                return EventResult::Continue;
            };
            if e.tree.node(obj).is_none_or(|n| n.child_count() == 0) {
                return EventResult::Continue;
            }
            let Some(cur) = focused.or_else(|| first_focusable(e, obj)) else {
                return EventResult::Continue;
            };
            set_dsc_focused(e, obj, Some(cur));
            let group = e.group_of(obj);
            // With SCROLL_FIRST a scrollable focused child scrolls by a quarter of its size
            // while it has content in the key's direction.
            let scroll_first =
                d.ctrl.contains(GridnavCtrl::SCROLL_FIRST) && e.has_flag(cur, ObjFlags::SCROLLABLE);
            let c = e.coords(cur);
            let (qw, qh) = ((c.width() / 4).max(1), (c.height() / 4).max(1));
            let guess = match key {
                Key::Right if !d.ctrl.contains(GridnavCtrl::VERTICAL_MOVE_ONLY) => {
                    if scroll_first && e.scroll_right(cur) > 0 {
                        e.scroll_by_bounded(cur, -qw, 0, true);
                        None
                    } else {
                        find_child(e, obj, cur, Find::Right).or_else(|| {
                            if d.ctrl.contains(GridnavCtrl::ROLLOVER) {
                                find_child(e, obj, cur, Find::NextRowFirst)
                                    .or_else(|| first_focusable(e, obj))
                            } else {
                                if let Some(g) = group {
                                    e.focus_next(g);
                                }
                                None
                            }
                        })
                    }
                }
                Key::Left if !d.ctrl.contains(GridnavCtrl::VERTICAL_MOVE_ONLY) => {
                    if scroll_first && e.scroll_left(cur) > 0 {
                        e.scroll_by_bounded(cur, qw, 0, true);
                        None
                    } else {
                        find_child(e, obj, cur, Find::Left).or_else(|| {
                            if d.ctrl.contains(GridnavCtrl::ROLLOVER) {
                                find_child(e, obj, cur, Find::PrevRowLast).or_else(|| last_focusable(e, obj))
                            } else {
                                if let Some(g) = group {
                                    e.focus_prev(g);
                                }
                                None
                            }
                        })
                    }
                }
                Key::Down if !d.ctrl.contains(GridnavCtrl::HORIZONTAL_MOVE_ONLY) => {
                    if scroll_first && e.scroll_bottom(cur) > 0 {
                        e.scroll_by_bounded(cur, 0, -qh, true);
                        None
                    } else {
                        find_child(e, obj, cur, Find::Bottom).or_else(|| {
                            if d.ctrl.contains(GridnavCtrl::ROLLOVER) {
                                find_child(e, obj, cur, Find::FirstRow)
                            } else {
                                if let Some(g) = group {
                                    e.focus_next(g);
                                }
                                None
                            }
                        })
                    }
                }
                Key::Up if !d.ctrl.contains(GridnavCtrl::HORIZONTAL_MOVE_ONLY) => {
                    if scroll_first && e.scroll_top(cur) > 0 {
                        e.scroll_by_bounded(cur, 0, qh, true);
                        None
                    } else {
                        find_child(e, obj, cur, Find::Top).or_else(|| {
                            if d.ctrl.contains(GridnavCtrl::ROLLOVER) {
                                find_child(e, obj, cur, Find::LastRow)
                            } else {
                                if let Some(g) = group {
                                    e.focus_prev(g);
                                }
                                None
                            }
                        })
                    }
                }
                _ => {
                    if group.is_some_and(|g| e.focused(g) == Some(obj)) {
                        e.send_event(cur, EventCode::Key, EventParam::Key(key));
                    }
                    None
                }
            };
            // Moving the group focus away sends `Defocused` to the container while this
            // handler runs (so it cannot see it): clear the child's focus here.
            if group.is_some_and(|g| e.focused(g) != Some(obj)) {
                e.clear_state(cur, State::FOCUSED | State::FOCUS_KEY);
            }
            if let Some(new) = guess.filter(|g| *g != cur && e.tree.contains(cur)) {
                e.clear_state(cur, State::FOCUSED | State::FOCUS_KEY);
                e.send_event(cur, EventCode::Defocused, EventParam::None);
                if e.tree.contains(new) {
                    e.add_state(new, State::FOCUSED | State::FOCUS_KEY);
                    e.send_event(new, EventCode::Focused, EventParam::None);
                    if e.tree.contains(new) {
                        e.scroll_to_view(new, true);
                    }
                    set_dsc_focused(e, obj, Some(new));
                }
            }
        }
        EventCode::Focused => {
            let f = focused.or_else(|| first_focusable(e, obj));
            set_dsc_focused(e, obj, f);
            if let Some(f) = f {
                e.add_state(f, State::FOCUSED | State::FOCUS_KEY);
                // Be sure the child is not stuck in the pressed state.
                e.clear_state(f, State::PRESSED);
                e.scroll_to_view(f, false);
            }
        }
        EventCode::Defocused => {
            if let Some(f) = focused {
                e.clear_state(f, State::FOCUSED | State::FOCUS_KEY);
            }
        }
        EventCode::ChildCreated => {
            let child = ev.target;
            if ev.current_target == obj && e.tree.parent(child) == Some(obj) && focused.is_none() {
                set_dsc_focused(e, obj, Some(child));
                if e.tree.node(obj).is_some_and(|n| n.state.contains(State::FOCUSED)) {
                    e.add_state(child, State::FOCUSED | State::FOCUS_KEY);
                    e.scroll_to_view(child, false);
                }
            }
        }
        EventCode::ChildDeleted => {
            if ev.current_target == obj && focused.is_none() {
                let f = first_focusable(e, obj);
                set_dsc_focused(e, obj, f);
            }
        }
        EventCode::Delete if ev.target == obj => {
            if let Some(i) = e.gridnavs.iter().position(|d| d.container == obj) {
                e.gridnavs.swap_remove(i);
            }
        }
        EventCode::Pressed
        | EventCode::Pressing
        | EventCode::PressLost
        | EventCode::ShortClicked
        | EventCode::LongPressed
        | EventCode::LongPressedRepeat
        | EventCode::Clicked
        | EventCode::Released => {
            // Forward press related events of keys and encoders to the focused child.
            let keys = matches!(
                e.active_input_kind(),
                Some(InputKind::Encoder | InputKind::Keypad)
            );
            let group_focused = e.group_of(obj).is_some_and(|g| e.focused(g) == Some(obj));
            if keys && group_focused {
                if let Some(f) = focused {
                    e.send_event(f, ev.code, ev.param);
                }
            }
        }
        _ => {}
    }
    EventResult::Continue
}

/// LVGL `obj_is_focusable`: not hidden, `CLICKABLE` and `CLICK_FOCUSABLE`.
fn focusable(e: &Engine, n: NodeId) -> bool {
    e.tree
        .node(n)
        .is_some_and(|x| !x.is_hidden() && x.flags.contains(ObjFlags::CLICKABLE | ObjFlags::CLICK_FOCUSABLE))
}

fn first_focusable(e: &Engine, obj: NodeId) -> Option<NodeId> {
    e.tree.children(obj).find(|c| focusable(e, *c))
}

fn last_focusable(e: &Engine, obj: NodeId) -> Option<NodeId> {
    e.tree.children_rev(obj).find(|c| focusable(e, *c))
}

/// LVGL `find_chid`: the focusable child (other than `start`) nearest to `start` in the
/// direction `mode`, by squared center distance. `Left`/`Right` only consider children whose
/// center is within half of `start`'s height vertically.
fn find_child(e: &Engine, obj: NodeId, start: NodeId, mode: Find) -> Option<NodeId> {
    let center = |n: NodeId| {
        let c = e.coords(n);
        (c.x0 + c.width() / 2, c.y0 + c.height() / 2)
    };
    let cont = e.coords(obj);
    let (xs, ys) = center(start);
    let h_half = e.coords(start).height() / 2;
    let h_max = cont.height() + e.scroll_top(obj) + e.scroll_bottom(obj);
    let mut best: Option<(NodeId, i64)> = None;
    for child in e.tree.children(obj) {
        if child == start || !focusable(e, child) {
            continue;
        }
        let (xc, yc) = center(child);
        let c = e.coords(child);
        let (x_err, y_err) = match mode {
            Find::Left | Find::Right => {
                let (xe, ye) = (xc - xs, yc - ys);
                if (mode == Find::Left && xe >= 0) || (mode == Find::Right && xe <= 0) || ye.abs() > h_half {
                    continue;
                }
                (xe, ye)
            }
            Find::Top | Find::Bottom => {
                let (xe, ye) = (xc - xs, yc - ys);
                if (mode == Find::Top && ye >= 0) || (mode == Find::Bottom && ye <= 0) {
                    continue;
                }
                (xe, ye)
            }
            Find::NextRowFirst => {
                let ye = yc - ys;
                if ye <= 0 {
                    continue;
                }
                (c.x0 - cont.x0, ye)
            }
            Find::PrevRowLast => {
                let ye = yc - ys;
                if ye >= 0 {
                    continue;
                }
                (cont.x1 - c.x1, ye)
            }
            Find::FirstRow => (xc - xs, c.y0 - cont.y0),
            Find::LastRow => (xc - xs, h_max - (c.y0 - cont.y0)),
        };
        let d = i64::from(x_err) * i64::from(x_err) + i64::from(y_err) * i64::from(y_err);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((child, d));
        }
    }
    best.map(|(n, _)| n)
}
