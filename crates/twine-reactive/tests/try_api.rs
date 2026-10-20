//! The `try_*` forms: every failure is an `Err` naming the handle kind, never a panic, and
//! never a fault (the caller decides). Runs in every profile.
//!
//! req: SAF-006, SAF-007

use std::cell::Cell;
use std::rc::Rc;

use twine_reactive::{HandleKind, Memo, ReactiveError, Runtime};

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

/// Every fault the runtime recorded since the last call, as a sum over kinds.
fn faults() -> u32 {
    let counts = rt().take_faults();
    counts.kinds().iter().map(|k| counts.get(k)).sum()
}

#[test]
fn try_forms_on_disposed_handles_are_errors() {
    let _ = faults();
    let cx = rt().create_root();
    let s = cx.signal(1u32);
    let (r, w) = s.split();
    let m = cx.memo(move || s.get() * 2);
    let v = cx.stored_value(String::from("a"));
    cx.dispose();

    let signal = Err(ReactiveError::Disposed(HandleKind::Signal));
    assert_eq!(s.try_get(), signal);
    assert_eq!(s.try_with(|n| *n), signal);
    assert_eq!(s.try_set(2), Err(ReactiveError::Disposed(HandleKind::Signal)));
    assert_eq!(
        s.try_update(|n| *n += 1),
        Err(ReactiveError::Disposed(HandleKind::Signal))
    );
    assert_eq!(r.try_get(), signal);
    assert_eq!(r.try_with(|n| *n), signal);
    assert_eq!(w.try_set(3), Err(ReactiveError::Disposed(HandleKind::Signal)));
    assert_eq!(
        w.try_update(|n| *n += 1),
        Err(ReactiveError::Disposed(HandleKind::Signal))
    );
    assert_eq!(m.try_get(), Err(ReactiveError::Disposed(HandleKind::Memo)));
    assert_eq!(m.try_with(|n| *n), Err(ReactiveError::Disposed(HandleKind::Memo)));
    let stored = Err(ReactiveError::Disposed(HandleKind::StoredValue));
    assert_eq!(v.try_get(), stored);
    assert_eq!(
        v.try_with(String::len),
        Err(ReactiveError::Disposed(HandleKind::StoredValue))
    );
    assert_eq!(
        v.try_with_mut(|s| s.len()),
        Err(ReactiveError::Disposed(HandleKind::StoredValue))
    );
    assert_eq!(
        v.try_set("b".into()),
        Err(ReactiveError::Disposed(HandleKind::StoredValue))
    );
    assert_eq!(faults(), 0, "try forms raise no fault");
}

