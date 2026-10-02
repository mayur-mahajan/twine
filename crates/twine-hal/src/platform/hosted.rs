//! [`StdPlatform`] (feature `platform-std`).

extern crate std;

use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

use twine_core::Instant;

use super::Platform;
use crate::Clock;

/// Notifications so far (wrapping): a waiter returns when it changed since it last looked.
static EVENTS: Mutex<u64> = Mutex::new(0);
/// Signalled by every notification.
static CHANGED: Condvar = Condvar::new();
/// Time zero of every `StdPlatform` clock (the first use).
static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();

fn events() -> MutexGuard<'static, u64> {
    EVENTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// [`Platform`] for hosted operating systems with `std` (feature `platform-std`: Linux, QNX,
/// macOS, Windows, the simulator, tests): a UI loop on a thread.
///
/// - Clock: `std::time::Instant` (monotonic), in microseconds since the first use of any
///   `StdPlatform` in the process, so every instance and clone reads the same time base.
/// - [`wait`](Platform::wait): a process-wide condition variable with timeout. Each platform
///   remembers the notifications it has seen; `wait` returns at once if one happened since
///   its previous `wait` returned, so a wake-up between the loop's check and `wait` is never
///   lost.
/// - [`notify`](Platform::notify): bumps the notification count and wakes every waiter.
///   Callable from any thread (not from a Unix signal handler: it takes a mutex). With
///   several UI threads in one process, a notify for one wakes all of them once (a spurious
///   return, harmless by the [`Platform`] contract).
/// - [`yield_now`](Platform::yield_now): `std::thread::yield_now`.
/// - [`in_interrupt`](Platform::in_interrupt): `false`.
///
/// ```
/// use twine_core::Duration;
/// use twine_hal::{Clock, Platform, StdPlatform};
///
/// let mut p = StdPlatform::new();
/// let t0 = p.now();
/// p.wait(Some(t0 + Duration::ms(5))); // sleeps until the deadline (or a notify)
/// assert!(p.now() >= t0 + Duration::ms(5));
///
/// StdPlatform::notify(); // e.g. `UiWaker::wake` from another thread
/// p.wait(None); // returns at once: the notify came after the previous wait
/// ```
#[derive(Clone, Debug)]
pub struct StdPlatform {
    /// The notification count when this platform last returned from `wait` (or was created).
    seen: u64,
}

impl Default for StdPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl StdPlatform {
    /// A platform for the calling thread's UI loop. Notifications made before this call do
    /// not count for its first `wait`. Never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::{Clock, StdPlatform};
    ///
    /// let p = StdPlatform::new();
    /// assert!(p.now() >= Instant::from_millis(0)); // microseconds since the first platform
    /// ```
    #[must_use]
    pub fn new() -> Self {
        let _ = EPOCH.get_or_init(std::time::Instant::now);
        Self { seen: *events() }
    }
}

impl Clock for StdPlatform {
    fn now(&self) -> Instant {
        let micros = EPOCH.get_or_init(std::time::Instant::now).elapsed().as_micros();
        Instant::from_micros(u64::try_from(micros).unwrap_or(u64::MAX))
    }
}

impl Platform for StdPlatform {
    fn wait(&mut self, deadline: Option<Instant>) {
        let mut count = events();
        loop {
            if *count != self.seen {
                self.seen = *count;
                return;
            }
            count = match deadline {
                None => CHANGED.wait(count).unwrap_or_else(PoisonError::into_inner),
                Some(d) => {
                    let now = self.now();
                    if now >= d {
                        return;
                    }
                    let left = std::time::Duration::from_micros(d.saturating_duration_since(now).as_micros());
                    match CHANGED.wait_timeout(count, left) {
                        Ok((g, _)) => g,
                        Err(e) => e.into_inner().0,
                    }
                }
            };
        }
    }

    fn yield_now(&mut self) {
        std::thread::yield_now();
    }

    fn notify() {
        let mut count = events();
        *count = count.wrapping_add(1);
        drop(count);
        CHANGED.notify_all();
    }
}
