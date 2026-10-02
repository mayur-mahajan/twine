//! Run loops: thin, optional adapters that step a [`Ui`] and sleep as it asks.
//!
//! Twine never owns the execution model: the UI is a step function ([`Ui::update`] returns a
//! [`Wake`]) plus wake-up signalling ([`UiWaker`](twine_reactive::UiWaker)). A run loop is a
//! few lines on top, and an application may always write its own. This module has the
//! blocking one, for every context that can block: bare metal, an RTOS task, a host thread.
//!
//! | Execution model | Adapter | Sleeps with |
//! |-----------------|---------|-------------|
//! | bare metal, RTOS task, host thread | [`blocking`], [`blocking_with`] | a [`Platform`] (`WFE`, a task notification, a condition variable) |
//! | embassy (async) | `twine_embassy::run`, `twine_embassy::run_with` | the executor (timer, task waker, input interrupt) |
//! | anything else | your loop around [`Ui::update`] / [`Ui::update_budgeted`] | your choice |
//!
//! ## What the blocking loop does
//!
//! ```text
//! loop {
//!     on_event(BeforeUpdate)
//!     match ui.update() {
//!         Wake::Now   => platform.yield_now()                         // and step again
//!         Wake::At(t) => { on_event(Idle(Some(t))); platform.wait(Some(t)); on_event(Woken) }
//!         Wake::Idle  => { on_event(Idle(None));    platform.wait(None);    on_event(Woken) }
//!         Wake::IdleFor(d) => { on_event(Inactive(d)); platform.wait(None);  on_event(Woken) }
//!     }
//! }
//! ```
//!
//! - **`Wake::Idle` never spins:** the loop waits with no deadline; only a wake-up ends the
//!   wait (a channel message, a [`Latest`](twine_reactive::Latest) set, [`Ui::notify_input`], an input interrupt calling the
//!   [`waker`](Ui::waker)).
//! - **`Wake::At(t)`** waits until `t` on the platform's clock, or a wake-up.
//! - **`Wake::Now`** steps again at once, after [`Platform::yield_now`] (an RTOS yields to
//!   the other ready tasks of its priority; bare metal does nothing). It cannot starve the
//!   UI's own work or livelock: the UI returns `Now` only while work is pending, and every
//!   update makes progress on it (the rest of a frame — at least one chunk with a
//!   [`StepBudget`](crate::StepBudget) —, the next
//!   [`messages_per_channel`](crate::UiBuilder::messages_per_channel) messages of a channel
//!   (at least one), the pending
//!   effects, a flush that is measured against `EngineConfig::flush_timeout` with the time of
//!   each step). Wake-ups that arrive meanwhile are taken by the next update without a wait;
//!   the [`notify`](Platform::notify) they made only ends the next wait early once (a spurious
//!   return the [`Platform`] contract allows). A `Now` streak does keep the CPU busy: with
//!   `EngineConfig::cooperative_flush` and no `hires_timer`, the loop polls a running DMA
//!   transfer instead of sleeping until its interrupt.
//! - **Wake-ups reach the wait:** the loop installs [`P::notify`](Platform::notify) on the
//!   UI's waker ([`UiWaker::set_notify`](twine_reactive::UiWaker::set_notify)) before its
//!   first step (as [`UiBuilder::platform`](crate::UiBuilder::platform) does), so every
//!   [`UiWaker::wake`](twine_reactive::UiWaker::wake) — from an interrupt handler, another
//!   task, thread or core — ends a wait in progress, and one made between the update and the
//!   wait makes that wait return at once. Nothing is lost and nothing polls.
//! - **One clock:** the deadlines are on the `Ui`'s clock and the platform waits on its own;
//!   build the `Ui` with [`UiBuilder::platform(&platform)`](crate::UiBuilder::platform) (or a
//!   clock with the same time base) so they agree.
//! - **No allocation, no dynamic dispatch** per iteration: the callback is a generic closure
//!   (with [`blocking`] it is empty and compiles away).
//!
//! ## Several displays, several UIs
//!
//! - **One `Ui`, several displays** ([`UiBuilder::display`](crate::UiBuilder::display),
//!   [`Ui::mount_on`]): one engine, one update cycle and one waker drive every display, so one
//!   loop — [`blocking`] or `twine_embassy::run` — drives them all. Each update renders every
//!   display that has something to draw (each with its own dirty areas); a wake-up from any of
//!   their inputs or channels ends the wait. This is the recommended set-up: the displays share
//!   fonts, caches and the reactive state at no extra cost.
//! - **Several `Ui`s on one runtime** (e.g. different engine configurations): each is a loop of
//!   its own — one embassy task per `Ui` (`twine_embassy::run` in each; the tasks share the
//!   thread's runtime), or a hand-written loop stepping each `Ui` and sleeping until the
//!   earliest deadline. [`blocking`] never returns, so it drives one `Ui` per thread. Updates of
//!   different `Ui`s never interfere: each runs only its own channel handlers and bindings, and
//!   a binding of one `Ui` triggered by another `Ui`'s update waits for its own `Ui`'s update,
//!   whose waker it wakes (see [`UiCore`](crate::UiCore) § Sharing the runtime).
//!
//! ## An RTOS task
//!
//! The UI runs in one task; [`Platform`] maps onto the kernel's task notification (or a binary
//! semaphore) with a timeout. Interrupts and other tasks talk to the UI through
//! [`Channel`](twine_reactive::Channel)s and the waker, whose `notify` gives the notification:
//!
//! | Kernel | `wait(deadline)` | `notify()` | `yield_now()` | `in_interrupt()` |
//! |--------|------------------|------------|---------------|------------------|
//! | `FreeRTOS` | `ulTaskNotifyTake(pdTRUE, ticks)` | `vTaskNotifyGiveFromISR` / `xTaskNotifyGive` | `taskYIELD()` | `xPortIsInsideInterrupt()` |
//! | Zephyr | `k_sem_take(&sem, timeout)` | `k_sem_give(&sem)` | `k_yield()` | `k_is_in_isr()` |
//! | `ThreadX` | `tx_semaphore_get(&sem, ticks)` | `tx_semaphore_ceiling_put(&sem, 1)` | `tx_thread_relinquish()` | the port's ISR nesting count |
//! | RTIC (v2, blocking idle) | `WFE` (`CortexMPlatform`) | `SEV` | — | `SCB::vect_active` |
//!
//! ```no_run
//! use twine_core::Instant;
//! use twine_hal::{Clock, Platform};
//! use twine_view::prelude::*;
//!
//! // The kernel's API (FreeRTOS here) through the application's bindings;
//! // `give_ui_notification` is the application's wrapper around `vTaskNotifyGiveFromISR`
//! // (in an interrupt) and `xTaskNotifyGive` (elsewhere) for the UI task's handle.
//! # #[allow(non_snake_case)] mod freertos {
//! #     pub fn xTaskGetTickCount() -> u32 { 0 }
//! #     pub fn ulTaskNotifyTake(_clear: bool, _ticks: u32) -> u32 { 0 }
//! #     pub fn give_ui_notification() {}
//! #     pub fn taskYIELD() {}
//! #     pub fn xPortIsInsideInterrupt() -> bool { false }
//! #     pub const PORT_MAX_DELAY: u32 = u32::MAX;
//! # }
//! use freertos::*;
//!
//! /// The UI task's platform: the tick count is the clock (1 kHz), a direct-to-task
//! /// notification wakes it.
//! #[derive(Clone, Copy)]
//! struct FreeRtos;
//!
//! impl Clock for FreeRtos {
//!     fn now(&self) -> Instant {
//!         Instant::from_millis(u64::from(xTaskGetTickCount()))
//!     }
//! }
//!
//! impl Platform for FreeRtos {
//!     fn wait(&mut self, deadline: Option<Instant>) {
//!         let ticks = match deadline {
//!             None => PORT_MAX_DELAY,
//!             Some(t) => {
//!                 let ms = t.saturating_duration_since(self.now()).as_millis();
//!                 if ms == 0 {
//!                     return;
//!                 }
//!                 u32::try_from(ms).unwrap_or(PORT_MAX_DELAY - 1)
//!             }
//!         };
//!         // A notification given since the last take is kept by the kernel: none is lost.
//!         ulTaskNotifyTake(true, ticks);
//!     }
//!     fn yield_now(&mut self) {
//!         taskYIELD();
//!     }
//!     fn notify() {
//!         give_ui_notification();
//!     }
//!     fn in_interrupt() -> bool {
//!         xPortIsInsideInterrupt()
//!     }
//! }
//!
//! // The UI task's entry point.
//! fn ui_task(display: impl twine_hal::DisplayDriver + 'static) -> ! {
//!     let mut platform = FreeRtos;
//!     let ui = Ui::builder(display)
//!         .runtime(Runtime::take().expect("runtime already taken")) // this task's runtime
//!         .platform(&platform)
//!         .buffers(BufferMode::alloc(BufferSpec::default()))
//!         .build(|_| label("Hello"));
//!     twine_view::run::blocking(ui, &mut platform)
//! }
//! ```
//!
//! The `rtos_notify` example of the `twine` crate runs this pattern on host threads.

