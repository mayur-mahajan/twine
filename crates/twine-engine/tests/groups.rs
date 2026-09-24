//! Focus groups: focus order, wrap, edge callback, hidden/disabled skipping, edit mode,
//! freezing, refocus on delete, default-group auto-add and pointer click focus.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{clickable, white_screen};
use twine_core::{Color, Duration, Point, Rect};
use twine_engine::{
    Engine, EngineConfig, GroupDef, GroupId, NodeId, Obj, ObjFlags, RefocusPolicy, State, Widget, WidgetClass,
};
use twine_style::{Selector, StyleProp};
use twine_testing::EngineHarness;

fn engine_with(n: usize) -> (Engine, GroupId, Vec<NodeId>) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let g = e.create_group().unwrap();
    let nodes: Vec<NodeId> = (0..n)
        .map(|_| {
            let id = e.create(root, Box::new(Obj)).unwrap();
            e.group_add(g, id);
            id
        })
        .collect();
    (e, g, nodes)
}

fn st(e: &Engine, n: NodeId) -> State {
    e.tree().node(n).unwrap().state()
}

#[test]
fn first_added_gets_focus() {
    let (e, g, n) = engine_with(3);
    assert_eq!(e.focused(g), Some(n[0]));
    assert!(st(&e, n[0]).contains(State::FOCUSED));
    assert!(!st(&e, n[0]).contains(State::FOCUS_KEY), "no keypad involved");
    assert_eq!(e.group_count(g), 3);
    assert_eq!(e.group_of(n[2]), Some(g));
}

#[test]
fn focus_next_prev_wrap() {
    let (mut e, g, n) = engine_with(3);
    e.focus_next(g);
    e.focus_next(g);
    assert_eq!(e.focused(g), Some(n[2]));
    assert!(!st(&e, n[0]).contains(State::FOCUSED));
    e.focus_next(g);
    assert_eq!(e.focused(g), Some(n[0]));
    e.focus_prev(g);
    assert_eq!(e.focused(g), Some(n[2]));
}

#[test]
fn no_wrap_calls_edge_cb() {
    let (mut e, g, n) = engine_with(2);
    e.set_wrap(g, false);
    let edges = Rc::new(RefCell::new(Vec::new()));
    let ed = edges.clone();
    e.set_edge_cb(g, Some(Box::new(move |_, next| ed.borrow_mut().push(next))));
    e.focus_prev(g);
    assert_eq!(e.focused(g), Some(n[0]));
    e.focus_next(g);
    e.focus_next(g);
    assert_eq!(e.focused(g), Some(n[1]));
    assert_eq!(*edges.borrow(), [false, true]);
}

#[test]
fn skips_hidden_and_disabled() {
    let (mut e, g, n) = engine_with(4);
    e.set_flag(n[1], ObjFlags::HIDDEN, true);
    e.add_state(n[2], State::DISABLED);
    e.focus_next(g);
    assert_eq!(e.focused(g), Some(n[3]));
    // A hidden ancestor hides the node too.
    let root = e.tree().parent(n[0]).unwrap();
    let inner = e.create(n[3], Box::new(Obj)).unwrap();
    e.group_add(g, inner);
    e.set_flag(n[3], ObjFlags::HIDDEN, true);
    e.focus_next(g);
    assert_eq!(e.focused(g), Some(n[0]));
    let _ = root;
}

#[test]
fn editing_adds_edited_state() {
    let (mut e, g, n) = engine_with(2);
    e.set_editing(g, true);
    assert!(e.group_editing(g));
    assert!(st(&e, n[0]).contains(State::EDITED | State::FOCUSED));
    e.set_editing(g, false);
    assert!(!st(&e, n[0]).contains(State::EDITED));
    // Focusing another node leaves edit mode.
    e.set_editing(g, true);
    e.focus(n[1]);
    assert!(!e.group_editing(g));
    assert!(!st(&e, n[1]).contains(State::EDITED));
}

