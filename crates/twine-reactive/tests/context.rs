//! Context values (R4.S04): provide and look up across nested scopes, shadowing, the
//! `MissingContext` error and the panicking form, disposal with the providing scope, and
//! allocation-free lookups.
//!
//! A separate test binary: it counts the allocations of each test thread with its own
//! allocator.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::rc::Rc;

use twine_reactive::{MissingContext, Runtime, Signal};

thread_local! {
    // `const` init, no destructor: safe to touch from inside the allocator.
    static ALLOCS: Cell<u32> = const { Cell::new(0) };
}

fn allocs() -> u32 {
    ALLOCS.try_with(Cell::get).unwrap_or(0)
}

struct Counting;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged, so the
// `GlobalAlloc` contract is upheld by `System`. The bookkeeping only updates a const-initialised
// thread-local `Cell` without a destructor, which neither allocates nor unwinds.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: forwarded unchanged (caller guarantees a non-zero-size layout).
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged (`ptr` came from `System.alloc` with `layout`).
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

/// A store-like context: a `Copy` struct of handles.
#[derive(Clone, Copy)]
struct Store {
    count: Signal<i32>,
    step: Signal<i32>,
}

#[test]
fn context_lookup_walks_parents() {
    let root = rt().create_root();
    root.provide(7u32);
    root.provide("theme");
    let deep = root.child().child().child();
    assert_eq!(deep.use_context::<u32>(), Ok(7));
    assert_eq!(deep.use_context::<&str>(), Ok("theme"));
    assert_eq!(deep.use_context::<u64>(), Err(MissingContext::of::<u64>()));
    assert_eq!(deep.use_context::<u32>().unwrap(), 7);
    // A value provided below is not visible above.
    deep.provide(1u64);
    assert!(root.use_context::<u64>().is_err());
    root.dispose();
}

#[test]
fn context_shadowing() {
    let root = rt().create_root();
    root.provide(1u8);
    let child = root.child();
    child.provide(2u8);
    let grandchild = child.child();
    let sibling = root.child();
    assert_eq!(grandchild.use_context::<u8>(), Ok(2));
    assert_eq!(sibling.use_context::<u8>(), Ok(1), "shadowing is per subtree");
    assert_eq!(root.use_context::<u8>(), Ok(1));
    // Providing again in the same scope replaces.
    root.provide(3u8);
    assert_eq!(root.use_context::<u8>(), Ok(3));
    assert_eq!(sibling.use_context::<u8>(), Ok(3));
    assert_eq!(grandchild.use_context::<u8>(), Ok(2));
    child.dispose();
    assert_eq!(root.use_context::<u8>(), Ok(3));
    root.dispose();
}

#[test]
fn context_store_handles_share_state() {
    let root = rt().create_root();
    root.provide(Store {
        count: root.signal(0),
        step: root.signal(2),
    });
    let a = root.child();
    let b = root.child().child();
    let sa = a.use_context::<Store>().unwrap();
    sa.count.update(|c| *c += sa.step.get());
    let sb = b.use_context::<Store>().unwrap();
    assert_eq!(sb.count.get(), 2, "every lookup sees the same signals");
    root.dispose();
}

#[test]
fn context_missing_error_names_the_type() {
    let root = rt().create_root();
    let Err(err) = root.child().use_context::<Store>() else {
        panic!("not provided")
    };
    assert!(err.type_name().ends_with("Store"), "{}", err.type_name());
    assert_eq!(
        err.to_string(),
        format!("context {} not provided", err.type_name())
    );
    assert_eq!(err.to_string(), "context context::Store not provided");
    // Usable as a `dyn Error`.
    let e: &dyn core::error::Error = &err;
    assert!(e.source().is_none());
    root.dispose();
}

// `expect_context` does not exist in the `safety` profile.
#[cfg(not(feature = "safety"))]
#[test]
#[should_panic(expected = "context u16 not provided")]
fn context_expect_panics_with_type_name() {
    let root = rt().create_root();
    let _: u16 = root.expect_context();
}

#[cfg(not(feature = "safety"))]
#[test]
fn context_expect_reports_the_caller_location() {
    std::thread_local! {
        static LINE: Cell<u32> = const { Cell::new(0) };
    }
    let root = rt().create_root();
    // The hook is process-wide; it records only panics raised from this file, per thread.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        if let Some(l) = info.location().filter(|l| l.file().ends_with("context.rs")) {
            LINE.with(|c| c.set(l.line()));
        }
    }));
    let line = line!() + 1;
    let r = std::panic::catch_unwind(|| root.expect_context::<u16>());
    std::panic::set_hook(prev);
    assert!(r.is_err());
    assert_eq!(
        LINE.with(Cell::get),
        line,
        "`#[track_caller]`: the panic points at the caller"
    );
    root.dispose();
}

#[test]
fn context_on_disposed_scope() {
    let root = rt().create_root();
    let child = root.child();
    child.provide(5u8);
    child.dispose();
    // A disposed scope provides nothing and ignores `provide`; neither panics.
    assert!(child.use_context::<u8>().is_err());
    child.provide(6u8);
    assert!(child.use_context::<u8>().is_err());
    root.dispose();
}

#[test]
fn context_is_dropped_with_its_scope() {
    let root = rt().create_root();
    let child = root.child();
    let token = Rc::new(());
    child.provide(Rc::clone(&token));
    let looked_up = child.child().use_context::<Rc<()>>().unwrap();
    assert_eq!(Rc::strong_count(&token), 3);
    drop(looked_up);
    // Replacing drops the old value.
    child.provide(Rc::new(()));
    assert_eq!(Rc::strong_count(&token), 1);
    child.provide(Rc::clone(&token));
    assert_eq!(Rc::strong_count(&token), 2);
    child.dispose();
    assert_eq!(Rc::strong_count(&token), 1, "the scope's value was dropped");
    // The parent's view is unaffected.
    assert!(root.use_context::<Rc<()>>().is_err());
    root.dispose();
}

#[test]
fn context_lookup_allocates_nothing() {
    let root = rt().create_root();
    root.provide(Store {
        count: root.signal(0),
        step: root.signal(1),
    });
    root.provide(3u8);
    let mut deep = root;
    for _ in 0..8 {
        deep = deep.child();
        deep.provide(deep.signal(0u16)); // unrelated values along the chain
    }
    let before = allocs();
    for _ in 0..100 {
        let s = deep.use_context::<Store>().unwrap();
        assert_eq!(s.step.get_untracked(), 1);
        assert_eq!(deep.use_context::<u8>(), Ok(3));
        assert!(deep.use_context::<u64>().is_err()); // a miss walks the whole chain
        assert!(deep.use_context::<Signal<u16>>().is_ok()); // nearest: the scope itself
    }
    assert_eq!(allocs() - before, 0, "context lookups allocate nothing");
    root.dispose();
}
