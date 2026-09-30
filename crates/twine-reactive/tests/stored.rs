//! Stored values ([`StoredValue`]).

use std::cell::Cell;
use std::rc::Rc;

use twine_reactive::{StoredValue, create_root, runtime_stats};

/// Compile-time check: the handle is `Copy` for any `T` (also non-`Clone` ones).
const _: fn() = || {
    fn is_copy<T: Copy>() {}
    struct NotClone;
    is_copy::<StoredValue<NotClone>>();
    is_copy::<StoredValue<Vec<String>>>();
};

#[test]
fn stored_value_roundtrip() {
    let cx = create_root();
    let v = cx.stored_value(vec![1, 2]);
    assert_eq!(v.with(Vec::len), 2);
    v.with_mut(|v| v.push(3));
    assert_eq!(v.get(), [1, 2, 3]);
    v.set(vec![9]);
    assert_eq!(v.try_get(), Some(vec![9]));
    assert_eq!(v.try_with(Vec::len), Some(1));
    assert_eq!(v.try_with_mut(Vec::pop), Some(Some(9)));
    assert!(v.is_alive());
    cx.dispose();
}

#[test]
fn stored_value_is_copy_into_many_closures() {
    let cx = create_root();
    let n = cx.stored_value(0u32);
    let inc = move || n.with_mut(|n| *n += 1);
    let read = move || n.get();
    for _ in 0..3 {
        inc();
    }
    assert_eq!(read(), 3);
    cx.dispose();
}

#[test]
fn stored_value_reads_do_not_track_and_writes_do_not_notify() {
    let cx = create_root();
    let v = cx.stored_value(1u32);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    cx.effect(move || {
        let _ = v.get();
        let _ = v.with(|x| *x);
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1);
    v.set(2);
    v.with_mut(|x| *x = 3);
    assert_eq!(runs.get(), 1, "stored values are not reactive");
    assert_eq!(v.get(), 3);
    cx.dispose();
}

#[test]
fn stored_value_disposed_with_its_scope() {
    struct Flag(Rc<Cell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let root = create_root();
    let child = root.child();
    let dropped = Rc::new(Cell::new(false));
    let v = child.stored_value(Flag(dropped.clone()));
    let nodes = runtime_stats().nodes;
    root.dispose();
    assert!(dropped.get(), "value dropped with the scope");
    assert!(!v.is_alive());
    assert!(v.try_with(|_| ()).is_none());
    assert!(v.try_with_mut(|_| ()).is_none());
    assert!(runtime_stats().nodes < nodes);
}

#[test]
#[should_panic(expected = "stored value used after its scope was disposed")]
fn stored_value_use_after_dispose_panics() {
    let cx = create_root();
    let v = cx.stored_value(1);
    cx.dispose();
    let _ = v.get();
}

#[test]
#[should_panic(expected = "disposed")]
fn stored_value_with_mut_after_dispose_panics() {
    let cx = create_root();
    let v = cx.stored_value(1);
    cx.dispose();
    v.with_mut(|n| *n += 1);
}

#[cfg(debug_assertions)]
#[test]
fn stored_value_use_after_dispose_names_creation_site() {
    let cx = create_root();
    let v = cx.stored_value(1);
    cx.dispose();
    let err = std::panic::catch_unwind(move || v.get()).unwrap_err();
    let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains(file!()), "panic names the creation site: {msg}");
}

#[test]
fn stored_value_old_value_drop_can_use_it_again() {
    struct Touch(Rc<Cell<Option<StoredValue<Touch>>>>);
    impl Drop for Touch {
        fn drop(&mut self) {
            if let Some(v) = self.0.get() {
                // The old value is dropped after the value was released: no borrow error.
                assert!(v.try_with(|_| ()).is_some());
            }
        }
    }
    let cx = create_root();
    let slot = Rc::new(Cell::new(None));
    let v = cx.stored_value(Touch(slot.clone()));
    slot.set(Some(v));
    v.set(Touch(Rc::new(Cell::new(None))));
    cx.dispose();
}

#[test]
fn stored_value_equality_and_debug() {
    let cx = create_root();
    let a = cx.stored_value(1);
    let b = cx.stored_value(1);
    let a2 = a;
    assert_eq!(a, a2);
    assert_ne!(a, b);
    assert!(format!("{a:?}").starts_with("StoredValue("));
    cx.dispose();
}
