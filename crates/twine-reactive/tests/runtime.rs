//! P07.S01: runtime storage and global access.

use twine_reactive::{create_root, debug_stats, reset};

#[test]
fn root_scope_creation_is_lazy_and_repeatable() {
    // Nothing exists before the first root is created on this thread.
    assert_eq!(debug_stats().scopes, 0);
    let a = create_root();
    let b = create_root();
    assert_ne!(a, b);
    assert!(a.is_alive() && b.is_alive());
    assert_eq!(debug_stats().scopes, 2);
    a.dispose();
    assert!(!a.is_alive());
    assert!(b.is_alive());
    assert_eq!(debug_stats().scopes, 1);
}

#[test]
fn reset_clears_everything() {
    let cx = create_root();
    let s = cx.signal(1u32);
    cx.effect(move || {
        s.get();
    });
    cx.child().signal("x");
    let st = debug_stats();
    assert_eq!(st.scopes, 2);
    assert_eq!(st.nodes, 3);
    reset();
    let st = debug_stats();
    assert_eq!((st.nodes, st.scopes, st.pending, st.deferred), (0, 0, 0, 0));
    assert!(!cx.is_alive());
    assert!(!s.is_alive());
    // The runtime is usable again and old handles never alias new nodes.
    let cx2 = create_root();
    let s2 = cx2.signal(2u32);
    assert_ne!(cx, cx2);
    assert_eq!(s.try_get(), None);
    assert_eq!(s2.get(), 2);
}

#[test]
fn handles_are_copy() {
    fn assert_copy<T: Copy>(_: T) {}
    let cx = create_root();
    let s = cx.signal(1);
    let m = cx.memo(move || s.get());
    let e = cx.effect(|| {});
    assert_copy(cx);
    assert_copy(s);
    assert_copy(s.read_only());
    assert_copy(s.write_only());
    assert_copy(m);
    assert_copy(e);
    let s2 = s;
    s2.set(5);
    assert_eq!(s.get(), 5);
}

#[test]
fn thread_local_runtimes_are_independent() {
    let cx = create_root();
    let s = cx.signal(1);
    let other = std::thread::spawn(|| {
        let st = debug_stats();
        let cx = create_root();
        cx.signal(10);
        cx.signal(20);
        (st.nodes, st.scopes, debug_stats().nodes)
    })
    .join()
    .unwrap();
    assert_eq!(other, (0, 0, 2));
    assert_eq!(debug_stats().nodes, 1);
    assert_eq!(s.get(), 1);
}
