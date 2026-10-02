//! [`RiscvPlatform`] (feature `platform-riscv`).

use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::Instant;

use super::Platform;
use crate::Clock;

/// Set by [`RiscvPlatform::notify`], consumed by [`RiscvPlatform::wait`] (load and store
/// only: no compare-and-swap, so it works on `riscv32imc`).
static PENDING: AtomicBool = AtomicBool::new(false);

/// [`Platform`] for single-hart RISC-V microcontrollers (feature `platform-riscv`, e.g.
/// ESP32-C3/C6, GD32VF103, CH32V), over the application's clock `C`.
///
/// - [`wait`](Platform::wait): `WFI` (wait for interrupt), unless the deadline has passed or
///   a [`notify`](Platform::notify) is pending. The pending flag is checked and `WFI` executed
///   inside one critical section: an interrupt arriving in between stays pending, and `WFI`
///   returns for a pending interrupt even while interrupts are masked, so no wake-up is lost.
///   The handler runs when the critical section ends. Use it only where the
///   `critical-section` implementation just masks interrupts (single hart): an implementation
///   that also takes a lock shared with another hart would be held while sleeping.
/// - The deadline is honoured at the granularity of the interrupts the application already
///   has: `WFI` arms no timer (the clock's tick, e.g. the machine timer, wakes the hart).
/// - [`in_interrupt`](Platform::in_interrupt): `false`. RISC-V has no architectural
///   "handler active" indicator: `mcause` keeps the cause of the *last* trap after `mret`,
///   and `mstatus` looks the same inside a handler and inside a critical section, so any
///   reading would report false positives in correct code. Platforms with an interrupt
///   controller that tracks the active level (e.g. CLIC's `mintstatus`) implement
///   [`Platform`] themselves.
///
/// The pending flag is process-wide: one UI loop per hart (several would share wake-ups,
/// which costs spurious returns, never lost ones).
///
/// ```no_run
/// use twine_core::Instant;
/// use twine_hal::{Clock, Platform, RiscvPlatform};
///
/// #[derive(Clone, Copy)]
/// struct MTime; // e.g. the machine timer
/// impl Clock for MTime {
///     fn now(&self) -> Instant {
///         Instant::from_micros(0) // read `mtime` here
///     }
/// }
///
/// let mut platform = RiscvPlatform::new(MTime);
/// platform.wait(None); // until an interrupt or `RiscvPlatform::<MTime>::notify()`
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct RiscvPlatform<C> {
    clock: C,
}

impl<C: Clock> RiscvPlatform<C> {
    /// The platform over `clock` (the time source of the deadlines). `const`, so it can
    /// initialise a `static`; touches no hardware; never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::{Clock, RiscvPlatform};
    ///
    /// #[derive(Clone, Copy)]
    /// struct MTime; // e.g. the machine timer
    /// impl Clock for MTime {
    ///     fn now(&self) -> Instant {
    ///         Instant::from_millis(42) // read the counter here
    ///     }
    /// }
    ///
    /// static PLATFORM: RiscvPlatform<MTime> = RiscvPlatform::new(MTime);
    /// assert_eq!(PLATFORM.now(), Instant::from_millis(42));
    /// ```
    pub const fn new(clock: C) -> Self {
        Self { clock }
    }

    /// The clock. Never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::{Clock, RiscvPlatform};
    ///
    /// #[derive(Clone, Copy)]
    /// struct MTime; // e.g. the machine timer
    /// impl Clock for MTime {
    ///     fn now(&self) -> Instant {
    ///         Instant::from_millis(42) // read the counter here
    ///     }
    /// }
    ///
    /// let platform = RiscvPlatform::new(MTime);
    /// assert_eq!(platform.clock().now(), Instant::from_millis(42));
    /// ```
    pub fn clock(&self) -> &C {
        &self.clock
    }
}

impl<C: Clock> Clock for RiscvPlatform<C> {
    #[inline]
    fn now(&self) -> Instant {
        self.clock.now()
    }
}

impl<C: Clock> Platform for RiscvPlatform<C> {
    fn wait(&mut self, deadline: Option<Instant>) {
        if deadline.is_some_and(|d| self.clock.now() >= d) {
            return;
        }
        critical_section::with(|_| {
            let pending = PENDING.load(Ordering::Acquire);
            PENDING.store(false, Ordering::Release);
            if !pending {
                riscv::asm::wfi();
            }
        });
    }

    fn notify() {
        PENDING.store(true, Ordering::Release);
    }
}
