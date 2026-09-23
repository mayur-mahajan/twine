//! P07.S03: effects, dependency tracking and flushing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::{batch, create_root, debug_stats, flush_effects_with, set_flush_iterations_limit};

fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let c = Rc::new(Cell::new(0));
    (c.clone(), c)
}

#[test]
fn effect_runs_once_on_creation() {
    let cx = create_root();
    let (runs, r) = counter();
    cx.effect(move || r.set(r.get() + 1));
    assert_eq!(runs.get(), 1);
}

#[test]
fn effect_reruns_on_dependency_change() {
    let cx = create_root();
    let a = cx.signal(1);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.effect(move || s.borrow_mut().push(a.get()));
    a.set(2);
    a.set(3);
    assert_eq!(*seen.borrow(), [1, 2, 3]);
}

#[test]
fn effect_does_not_rerun_on_unrelated_change() {
    let cx = create_root();
    let (a, b) = (cx.signal(1), cx.signal(1));
    let (runs, r) = counter();
    cx.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    b.set(2);
    b.set(3);
    assert_eq!(runs.get(), 1);
    a.set(5);
    assert_eq!(runs.get(), 2);
}

#[test]
fn effect_dynamic_dependencies_are_retracked() {
    let cx = create_root();
    let flag = cx.signal(true);
    let (a, b) = (cx.signal(0), cx.signal(0));
    let (runs, r) = counter();
    cx.effect(move || {
        r.set(r.get() + 1);
        if flag.get() {
            b.get()
        } else {
            a.get()
        };
    });
    assert_eq!(runs.get(), 1);
    b.set(1); // dependency
    assert_eq!(runs.get(), 2);
    a.set(1); // not a dependency yet
    assert_eq!(runs.get(), 2);
    flag.set(false);
    assert_eq!(runs.get(), 3);
    b.set(2); // no longer a dependency
    assert_eq!(runs.get(), 3);
    a.set(2);
    assert_eq!(runs.get(), 4);
}

#[test]
fn effect_set_inside_effect_no_borrow_panic() {
    let cx = create_root();
    let a = cx.signal(1);
    let b = cx.signal(0);
    cx.effect(move || {
        let v = a.get();
        b.set(v * 10); // re-enters the runtime from user code (R1)
        let _ = b.get_untracked();
        a.with_untracked(|_| ());
    });
    assert_eq!(b.get(), 10);
    a.set(2);
    assert_eq!(b.get(), 20);
}

#[test]
fn effect_creates_signal_inside() {
    let cx = create_root();
    let a = cx.signal(1);
    let made = Rc::new(RefCell::new(Vec::new()));
    let m = made.clone();
    cx.effect(move || {
        let s = cx.signal(a.get() * 2);
        m.borrow_mut().push(s.get());
    });
    a.set(4);
    assert_eq!(*made.borrow(), [2, 8]);
    assert_eq!(debug_stats().nodes, 4); // a, effect, two inner signals
}

#[test]
fn effect_sets_other_signal_triggers_second_effect_in_same_flush() {
    let cx = create_root();
    let a = cx.signal(1);
    let b = cx.signal(0);
    let log = Rc::new(RefCell::new(Vec::new()));
    let l1 = log.clone();
    cx.effect(move || {
        let v = a.get();
        l1.borrow_mut().push(format!("e1 {v}"));
        b.set(v + 100);
    });
    let l2 = log.clone();
    cx.effect(move || l2.borrow_mut().push(format!("e2 {}", b.get())));
    log.borrow_mut().clear();
    let runs_before = debug_stats().effect_runs;
    let mut ctx = 0u8;
    batch(|| {
        a.set(2);
        flush_effects_with(&mut ctx);
        // Both effects ran within this one flush.
        assert_eq!(*log.borrow(), ["e1 2", "e2 102"]);
    });
    assert_eq!(debug_stats().effect_runs - runs_before, 2);
}

