//! Generation width per profile (R5.S04): stale-handle detection after many reuses of one
//! slot, retirement in the default profile, none in the safety profile (48-bit generations),
//! and the arithmetic behind the documented generation budget.
//!
//! Runs in both profiles (`cargo test -p twine-core` and `--features safety`). The churn test
//! scales its cycle count under Miri (10⁵ arena operations take minutes there).
//!
//! req: SAF-011

use twine_core::arena::{Generation, MAX_GENERATION, RawId};
use twine_core::{Arena, Id};

/// Insert/remove cycles of the churn test: past the 16-bit range twice natively.
const CYCLES: u32 = if cfg!(miri) { 2_000 } else { 2 * 65_536 + 10 };

/// A generation as `u64` in both profiles (the identity in the safety profile, whose
/// generations are `u64` already).
#[allow(clippy::useless_conversion)]
fn wide(g: Generation) -> u64 {
    u64::from(g)
}

/// Seconds in a (Julian) year.
const YEAR_S: u64 = 31_557_600;

/// One value churned in a one-slot arena: every reuse lands on the same slot (the rotation
/// has nowhere else to go), so the slot's generation advances once per cycle.
#[test]
fn stale_handles_never_resolve_after_many_reuses() {
    let mut a: Arena<u32> = Arena::new();
    let first = a.insert(0).unwrap();
    let mut id = first;
    let mut samples: Vec<Id<u32>> = Vec::new();
    for cycle in 1..=CYCLES {
        assert_eq!(a.remove(id), Some(cycle - 1));
        id = a.insert(cycle).unwrap();
        assert_eq!(a.get(id), Some(&cycle));
        if cycle % 4_096 == 1 {
            samples.push(id);
        }
    }
    // Every earlier handle is stale, including ones whose generation the default profile's
    // slot would have reached again had it wrapped instead of retiring.
    assert_eq!(a.get(first), None);
    for &old in samples.iter().filter(|&&s| s != id) {
        assert_eq!(a.get(old), None, "{old} resolved after {CYCLES} reuses");
        assert!(!a.contains(old));
    }
    if twine_core::SAFETY_PROFILE {
        // 48-bit generations: the slot is still in service.
        assert_eq!((a.retired(), a.slot_count()), (0, 1));
        assert_eq!(wide(id.generation()), u64::from(CYCLES) + 1);
    } else if !cfg!(miri) {
        // 16-bit generations: a slot serves 65 535 values, then retires; its successor
        // takes over (memory grows by one slot per 65 535 insertions, never faster).
        let per_slot = u32::from(u16::MAX);
        let retired = (CYCLES + 1) / per_slot;
        assert_eq!(a.retired(), retired as usize);
        assert_eq!(a.slot_count(), a.retired() + 1);
        assert_eq!(u32::from(id.index()), retired);
    }
}

/// The documented budget: no slot retires before `(P − 1)(K(F + 1) + 1)` insertions, with
/// `F` free slots and `K = ⌊(G − 1) / P⌋`. Checks the numbers of the table in the `Arena`
/// docs for this build's width.
#[test]
fn generation_budget_matches_the_documented_table() {
    let g = wide(MAX_GENERATION);
    let p = u64::from(Arena::<()>::ROTATE_PERIOD);
    let k = (g - 1) / p;
    let bound = |free: u64| (p - 1) * (k * (free + 1) + 1);
    if twine_core::SAFETY_PROFILE {
        assert_eq!(g, (1 << 48) - (1 << 32) - 1);
        assert_eq!(k, 68_718_428_159);
        assert_eq!(bound(1), 562_803_926_626_305);
        // One slot alone (no rotation): 8 900 years at 1 000 insertions/s, 89 at 100 000.
        assert!(g / 1_000 >= 8_900 * YEAR_S);
        assert!(g / 100_000 >= 89 * YEAR_S);
        // One free slot: 17 800 years at 1 000/s, 178 at 100 000/s; 15: 1 400 at 100 000/s.
        assert!(bound(1) / 1_000 >= 17_800 * YEAR_S);
        assert!(bound(1) / 100_000 >= 178 * YEAR_S);
        assert!(bound(15) / 100_000 >= 1_400 * YEAR_S);
    } else {
        assert_eq!(k, 15);
        assert_eq!(bound(1), 126_945);
        assert_eq!(bound(15), 986_895);
        // At 100 insertions per second: 11 minutes for one slot alone, 21 with one free
        // slot, 2.7 h with 15.
        assert!(g / 100 >= 10 * 60);
        assert!(bound(1) / 100 >= 21 * 60);
        assert!(bound(15) / 100 >= 27 * 360);
    }
}

/// The handle sizes each profile promises (also `const`-asserted in `twine-core`).
#[test]
fn id_sizes_per_profile() {
    use core::mem::size_of;
    if twine_core::SAFETY_PROFILE {
        assert_eq!(size_of::<Id<u32>>(), 8);
        assert_eq!(size_of::<Option<Id<u32>>>(), 8);
        assert_eq!(size_of::<RawId>(), 8);
        assert_eq!(size_of::<Generation>(), 8); // a 48-bit value
    } else {
        assert_eq!(size_of::<Id<u32>>(), 4);
        assert_eq!(size_of::<Option<Id<u32>>>(), 6);
        assert_eq!(size_of::<RawId>(), 4);
        assert_eq!(size_of::<Generation>(), 2);
    }
}
