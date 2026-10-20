//! Scopes — nesting, disposal, cleanup (context values: `tests/context.rs`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

type Log = Rc<RefCell<Vec<String>>>;

fn log() -> (Log, Log) {
    let l: Log = Rc::default();
    (l.clone(), l)
}

#[test]
fn scope_dispose_removes_nodes_and_effects_stop() {
    let root = rt().create_root();
    let a = root.signal(0);
    let child = root.child();
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    child.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    let m = child.memo(move || a.get());
    assert_eq!(rt().stats().nodes, 3);
    child.dispose();
    assert_eq!(rt().stats().nodes, 1);
    assert!(!m.is_alive());
    a.set(1);
    assert_eq!(runs.get(), 1);
}

#[test]
fn scope_child_disposed_with_parent() {
    let root = rt().create_root();
    let c1 = root.child();
    let c2 = c1.child();
    let s = c2.signal(1);
    root.dispose();
    assert!(!c1.is_alive() && !c2.is_alive() && !root.is_alive());
    assert!(!s.is_alive());
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
}

#[test]
fn scope_dispose_order_children_reverse_then_cleanups_reverse() {
    let root = rt().create_root();
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
    let root = rt().create_root();
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
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
}

#[test]
fn scope_cleanup_can_register_on_disposing_scope() {
    let root = rt().create_root();
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
fn scope_double_dispose_noop() {
    let root = rt().create_root();
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
    assert!(root.use_context::<u8>().is_err());
}

// With `no-panic-runtime` the creation is skipped (tests/no_panic_runtime.rs).
#[cfg(not(feature = "no-panic-runtime"))]
#[test]
#[should_panic(expected = "scope used after it was disposed")]
fn scope_creating_signal_on_disposed_scope_panics() {
    let root = rt().create_root();
    root.dispose();
    let _ = root.signal(1);
}

#[test]
fn scope_disposing_scope_inside_its_own_effect_is_safe() {
    let root = rt().create_root();
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
    let root = rt().create_root();
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
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes, st.pending, st.deferred), (0, 0, 0, 0));
}

#[test]
fn scope_values_and_closures_dropped_on_dispose() {
    let root = rt().create_root();
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

// Not built under Miri: wearing out a slot takes 65 535 dispose cycles by definition (instant
// natively, hours under Miri); the retirement logic itself has no `unsafe` (it is twine-core's
// `Arena`, `#![forbid(unsafe_code)]`), so Miri has nothing to add here. Not built in the safety
// profile either: its 48-bit generations need 2.8 × 10¹⁴ cycles (twine-core tests that width's
// retirement path directly).
#[cfg(not(any(miri, feature = "safety")))]
#[test]
fn runtime_stats_reports_retired_slots() {
    // A fresh runtime (one per test thread): the child scope's slot is the only free one, so
    // it is reused until its 65 535 generations are spent, then retired.
    let root = rt().create_root();
    assert_eq!(rt().stats().retired_slots, 0);
    for _ in 0..u16::MAX - 1 {
        root.child().dispose();
    }
    assert_eq!(rt().stats().retired_slots, 0);
    root.child().dispose();
    assert_eq!(rt().stats().retired_slots, 1);
    // A retired slot is replaced: scopes keep working.
    let c = root.child();
    let s = c.signal(1);
    assert_eq!(s.get(), 1);
    c.dispose();
    assert_eq!(rt().stats().scopes, 1);
}

/// Disposal walks the scope tree through its links, not by recursion: a chain far deeper than
/// any recursion budget is disposed on a test thread's default stack (it used to recurse once
/// per level).
#[test]
fn regression_deep_scope_chain_disposal_does_not_recurse() {
    // Scaled under Miri (slow); natively deep enough (`max_scopes` allows 65 535) to
    // overflow a recursive disposal on the 2 MiB test-thread stack.
    let depth = if cfg!(miri) { 300 } else { 60_000 };
    let root = rt().create_root();
    let (order, o) = log();
    let mut s = root;
    for i in 0..depth {
        s = s.child();
        s.signal(i);
        if i % 20_000 == 0 {
            let o = o.clone();
            s.on_cleanup(move || o.borrow_mut().push(format!("{i}")));
        }
    }
    root.dispose();
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
    // Deepest first.
    let expected: Vec<String> = (0..depth)
        .filter(|i| i % 20_000 == 0)
        .rev()
        .map(|i| format!("{i}"))
        .collect();
    assert_eq!(*order.borrow(), expected);
}

/// A cleanup deep in the tree that disposes the root, a sibling or its own parent ends or
/// redirects the walk without touching dead scopes twice.
#[test]
fn scope_cleanup_disposing_ancestors_or_siblings_is_safe() {
    let root = rt().create_root();
    let (order, o) = log();
    let a = root.child();
    let b = root.child();
    let b1 = b.child();
    let push = |o: &Log, s: &'static str| {
        let o = o.clone();
        move || o.borrow_mut().push(s.to_string())
    };
    a.on_cleanup(push(&o, "a"));
    b.on_cleanup(push(&o, "b"));
    let o2 = o.clone();
    b1.on_cleanup(move || {
        o2.borrow_mut().push("b1".into());
        a.dispose(); // a sibling of the scope being disposed
        b.dispose(); // its own parent, mid-walk
    });
    root.on_cleanup(push(&o, "root"));
    root.dispose();
    assert_eq!(*order.borrow(), ["b1", "a", "b", "root"]);
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));

    // A cleanup disposing the root that is being disposed ends the walk cleanly.
    let root = rt().create_root();
    let c = root.child().child();
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    c.on_cleanup(move || {
        r.set(r.get() + 1);
        root.dispose();
    });
    root.on_cleanup({
        let r = ran.clone();
        move || r.set(r.get() + 10)
    });
    root.dispose();
    assert_eq!(ran.get(), 11, "each cleanup ran once");
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes), (0, 0));
}
