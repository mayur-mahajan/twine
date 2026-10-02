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
//! use twine_reactive::Runtime;
//!
//! let rt = Runtime::take().expect("runtime already taken"); // once, in the UI context
//! let cx = rt.create_root();
//! let (a, b) = (cx.signal(1), cx.signal(2));
//! let sum = cx.memo(move || a.get() + b.get());
//! let parity = sum.map(|s| s % 2);
//! cx.effect(move || println!("sum = {}, parity = {}", sum.get(), parity.get()));
//! a.set(3); // effect runs once: sum = 5, parity = 1
//! rt.batch(|| {
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
//! | [`Runtime`] | the token of the UI context's runtime ([`Runtime::take`]); root scopes ([`Runtime::create_root`]) and every runtime-wide operation |
//! | [`Scope`] | owns nodes, child scopes, cleanups, context values |
//! | [`Signal`], [`ReadSignal`], [`WriteSignal`] | reactive values |
//! | [`Memo`] | lazy, cached, equality-checked derived values |
//! | [`StoredValue`] | a plain, non-reactive value owned by a scope (`Copy` handle) |
//! | [`EffectId`] | side effects re-run when their dependencies change |
//! | [`Runtime::batch`], [`Runtime::untrack`] | group writes; read without subscribing |
//! | [`Channel`], [`Overflow`] | ISR/task-safe FIFO into the UI ([`Scope::on_message`]); full-queue policy |
//! | [`Latest`] | ISR/task-safe latest-value cell into the UI ([`Scope::watch`], [`Scope::on_latest`]): no queue, nothing dropped |
//! | [`Outbox`] | FIFO from the UI to a task: `try_recv`, or `recv().await` (feature `async`) |
//! | [`UiWaker`], [`WakerLease`] | wake-ups of the UI loop (and of an `Outbox` consumer); one waker per root scope ([`Scope::set_ui_waker`]); [`WakerLease::set_heap_limit`] caps the heap the waker pool may grow to |
//! | [`Runtime::take_faults`], [`Runtime::set_fault_hook`], [`Runtime::stats`] | faults the runtime recovered from, counters |
//! | [`Runtime::memory`], [`RuntimeMemory`] | the runtime's heap use, by part (arenas, values, scope data, queues) |
//! | [`TakeOnce`] | a `static` value handed out once as `&'static mut` (draw buffers, DMA memory), from any context |
//! | [`set_interrupt_probe`], [`in_interrupt`] | the platform's "am I in an interrupt?" answer, for diagnostics |
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
//!    `Ui` wraps event dispatch and [`Runtime::drain_channels`] in [`Runtime::batch`], so this mostly happens in
//!    tests that call `set` directly.
//! 3. **[`Runtime::flush_effects_with`]** at the `Ui`'s fixed point in the update cycle, with the engine
//!    as context. A binding that finds no engine (`downcast_mut` fails) calls
//!    [`Runtime::defer_current_effect`]; deferred effects are re-queued by the next flush with a
//!    non-`()` context. [`Runtime::has_pending_effects`] reports pending or deferred work.
//!
//! Code that runs without a context parameter (event handlers, user effects, timer
//! callbacks) reaches the engine through the *ambient* slot: [`Runtime::provide_ambient`] lends a
//! `&mut dyn Any` for the duration of a call, [`Runtime::with_ambient`] borrows it exclusively
//! (taking it out of the slot, so nested borrows see `None`) and [`Runtime::ambient_is`] tests its type
//! without borrowing it. A binding whose widget was deleted disposes itself with
//! [`Runtime::dispose_current_effect`].
//!
//! ### Several UIs on one runtime: roots
//!
//! Each `Ui` owns one root scope, and its update runs as that root's update
//! ([`Scope::activate`]). While a root is active, only its effects run with the flush context and
//! the ambient value; an effect of another root that a shared signal triggers meanwhile runs
//! with `()` and an empty ambient slot (a binding defers itself, and
//! [`Runtime::defer_current_effect`] wakes its root's waker so its own UI runs it), and a flush
//! with a context re-queues only the active root's deferred effects. Channel handlers are
//! delivered per root ([`Scope::drain_channels`], [`Scope::any_channel_pending`]) and pending
//! work is asked per root ([`Scope::has_pending_effects`]). So a binding only ever writes into
//! the engine it was built for. Outside every update no root is active and nothing is
//! filtered. Cost: one comparison per effect run; the rare foreign effect runs out of line.
//!
//! A flush processes effects in rounds (effects queued by one round form the next); after
//! [`Runtime::set_flush_iterations_limit`] rounds (default 100) the rest are dropped with an `error!`.
//!
//! ## Faults
//!
//! Situations the runtime recovers from but that the application must be able to see are
//! recorded as faults (the [`FaultKind`] vocabulary of `twine-core`): an effect loop cut by the
//! iteration limit ([`FaultKind::EffectLoopCut`] — dropped effects may leave bound values
//! outdated), the deep-chain guard ([`FaultKind::DepthGuard`]) and messages dropped by a full
//! [`Channel`] ([`FaultKind::ChannelOverflow`]). Each is logged, counted in [`Runtime::stats`],
//! reported to the [`Runtime::set_fault_hook`] function and kept for [`Runtime::take_faults`]; `Ui` forwards them
//! to the engine's fault stream at every update.
//!
//! ## Rule R1 (re-entrancy)
//!
//! The runtime never holds a borrow of its internal state while user code runs (closures,
//! cleanups, `Clone`/`PartialEq`/`Drop` of values), so effects, memos, cleanups and channel
//! handlers may freely create, read, write and dispose reactive nodes.
//!
//! ## Atomics on chips without compare-and-swap
//!
//! The crate uses `portable-atomic` and does not choose its fallback: the application does. On
//! targets without atomic read-modify-write instructions — `thumbv6m` (RP2040, Cortex-M0/M0+)
//! and single-core `riscv32imc` (ESP32-C3) — add one of:
//!
//! - `portable-atomic = { version = "1", features = ["critical-section"] }` plus a
//!   critical-section implementation (e.g. `embassy-rp`'s `critical-section-impl`,
//!   `cortex-m`'s `critical-section-single-core`), or
//! - `--cfg portable_atomic_unsafe_assume_single_core` in `RUSTFLAGS` (single-core chips only;
//!   esp-hal already enables `portable-atomic`'s `unsafe-assume-single-core` on the ESP32-C3).
//!
//! Not both: `portable-atomic` rejects the combination. Other targets need nothing.
//!
//! ## The runtime token and features
//!
//! The runtime belongs to one execution context (the UI thread or task): the one that took its
//! [`Runtime`] token with [`Runtime::take`]. Everything that reaches the runtime needs that
//! token or a handle made from it, and tokens and handles are `!Send`/`!Sync`, so the compiler
//! keeps the runtime in its context — no `unsafe` binding, no run-time check. The
//! cross-context types — [`Channel`], [`Latest`], [`Outbox`] and [`UiWaker`] — are the only
//! `Sync` ones: they never touch the runtime, use only `critical-section` and
//! `portable-atomic` (no compare-and-swap), allocate nothing when used, and may be used from
//! interrupts, other tasks, other cores and other threads.
//!
//! **Ports.** The UI registers with a cross-context object by `&'static` reference
//! ([`Scope::on_message`] § Ports explains why): declare them in `main` (or the board code)
//! and pass the application a `Copy` struct of references — `Ports { sensor: &SENSOR, cmd:
//! &CMD }` — instead of letting it name globals. Tests then give each UI its own objects.
//!
//! - `std`: one runtime per thread (`thread_local!`); [`Runtime::take`] succeeds once per
//!   thread, `Runtime::current_thread` always. Used on the host, in the simulator and in
//!   tests.
//! - without `std`: one runtime per process, in a `static`; [`Runtime::take`] succeeds once.
//! - `debug-checks`: every runtime access asks the interrupt probe ([`set_interrupt_probe`],
//!   installed by `UiBuilder::platform`) and panics when it reports interrupt context (a
//!   safety net for `unsafe` code; costs one probe call per access, nothing without the
//!   feature).
//! - `async`: `Outbox::recv`, an executor-agnostic future (no dependency).
//! - `log` / `defmt`: logging backend (target `twine::reactive`).
//!
//! The cross-context types use `critical-section`; the final binary provides its implementation (embassy,
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
mod latest;
mod memo;
mod outbox;
mod queue;
mod runtime;
mod scope;
mod signal;
mod stored;
mod take_once;

