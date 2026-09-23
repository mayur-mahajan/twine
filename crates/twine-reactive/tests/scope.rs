//! Scopes — nesting, disposal, cleanup, context.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::{create_root, debug_stats};

type Log = Rc<RefCell<Vec<String>>>;

fn log() -> (Log, Log) {
    let l: Log = Rc::default();
    (l.clone(), l)
}

#[test]
fn scope_dispose_removes_nodes_and_effects_stop() {
    let root = create_root();
    let a = root.signal(0);
    let child = root.child();
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    child.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    let m = child.memo(move || a.get());
    assert_eq!(debug_stats().nodes, 3);
    child.dispose();
    assert_eq!(debug_stats().nodes, 1);
    assert!(!m.is_alive());
    a.set(1);
    assert_eq!(runs.get(), 1);
}

#[test]
fn scope_child_disposed_with_parent() {
    let root = create_root();
    let c1 = root.child();
    let c2 = c1.child();
    let s = c2.signal(1);
    root.dispose();
    assert!(!c1.is_alive() && !c2.is_alive() && !root.is_alive());
    assert!(!s.is_alive());
    let st = debug_stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
}

#[test]
fn scope_dispose_order_children_reverse_then_cleanups_reverse() {
    let root = create_root();
    let (order, o) = log();
    let push = |o: &Log, s: &str| {
        let o = o.clone();
        let s = s.to_string();
        move || o.borrow_mut().push(s)
    };
    root.on_cleanup(push(&o, "root cleanup 1"));
    let c1 = root.child();
    c1.on_cleanup(push(&o, "c1 cleanup"));
    let c1a = c1.child();
    c1a.on_cleanup(push(&o, "c1a cleanup"));
    let c2 = root.child();
    c2.on_cleanup(push(&o, "c2 cleanup 1"));
    c2.on_cleanup(push(&o, "c2 cleanup 2"));
    root.on_cleanup(push(&o, "root cleanup 2"));
    root.dispose();
    assert_eq!(
        *order.borrow(),
        [
            "c2 cleanup 2",
            "c2 cleanup 1",
            "c1a cleanup",
            "c1 cleanup",
            "root cleanup 2",
            "root cleanup 1",
        ]
    );
}

#[test]
fn scope_cleanup_can_set_signals_and_create_scopes() {
    let root = create_root();
    let status = root.signal(String::from("open"));
    let (seen, s) = log();
    root.effect(move || s.borrow_mut().push(status.get()));
    let child = root.child();
    let made = Rc::new(Cell::new(None));
    let m = made.clone();
    child.on_cleanup(move || {
        // R1: the runtime is fully usable inside a cleanup.
        status.set("closing".into());
        status.set("closed".into());
        let other = root.child();
        other.signal(5);
        m.set(Some(other));
    });
    child.dispose();
    // Writes made during dispose are batched: the effect ran once, with the final value.
    assert_eq!(*seen.borrow(), ["open", "closed"]);
    let other = made.get().unwrap();
    assert!(other.is_alive());
    root.dispose();
    assert!(!other.is_alive());
    let st = debug_stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
}

#[test]
fn scope_cleanup_can_register_on_disposing_scope() {
    let root = create_root();
    let (order, o) = log();
    let o2 = o.clone();
    root.on_cleanup(move || {
        o.borrow_mut().push("first".into());
        let o3 = o.clone();
        root.on_cleanup(move || o3.borrow_mut().push("late".into()));
    });
    root.dispose();
    assert_eq!(*order.borrow(), ["first", "late"]);
    drop(o2);
}

#[test]
fn scope_context_lookup_walks_parents() {
    let root = create_root();
    root.provide(7u32);
    root.provide("theme");
    let deep = root.child().child().child();
    assert_eq!(deep.use_context::<u32>(), Some(7));
    assert_eq!(deep.use_context::<&str>(), Some("theme"));
    assert_eq!(deep.use_context::<u64>(), None);
    assert_eq!(deep.expect_context::<u32>(), 7);
}

#[test]
fn scope_context_shadowing() {
    let root = create_root();
    root.provide(1u8);
    let child = root.child();
    child.provide(2u8);
    let grandchild = child.child();
    assert_eq!(grandchild.use_context::<u8>(), Some(2));
    assert_eq!(root.use_context::<u8>(), Some(1));
    // Providing again in the same scope replaces.
    root.provide(3u8);
    assert_eq!(root.use_context::<u8>(), Some(3));
    assert_eq!(grandchild.use_context::<u8>(), Some(2));
    child.dispose();
    assert_eq!(root.use_context::<u8>(), Some(3));
}

#[test]
#[should_panic(expected = "context u16 not provided")]
fn scope_expect_context_panics_with_type_name() {
    let root = create_root();
    let _: u16 = root.expect_context();
}

#[test]
fn scope_double_dispose_noop() {
    let root = create_root();
    let child = root.child();
    let n = Rc::new(Cell::new(0));
    let c = n.clone();
    child.on_cleanup(move || c.set(c.get() + 1));
    child.dispose();
    child.dispose();
    root.dispose();
    root.dispose();
    assert_eq!(n.get(), 1);
    // Operations on a dead scope that cannot fail loudly are ignored.
    root.on_cleanup(|| panic!("never runs"));
    root.provide(1u8);
    assert_eq!(root.use_context::<u8>(), None);
}

#[test]
#[should_panic(expected = "scope used after it was disposed")]
fn scope_creating_signal_on_disposed_scope_panics() {
    let root = create_root();
    root.dispose();
    let _ = root.signal(1);
}

#[test]
fn scope_disposing_scope_inside_its_own_effect_is_safe() {
    let root = create_root();
    let a = root.signal(0);
    let child = root.child();
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let after = Rc::new(Cell::new(0));
    let af = after.clone();
    child.effect(move || {
        r.set(r.get() + 1);
        if a.get() == 1 {
            child.dispose(); // disposes this very effect while it runs
        }
    });
    root.effect(move || {
        a.get();
        af.set(af.get() + 1);
    });
    a.set(1);
    assert_eq!(runs.get(), 2);
    assert_eq!(after.get(), 2, "the flush continued after the disposal");
    a.set(2);
    assert_eq!(runs.get(), 2);
    assert!(!child.is_alive());
}

#[test]
fn scope_no_leaks_after_root_dispose() {
    let root = create_root();
    let a = root.signal(1);
    for i in 0..10 {
        let c = root.child();
        let s = c.signal(i);
        let m = c.memo(move || a.get() + s.get());
        c.effect(move || {
            m.get();
        });
        c.provide(i);
        c.on_cleanup(|| {});
    }
    root.dispose();
    let st = debug_stats();
    assert_eq!((st.nodes, st.scopes, st.pending, st.deferred), (0, 0, 0, 0));
}

#[test]
fn scope_values_and_closures_dropped_on_dispose() {
    let root = create_root();
    let token = Rc::new(());
    let t1 = token.clone();
    let t2 = token.clone();
    let t3 = token.clone();
    root.signal(token.clone());
    root.effect(move || {
        let _ = &t1;
    });
    root.on_cleanup(move || drop(t2));
    root.provide(t3);
    assert_eq!(Rc::strong_count(&token), 5);
    root.dispose();
    assert_eq!(Rc::strong_count(&token), 1);
}
