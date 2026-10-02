//! The bounded waker pool (R3.S02, rework F10a): `WakerLease::STATIC_SLOTS` leases without
//! allocation, at most `heap_limit` heap wakers beyond (reused, never freed), and a shared
//! fallback waker — no allocation — once the pool is exhausted.
//!
//! A separate test binary: the pool and its limit are process-wide, and this binary counts the
//! allocations of its one test thread with its own allocator.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use twine_reactive::{UiWaker, WakerLease};

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

/// The allocations made by `f` on this thread.
fn count<R>(f: impl FnOnce() -> R) -> (R, u32) {
    let before = allocs();
    let r = f();
    (r, allocs() - before)
}

#[test]
fn pool_is_static_then_bounded_heap_then_shared() {
    assert_eq!(WakerLease::heap_limit(), WakerLease::DEFAULT_HEAP_LIMIT);
    // Up to four UIs: static slots, no allocation.
    let (statics, n) = count(|| {
        (0..WakerLease::STATIC_SLOTS)
            .map(|_| UiWaker::lease())
            .collect::<heapless::Vec<_, 4>>()
    });
    assert_eq!(n, 0, "no allocation for the first STATIC_SLOTS leases");
    assert!(statics.iter().all(|l| !l.is_shared()));
    assert_eq!(WakerLease::heap_allocated(), 0);

    // One heap waker allowed.
    WakerLease::set_heap_limit(1);
    let (heap, n) = count(UiWaker::lease);
    assert_eq!(n, 1, "the fifth lease allocates its waker once");
    assert!(!heap.is_shared());
    assert_eq!(WakerLease::heap_allocated(), 1);

    // Exhausted: the shared fallback, without allocating.
    let ((a, b), n) = count(|| (UiWaker::lease(), UiWaker::lease()));
    assert_eq!(n, 0, "an exhausted pool never allocates");
    assert!(a.is_shared() && b.is_shared());
    assert!(core::ptr::eq(a.get(), b.get()), "one shared waker");
    assert!(statics.iter().all(|l| !core::ptr::eq(l.get(), a.get())));
    assert!(!core::ptr::eq(heap.get(), a.get()));
    // Dropping a shared lease does not reset the waker the other owner still uses.
    a.get().wake();
    drop(a);
    assert!(b.get().is_set());
    drop(b);
    assert_eq!(WakerLease::heap_allocated(), 1);

    // A returned heap waker is reused without allocating, even with the heap now forbidden.
    let heap_waker: *const UiWaker = heap.get();
    drop(heap);
    WakerLease::set_heap_limit(0);
    let (again, n) = count(UiWaker::lease);
    assert_eq!(n, 0);
    assert!(!again.is_shared());
    assert!(core::ptr::eq(again.get(), heap_waker));
    assert_eq!(WakerLease::heap_allocated(), 1, "never freed, never re-allocated");

    // Returned static slots serve new leases first.
    drop(again);
    drop(statics);
    let (fresh, n) = count(UiWaker::lease);
    assert_eq!(n, 0);
    assert!(!fresh.is_shared());
    WakerLease::set_heap_limit(WakerLease::DEFAULT_HEAP_LIMIT);
}
