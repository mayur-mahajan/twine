//! `TakeOnce` (R3.S03): a `static` handed out once, from any context.

use std::sync::atomic::{AtomicUsize, Ordering};

use twine_reactive::TakeOnce;

#[test]
fn take_hands_out_the_value_once() {
    static CELL: TakeOnce<[u32; 3]> = TakeOnce::new([1, 2, 3]);
    assert!(!CELL.is_taken());
    let v = CELL.take().expect("first take");
    assert_eq!(*v, [1, 2, 3]);
    v[0] = 9;
    assert!(CELL.take().is_none());
    assert!(CELL.is_taken());
    assert_eq!(v[0], 9, "the reference stays valid after a refused take");
}

/// Contexts racing for the value: exactly one wins (the claim is one critical section).
#[test]
fn concurrent_takes_hand_out_one_reference() {
    static CELL: TakeOnce<u64> = TakeOnce::new(0);
    static WINS: AtomicUsize = AtomicUsize::new(0);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                if let Some(v) = CELL.take() {
                    *v += 1;
                    WINS.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(WINS.load(Ordering::SeqCst), 1);
    assert!(CELL.take().is_none());
}

#[test]
fn debug_does_not_read_the_value() {
    static CELL: TakeOnce<u8> = TakeOnce::new(7);
    let _v = CELL.take();
    assert_eq!(format!("{CELL:?}"), "TakeOnce { taken: true, .. }");
}