use twine_core::{Duration, Instant};
use twine_engine::Wake;
use twine_hal::Platform;

use crate::Ui;

/// What a run loop is doing, reported to the callback of [`blocking_with`] (and of
/// `twine_embassy::run_with`) together with the `Ui`, so the application keeps access to the
/// UI, its engine and its display while the loop owns them: power management (backlight,
/// panel sleep, clocks), fault handling, watchdogs. The display driver is
/// `ui.engine_mut().driver_mut::<MyPanel>(ui.display())`.
///
/// One iteration reports `BeforeUpdate`, then — unless the update returned
/// [`Wake::Now`] — `Idle` before the loop sleeps and `Woken` after.
///
/// ```
/// use twine_view::run::LoopEvent;
/// use twine_view::prelude::*;
///
/// fn on_event(ui: &mut Ui, event: LoopEvent) {
///     match event {
///         LoopEvent::BeforeUpdate => {
///             let _faults = ui.take_faults(); // e.g. log them, feed the watchdog
///         }
///         LoopEvent::Idle(None) => { /* nothing scheduled: dim the backlight, stop clocks */ }
///         LoopEvent::Idle(Some(t)) => {
///             let _sleep = t.saturating_duration_since(ui.now()); // pick a sleep mode
///         }
///         LoopEvent::Inactive(_for) => { /* no input for `idle_timeout`: deep sleep */ }
///         LoopEvent::Woken => { /* restore what `Idle` changed */ }
///         _ => {}
///     }
/// }
/// # let _ = on_event;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum LoopEvent {
    /// The loop is about to update the UI (every iteration, also after [`Wake::Now`]). Changes
    /// made here — signals, widgets, the theme — are part of this update.
    BeforeUpdate,
    /// The update is done and the loop is about to sleep until the instant (on the `Ui`'s
    /// clock; `None`: until a wake-up). A signal written here wakes the `Ui` by itself (the
    /// bindings it changes defer to the next update and wake the `Ui`); other changes — the
    /// engine or display touched directly — are not drawn until the next wake-up: follow them
    /// with [`Ui::notify_input`] (or a [`UiWaker::wake`](twine_reactive::UiWaker::wake)), which
    /// makes the wait return at once.
    Idle(Option<Instant>),
    /// Like `Idle(None)` (nothing scheduled, the loop sleeps until a wake-up), and there has
    /// been no user input for this long — at least `EngineConfig::idle_timeout`
    /// ([`Wake::IdleFor`]): the moment to pick a deep-sleep mode or put the display to sleep
    /// ([`Ui::set_display_sleep`]; follow it with [`Ui::notify_input`] so it is applied before
    /// the sleep). Reported instead of `Idle(None)`, only with an `idle_timeout`.
    Inactive(Duration),
    /// The sleep ended: the deadline passed, the UI was woken, or (as platforms may) for no
    /// reason. The next event is `BeforeUpdate`.
    Woken,
}

