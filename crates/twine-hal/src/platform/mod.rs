//! Platform services: [`Platform`] (wait for a deadline or a wake-up, blocking) and
//! `AsyncPlatform` (the same for async runtimes, feature `async`), plus opt-in
//! implementations behind cargo features.
//!
//! Twine never owns the execution model: the UI is a step function (`Ui::update` returns
//! when to run again) plus wake-up signalling (`UiWaker`). A run loop needs three platform
//! services, all supplied by the application:
//!
//! | Service | Item | Bare metal | RTOS | Hosted OS |
//! |---------|------|------------|------|-----------|
//! | time | [`Clock`] (the supertrait) | a hardware timer | the kernel tick | `std::time` |
//! | sleep until a deadline or a wake-up | [`Platform::wait`] | `WFE` / `WFI` | task notification with timeout | condition variable with timeout |
//! | wake the sleeping loop | [`Platform::notify`], installed with `UiWaker::set_notify` | `SEV` / a flag | give the notification | notify the condition variable |
//! | "am I in an interrupt?" | [`Platform::in_interrupt`] | an interrupt-status register | the kernel's ISR query | `false` |
//!
//! **One time source.** A platform *is* the clock of the loop it runs: the `Ui` is built with
//! a clone of it (`UiBuilder::platform(&platform)`, which also installs `notify` on the `Ui`'s
//! waker and registers `in_interrupt`), and the loop waits with the platform itself, so the
//! deadlines the `Ui` returns and the time the platform waits for are on the same clock.
//! Clones of every platform here read the same time base.
//!
//! ## Implementations (opt-in features, none by default)
//!
//! | Feature | Type | `wait` | `notify` | `in_interrupt` |
//! |---------|------|--------|----------|----------------|
//! | `platform-cortex-m` | `CortexMPlatform` | `WFE` | `SEV` | `SCB::vect_active() != ThreadMode` |
//! | `platform-riscv` | `RiscvPlatform` | `WFI` inside a critical section | a pending flag | `false` (no architectural indicator) |
//! | `platform-std` | `StdPlatform` | `Condvar::wait_timeout` | `Condvar::notify_all` | `false` |
//! | `platform-embassy` | `EmbassyPlatform` (an `AsyncPlatform`) | `embassy_time::Timer` | — (task wakers) | `false` |
//!
//! An RTOS port implements [`Platform`] in a few lines (see its example; `twine::run`
//! documents the mapping for `FreeRTOS`, Zephyr and `ThreadX`). The blocking run loop over any
//! `Platform` is `twine::run::blocking`.

#[cfg(feature = "platform-cortex-m")]
mod cortex_m;
#[cfg(feature = "platform-embassy")]
mod embassy;
#[cfg(feature = "platform-std")]
mod hosted;
#[cfg(feature = "platform-riscv")]
mod riscv;

#[cfg(feature = "async")]
use core::future::Future;
#[cfg(feature = "async")]
use core::pin::Pin;
#[cfg(feature = "async")]
use core::task::{Context, Poll};

#[cfg(feature = "platform-cortex-m")]
pub use self::cortex_m::CortexMPlatform;
#[cfg(feature = "platform-embassy")]
pub use self::embassy::EmbassyPlatform;
#[cfg(feature = "platform-std")]
pub use self::hosted::StdPlatform;
#[cfg(feature = "platform-riscv")]
pub use self::riscv::RiscvPlatform;
use crate::Clock;
use twine_core::Instant;

