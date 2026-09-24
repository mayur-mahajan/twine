//! # twine-reactive
//!
//! Fine-grained, push-pull, glitch-free reactive signals for a single UI thread
//! The declarative view layer (`twine-view`) builds on it: every
//! widget property bound to a signal is an [effect](Scope::effect) that writes that one
//! property, so a signal change runs exactly the bindings that depend on it (P2).
//!
//! It sits directly above `twine-core` in the layering: `no_std` + `alloc`, depends only on
//! `twine-core`, `critical-section`, `portable-atomic` and `heapless`, and uses no `Arc` or CAS
//! atomics (it builds for thumbv6m).
//!
//! ```
//! use twine_reactive::{batch, create_root};
//!
//! let cx = create_root();
//! let (a, b) = (cx.signal(1), cx.signal(2));
//! let sum = cx.memo(move || a.get() + b.get());
//! let parity = sum.map(|s| s % 2);
//! cx.effect(move || println!("sum = {}, parity = {}", sum.get(), parity.get()));
//! a.set(3); // effect runs once: sum = 5, parity = 1
//! batch(|| {
//!     a.set(1);
//!     b.set(1);
//! }); // effect runs once for both writes
//! assert_eq!(sum.get(), 2);
//! cx.dispose();
//! ```
//!
//! ## Primitives
//!
//! | Item | Role |
//! |------|------|
//! | [`Scope`] | owns nodes, child scopes, cleanups, context values; [`create_root`] makes one |
//! | [`Signal`], [`ReadSignal`], [`WriteSignal`] | reactive values |
//! | [`Memo`] | lazy, cached, equality-checked derived values |
//! | [`EffectId`] | side effects re-run when their dependencies change |
//! | [`batch`], [`untrack`] | group writes; read without subscribing |
//! | [`Channel`], [`UiWaker`] | ISR/task-safe message delivery into the UI |
//!
//! All handles are `Copy` and `!Send`/`!Sync`. Using a handle whose scope was disposed panics
//! (with the creation location in debug builds); `try_get`/`is_alive` exist for handles
//! intentionally shared across scopes.
//!
//! ## Algorithm (push-pull coloring)
//!
//! A write **pushes** colors: direct subscribers become `Dirty`, their descendants `Check`
//! (descent stops at nodes already colored); effects reached are queued once. Reading a memo,
//! or flushing a queued effect, **pulls**: a `Check` node first brings its sources up to date
//! in read order and only recomputes if one of them actually changed; a memo whose new value
//! equals the old one does not mark its subscribers, so propagation stops there.
//!
//! ```text
//!   a.set(3)                         a ──► sum ──► parity ──► E2
//!                                          │
//!   push:  sum = Dirty                     └────────────────► E1
//!          parity, E1, E2 = Check;  E1, E2 queued
//!
//!   flush E1: Check → pull sum (recompute, changed) → E1 Dirty → run E1
//!   flush E2: Check → pull parity → pull sum (Clean) → recompute parity
//!             parity unchanged → E2 stays Check → Clean, not run
//! ```
//!
//! Every computation therefore runs at most once per change and never observes a mix of old
//! and new values (glitch-free: in a diamond `a → b, a → c, b + c → d`, `d` runs once per
//! write of `a`). Memos are lazy: they compute on first read, not at creation.
//!
//! **Deep chains.** The pull is recursive. Beyond a depth of 256 nested checks a node is
//! recomputed without checking its sources (an `error!` is logged; results stay correct,
//! only extra work is done). Computing a long chain for the first time still recurses once
//! per memo through user code, so keep memo chains shallow on MCUs with small stacks.
//!
//! ## Effects and the view layer
//!
//! Effects receive a `&mut dyn Any` context ([`Scope::effect_with_cx`]). The view layer uses
//! three entry points:
//!
//! 1. **Creation**: an effect created outside a flush runs immediately with a `()` context;
//!    created during a flush, it runs later in that same flush with the flush's context.
//! 2. **Automatic flush**: a write outside any batch or flush flushes at once with `()`.
//!    `Ui` wraps event dispatch and [`drain_channels`] in [`batch`], so this mostly happens in
//!    tests that call `set` directly.
//! 3. **[`flush_effects_with`]** at the `Ui`'s fixed point in the update cycle, with the engine
//!    as context. A binding that finds no engine (`downcast_mut` fails) calls
//!    [`defer_current_effect`]; deferred effects are re-queued by the next flush with a
//!    non-`()` context. [`has_pending_effects`] reports pending or deferred work.
//!
//! Code that runs without a context parameter (event handlers, user effects, timer
//! callbacks) reaches the engine through the *ambient* slot: [`provide_ambient`] lends a
//! `&mut dyn Any` for the duration of a call, [`with_ambient`] borrows it exclusively
//! (taking it out of the slot, so nested borrows see `None`). A binding whose widget was
//! deleted disposes itself with [`dispose_current_effect`].
//!
//! A flush processes effects in rounds (effects queued by one round form the next); after
//! [`set_flush_iterations_limit`] rounds (default 100) the rest are dropped with an `error!`.
//!
//! ## Rule R1 (re-entrancy)
//!
//! The runtime never holds a borrow of its internal state while user code runs (closures,
//! cleanups, `Clone`/`PartialEq`/`Drop` of values), so effects, memos, cleanups and channel
//! handlers may freely create, read, write and dispose reactive nodes.
//!
//! ## Global runtime and features
//!
//! - `std`: one runtime per thread (`thread_local!`). Used on the host, in the simulator and
//!   in tests.
//! - without `std`: one runtime in a `static`. It must be bound to the UI context once with
//!   `unsafe { twine_reactive::bind_to_current_context() }`; the safety contract is that the
//!   runtime is then only used from that context (see that function). [`Channel`] and
//!   [`UiWaker`] never touch the runtime and may be used from interrupts and other tasks.
//! - `log` / `defmt`: logging backend (target `twine::reactive`).
//!
//! [`Channel`] uses `critical-section`; the final binary provides its implementation (embassy,
//! esp-hal, `cortex-m`'s single-core implementation, or `critical-section/std` on the host).
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod ambient;
mod batch;
mod channel;
mod effect;
mod global;
mod handles;
mod key_list;
mod memo;
mod runtime;
mod scope;
mod signal;

pub use ambient::{provide_ambient, with_ambient};
pub use batch::{
    batch, defer_current_effect, dispose_current_effect, flush_effects_with, has_pending_effects,
    set_flush_iterations_limit, untrack,
};
pub use channel::{Channel, UiWaker, any_channel_pending, drain_channels, register_waker};
pub use effect::EffectId;
pub use global::{bind_to_current_context, create_root, debug_stats, reset};
pub use memo::Memo;
pub use scope::Scope;
pub use signal::{ReadSignal, Signal, WriteSignal};

/// Object counts and counters of the current runtime, from [`debug_stats`] (tests and
/// diagnostics; not a stable API).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Live signals, memos and effects.
    pub nodes: usize,
    /// Live scopes.
    pub scopes: usize,
    /// Effects queued to run.
    pub pending: usize,
    /// Effects waiting for a real context.
    pub deferred: usize,
    /// `on_message` registrations.
    pub channels: usize,
    /// Signal writes that notified subscribers.
    pub writes: u64,
    /// Effect runs.
    pub effect_runs: u64,
    /// Memo recomputations.
    pub memo_runs: u64,
    /// Times the deep-chain guard (depth > 256) triggered.
    pub depth_guard_hits: u64,
    /// Flushes cut by the iteration limit.
    pub loop_cuts: u64,
}