#[test]
fn try_creation_in_disposed_scope_is_an_error_and_drops_the_inputs() {
    struct Count(Rc<Cell<u32>>);
    impl Drop for Count {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let _ = faults();
    let cx = rt().create_root();
    cx.dispose();
    let dropped = Rc::new(Cell::new(0));
    let scope = ReactiveError::Disposed(HandleKind::Scope);
    assert_eq!(cx.try_signal(Count(dropped.clone())).err(), Some(scope));
    assert_eq!(cx.try_stored_value(Count(dropped.clone())).err(), Some(scope));
    let c = Count(dropped.clone());
    assert_eq!(cx.try_memo(move || c.0.get()).err(), Some(scope));
    let ran = Rc::new(Cell::new(false));
    let (c, r) = (Count(dropped.clone()), ran.clone());
    assert_eq!(
        cx.try_effect(move || {
            let _ = &c;
            r.set(true);
        })
        .err(),
        Some(scope)
    );
    assert!(!ran.get(), "the effect never ran");
    assert_eq!(cx.try_child(), Err(scope));
    assert_eq!(
        dropped.get(),
        4,
        "values and closures of refused nodes are dropped"
    );
    assert_eq!(rt().stats().nodes, 0);
    assert_eq!(faults(), 0);
}

#[test]
fn try_forms_succeed_on_live_handles() {
    let cx = rt().try_create_root().unwrap();
    let s = cx.try_signal(1).unwrap();
    let m = cx.try_memo(move || s.get() + 1).unwrap();
    let seen = Rc::new(Cell::new(0));
    let sn = seen.clone();
    let e = cx.try_effect(move || sn.set(m.get())).unwrap();
    assert_eq!(seen.get(), 2);
    assert_eq!(s.try_set(5), Ok(()));
    assert_eq!(seen.get(), 6);
    assert_eq!(s.try_update(|n| *n += 1), Ok(()));
    assert_eq!(m.try_get(), Ok(7));
    let child = cx.try_child().unwrap();
    let v = child.try_stored_value(vec![1]).unwrap();
    assert_eq!(v.try_with_mut(|v| v.push(2)), Ok(()));
    assert_eq!(v.try_get(), Ok(vec![1, 2]));
    assert!(e.is_alive());
    cx.dispose();
}

#[test]
fn try_set_inside_a_read_of_the_same_value_is_borrowed() {
    let cx = rt().create_root();
    let s = cx.signal(1);
    let borrowed = Err(ReactiveError::Borrowed(HandleKind::Signal));
    assert_eq!(s.with(|_| s.try_set(2)), borrowed);
    assert_eq!(s.with(|_| s.try_update(|n| *n = 3)), borrowed);
    // A read inside `update` of the same signal.
    let mut inner = None;
    s.update(|_| inner = Some(s.try_get()));
    assert_eq!(inner, Some(Err(ReactiveError::Borrowed(HandleKind::Signal))));
    assert_eq!(s.get(), 1);

    let v = cx.stored_value(0);
    assert_eq!(
        v.with(|_| v.try_set(1)),
        Err(ReactiveError::Borrowed(HandleKind::StoredValue))
    );
    assert_eq!(
        v.with_mut(|_| v.try_with(|n| *n)),
        Err(ReactiveError::Borrowed(HandleKind::StoredValue))
    );
    assert_eq!(
        v.with(|_| v.try_with(|n| *n)),
        Ok(0),
        "nested shared reads are fine"
    );
    cx.dispose();
}

#[test]
fn try_get_of_a_cyclic_memo_is_cycle() {
    let _ = faults();
    let cx = rt().create_root();
    let slot: Rc<Cell<Option<Memo<u32>>>> = Rc::new(Cell::new(None));
    let inner = Rc::new(Cell::new(None));
    let (sl, inn) = (slot.clone(), inner.clone());
    let m = cx.memo(move || {
        if let Some(m) = sl.get() {
            inn.set(Some(m.try_get()));
        }
        1u32
    });
    slot.set(Some(m));
    assert_eq!(m.try_get(), Ok(1));
    assert_eq!(inner.get(), Some(Err(ReactiveError::Cycle)));
    assert_eq!(faults(), 0);
    cx.dispose();
}

#[test]
fn reactive_error_display_and_fault_kinds() {
    use twine_reactive::FaultKind;
    let cases = [
        (
            ReactiveError::Disposed(HandleKind::Signal),
            "signal used after its scope was disposed",
            FaultKind::DisposedHandle,
        ),
        (
            ReactiveError::Disposed(HandleKind::Scope),
            "scope used after it was disposed",
            FaultKind::DisposedHandle,
        ),
        (
            ReactiveError::Borrowed(HandleKind::StoredValue),
            "stored value already borrowed (accessed inside an access of the same value)",
            FaultKind::ReentrantAccess,
        ),
        (
            ReactiveError::Capacity(HandleKind::Memo),
            "too many reactive nodes (Limits::max_reactive_nodes)",
            FaultKind::Capacity,
        ),
        (
            ReactiveError::Capacity(HandleKind::Scope),
            "too many reactive scopes (Limits::max_scopes)",
            FaultKind::Capacity,
        ),
        (
            ReactiveError::Cycle,
            "cycle detected (a memo reads itself, directly or indirectly)",
            FaultKind::MemoCycle,
        ),
    ];
    for (e, msg, kind) in cases {
        assert_eq!(e.to_string(), msg);
        assert_eq!(e.fault_kind(), kind);
        let _: &dyn core::error::Error = &e;
    }
}
