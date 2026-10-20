//! R5.S03 completion: disposing scopes allocates nothing — no garbage list, no recursion
//! state, no channel scratch — so a sealed UI can replace views (dispose and rebuild within
//! preallocated limits) without the disposal half allocating. This binary installs a counting
//! global allocator (per thread, so parallel tests do not interfere).
//!
//! req: SAF-010

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_core::Limits;
use twine_reactive::{Channel, Runtime};

struct Counting;

std::thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator; only counts (a const thread-local `Cell`, which
// does not allocate).
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Allocations made by `f` on this thread.
fn allocs_in(f: impl FnOnce()) -> usize {
    let before = ALLOCS.with(Cell::get);
    f();
    ALLOCS.with(Cell::get) - before
}

static EVENTS: Channel<u32, 4> = Channel::new();

#[test]
fn disposing_a_populated_tree_allocates_nothing() {
    let rt = Runtime::current_thread();
    rt.set_limits(
        &Limits::DEFAULT
            .max_reactive_nodes(256)
            .max_scopes(64)
            .max_pending_effects(64)
            .preallocated(),
    )
    .unwrap();
    let root = rt.create_root();
    let src = root.signal(0u32);
    let seen = Rc::new(RefCell::new(Vec::with_capacity(64)));
    for i in 0..8u32 {
        let c = root.child();
        let s = c.signal(i);
        let m = c.memo(move || src.get() + s.get());
        let seen = seen.clone();
        c.effect(move || {
            let v = m.get();
            if seen.borrow().len() < seen.borrow().capacity() {
                seen.borrow_mut().push(v);
            }
        });
        c.on_message(&EVENTS, |_| {});
        c.provide(i);
        c.on_cleanup(|| {});
        let g = c.child();
        g.signal(i);
    }
    // An effect queued but not yet run when the tree goes.
    src.set(1);
    let n = allocs_in(|| root.dispose());
    assert_eq!(n, 0, "disposal allocated {n} time(s)");
    let st = rt.stats();
    assert_eq!((st.nodes, st.scopes, st.pending), (0, 0, 0));
}
