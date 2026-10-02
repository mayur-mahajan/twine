//! Several roots (one per UI) on one runtime (R3.S08): the active root's update runs only its
//! own effects with the flush context and the ambient value; other roots' effects run without
//! them, defer to their own root and wake its waker; draining and pending work are per root.

use std::cell::Cell;
use std::rc::Rc;

use twine_reactive::{Channel, Runtime, UiWaker};

fn rt() -> Runtime {
    Runtime::current_thread()
}

/// What an effect saw when it ran: the value, whether it had the `u32` context, and whether
/// the ambient slot held a `u32`.
type Seen = Rc<Cell<Option<(u32, bool, bool)>>>;

#[test]
fn foreign_effects_see_neither_context_nor_ambient() {
    let rt = rt();
    let (a, b) = (rt.create_root(), rt.create_root());
    let shared = rt.create_root().signal(0u32);
    let watch = |root: twine_reactive::Scope| -> Seen {
        let seen: Seen = Rc::default();
        let s = seen.clone();
        root.effect_with_cx(move |ctx| {
            let v = shared.get();
            s.set(Some((v, ctx.is::<u32>(), rt.ambient_is::<u32>())));
        });
        seen
    };
    let (seen_a, seen_b) = (watch(a), watch(b));
    let mut engine_a = 0u32;
    let mut ctx_a = 0u32;
    a.activate(|| {
        rt.provide_ambient(&mut engine_a, || {
            rt.batch(|| {
                shared.set(1);
                rt.flush_effects_with(&mut ctx_a);
            });
        });
    });
    assert_eq!(
        seen_a.get(),
        Some((1, true, true)),
        "the active root's effect gets both"
    );
    assert_eq!(
        seen_b.get(),
        Some((1, false, false)),
        "another root's effect gets neither"
    );
}

#[test]
fn ambient_and_active_root_are_restored_after_a_foreign_effect() {
    let rt = rt();
    let (a, b) = (rt.create_root(), rt.create_root());
    let x = rt.create_root().signal(0);
    b.effect(move || {
        x.get();
    });
    let after = Rc::new(Cell::new(false));
    let af = after.clone();
    a.effect(move || {
        x.get();
        af.set(rt.ambient_is::<u8>());
    });
    let mut v = 0u8;
    a.activate(|| rt.provide_ambient(&mut v, || x.set(1)));
    assert!(
        after.get(),
        "A's effect, flushed after B's, still sees the ambient value"
    );
}

#[test]
fn deferred_effects_requeue_only_for_their_root_and_wake_it() {
    static WAKER_B: UiWaker = UiWaker::new();
    let rt = rt();
    let (a, b) = (rt.create_root(), rt.create_root());
    b.set_ui_waker(&WAKER_B);
    let shared = rt.create_root().signal(0u32);
    let runs_b = Rc::new(Cell::new(0u32));
    let r = runs_b.clone();
    b.effect_with_cx(move |ctx| {
        let v = shared.get();
        if ctx.is::<u32>() {
            r.set(v);
        } else {
            rt.defer_current_effect();
        }
    });
    // Created without a context: deferred (B woken); B's update runs it.
    assert!(WAKER_B.take());
    let mut ctx = 0u32;
    b.activate(|| rt.flush_effects_with(&mut ctx));
    assert!(!b.has_pending_effects());
    a.activate(|| {
        rt.batch(|| {
            shared.set(5);
            rt.flush_effects_with(&mut ctx);
        });
    });
    assert_eq!(runs_b.get(), 0, "B's effect did not run with A's context");
    assert!(b.has_pending_effects() && !a.has_pending_effects());
    assert!(WAKER_B.take(), "B was woken to run its deferred effect");
    // A flush of A does not re-queue it either.
    a.activate(|| rt.flush_effects_with(&mut ctx));
    assert_eq!(runs_b.get(), 0);
    b.activate(|| rt.flush_effects_with(&mut ctx));
    assert_eq!(runs_b.get(), 5);
    assert!(!b.has_pending_effects());
}

#[test]
fn defer_in_own_update_does_not_wake() {
    static WAKER: UiWaker = UiWaker::new();
    let rt = rt();
    let a = rt.create_root();
    a.set_ui_waker(&WAKER);
    let x = a.signal(0);
    a.effect_with_cx(move |ctx| {
        x.get();
        if !ctx.is::<u32>() {
            rt.defer_current_effect();
        }
    });
    WAKER.take(); // the creation deferred without an active root: woken
    a.activate(|| x.set(1)); // own update, no context: deferred, busy, but no wake-up
    assert!(!WAKER.is_set());
    assert!(a.has_pending_effects(), "the update sees its own deferred work");
    let mut ctx = 0u32;
    a.activate(|| rt.flush_effects_with(&mut ctx));
    assert!(!a.has_pending_effects());
    x.set(2); // between updates: deferred and woken
    assert!(WAKER.take());
}

#[test]
fn drain_and_pending_messages_are_per_root() {
    // A `static` (not `Box::leak`): `Channel::new` is `const`, and Miri reports leaked heap memory.
    static CH_A: Channel<u8, 2> = Channel::new();
    let rt = rt();
    let (a, b) = (rt.create_root(), rt.create_root());
    let ch_a: &'static Channel<u8, 2> = &CH_A;
    let got = Rc::new(Cell::new(0u8));
    let g = got.clone();
    a.child().child().on_message(ch_a, move |v| g.set(v));
    ch_a.try_send(9).unwrap();
    assert!(a.any_channel_pending() && !b.any_channel_pending());
    assert_eq!(b.drain_channels(8), 0);
    assert_eq!(got.get(), 0);
    assert_eq!(
        a.child().drain_channels(8),
        1,
        "any scope of the root drains the root's"
    );
    assert_eq!(got.get(), 9);
}

#[test]
fn activate_nests_and_restores_on_panic() {
    let rt = rt();
    let (a, b) = (rt.create_root(), rt.create_root());
    let shared = rt.create_root().signal(0u32);
    let ctx_seen = Rc::new(Cell::new(false));
    let c = ctx_seen.clone();
    a.effect_with_cx(move |ctx| {
        shared.get();
        c.set(ctx.is::<u32>());
    });
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.activate(|| panic!("boom"))));
    assert!(r.is_err());
    // No root is active any more: A's effect gets the context of an unfiltered flush.
    let mut ctx = 0u32;
    rt.batch(|| {
        shared.set(1);
        rt.flush_effects_with(&mut ctx);
    });
    assert!(ctx_seen.get());
    // Nested: the inner root wins, then the outer one is back.
    b.activate(|| {
        a.activate(|| {
            rt.batch(|| {
                shared.set(2);
                rt.flush_effects_with(&mut ctx);
            });
        });
        assert!(ctx_seen.get());
        rt.batch(|| {
            shared.set(3);
            rt.flush_effects_with(&mut ctx);
        });
        assert!(!ctx_seen.get(), "B active again: A's effect is foreign");
    });
}
