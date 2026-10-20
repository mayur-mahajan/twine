//! Allocation counting: [`CountingAllocator`], [`count_allocs`] and
//! [`assert_no_alloc_after_seal`].
//!
//! Install the allocator in a test binary (one integration test file) that needs it:
//!
//! ```standalone_crate
//! use twine_testing::alloc::{count_allocs, CountingAllocator};
//!
//! #[global_allocator]
//! static A: CountingAllocator = CountingAllocator;
//!
//! # fn main() {
//! let (v, stats) = count_allocs(|| vec![1u8, 2, 3]);
//! assert_eq!(v.len(), 3);
//! assert_eq!(stats.allocs, 1);
//! # }
//! ```
//!
//! Counters are **per thread**, so tests running in parallel do not disturb each other. Without
//! the global allocator installed, [`count_allocs`] always reports zero.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Allocation statistics of one thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AllocStats {
    /// `alloc` / `alloc_zeroed` calls.
    pub allocs: u64,
    /// `dealloc` calls.
    pub deallocs: u64,
    /// `realloc` calls.
    pub reallocs: u64,
    /// Bytes requested by `alloc`, `alloc_zeroed` and `realloc` (new size).
    pub bytes: u64,
    /// Live heap bytes: allocated minus freed (a difference of two snapshots is the growth
    /// of the heap in between).
    pub live: i64,
    /// The highest `live` reached (the heap high-water mark). In the statistics of
    /// [`count_allocs`]: the highest growth of the heap during the call, transient
    /// allocations included.
    pub peak: i64,
}

impl AllocStats {
    fn since(self, before: AllocStats) -> AllocStats {
        AllocStats {
            allocs: self.allocs - before.allocs,
            deallocs: self.deallocs - before.deallocs,
            reallocs: self.reallocs - before.reallocs,
            bytes: self.bytes - before.bytes,
            live: self.live - before.live,
            peak: self.peak - before.live,
        }
    }
}

thread_local! {
    // `const` initialisation: no lazy-init allocation and no destructor, so accessing it from
    // inside the allocator cannot recurse.
    static STATS: Cell<AllocStats> = const {
        Cell::new(AllocStats { allocs: 0, deallocs: 0, reallocs: 0, bytes: 0, live: 0, peak: 0 })
    };
}

fn record(f: impl FnOnce(&mut AllocStats)) {
    // `try_with`: during thread teardown the slot may be gone; such allocations are not counted.
    let _ = STATS.try_with(|s| {
        let mut v = s.get();
        f(&mut v);
        v.peak = v.peak.max(v.live);
        s.set(v);
    });
}

/// The statistics of the current thread since it started.
#[must_use]
pub fn current() -> AllocStats {
    STATS.try_with(Cell::get).unwrap_or_default()
}

/// Runs `f` and returns its result with the allocations it made on this thread (`peak`: the
/// highest heap growth during `f`; it restarts the thread's high-water mark, so nested calls
/// report the outer peak from the inner call on).
pub fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, AllocStats) {
    let _ = STATS.try_with(|s| {
        let mut v = s.get();
        v.peak = v.live;
        s.set(v);
    });
    let before = current();
    let r = f();
    (r, current().since(before))
}

/// Runs `f` **sealed** and asserts that it allocated nothing: no allocation, reallocation or
/// growth on this thread seen by the [`CountingAllocator`] (which must be the test binary's
/// global allocator — checked first with a probe allocation), and no growth point noted by
/// Twine's allocation guard ([`twine_core::alloc_guard`], which names the site). Returns `f`'s
/// result.
///
/// The seal is taken for `f` (and the guard's counts cleared); if the thread was sealed before
/// (e.g. by `Ui::seal`), it stays sealed afterwards, otherwise it is unsealed again. Freeing is
/// allowed. Use it around steady-state updates of a sealed UI: the counting allocator proves
/// the claim for the scenario (it sees every allocation, the application's included), and the
/// guard check proves that Twine's own detection agrees.
///
/// # Panics
/// When the counting allocator is not installed, or when `f` allocated (the message has the
/// allocator's statistics and the last growth point the guard noted).
///
/// ```standalone_crate
/// use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal};
///
/// #[global_allocator]
/// static A: CountingAllocator = CountingAllocator;
///
/// # fn main() {
/// let mut v: Vec<u32> = Vec::with_capacity(8);
/// assert_no_alloc_after_seal(|| v.push(1)); // within capacity
/// let r = std::panic::catch_unwind(move || assert_no_alloc_after_seal(|| vec![1u8]));
/// assert!(r.is_err());
/// # }
/// ```
#[track_caller]
pub fn assert_no_alloc_after_seal<R>(f: impl FnOnce() -> R) -> R {
    use twine_core::alloc_guard;
    let ((), probe) = count_allocs(|| drop(std::hint::black_box(Box::new(0u8))));
    assert!(
        probe.allocs > 0,
        "assert_no_alloc_after_seal: install `twine_testing::alloc::CountingAllocator` as the \
         test binary's #[global_allocator]"
    );
    let was_sealed = alloc_guard::is_sealed();
    alloc_guard::seal();
    let (r, stats) = count_allocs(f);
    let noted = alloc_guard::total();
    let last = alloc_guard::take().map(|r| r.last);
    if !was_sealed {
        alloc_guard::unseal();
    }
    assert!(
        stats.allocs == 0 && stats.reallocs == 0 && noted == 0,
        "allocation after seal: {stats:?}; Twine's guard noted {noted} growth(s) (last: {last:?})"
    );
    r
}

/// A global allocator that forwards to [`System`] and counts calls per thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct CountingAllocator;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged, so the
// `GlobalAlloc` contract is upheld by `System`. The bookkeeping only touches a const-initialised
// thread-local `Cell` without a destructor, which never allocates and never unwinds.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(|s| {
            s.allocs += 1;
            s.bytes += layout.size() as u64;
            s.live += layout.size() as i64;
        });
        // SAFETY: forwarded unchanged; the caller guarantees `layout` has non-zero size.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(|s| {
            s.allocs += 1;
            s.bytes += layout.size() as u64;
            s.live += layout.size() as i64;
        });
        // SAFETY: forwarded unchanged; the caller guarantees `layout` has non-zero size.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(|s| {
            s.deallocs += 1;
            s.live -= layout.size() as i64;
        });
        // SAFETY: forwarded unchanged; `ptr` was allocated by `System` (through this type)
        // with `layout`, as the caller guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(|s| {
            s.reallocs += 1;
            s.bytes += new_size as u64;
            s.live += new_size as i64 - layout.size() as i64;
        });
        // SAFETY: forwarded unchanged; the caller upholds `realloc`'s contract for `ptr`,
        // `layout` and `new_size`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