#[test]
fn effect_context_passed_to_effect() {
    let cx = create_root();
    let a = cx.signal(1u32);
    cx.effect_with_cx(move |ctx| {
        let v = a.get();
        if let Some(total) = ctx.downcast_mut::<u32>() {
            *total += v;
        }
    });
    let mut total = 0u32;
    batch(|| {
        a.set(5);
        flush_effects_with(&mut total);
    });
    assert_eq!(total, 5);
    // Effects created during a flush get that flush's context.
    let mut total2 = 0u32;
    let b = cx.signal(0u32);
    cx.effect_with_cx(move |_| {
        if b.get() == 1 {
            cx.effect_with_cx(|ctx| {
                if let Some(t) = ctx.downcast_mut::<u32>() {
                    *t += 1000;
                }
            });
        }
    });
    batch(|| {
        b.set(1);
        flush_effects_with(&mut total2);
    });
    assert_eq!(total2, 1000);
}

#[test]
fn effect_nested_flush_is_noop() {
    let cx = create_root();
    let a = cx.signal(0);
    let b = cx.signal(0);
    let order = Rc::new(RefCell::new(Vec::new()));
    let o = order.clone();
    cx.effect(move || {
        let v = a.get();
        b.set(v);
        flush_effects_with(&mut ()); // no-op: the outer flush continues
        o.borrow_mut().push("e1 end");
    });
    let o = order.clone();
    cx.effect(move || {
        b.get();
        o.borrow_mut().push("e2");
    });
    order.borrow_mut().clear();
    a.set(1);
    assert_eq!(*order.borrow(), ["e1 end", "e2"]);
}

#[test]
fn effect_infinite_loop_is_cut_and_logged() {
    let cx = create_root();
    set_flush_iterations_limit(10);
    let a = cx.signal(0u32);
    let (runs, r) = counter();
    cx.effect(move || {
        r.set(r.get() + 1);
        let v = a.get();
        a.set(v + 1);
    });
    assert_eq!(runs.get(), 11); // limit + 1
    assert_eq!(debug_stats().loop_cuts, 1);
    assert_eq!(debug_stats().pending, 0);
    // The runtime is still usable afterwards.
    let (runs2, r2) = counter();
    let b = cx.signal(0);
    cx.effect(move || {
        b.get();
        r2.set(r2.get() + 1);
    });
    b.set(1);
    assert_eq!(runs2.get(), 2);
}

#[test]
fn effect_many_independent_effects_are_not_a_loop() {
    // 500 effects on one signal is one round, far below the limit (fewer under Miri).
    const N: u32 = if cfg!(miri) { 150 } else { 500 };
    let cx = create_root();
    let a = cx.signal(0);
    let (runs, r) = counter();
    for _ in 0..N {
        let r = r.clone();
        cx.effect(move || {
            a.get();
            r.set(r.get() + 1);
        });
    }
    a.set(1);
    assert_eq!(runs.get(), 2 * N);
    assert_eq!(debug_stats().loop_cuts, 0);
}

#[test]
fn effect_disposed_effect_never_runs() {
    let cx = create_root();
    let a = cx.signal(0);
    let (runs, r) = counter();
    let e = cx.effect(move || {
        a.get();
        r.set(r.get() + 1);
    });
    assert!(e.is_alive());
    e.dispose();
    e.dispose(); // no-op
    assert!(!e.is_alive());
    a.set(1);
    assert_eq!(runs.get(), 1);
    // Disposed while queued in a batch.
    let (runs2, r2) = counter();
    let e2 = cx.effect(move || {
        a.get();
        r2.set(r2.get() + 1);
    });
    batch(|| {
        a.set(2);
        e2.dispose();
    });
    assert_eq!(runs2.get(), 1);
    assert_eq!(debug_stats().pending, 0);
}

#[test]
fn effect_panicking_effect_leaves_runtime_usable() {
    let cx = create_root();
    let a = cx.signal(0);
    cx.effect(move || {
        assert!(a.get() != 1, "boom");
    });
    let r = std::panic::catch_unwind(|| a.set(1));
    assert!(r.is_err());
    // Flushing works again.
    let (runs, rr) = counter();
    let b = cx.signal(0);
    cx.effect(move || {
        b.get();
        rr.set(rr.get() + 1);
    });
    b.set(1);
    assert_eq!(runs.get(), 2);
}
