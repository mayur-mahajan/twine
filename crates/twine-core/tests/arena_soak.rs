//! Soak tests for `Arena` generation wear (R0.S07): the rotating reuse order (a hot LIFO stack
//! plus a FIFO reserve every `ROTATE_PERIOD` generations) spreads the 16-bit generations over
//! every free slot, so a small arena under heavy churn retires nothing until the documented
//! bound, and its memory stays flat.
//!
//! Not built under Miri: proving the retirement bound takes millions of cycles by definition
//! (≈ 10⁷ here, 0.15 s natively, days under Miri), and the arena contains no `unsafe` for Miri to
//! check (`twine-core` is `#![forbid(unsafe_code)]`). The arena's model proptest and unit tests
//! still run under Miri.
#![cfg(not(miri))]

use twine_core::{Arena, Id};

/// Slots of the soak arena.
const SLOTS: usize = 64;
/// Rotation period `P`.
const P: usize = Arena::<u32>::ROTATE_PERIOD as usize;
/// Parkings in a slot's lifetime, `K = ⌊65 534 / P⌋`.
const K: usize = (u16::MAX as usize - 1) / P;

/// The documented bound: with `SLOTS` slots and at most `live` values live, no slot retires
/// before `(P − 1) × (K × (S − L + 1) + 1)` insertions.
const fn bound(live: usize) -> usize {
    (P - 1) * (K * (SLOTS - live + 1) + 1)
}

/// Deterministic xorshift32, so a failure is reproducible.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// An arena of exactly `SLOTS` slots, all free.
fn warm_arena() -> Arena<u32> {
    let mut a = Arena::with_capacity(SLOTS as u16);
    let ids: Vec<Id<u32>> = (0..SLOTS as u32).map(|i| a.insert(i).unwrap()).collect();
    for id in ids {
        a.remove(id).unwrap();
    }
    assert_eq!(
        (a.len(), a.slot_count(), a.capacity(), a.retired()),
        (0, SLOTS, SLOTS, 0)
    );
    a
}

/// Checks the heap is flat while nothing retired, and that growth afterwards only replaces
/// retired slots.
fn check_flat(a: &Arena<u32>, cycle: usize) {
    if a.retired() == 0 {
        assert_eq!(a.slot_count(), SLOTS, "cycle {cycle}");
        assert_eq!(a.capacity(), SLOTS, "cycle {cycle}");
    } else {
        assert!(a.slot_count() - a.retired() <= SLOTS, "cycle {cycle}");
    }
}

/// 10⁷ create/delete cycles (one insert + one random remove each, at most 8 live) on a
/// 64-slot arena. No slot may retire before the documented bound (≈ 3.5 × 10⁶ insertions);
/// until then the slot storage must not grow at all. 10⁷ cycles runs well past the budget of
/// 64 slots (64 × 65 535 ≈ 4.2 × 10⁶), so the test also checks that retirement, once it
/// starts, grows memory only by the retired slots.
#[test]
fn soak_retires_nothing_before_the_bound() {
    const CYCLES: usize = 10_000_000;
    const MAX_LIVE: usize = 8;
    let mut a = warm_arena();
    let mut rng = Rng(0x2545_f491);
    let mut live: Vec<Id<u32>> = Vec::with_capacity(MAX_LIVE);
    let mut first_retirement = None;
    for cycle in 0..CYCLES {
        while live.len() < MAX_LIVE {
            let id = a.insert(cycle as u32).unwrap();
            assert!(id.generation() >= 1 && id != Id::INVALID);
            live.push(id);
        }
        let victim = live.swap_remove(rng.next() as usize % live.len());
        assert!(a.remove(victim).is_some());
        check_flat(&a, cycle);
        if a.retired() > 0 {
            first_retirement.get_or_insert(cycle);
        }
    }
    let first = first_retirement.expect("10^7 cycles exceed the 64-slot budget");
    // One insertion per cycle (plus MAX_LIVE - 1 at the start).
    assert!(
        first + MAX_LIVE >= bound(MAX_LIVE),
        "retired after {first} cycles, bound {}",
        bound(MAX_LIVE)
    );
    assert!(a.retired() > 0 && a.retired() < 3 * SLOTS);
}

/// The pattern plain LIFO handled worst: one value churned while every other slot is free.
/// LIFO hammered a single slot (retiring it after 65 535 cycles); the rotation moves on to the
/// next free slot every `ROTATE_PERIOD` generations, so all 64 slots share the wear and the
/// first retirement comes only after the bound for `L = 1` (≈ 3.9 × 10⁶ cycles).
#[test]
fn regression_single_value_churn_rotates_through_free_slots() {
    let mut a = warm_arena();
    let mut id = a.insert(0).unwrap();
    let mut max_gen = [0u16; SLOTS];
    let mut cycles = 0usize;
    while a.retired() == 0 {
        a.remove(id).unwrap();
        cycles += 1;
        if a.retired() > 0 {
            break;
        }
        id = a.insert(cycles as u32).unwrap();
        let g = &mut max_gen[usize::from(id.index())];
        *g = (*g).max(id.generation());
        // Wear stays level: once every slot had a turn, no slot runs more than one period
        // (plus the warm-up generation) ahead of the least-worn one.
        if cycles % (P * SLOTS) == 0 {
            let (lo, hi) = (max_gen.iter().min().unwrap(), max_gen.iter().max().unwrap());
            assert!(usize::from(hi - lo) <= P + 1, "cycle {cycles}: {lo}..{hi}");
        }
        check_flat(&a, cycles);
    }
    assert!(
        cycles >= bound(1),
        "retired after {cycles} cycles, bound {}",
        bound(1)
    );
    assert!(
        max_gen
            .iter()
            .all(|&g| g >= u16::MAX - Arena::<u32>::ROTATE_PERIOD),
        "{max_gen:?}"
    );
}
