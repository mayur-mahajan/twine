//! Signals.

use std::rc::Rc;

use twine_reactive::{create_root, debug_stats, reset};

#[test]
fn signal_get_set_roundtrip() {
    let cx = create_root();
    let s = cx.signal(1u32);
    assert_eq!(s.get(), 1);
    s.set(42);
    assert_eq!(s.get(), 42);
    assert_eq!(s.get_untracked(), 42);
    assert_eq!(s.try_get(), Some(42));
}

#[test]
fn signal_update_mutates_in_place() {
    let cx = create_root();
    let v = cx.signal(vec![1, 2]);
    let ptr_before = v.with(Vec::as_ptr);
    v.update(|v| v[0] = 9);
    assert_eq!(v.get(), [9, 2]);
    assert_eq!(v.with(Vec::as_ptr), ptr_before, "update must not reallocate");
}

#[test]
fn signal_with_borrows_without_clone() {
    struct NoClone(u32);
    let cx = create_root();
    let s = cx.signal(NoClone(7));
    assert_eq!(s.with(|v| v.0), 7);
    assert_eq!(s.with_untracked(|v| v.0 + 1), 8);
    // `Rc` strong count proves no clone of the value is made by `with`.
    let rc = Rc::new(5);
    let r = cx.signal(rc.clone());
    r.with(|inner| assert_eq!(Rc::strong_count(inner), 2));
}

#[test]
fn signal_split_handles_share_value() {
    let cx = create_root();
    let s = cx.signal(String::from("a"));
    let (r, w) = s.split();
    w.set("b".into());
    assert_eq!(r.get(), "b");
    w.update(|v| v.push('c'));
    assert_eq!(s.get(), "bc");
    assert_eq!(r.with(String::len), 2);
    assert!(r.is_alive() && w.is_alive());
    assert_eq!(r.try_get().as_deref(), Some("bc"));
    assert_eq!(s.read_only(), r);
    assert_eq!(s.write_only(), w);
}

#[test]
fn signal_set_if_changed_equal_value_is_noop() {
    let cx = create_root();
    let s = cx.signal(3);
    let w0 = debug_stats().writes;
    s.set_if_changed(3);
    assert_eq!(debug_stats().writes, w0);
    s.write_only().set_if_changed(3);
    assert_eq!(debug_stats().writes, w0);
    s.set_if_changed(4);
    assert_eq!(debug_stats().writes, w0 + 1);
    assert_eq!(s.get(), 4);
    s.set(4); // `set` always notifies
    assert_eq!(debug_stats().writes, w0 + 2);
}

#[test]
fn signal_try_get_after_reset_is_none() {
    let cx = create_root();
    let s = cx.signal(1);
    let r = s.read_only();
    reset();
    assert_eq!(s.try_get(), None);
    assert_eq!(r.try_get(), None);
    assert!(!s.is_alive());
    assert!(!s.write_only().is_alive());
}

#[test]
#[should_panic(expected = "disposed")]
fn signal_use_after_dispose_panics_with_location() {
    let cx = create_root();
    let s = cx.signal(1);
    cx.dispose();
    let _ = s.get();
}

#[cfg(debug_assertions)]
#[test]
fn signal_use_after_dispose_message_names_creation_site() {
    let cx = create_root();
    let s = cx.signal(1);
    let line = line!() - 1;
    cx.dispose();
    let err = std::panic::catch_unwind(move || s.set(2)).unwrap_err();
    let msg = err.downcast_ref::<String>().unwrap();
    assert!(msg.contains("signal used after its scope was disposed"), "{msg}");
    assert!(msg.contains(&format!("signal.rs:{line}")), "{msg}");
}

#[test]
fn signals_of_different_types_coexist() {
    #[derive(Clone, PartialEq, Debug)]
    struct P {
        x: i32,
    }
    let cx = create_root();
    let a = cx.signal(1u8);
    let b = cx.signal("str");
    let c = cx.signal(P { x: 3 });
    let d = cx.signal(vec![1.5f32]);
    let e = cx.signal(());
    a.set(2);
    c.update(|p| p.x += 1);
    assert_eq!(a.get(), 2);
    assert_eq!(b.get(), "str");
    assert_eq!(c.get(), P { x: 4 });
    assert_eq!(d.with(Vec::len), 1);
    e.set(());
    assert_eq!(debug_stats().nodes, 5);
}

#[test]
#[should_panic(expected = "already")]
fn signal_set_inside_with_of_same_signal_panics() {
    let cx = create_root();
    let s = cx.signal(1);
    s.with(|_| s.set(2));
}

#[test]
fn signal_drop_of_old_value_can_touch_runtime() {
    // R1: the old value is dropped without any runtime borrow held.
    struct Touch(Option<twine_reactive::Signal<u32>>);
    impl Drop for Touch {
        fn drop(&mut self) {
            if let Some(s) = self.0 {
                s.set(s.get_untracked() + 1);
            }
        }
    }
    let cx = create_root();
    let counter = cx.signal(0u32);
    let s = cx.signal(Touch(Some(counter)));
    s.set(Touch(None));
    assert_eq!(counter.get(), 1);
}
