//! `batch`, `untrack`, deferred effects.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let c = Rc::new(Cell::new(0));
    (c.clone(), c)
}

#[test]
fn batch_defers_effects_until_end() {
    let cx = rt().create_root();
    let (a, b) = (cx.signal(1), cx.signal(2));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.effect(move || s.borrow_mut().push(a.get() + b.get()));
    rt().batch(|| {
        a.set(10);
        assert_eq!(*seen.borrow(), [3], "no effect inside the batch");
        b.set(20);
        assert!(rt().has_pending_effects());
    });
    assert_eq!(*seen.borrow(), [3, 30]);
}

#[test]
fn batch_nested_batch_flushes_once_at_outer_end() {
    let cx = rt().create_root();
    let a = cx.signal(0);
    let (runs, r) = counter();
    cx.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    rt().batch(|| {
        a.set(1);
        rt().batch(|| a.set(2));
        assert_eq!(runs.get(), 1, "inner batch end does not flush");
        a.set(3);
    });
    assert_eq!(runs.get(), 2);
}

#[test]
fn batch_returns_value() {
    let cx = rt().create_root();
    let a = cx.signal(4);
    assert_eq!(rt().batch(|| a.get() * 2), 8);
    assert_eq!(rt().batch(|| "x"), "x");
}

#[test]
fn batch_panic_restores_depth() {
    let cx = rt().create_root();
    let a = cx.signal(0);
    let (runs, r) = counter();
    cx.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    let res = std::panic::catch_unwind(|| {
        rt().batch(|| {
            a.set(1);
            panic!("inside batch");
        })
    });
    assert!(res.is_err());
    assert_eq!(runs.get(), 1, "no effects run while unwinding");
    // Depth is back to 0: a plain write flushes immediately (including the queued effect).
    a.set(2);
    assert_eq!(runs.get(), 2);
}

#[test]
fn untrack_does_not_subscribe() {
    let cx = rt().create_root();
    let (a, b) = (cx.signal(0), cx.signal(0));
    let m = cx.memo(move || a.get() + rt().untrack(|| b.get()));
    let (runs, r) = counter();
    cx.effect(move || {
        rt().untrack(|| a.get());
        m.get();
        r.set(r.get() + 1);
    });
    b.set(5);
    assert_eq!(runs.get(), 1);
    assert_eq!(m.get(), 0, "memo did not subscribe to b");
    a.set(1);
    assert_eq!(runs.get(), 2);
    assert_eq!(m.get(), 6);
    // untrack restores tracking afterwards and returns the value.
    assert_eq!(rt().untrack(|| 42), 42);
}

struct TestEngine {
    applied: Vec<i32>,
}

#[test]
fn defer_deferred_effect_waits_for_real_context() {
    let cx = rt().create_root();
    let a = cx.signal(1);
    let (plain_runs, pr) = counter();
    cx.effect_with_cx(move |ctx| {
        let v = a.get();
        if let Some(engine) = ctx.downcast_mut::<TestEngine>() {
            engine.applied.push(v);
        } else {
            pr.set(pr.get() + 1);
            rt().defer_current_effect();
        }
    });
    assert_eq!(plain_runs.get(), 1);
    assert!(rt().has_pending_effects());
    assert_eq!(rt().stats().deferred, 1);
    // A `()` flush (automatic, after a set) does not run it again.
    a.set(2);
    rt().flush_effects_with(&mut ());
    assert_eq!(plain_runs.get(), 1);
    let mut engine = TestEngine { applied: Vec::new() };
    rt().flush_effects_with(&mut engine);
    assert_eq!(engine.applied, [2]);
    assert!(!rt().has_pending_effects());
    // Later changes flushed with the engine apply directly.
    rt().batch(|| {
        a.set(3);
        rt().flush_effects_with(&mut engine);
    });
    assert_eq!(engine.applied, [2, 3]);
}

#[test]
fn defer_outside_effect_is_ignored() {
    let _cx = rt().create_root();
    rt().defer_current_effect();
    assert!(!rt().has_pending_effects());
}

#[test]
fn defer_has_pending_reports_deferred() {
    let cx = rt().create_root();
    assert!(!rt().has_pending_effects());
    cx.effect(|| rt().defer_current_effect());
    assert!(rt().has_pending_effects());
    let st = rt().stats();
    assert_eq!((st.pending, st.deferred), (0, 1));
    rt().flush_effects_with(&mut 0u8); // runs (and defers) again
    assert!(rt().has_pending_effects());
    cx.dispose();
    assert!(!rt().has_pending_effects(), "disposal removes deferred effects");
}