/// Sleeping and waking for a blocking run loop (bare metal, an RTOS task, a host thread),
/// on top of the platform's [`Clock`].
///
/// A run loop steps the UI and waits as it says (`twine::run::blocking` is that loop):
///
/// | `Ui::update` returns | the loop calls |
/// |----------------------|----------------|
/// | `Wake::Now` | [`yield_now`](Self::yield_now), then steps again |
/// | `Wake::At(t)` | [`wait(Some(t))`](Self::wait) |
/// | `Wake::Idle` | [`wait(None)`](Self::wait) |
///
/// **Contract.** [`wait`](Self::wait) returns at `deadline` at the latest (`None`: no
/// deadline), as soon as possible after a [`notify`](Self::notify) — and **never sleeps
/// through a `notify` made after the previous `wait` returned**, so a wake-up between the
/// loop's last check and the call to `wait` is not lost — or earlier for any other reason
/// (an unrelated interrupt): callers loop, so early returns are harmless. `notify` is a plain
/// `fn()`, installed on the UI's waker with `UiWaker::set_notify` (`UiBuilder::platform` does
/// it), and is called from wherever the UI is woken: interrupt handlers, other tasks, threads
/// or cores.
///
/// The trait is object safe (`notify` and `in_interrupt` are associated functions, used as
/// `fn` pointers, so they require `Self: Sized`).
///
/// ```
/// use core::sync::atomic::{AtomicBool, Ordering};
/// use twine_core::{Duration, Instant};
/// use twine_hal::{Clock, Platform};
///
/// /// An RTOS port: the kernel's tick is the clock, a binary semaphore wakes the UI task.
/// #[derive(Clone)]
/// struct Rtos;
/// static GIVEN: AtomicBool = AtomicBool::new(false); // stands in for the semaphore
/// static TICKS_MS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
///
/// impl Clock for Rtos {
///     fn now(&self) -> Instant {
///         Instant::from_millis(TICKS_MS.load(Ordering::Relaxed)) // e.g. `xTaskGetTickCount()`
///     }
/// }
/// impl Platform for Rtos {
///     fn wait(&mut self, deadline: Option<Instant>) {
///         // e.g. `xSemaphoreTake(UI_SEM, ticks_until(deadline))`; simulated here.
///         if !GIVEN.swap(false, Ordering::AcqRel) {
///             if let Some(d) = deadline {
///                 TICKS_MS.store(d.as_millis().max(TICKS_MS.load(Ordering::Relaxed)), Ordering::Relaxed);
///             }
///         }
///     }
///     fn yield_now(&mut self) {
///         // e.g. `taskYIELD()`: other ready tasks of this priority run first.
///     }
///     fn notify() {
///         GIVEN.store(true, Ordering::Release); // e.g. `xSemaphoreGiveFromISR(UI_SEM, ..)`
///     }
/// }
///
/// let mut p = Rtos;
/// p.wait(Some(Instant::from_millis(20))); // nothing pending: sleeps until the deadline
/// assert_eq!(p.now(), Instant::from_millis(20));
/// Rtos::notify(); // a wake-up before the next wait is not lost
/// p.wait(None);
/// assert_eq!(p.now(), Instant::from_millis(20)); // returned at once
/// assert!(!Rtos::in_interrupt()); // the default
/// ```
pub trait Platform: Clock {
    /// Blocks (or yields the CPU) until `deadline` (on this platform's [`Clock`]; `None`: no
    /// deadline) or a [`notify`](Self::notify), whichever comes first; may return earlier
    /// (see the contract on [`Platform`]). Returns at once when `deadline` has passed or a
    /// `notify` happened since the previous `wait` returned. Never panics.
    fn wait(&mut self, deadline: Option<Instant>);

    /// Lets other work of equal priority run before the loop steps the UI again: called by run
    /// loops (`twine::run::blocking`) between two steps when the UI has more work at once
    /// (`Wake::Now`, e.g. the rest of a frame rendered in budgeted chunks). Unlike
    /// [`wait`](Self::wait) it never sleeps: it only gives up the rest of the time slice.
    ///
    /// Default: nothing (bare metal has no other task to yield to). An RTOS port calls its
    /// kernel's yield (`taskYIELD()`, `k_yield()`, `tx_thread_relinquish()`); `StdPlatform`
    /// calls `std::thread::yield_now`. Never panics.
    #[inline]
    fn yield_now(&mut self) {}