/// Runs `ui` forever on `platform`: [`blocking_with`] without a callback.
///
/// Usable wherever the caller may block: bare metal (`CortexMPlatform`, `RiscvPlatform`), an
/// RTOS task (a [`Platform`] over the kernel's task notification), a host thread
/// (`StdPlatform`). See the [module documentation](self) for how each [`Wake`] is honoured.
/// Never returns; never panics (unless the platform or the application does); allocates
/// nothing per iteration.
///
/// ```no_run
/// use twine_hal::StdPlatform;
/// use twine_view::prelude::*;
///
/// fn main_loop(display: impl twine_hal::DisplayDriver + 'static) -> ! {
///     let mut platform = StdPlatform::new(); // or `CortexMPlatform::new(timer)`, an RTOS port, …
///     let ui = Ui::builder(display)
///         .runtime(Runtime::take().expect("runtime already taken"))
///         .platform(&platform)
///         .buffers(BufferMode::alloc(BufferSpec::default()))
///         .build(|_| label("Hello"));
///     twine_view::run::blocking(ui, &mut platform)
/// }
/// ```
pub fn blocking<P: Platform>(ui: Ui, platform: &mut P) -> ! {
    blocking_with(ui, platform, |_, _| {})
}

/// Runs `ui` forever on `platform`, reporting each step of the loop to `on_event` with the
/// `Ui` (see [`LoopEvent`]): update, then sleep with [`Platform::wait`] as the returned
/// [`Wake`] says — no deadline for [`Wake::Idle`] (no spinning), until `t` for
/// [`Wake::At(t)`](Wake::At), and no sleep for [`Wake::Now`] (after
/// [`Platform::yield_now`]).
///
/// Before the first update it installs [`P::notify`](Platform::notify) on the UI's waker
/// ([`UiWaker::set_notify`](twine_reactive::UiWaker::set_notify), replacing another notify
/// function), so channel messages, [`Ui::notify_input`] and input interrupts end a wait in
/// progress. See the [module documentation](self) for the details (no lost wake-ups,
/// starvation, the clock).
///
/// Never returns; never panics (unless the platform or `on_event` does); allocates nothing per
/// iteration.
///
/// ```no_run
/// use twine_hal::StdPlatform;
/// use twine_view::prelude::*;
/// use twine_view::run::{self, LoopEvent};
///
/// fn main_loop(display: impl twine_hal::DisplayDriver + 'static) -> ! {
///     let mut platform = StdPlatform::new();
///     let ui = Ui::builder(display)
///         .runtime(Runtime::take().expect("runtime already taken"))
///         .platform(&platform)
///         .buffers(BufferMode::alloc(BufferSpec::default()))
///         .build(|_| label("Hello"));
///     run::blocking_with(ui, &mut platform, |ui, event| match event {
///         LoopEvent::Idle(None) => { /* e.g. dim the backlight */ }
///         LoopEvent::Woken => { /* full brightness */ }
///         LoopEvent::BeforeUpdate if ui.take_faults().contains(FaultKind::FlushTimeout) => {
///             ui.recover_display(); // e.g. after re-initialising the panel
///         }
///         _ => {}
///     })
/// }
/// ```
pub fn blocking_with<P, F>(mut ui: Ui, platform: &mut P, mut on_event: F) -> !
where
    P: Platform,
    F: FnMut(&mut Ui, LoopEvent),
{
    ui.waker().set_notify(P::notify);
    twine_core::info!(target: "twine::run", "blocking run loop started");
    loop {
        on_event(&mut ui, LoopEvent::BeforeUpdate);
        let deadline = match ui.update() {
            Wake::Now => {
                platform.yield_now();
                continue;
            }
            Wake::At(t) => Some(t),
            Wake::Idle => None,
            Wake::IdleFor(inactive) => {
                on_event(&mut ui, LoopEvent::Inactive(inactive));
                platform.wait(None);
                on_event(&mut ui, LoopEvent::Woken);
                continue;
            }
        };
        on_event(&mut ui, LoopEvent::Idle(deadline));
        platform.wait(deadline);
        on_event(&mut ui, LoopEvent::Woken);
    }
}
