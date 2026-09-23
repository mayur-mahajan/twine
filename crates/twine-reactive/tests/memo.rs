//! Memos and push-pull coloring.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::{Memo, batch, create_root, debug_stats};

fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let c = Rc::new(Cell::new(0));
    (c.clone(), c)
}

#[test]
fn memo_is_lazy() {
    let cx = create_root();
    let a = cx.signal(1);
    let (runs, r) = counter();
    let m = cx.memo(move || {
        r.set(r.get() + 1);
        a.get() + 1
    });
    assert_eq!(runs.get(), 0);
    a.set(2);
    assert_eq!(runs.get(), 0, "no reader, no recompute");
    assert_eq!(m.get(), 3);
    assert_eq!(runs.get(), 1);
}

#[test]
fn memo_caches_until_dependency_changes() {
    let cx = create_root();
    let a = cx.signal(1);
    let (runs, r) = counter();
    let m = cx.memo(move || {
        r.set(r.get() + 1);
        a.get() * 2
    });
    assert_eq!(m.get(), 2);
    assert_eq!(m.get(), 2);
    assert_eq!(m.get_untracked(), 2);
    assert_eq!(m.with(|v| *v), 2);
    assert_eq!(runs.get(), 1);
    a.set(5);
    a.set(6);
    assert_eq!(m.get(), 12);
    assert_eq!(runs.get(), 2);
    assert_eq!(m.try_get(), Some(12));
}

#[test]
fn memo_diamond_runs_effect_once_per_change() {
    let cx = create_root();
    let a = cx.signal(1);
    let b = cx.memo(move || a.get() + 1);
    let c = cx.memo(move || a.get() * 10);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.effect(move || s.borrow_mut().push((b.get(), c.get())));
    for v in 2..6 {
        a.set(v);
    }
    // One run per write, and every observation is consistent (b = a + 1, c = 10 a).
    assert_eq!(*seen.borrow(), [(2, 10), (3, 20), (4, 30), (5, 40), (6, 50)]);
    // Through a memo diamond D = B + C.
    let d = cx.memo(move || b.get() + c.get());
    let (runs, r) = counter();
    cx.effect(move || {
        d.get();
        r.set(r.get() + 1);
    });
    a.set(7);
    assert_eq!(runs.get(), 2);
    assert_eq!(d.get(), 8 + 70);
}

#[test]
fn memo_equal_value_stops_propagation() {
    let cx = create_root();
    let x = cx.signal(1);
    let (memo_runs, mr) = counter();
    let parity = cx.memo(move || {
        mr.set(mr.get() + 1);
        x.get() % 2
    });
    let (runs, r) = counter();
    cx.effect(move || {
        parity.get();
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1);
    x.set(3); // parity unchanged
    x.set(5);
    assert_eq!(runs.get(), 1);
    assert_eq!(memo_runs.get(), 3);
    x.set(4); // parity changed
    assert_eq!(runs.get(), 2);
    // Downstream memo is not recomputed either.
    let (down_runs, dr) = counter();
    let down = parity.map(move |p| {
        dr.set(dr.get() + 1);
        p * 100
    });
    assert_eq!(down.get(), 0);
    x.set(6);
    assert_eq!(down.get(), 0);
    assert_eq!(down_runs.get(), 1);
}

#[test]
fn memo_chain_of_10_memos_recomputes_once_each() {
    let cx = create_root();
    let a = cx.signal(0u64);
    let runs = Rc::new(RefCell::new([0u32; 10]));
    let mut prev: Option<Memo<u64>> = None;
    for i in 0..10 {
        let runs = runs.clone();
        let p = prev;
        prev = Some(cx.memo(move || {
            runs.borrow_mut()[i] += 1;
            p.map_or_else(|| a.get(), |p| p.get()) + 1
        }));
    }
    let last = prev.unwrap();
    let (eff, e) = counter();
    cx.effect(move || {
        last.get();
        e.set(e.get() + 1);
    });
    assert_eq!(*runs.borrow(), [1; 10]);
    a.set(1);
    assert_eq!(*runs.borrow(), [2; 10]);
    assert_eq!(last.get(), 11);
    assert_eq!(eff.get(), 2);
}

#[test]
#[should_panic(expected = "disposed")]
fn memo_reading_disposed_signal_panics() {
    let root = create_root();
    let child = root.child();
    let s = child.signal(1);
    let m = root.memo(move || s.get());
    child.dispose();
    let _ = m.get();
}

#[test]
#[should_panic(expected = "memo used after its scope was disposed")]
fn memo_use_after_dispose_panics() {
    let cx = create_root();
    let m = cx.memo(|| 1);
    cx.dispose();
    let _ = m.get();
}

#[test]
#[should_panic(expected = "cycle")]
fn memo_cycle_panics() {
    let cx = create_root();
    let slot: Rc<Cell<Option<Memo<u32>>>> = Rc::new(Cell::new(None));
    let s = slot.clone();
    let m = cx.memo(move || s.get().map_or(0, |m| m.get()) + 1);
    slot.set(Some(m));
    let _ = m.get();
}

#[test]
fn memo_deep_chain_depth_guard_logs() {
    // 300 chained memos: checking recurses once per level. Run on a thread with a large stack
    // (it has its own runtime) so the test does not depend on the harness's default stack.
    let hits = std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(|| {
            let cx = create_root();
            let a = cx.signal(1u64);
            let mut prev: Option<Memo<u64>> = None;
            for _ in 0..300 {
                let p = prev;
                prev = Some(cx.memo(move || p.map_or_else(|| a.get(), |p| p.get()) + 1));
            }
            let last = prev.unwrap();
            assert_eq!(last.get(), 301);
            assert_eq!(debug_stats().depth_guard_hits, 0);
            a.set(2);
            assert_eq!(last.get(), 302, "values stay correct past the guard");
            debug_stats().depth_guard_hits
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(hits > 0, "guard must trigger for a 300-deep check");
}

#[test]
fn memo_map_on_signal_readsignal_and_memo() {
    let cx = create_root();
    let a = cx.signal(2);
    let m1 = a.map(|v| v + 1);
    let m2 = a.read_only().map(|v| v * 2);
    let m3 = m1.map(|v| v * 100);
    assert_eq!((m1.get(), m2.get(), m3.get()), (3, 4, 300));
    batch(|| a.set(3));
    assert_eq!((m1.get(), m2.get(), m3.get()), (4, 6, 400));
    assert_eq!(m3.with_untracked(|v| *v), 400);
}

#[test]
fn memo_can_create_and_set_signals_inside() {
    // R1: memo bodies may re-enter the runtime.
    let cx = create_root();
    let a = cx.signal(1);
    let side = cx.signal(0);
    let m = cx.memo(move || {
        let tmp = cx.signal(a.get());
        side.set(tmp.get_untracked());
        tmp.get_untracked() * 3
    });
    assert_eq!(m.get(), 3);
    assert_eq!(side.get(), 1);
    a.set(2);
    assert_eq!(m.get(), 6);
}
