//! [`CortexMPlatform`] (feature `platform-cortex-m`).

use cortex_m::peripheral::SCB;
use cortex_m::peripheral::scb::VectActive;
use twine_core::Instant;

use super::Platform;
use crate::Clock;

/// [`Platform`] for Arm Cortex-M (feature `platform-cortex-m`, any core from M0 to M85,
/// single- or multi-core), over the application's clock `C`.
///
/// - [`wait`](Platform::wait): `WFE` (wait for event), unless the deadline has passed. The
///   core sleeps until an event or an interrupt; a [`notify`](Platform::notify) (`SEV`) made
///   at any time after the previous `WFE` — even in an interrupt handler that ran just before
///   this `WFE` — leaves the event register set, so the `WFE` returns at once: no wake-up is
///   lost, without a critical section (a multi-core critical section such as RP2040's
///   spinlock is never held while sleeping, and `SEV` also wakes the other core).
/// - The deadline is honoured at the granularity of the interrupts the application already
///   has: `WFE` arms no timer. The clock's own tick (e.g. `SysTick`, or the time driver's
///   alarm) wakes the core, and the run loop waits again until the deadline; for a tickless
///   sleep, implement [`Platform`] with the timer's compare interrupt.
/// - [`in_interrupt`](Platform::in_interrupt): `SCB::vect_active() != ThreadMode` (the ICSR
///   `VECTACTIVE` field), i.e. any exception or interrupt handler is active.
///
/// Clones share the clock `C` (a `Copy` hardware-timer clock is typical), so the `Ui` built
/// with one reads the same time as the loop waiting with another.
///
/// ```no_run
/// use twine_core::Instant;
/// use twine_hal::{Clock, CortexMPlatform, Platform};
///
/// #[derive(Clone, Copy)]
/// struct Timer; // e.g. a 1 MHz hardware timer
/// impl Clock for Timer {
///     fn now(&self) -> Instant {
///         Instant::from_micros(0) // read the counter here
///     }
/// }
///
/// let mut platform = CortexMPlatform::new(Timer);
/// // `Ui::builder(display).runtime(rt).platform(&platform)` installs `CortexMPlatform::<Timer>::notify`
/// // (SEV) on the UI's waker; the loop then sleeps with:
/// platform.wait(Some(Instant::from_millis(16)));
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct CortexMPlatform<C> {
    clock: C,
}

impl<C: Clock> CortexMPlatform<C> {
    /// The platform over `clock` (the time source of the deadlines). `const`, so it can
    /// initialise a `static`; touches no hardware; never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::{Clock, CortexMPlatform};
    ///
    /// #[derive(Clone, Copy)]
    /// struct Timer; // e.g. a 1 MHz hardware timer
    /// impl Clock for Timer {
    ///     fn now(&self) -> Instant {
    ///         Instant::from_millis(42) // read the counter here
    ///     }
    /// }
    ///
    /// static PLATFORM: CortexMPlatform<Timer> = CortexMPlatform::new(Timer);
    /// assert_eq!(PLATFORM.now(), Instant::from_millis(42));
    /// ```
    pub const fn new(clock: C) -> Self {
        Self { clock }
    }

    /// The clock. Never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::{Clock, CortexMPlatform};
    ///
    /// #[derive(Clone, Copy)]
    /// struct Timer; // e.g. a 1 MHz hardware timer
    /// impl Clock for Timer {
    ///     fn now(&self) -> Instant {
    ///         Instant::from_millis(42) // read the counter here
    ///     }
    /// }
    ///
    /// let platform = CortexMPlatform::new(Timer);
    /// assert_eq!(platform.clock().now(), Instant::from_millis(42));
    /// ```
    pub fn clock(&self) -> &C {
        &self.clock
    }
}

impl<C: Clock> Clock for CortexMPlatform<C> {
    #[inline]
    fn now(&self) -> Instant {
        self.clock.now()
    }
}

impl<C: Clock> Platform for CortexMPlatform<C> {
    fn wait(&mut self, deadline: Option<Instant>) {
        if deadline.is_some_and(|d| self.clock.now() >= d) {
            return;
        }
        cortex_m::asm::wfe();
    }

    fn notify() {
        cortex_m::asm::sev();
    }

    fn in_interrupt() -> bool {
        SCB::vect_active() != VectActive::ThreadMode
    }
}