#[test]
fn frozen_group_ignores_focus_changes() {
    let (mut e, g, n) = engine_with(3);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let c = calls.clone();
    e.set_focus_cb(g, Some(Box::new(move |_, id| c.borrow_mut().push(id))));
    e.focus_freeze(g, true);
    e.focus_next(g);
    e.focus(n[2]);
    assert_eq!(e.focused(g), Some(n[0]));
    e.focus_freeze(g, false);
    e.focus(n[2]);
    assert_eq!(e.focused(g), Some(n[2]));
    assert_eq!(*calls.borrow(), [n[2]]);
}

#[test]
fn delete_focused_refocuses_next() {
    let (mut e, g, n) = engine_with(3);
    e.set_refocus_policy(g, RefocusPolicy::Next);
    e.focus(n[1]);
    e.delete(n[1]).unwrap();
    assert_eq!(e.focused(g), Some(n[2]));
    assert_eq!(e.group_count(g), 2);
}

#[test]
fn delete_focused_refocuses_prev_with_policy() {
    let (mut e, g, n) = engine_with(3);
    e.set_refocus_policy(g, RefocusPolicy::Prev);
    e.focus(n[1]);
    e.delete(n[1]).unwrap();
    assert_eq!(e.focused(g), Some(n[0]));
    // Removing the last node leaves no focus; stale ids are ignored.
    e.delete(n[0]).unwrap();
    e.delete(n[2]).unwrap();
    assert_eq!(e.focused(g), None);
    assert_eq!(e.group_count(g), 0);
    e.focus(n[0]);
    e.group_add(g, n[0]);
    e.group_remove(n[0]);
    assert_eq!(e.group_count(g), 0);
}

#[test]
fn default_group_auto_add_by_class() {
    struct Knob;
    static KNOB: WidgetClass = WidgetClass::new("knob").group_def(GroupDef::True);
    impl Widget for Knob {
        fn class(&self) -> &'static WidgetClass {
            &KNOB
        }
    }
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let g = e.create_group().unwrap();
    e.set_default_group(Some(g));
    let k = e.create(root, Box::new(Knob)).unwrap();
    let o = e.create(root, Box::new(Obj)).unwrap();
    assert_eq!(e.group_of(k), Some(g));
    assert_eq!(e.group_of(o), None);
    assert_eq!(e.focused(g), Some(k));
    e.delete_group(g);
    assert_eq!(e.default_group(), None);
    assert_eq!(e.group_of(k), None);
    assert!(!st(&e, k).contains(State::FOCUSED));
}

#[test]
fn pointer_click_focuses_without_focus_key() {
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        for x in [0, 50] {
            let b = clickable(e, s, Rect::from_xywh(x, 0, 40, 40));
            e.group_add(g, b);
            ids.push(b);
        }
    });
    h.run_until_idle();
    let g = h.engine().group_of(ids[0]).unwrap();
    h.tap(Point::new(60, 10));
    assert_eq!(h.engine().focused(g), Some(ids[1]));
    let s = st(h.engine(), ids[1]);
    assert!(s.contains(State::FOCUSED) && !s.contains(State::FOCUS_KEY));
    assert!(!st(h.engine(), ids[0]).contains(State::FOCUSED));
}

#[test]
fn focus_state_change_invalidates_only_nodes() {
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        for x in [0, 50] {
            let b = clickable(e, s, Rect::from_xywh(x, 10, 40, 40));
            e.set_local_prop(
                b,
                Selector::MAIN.with_state(State::FOCUSED),
                StyleProp::BgColor(Color::RED),
            );
            e.group_add(g, b);
            ids.push(b);
        }
    });
    h.run_until_idle();
    let g = h.engine().group_of(ids[0]).unwrap();
    h.clock().advance(Duration::ms(20));
    h.engine_mut().focus_next(g);
    h.update();
    let mut areas: Vec<Rect> = h.flushes().iter().map(|f| f.area).collect();
    areas.sort_by_key(|r| r.x0);
    assert_eq!(h.last_frame().dirty_px, 2 * 40 * 40, "{areas:?}");
    assert_eq!(h.pixel(60, 20), Color::RED);
}
