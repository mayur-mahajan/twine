//! Event dispatch: class handler, user handlers, filters, bubbling, deletion safety and
//! lifecycle events.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use twine_engine::{
    Engine, EngineConfig, Event, EventCode, EventCx, EventFilter, EventParam, EventResult, NodeId, Obj,
    ObjFlags, Widget, WidgetClass,
};

type Log = Rc<RefCell<Vec<String>>>;

struct Probe(Log, EventResult);
static PROBE: WidgetClass = WidgetClass::new("probe");
impl Widget for Probe {
    fn class(&self) -> &'static WidgetClass {
        &PROBE
    }
    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.code == EventCode::Clicked {
            self.0.borrow_mut().push(format!("class@{}", cx.node().index()));
        }
        self.1
    }
}

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).unwrap()
}

fn push(log: &Log, s: &'static str) -> impl FnMut(&mut EventCx<'_>, &Event) -> EventResult + 'static {
    let l = log.clone();
    move |_, _| {
        l.borrow_mut().push(s.to_string());
        EventResult::Continue
    }
}

fn click(e: &mut Engine, n: NodeId) -> EventResult {
    e.send_event(n, EventCode::Clicked, EventParam::None)
}

#[test]
fn class_handler_runs_before_user_handlers() {
    let mut e = engine();
    let log: Log = Rc::default();
    let n = e
        .create_root(Box::new(Probe(log.clone(), EventResult::Continue)))
        .unwrap();
    e.add_event_handler(n, EventFilter::All, push(&log, "user"));
    click(&mut e, n);
    assert_eq!(*log.borrow(), [format!("class@{}", n.index()), "user".into()]);
}

#[test]
fn user_handlers_run_in_registration_order() {
    let mut e = engine();
    let log: Log = Rc::default();
    let n = e.create_root(Box::new(Obj)).unwrap();
    for s in ["a", "b", "c"] {
        e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), push(&log, s));
    }
    click(&mut e, n);
    assert_eq!(*log.borrow(), ["a", "b", "c"]);
}

#[test]
fn filter_by_code() {
    let mut e = engine();
    let log: Log = Rc::default();
    let n = e.create_root(Box::new(Obj)).unwrap();
    e.add_event_handler(n, EventFilter::Code(EventCode::Pressed), push(&log, "pressed"));
    e.add_event_handler(n, EventFilter::All, push(&log, "all"));
    click(&mut e, n);
    e.send_event(n, EventCode::Pressed, EventParam::None);
    assert_eq!(*log.borrow(), ["all", "pressed", "all"]);
}

#[test]
fn bubbling_requires_flag() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let child = e.create(root, Box::new(Obj)).unwrap();
    e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), push(&log, "root"));
    click(&mut e, child);
    assert!(log.borrow().is_empty());
    e.set_flag(child, ObjFlags::EVENT_BUBBLE, true);
    click(&mut e, child);
    assert_eq!(*log.borrow(), ["root"]);
    // Codes that never bubble.
    e.add_event_handler(
        root,
        EventFilter::Code(EventCode::StyleChanged),
        push(&log, "style"),
    );
    e.send_event(child, EventCode::StyleChanged, EventParam::None);
    assert_eq!(*log.borrow(), ["root"]);
}

#[test]
fn bubbling_reports_current_target() {
    let mut e = engine();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let child = e.create(root, Box::new(Obj)).unwrap();
    e.set_flag(child, ObjFlags::EVENT_BUBBLE, true);
    let seen = Rc::new(RefCell::new(None));
    let s = seen.clone();
    e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), move |cx, ev| {
        *s.borrow_mut() = Some((ev.target, ev.current_target, cx.target(), cx.node()));
        EventResult::Continue
    });
    click(&mut e, child);
    assert_eq!(*seen.borrow(), Some((child, root, child, root)));
}

