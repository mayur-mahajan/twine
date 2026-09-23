//! `CountingAllocator`. This test binary installs it as the global allocator.

use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

#[test]
fn counts_vec_allocation() {
    let (v, stats) = count_allocs(|| {
        let mut v: Vec<u32> = Vec::with_capacity(4);
        v.extend([1, 2, 3, 4]);
        v.push(5); // grows → realloc
        v
    });
    assert_eq!(v.len(), 5);
    assert_eq!(stats.allocs, 1);
    assert_eq!(stats.reallocs, 1);
    assert!(stats.bytes >= 16 + 20, "{stats:?}");
    let ((), stats) = count_allocs(|| drop(v));
    assert_eq!(stats.deallocs, 1);
}

#[test]
fn zero_for_stack_only_code() {
    let (sum, stats) = count_allocs(|| {
        let a = [1u64; 64];
        a.iter().sum::<u64>()
    });
    assert_eq!(sum, 64);
    assert_eq!(stats, twine_testing::alloc::AllocStats::default());
}

#[test]
fn counters_are_per_thread() {
    let other = std::thread::spawn(|| count_allocs(|| drop(vec![0u8; 100])).1)
        .join()
        .unwrap();
    assert_eq!((other.allocs, other.deallocs, other.bytes), (1, 1, 100));
}
