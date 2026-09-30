//! Runtime faults (R0.S01): effect loop cuts, the depth guard and dropped channel messages are
//! recorded, counted, reported to the hook and returned by `take_faults`.

use std::cell::Cell;

use twine_reactive::{
    Channel, FaultKind, Memo, create_root, drain_channels, runtime_stats, set_fault_hook,
    set_flush_iterations_limit, take_faults,
};

thread_local! {
    static SEEN: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

/// Counts (calls, occurrences) of `ChannelOverflow` on this thread.
fn hook(kind: FaultKind, n: u32) {
    if kind == FaultKind::ChannelOverflow {
        SEEN.with(|s| {
            let (c, o) = s.get();
            s.set((c + 1, o + n));
        });
    }
}

static CH: Channel<u8, 2> = Channel::new();

#[test]
fn dropped_channel_messages_are_a_fault() {
    let _ = take_faults();
    set_fault_hook(Some(hook));
    let cx = create_root();
    let got = cx.signal(0u32);
    cx.on_message(&CH, move |_| got.update(|n| *n += 1));
    for i in 0..5 {
        let _ = CH.try_send(i); // capacity 2: three are dropped
    }
    drain_channels(16);
    assert_eq!(got.get(), 2);
    let f = take_faults();
    assert_eq!(f.get(FaultKind::ChannelOverflow), 3);
    assert_eq!(SEEN.with(Cell::get), (1, 3));
    assert_eq!(runtime_stats().faults.get(FaultKind::ChannelOverflow), 3);
    set_fault_hook(None);
    cx.dispose();
}

#[test]
fn effect_loop_cut_is_a_fault() {
    let _ = take_faults();
    let cx = create_root();
    set_flush_iterations_limit(5);
    let a = cx.signal(0u32);
    cx.effect(move || a.set(a.get() + 1));
    let f = take_faults();
    assert_eq!(f.get(FaultKind::EffectLoopCut), 1);
    assert!(take_faults().kinds().is_empty(), "taken faults are cleared");
    assert_eq!(
        runtime_stats().faults.get(FaultKind::EffectLoopCut),
        1,
        "lifetime count kept"
    );
    cx.dispose();
}

#[test]
fn depth_guard_is_a_fault() {
    // Deep check recursion: run on a thread with a large stack (it has its own runtime).
    let n = std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(|| {
            let cx = create_root();
            let a = cx.signal(1u64);
            let mut prev: Option<Memo<u64>> = None;
            for _ in 0..300 {
                let p = prev;
                prev = Some(cx.memo(move || p.map_or_else(|| a.get(), |p| p.get()) + 1));
            }
            let last = prev.unwrap();
            assert_eq!(last.get(), 301);
            a.set(2);
            assert_eq!(last.get(), 302);
            take_faults().get(FaultKind::DepthGuard)
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(n > 0);
}