#[test]
fn stop_bubbling_stops() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let child = e.create(root, Box::new(Obj)).unwrap();
    e.set_flag(child, ObjFlags::EVENT_BUBBLE, true);
    e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), push(&log, "root"));
    let h = e.add_event_handler(child, EventFilter::Code(EventCode::Clicked), |_, _| {
        EventResult::Stop
    });
    e.add_event_handler(child, EventFilter::Code(EventCode::Clicked), push(&log, "child2"));
    assert_eq!(click(&mut e, child), EventResult::Stop);
    assert_eq!(
        *log.borrow(),
        ["child2"],
        "Stop still runs the node's other handlers"
    );
    e.remove_event_handler(child, h);
    e.add_event_handler(child, EventFilter::Code(EventCode::Clicked), |cx, _| {
        cx.stop_bubbling();
        EventResult::Continue
    });
    click(&mut e, child);
    assert_eq!(*log.borrow(), ["child2", "child2"]);
}

#[test]
fn consumed_skips_user_handlers() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let n = e
        .create(root, Box::new(Probe(log.clone(), EventResult::Consumed)))
        .unwrap();
    e.set_flag(n, ObjFlags::EVENT_BUBBLE, true);
    e.add_event_handler(n, EventFilter::All, push(&log, "user"));
    e.add_event_handler(root, EventFilter::All, push(&log, "root"));
    log.borrow_mut().clear();
    assert_eq!(click(&mut e, n), EventResult::Consumed);
    assert_eq!(*log.borrow(), [format!("class@{}", n.index())]);
    // A user handler returning Consumed stops the following ones.
    let m = e.create(root, Box::new(Obj)).unwrap();
    e.add_event_handler(m, EventFilter::All, |_, _| EventResult::Consumed);
    e.add_event_handler(m, EventFilter::All, push(&log, "never"));
    log.borrow_mut().clear();
    click(&mut e, m);
    assert!(log.borrow().is_empty());
}

#[test]
fn handler_deleting_target_stops_dispatch() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let n = e.create(root, Box::new(Obj)).unwrap();
    e.set_flag(n, ObjFlags::EVENT_BUBBLE, true);
    e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), |cx, _| {
        let id = cx.node();
        cx.engine_mut().delete(id).unwrap();
        EventResult::Continue
    });
    e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), push(&log, "after"));
    e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), push(&log, "root"));
    assert_eq!(click(&mut e, n), EventResult::Consumed);
    assert!(log.borrow().is_empty());
    assert!(!e.tree().contains(n));
}

#[test]
fn handler_deleting_parent_during_bubble_is_safe() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mid = e.create(root, Box::new(Obj)).unwrap();
    let leaf = e.create(mid, Box::new(Obj)).unwrap();
    e.set_flag(leaf, ObjFlags::EVENT_BUBBLE, true);
    e.set_flag(mid, ObjFlags::EVENT_BUBBLE, true);
    e.add_event_handler(leaf, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        cx.engine_mut().delete(mid).unwrap();
        EventResult::Continue
    });
    e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), push(&log, "root"));
    assert_eq!(click(&mut e, leaf), EventResult::Consumed);
    assert!(!e.tree().contains(mid) && !e.tree().contains(leaf));
    assert!(log.borrow().is_empty());
    e.tree().check_invariants().unwrap();
}

#[test]
fn handler_removing_itself_is_safe() {
    let mut e = engine();
    let log: Log = Rc::default();
    let n = e.create_root(Box::new(Obj)).unwrap();
    let id = Rc::new(RefCell::new(None));
    let (i, l) = (id.clone(), log.clone());
    let h = e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        l.borrow_mut().push("once".into());
        let node = cx.node();
        assert!(cx.engine_mut().remove_event_handler(node, i.borrow().unwrap()));
        EventResult::Continue
    });
    *id.borrow_mut() = Some(h);
    e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), push(&log, "other"));
    click(&mut e, n);
    click(&mut e, n);
    assert_eq!(*log.borrow(), ["once", "other", "other"]);
}

