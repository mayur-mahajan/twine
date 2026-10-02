//! Memory per signal (target: ≤ 96 bytes per `u32` signal incl. its `Rc`).
//!
//! `twine_testing::alloc::CountingAllocator` counts requested bytes (every `realloc` step
//! included), which over-states the live footprint of growing vectors; this binary installs a
//! minimal allocator that tracks **live** bytes per thread instead.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

thread_local! {
    // `const` init, no destructor: safe to touch from inside the allocator.
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn add(n: isize) {
    let _ = LIVE.try_with(|l| l.set(l.get() + n));
}

fn live() -> isize {
    LIVE.try_with(Cell::get).unwrap_or(0)
}

struct LiveBytes;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged, so the
// `GlobalAlloc` contract is upheld by `System`. The bookkeeping only updates a const-initialised
// thread-local `Cell` without a destructor, which neither allocates nor unwinds.
unsafe impl GlobalAlloc for LiveBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        add(layout.size() as isize);
        // SAFETY: forwarded unchanged (caller guarantees a non-zero-size layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add(-(layout.size() as isize));
        // SAFETY: forwarded unchanged; `ptr` was allocated by `System` with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add(new_size as isize - layout.size() as isize);
        // SAFETY: forwarded unchanged; the caller upholds `realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: LiveBytes = LiveBytes;

#[test]
fn signal_memory_bytes() {
    // A fresh thread has an empty runtime. 1024 signals make every vector's capacity exact
    // (arena slots and the scope's node list grow by doubling).
    let per_signal = std::thread::spawn(|| {
        const N: isize = 1024;
        let cx = rt().create_root();
        let before = live();
        let signals: Vec<_> = (0..N).map(|i| cx.signal(i as u32)).collect();
        let handles = std::mem::size_of_val(signals.as_slice()) as isize;
        let used = live() - before - handles; // the test's own handle vector does not count
        drop(signals);
        cx.dispose();
        used / N
    })
    .join()
    .unwrap();
    println!("memory per u32 signal: {per_signal} bytes");
    assert!(per_signal <= 96, "{per_signal} bytes per signal (budget 96)");
}

// Not built under Miri: reaching the arenas' steady state takes 20 000 create/dispose cycles
// (milliseconds natively, hours under Miri), and Miri already reports any leaked allocation at the
// end of every test it runs, so the leak check itself is covered there.
#[cfg(not(miri))]
#[test]
fn stored_value_churn_does_not_leak() {
    // Create and dispose scopes holding heap-owning stored values; after a warm-up the live
    // heap must stay flat. The warm-up covers more than `Arena::ROTATE_PERIOD` (4 096) reuses
    // of every slot, so the arenas' wear-leveling reserves (R0.S07) are at their peak size too.
    let growth = std::thread::spawn(|| {
        let root = rt().create_root();
        let cycle = || {
            let cx = root.child();
            for i in 0..16u32 {
                let v = cx.stored_value(vec![i; 8]);
                v.with_mut(|v| v.push(i));
                v.set(vec![i; 4]);
            }
            cx.dispose();
        };
        for _ in 0..10_000 {
            cycle();
        }
        let before = live();
        for _ in 0..10_000 {
            cycle();
        }
        let growth = live() - before;
        root.dispose();
        growth
    })
    .join()
    .unwrap();
    assert_eq!(
        growth, 0,
        "stored value create/dispose churn leaked {growth} bytes"
    );
}

#[test]
fn memory_report_matches_the_live_heap() {
    // `Runtime::memory` counts capacities and box / `Rc` sizes; on a fresh thread (an empty
    // runtime) its growth must equal the heap the scene actually took, byte for byte.
    let (reported, taken, m) = std::thread::spawn(|| {
        let rt = rt();
        let (before, before_live) = (rt.memory(), live());
        let cx = rt.create_root();
        let a = cx.signal(1u32);
        let big = cx.signal([7u8; 100]);
        let doubled = cx.memo(move || a.get() * 2);
        let seen = cx.stored_value(0u32);
        cx.effect(move || seen.set(doubled.get() + u32::from(big.get()[0])));
        let child = cx.child();
        let captured = [1u64; 4];
        child.on_cleanup(move || assert_eq!(captured[0], 1));
        child.provide(5u16);
        for i in 0..40u32 {
            let s = child.signal(i);
            // Many subscribers: the signal's subscriber list spills to the heap.
            for _ in 0..5 {
                let _ = child.memo(move || s.get());
            }
        }
        a.set(2);
        let m = rt.memory();
        let reported = m.bytes() as isize - before.bytes() as isize;
        let taken = live() - before_live;
        cx.dispose();
        (reported, taken, m)
    })
    .join()
    .unwrap();
    assert!(m.nodes >= 245 && m.scopes >= 2, "{m:?}");
    assert!(
        m.values > 0 && m.scope_data > 0 && m.arenas > 0 && m.queues > 0,
        "{m:?}"
    );
    assert_eq!(
        reported, taken,
        "reported {reported} B, the heap grew by {taken} B ({m:?})"
    );
}