    /// Wakes a [`wait`](Self::wait) in progress, or makes the next one return at once.
    /// Interrupt-safe, non-blocking and allocation-free: called from the context that wakes
    /// the UI (interrupt handler, task, thread, core) through `UiWaker::set_notify`.
    fn notify()
    where
        Self: Sized;

    /// Whether the caller runs in interrupt (exception) context. Default: `false`.
    ///
    /// Used by diagnostics (registered as the reactive runtime's interrupt probe by
    /// `UiBuilder::platform`); must be cheap, side-effect free and callable from any context.
    #[must_use]
    fn in_interrupt() -> bool
    where
        Self: Sized,
    {
        false
    }
}

/// Waiting for a deadline in an async runtime (feature `async`), on top of the platform's
/// [`Clock`]: the time source and timer of the async UI (`AsyncUi`), which bounds every
/// display flush with `EngineConfig::flush_timeout` by racing the flush against
/// [`poll_wait_until`](Self::poll_wait_until).
///
/// Executor-agnostic: the timer is polled like a future with the task's [`Context`], so it
/// works under embassy, RTIC's async tasks or any other executor that has a timer. Wake-ups
/// of the UI itself go through the `UiWaker`'s task waker, so there is no `notify`. The trait
/// is object safe (`AsyncUi` keeps it as `Box<dyn AsyncPlatform>`).
///
/// ```
/// use core::cell::Cell;
/// use core::future::Future;
/// use core::task::{Context, Poll, Waker};
/// use twine_core::Instant;
/// use twine_hal::{AsyncPlatform, Clock};
///
/// /// A clock moved by hand whose timer completes once the time has come.
/// struct Manual(Cell<Instant>);
/// impl Clock for Manual {
///     fn now(&self) -> Instant {
///         self.0.get()
///     }
/// }
/// impl AsyncPlatform for Manual {
///     fn poll_wait_until(&mut self, deadline: Instant, _cx: &mut Context<'_>) -> Poll<()> {
///         // A real timer also arranges for `_cx.waker()` to be woken at `deadline`.
///         if self.now() >= deadline { Poll::Ready(()) } else { Poll::Pending }
///     }
/// }
///
/// let mut p = Manual(Cell::new(Instant::from_millis(0)));
/// let mut cx = Context::from_waker(Waker::noop());
/// let deadline = Instant::from_millis(5);
/// assert!(p.poll_wait_until(deadline, &mut cx).is_pending());
/// p.0.set(deadline);
/// assert!(core::pin::pin!(p.wait_until(deadline)).poll(&mut cx).is_ready());
/// ```
#[cfg(feature = "async")]
pub trait AsyncPlatform: Clock {
    /// `Ready` once `deadline` (on this platform's [`Clock`]) has passed; otherwise arranges
    /// for `cx.waker()` to be woken at `deadline` and returns `Pending`. May be polled any
    /// number of times, with any deadline (each poll stands alone; no state is kept between
    /// polls). Never panics.
    fn poll_wait_until(&mut self, deadline: Instant, cx: &mut Context<'_>) -> Poll<()>;

    /// A future that completes at `deadline` (a [`poll_wait_until`](Self::poll_wait_until)
    /// loop).
    fn wait_until(&mut self, deadline: Instant) -> WaitUntil<'_, Self>
    where
        Self: Sized,
    {
        WaitUntil {
            platform: self,
            deadline,
        }
    }

    /// Whether the caller runs in interrupt context (see [`Platform::in_interrupt`]).
    /// Default: `false`.
    #[must_use]
    fn in_interrupt() -> bool
    where
        Self: Sized,
    {
        false
    }
}

/// The future of [`AsyncPlatform::wait_until`]: completes at its deadline.
#[cfg(feature = "async")]
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct WaitUntil<'a, P: AsyncPlatform + ?Sized> {
    platform: &'a mut P,
    deadline: Instant,
}

#[cfg(feature = "async")]
impl<P: AsyncPlatform + ?Sized> Future for WaitUntil<'_, P> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        this.platform.poll_wait_until(this.deadline, cx)
    }
}