#[test]
fn handler_added_during_dispatch_runs_next_time() {
    let mut e = engine();
    let log: Log = Rc::default();
    let n = e.create_root(Box::new(Obj)).unwrap();
    let l = log.clone();
    let added = Rc::new(RefCell::new(false));
    e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        if !*added.borrow() {
            *added.borrow_mut() = true;
            let node = cx.node();
            cx.engine_mut()
                .add_event_handler(node, EventFilter::Code(EventCode::Clicked), push(&l, "new"));
        }
        EventResult::Continue
    });
    click(&mut e, n);
    assert!(log.borrow().is_empty());
    click(&mut e, n);
    assert_eq!(*log.borrow(), ["new"]);
}

#[test]
fn delete_sends_delete_bottom_up_then_child_deleted() {
    let mut e = engine();
    let log: Log = Rc::default();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let a = e.create(root, Box::new(Obj)).unwrap();
    let b = e.create(a, Box::new(Obj)).unwrap();
    let c1 = e.create(b, Box::new(Obj)).unwrap();
    let c2 = e.create(b, Box::new(Obj)).unwrap();
    let names = [(root, "root"), (a, "a"), (b, "b"), (c1, "c1"), (c2, "c2")];
    for (n, name) in names {
        let l = log.clone();
        e.add_event_handler(n, EventFilter::All, move |_, ev| {
            if matches!(ev.code, EventCode::Delete | EventCode::ChildDeleted) {
                l.borrow_mut().push(format!("{:?} {name}", ev.code));
            }
            EventResult::Continue
        });
    }
    e.delete(a).unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "Delete c1",
            "Delete c2",
            "Delete b",
            "Delete a",
            "ChildDeleted root"
        ]
    );
}

struct Created(Rc<RefCell<u32>>);
static CREATED: WidgetClass = WidgetClass::new("created");
impl Widget for Created {
    fn class(&self) -> &'static WidgetClass {
        &CREATED
    }
    fn event(&mut self, _cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.code == EventCode::Create {
            *self.0.borrow_mut() += 1;
        }
        EventResult::Continue
    }
}

#[test]
fn create_sends_create_and_child_created() {
    let mut e = engine();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    e.add_event_handler(root, EventFilter::Code(EventCode::ChildCreated), move |cx, ev| {
        s.borrow_mut().push((ev.target, cx.node()));
        EventResult::Continue
    });
    let count = Rc::new(RefCell::new(0));
    let child = e.create(root, Box::new(Created(count.clone()))).unwrap();
    assert_eq!(*count.borrow(), 1);
    assert_eq!(*seen.borrow(), [(child, root)]);
}

#[test]
fn nested_send_from_handler_works() {
    let mut e = engine();
    let log: Log = Rc::default();
    let a = e.create_root(Box::new(Obj)).unwrap();
    let b = e.create_root(Box::new(Obj)).unwrap();
    let custom = e.register_event_code();
    e.add_event_handler(b, EventFilter::Code(custom), push(&log, "b got custom"));
    e.add_event_handler(a, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        cx.send(b, custom, EventParam::Value(7))
    });
    e.add_event_handler(a, EventFilter::Code(EventCode::Clicked), push(&log, "a after"));
    click(&mut e, a);
    assert_eq!(*log.borrow(), ["b got custom", "a after"]);
}

#[test]
fn size_changed_carries_old_area() {
    let mut e = engine();
    let n = e.create_root(Box::new(Obj)).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    e.add_event_handler(n, EventFilter::Code(EventCode::SizeChanged), move |_, ev| {
        s.borrow_mut().push(ev.param);
        EventResult::Continue
    });
    let r0 = twine_core::Rect::from_xywh(0, 0, 10, 10);
    e.place(n, r0);
    e.place(n, twine_core::Rect::from_xywh(5, 5, 10, 10)); // move only: no event
    e.place(n, twine_core::Rect::from_xywh(5, 5, 20, 10));
    assert_eq!(
        *seen.borrow(),
        [
            EventParam::Area(twine_core::Rect::ZERO),
            EventParam::Area(twine_core::Rect::from_xywh(5, 5, 10, 10))
        ]
    );
}