pub use channel::{Channel, UiWaker, WakerLease};
pub use effect::EffectId;
pub use global::{Runtime, in_interrupt, interrupt_probe, set_interrupt_probe};
pub use latest::Latest;
pub use memo::Memo;
pub use outbox::Outbox;
#[cfg(feature = "async")]
pub use outbox::Recv;
pub use queue::Overflow;
pub use runtime::FaultHook;
pub use scope::Scope;
pub use signal::{ReadSignal, Signal, WriteSignal};
pub use stored::StoredValue;
pub use take_once::TakeOnce;
pub use twine_core::fault::{FaultCounts, FaultKind, Faults};

/// What a reactive runtime's heap is used for, from [`Runtime::memory`].
///
/// **Units:** `nodes` and `scopes` are counts; every other field is in **bytes**, counted as
/// what was asked of the allocator (capacities, box and `Rc` sizes), without allocator
/// overhead. Not included: what values own themselves (a `String` in a signal) and the
/// `static` channels (`Channel`, `Latest`, `Outbox` live in the application's statics).
///
/// ```
/// let rt = twine_reactive::Runtime::take().unwrap();
/// let before = rt.memory();
/// let cx = rt.create_root();
/// let s = cx.signal(1u32);
/// let m = rt.memory();
/// assert_eq!(m.nodes, before.nodes + 1);
/// assert!(m.values > before.values); // the signal's value cell
/// assert!(m.bytes() >= m.arenas + m.values);
/// # let _ = s;
/// cx.dispose();
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RuntimeMemory {
    /// Live signals, memos, effects and stored values (a count).
    pub nodes: usize,
    /// Live scopes (a count).
    pub scopes: usize,
    /// The node and scope arenas (every slot, by capacity, and their free lists).
    pub arenas: usize,
    /// Value cells of signals, memos and stored values, the computations (closures and
    /// source lists) of memos and effects, and spilled subscriber lists.
    pub values: usize,
    /// Per scope: child and node lists, cleanup closures and provided context values.
    pub scope_data: usize,
    /// The effect queues, `on_message` / `watch` registrations, the staleness work stack and
    /// the root wakers.
    pub queues: usize,
}

impl RuntimeMemory {
    /// Total heap bytes: `arenas + values + scope_data + queues`.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.arenas + self.values + self.scope_data + self.queues
    }
}

/// Object counts and counters of a runtime, from [`Runtime::stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Live signals, memos, effects and stored values.
    pub nodes: usize,
    /// Live scopes.
    pub scopes: usize,
    /// Effects queued to run.
    pub pending: usize,
    /// Effects waiting for a real context.
    pub deferred: usize,
    /// `on_message`, `on_latest` and `watch` registrations.
    pub channels: usize,
    /// Signal writes that notified subscribers.
    pub writes: u64,
    /// Effect runs.
    pub effect_runs: u64,
    /// Memo recomputations.
    pub memo_runs: u64,
    /// Faults recorded since start-up, per kind (see [`Runtime::take_faults`]).
    pub faults: FaultCounts,
    /// Retired arena slots of nodes and scopes: slots that used up their 65 535 generations
    /// and are never reused ([`twine_core::Arena::retired`]). Non-zero only after millions of
    /// create/dispose cycles; each one is a few dozen bytes the runtime can no longer reuse.
    pub retired_slots: usize,
}
