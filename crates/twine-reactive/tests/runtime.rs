//! Runtime storage and global access.

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[test]
fn root_scope_creation_is_lazy_and_repeatable() {
    // Nothing exists before the first root is created on this thread.
    assert_eq!(rt().stats().scopes, 0);
    let a = rt().create_root();
    let b = rt().create_root();
    assert_ne!(a, b);
    assert!(a.is_alive() && b.is_alive());
    assert_eq!(rt().stats().scopes, 2);
    a.dispose();
    assert!(!a.is_alive());
    assert!(b.is_alive());
    assert_eq!(rt().stats().scopes, 1);
}

#[test]
fn reset_clears_everything() {
    let cx = rt().create_root();
    let s = cx.signal(1u32);
    cx.effect(move || {
        s.get();
    });
    cx.child().signal("x");
    let st = rt().stats();
    assert_eq!(st.scopes, 2);
    assert_eq!(st.nodes, 3);
    rt().reset();
    let st = rt().stats();
    assert_eq!((st.nodes, st.scopes, st.pending, st.deferred), (0, 0, 0, 0));
    assert!(!cx.is_alive());
    assert!(!s.is_alive());
    // The runtime is usable again and old handles never alias new nodes.
    let cx2 = rt().create_root();
    let s2 = cx2.signal(2u32);
    assert_ne!(cx, cx2);
    assert_eq!(s.try_get(), None);
    assert_eq!(s2.get(), 2);
}

#[test]
fn handles_are_copy() {
    fn assert_copy<T: Copy>(_: T) {}
    let cx = rt().create_root();
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
    let cx = rt().create_root();
    let s = cx.signal(1);
    let other = std::thread::spawn(|| {
        let st = rt().stats();
        let cx = rt().create_root();
        cx.signal(10);
        cx.signal(20);
        (st.nodes, st.scopes, rt().stats().nodes)
    })
    .join()
    .unwrap();
    assert_eq!(other, (0, 0, 2));
    assert_eq!(rt().stats().nodes, 1);
    assert_eq!(s.get(), 1);
}

#[test]
fn take_succeeds_once_per_thread() {
    let first = Runtime::take().expect("the first take succeeds");
    assert!(Runtime::take().is_none(), "a second take fails");
    assert!(Runtime::take().is_none(), "and keeps failing");
    // Copies and the thread's token are the same runtime.
    assert_eq!(Runtime::current_thread(), first);
    let cx = first.create_root();
    assert_eq!(cx.runtime(), first);
    assert_eq!(Runtime::from(cx), first);
    cx.dispose();
    // With `std` every thread has its own runtime, so another thread can take its own.
    std::thread::spawn(|| {
        assert!(Runtime::take().is_some());
        assert!(Runtime::take().is_none());
    })
    .join()
    .unwrap();
}

#[test]
fn current_thread_does_not_claim_the_runtime() {
    let cx = Runtime::current_thread().create_root();
    assert!(
        Runtime::take().is_some(),
        "`current_thread` left the claim to `take`"
    );
    cx.dispose();
}

#[cfg(feature = "debug-checks")]
std::thread_local! {
    /// Simulated interrupt context of this test thread.
    static IN_ISR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(feature = "debug-checks")]
fn isr_probe() -> bool {
    IN_ISR.with(std::cell::Cell::get)
}

/// Runs `f` as if in an interrupt handler (for `isr_probe`).
#[cfg(feature = "debug-checks")]
fn in_isr<R>(f: impl FnOnce() -> R) -> R {
    IN_ISR.with(|c| c.set(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    IN_ISR.with(|c| c.set(false));
    r.unwrap_or_else(|p| std::panic::resume_unwind(p))
}

/// With `debug-checks`, any runtime access while the probe reports interrupt context panics,
/// before the runtime is touched; `Channel`, `UiWaker` and the probe functions keep working
/// there. (Built with the feature only: `cargo test -p twine-reactive --features
/// debug-checks`, and in workspace runs, where `twine-view`'s tests enable it; the benches of
/// this crate must measure the build without it.)
#[cfg(feature = "debug-checks")]
#[test]
fn runtime_used_from_interrupt_context_panics_with_debug_checks() {
    static CH: twine_reactive::Channel<u8, 2> = twine_reactive::Channel::new();
    twine_reactive::set_interrupt_probe(Some(isr_probe));
    let cx = rt().create_root();
    let s = cx.signal(1u8);
    cx.on_message(&CH, move |v| s.set(v));
    assert_eq!(s.get(), 1, "thread context: fine");

    let read = std::panic::catch_unwind(|| in_isr(|| s.get()));
    assert!(read.is_err(), "a signal read in interrupt context panics");
    let batch = std::panic::catch_unwind(|| in_isr(|| rt().batch(|| ())));
    assert!(batch.is_err(), "so does any runtime-wide operation");
    // What an interrupt handler may do: send a message, ask the probe.
    in_isr(|| {
        assert!(twine_reactive::in_interrupt());
        CH.try_send(7).unwrap();
    });

    // Back in thread context the runtime is intact.
    assert_eq!(rt().drain_channels(4), 1);
    assert_eq!(s.get(), 7);
    twine_reactive::set_interrupt_probe(None);
    cx.dispose();
}
