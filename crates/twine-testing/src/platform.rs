//! [`MockPlatform`]: a [`Platform`] and [`AsyncPlatform`] on simulated time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll, Waker};
use std::vec::Vec;

use twine_core::{Duration, Instant};
use twine_hal::{AsyncPlatform, Clock, Platform};

use crate::MockClock;

/// Notifications made through [`MockPlatform::notify`] (process-wide: `notify` is a plain
/// `fn()` and may be called from any thread).
static NOTIFIES: AtomicU32 = AtomicU32::new(0);

std::thread_local! {
    /// Whether this thread currently simulates interrupt context ([`MockPlatform::interrupt`]).
    static IN_INTERRUPT: Cell<bool> = const { Cell::new(false) };
}

/// What the clones of one `MockPlatform` share.
#[derive(Debug, Default)]
struct State {
    /// The deadline of every `wait`, in order.
    deadlines: RefCell<Vec<Option<Instant>>>,
    /// `NOTIFIES` when the last `wait` returned (or at creation).
    seen: Cell<u32>,
    /// Tasks waiting in `poll_wait_until`, woken when the time moves.
    timers: RefCell<Vec<Waker>>,
    /// `yield_now` calls.
    yields: Cell<usize>,
}

/// A [`Platform`] (blocking run loops) and [`AsyncPlatform`] (`AsyncUi`) on simulated time,
/// for tests: waits do not block, they move a [`MockClock`].
///
/// - [`wait`](Platform::wait) records its deadline ([`waits`](Self::waits),
///   [`deadlines`](Self::deadlines)). If a [`notify`](Platform::notify) happened since the
///   previous `wait` returned, it returns at once (time unchanged); otherwise it "sleeps"
///   until the deadline (the clock jumps there; `None` returns at once, as a wake-up would
///   end it).
/// - [`yield_now`](Platform::yield_now) is counted ([`yields`](Self::yields)); the time does
///   not move.
/// - [`notify`](Platform::notify) counts process-wide ([`notifications`](Self::notifications)):
///   it is a plain `fn()`, callable from any thread like a real platform's. Tests running in
///   parallel in one binary see each other's notifications (spurious early returns, allowed by
///   the [`Platform`] contract); compare counts before and after, or check `>=`.
/// - [`in_interrupt`](Platform::in_interrupt) is `true` inside [`interrupt`](Self::interrupt)
///   on the calling thread (simulated interrupt context).
/// - [`poll_wait_until`](AsyncPlatform::poll_wait_until) is ready once the clock reached the
///   deadline; [`advance`](Self::advance) and [`set`](Self::set) wake the waiting tasks.
///
/// Cheap to clone: clones share the clock and the records, so a test keeps one handle while
/// the `Ui` owns another.
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_hal::{Clock, Platform};
/// use twine_testing::MockPlatform;
///
/// let mut p = MockPlatform::new();
/// p.wait(Some(Instant::from_millis(16)));
/// assert_eq!(p.now(), Instant::from_millis(16)); // slept until the deadline
/// MockPlatform::notify();
/// p.wait(Some(Instant::from_millis(100)));
/// assert_eq!(p.now(), Instant::from_millis(16)); // woken at once
/// assert_eq!(p.deadlines(), [Some(Instant::from_millis(16)), Some(Instant::from_millis(100))]);
/// assert!(MockPlatform::interrupt(MockPlatform::in_interrupt));
/// assert!(!MockPlatform::in_interrupt());
/// ```
#[derive(Clone, Debug)]
pub struct MockPlatform {
    clock: MockClock,
    state: Rc<State>,
}

impl Default for MockPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPlatform {
    /// A platform on a new [`MockClock`] at `Instant` 0, with no waits recorded and the
    /// process-wide notifications made so far already seen (an earlier `notify` does not end
    /// its first `wait`). Never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::Clock;
    /// use twine_testing::MockPlatform;
    ///
    /// let p = MockPlatform::new();
    /// assert_eq!(p.now(), Instant::from_millis(0));
    /// assert_eq!(p.waits(), 0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(MockClock::new())
    }

    /// A platform on `clock` (shared with the caller). Moving the time through `clock` itself
    /// does not wake tasks waiting in `poll_wait_until`; use [`advance`](Self::advance).
    #[must_use]
    pub fn with_clock(clock: MockClock) -> Self {
        let state = State::default();
        state.seen.set(NOTIFIES.load(Ordering::Acquire));
        Self {
            clock,
            state: Rc::new(state),
        }
    }

    /// The clock (a clone shares the time).
    #[must_use]
    pub fn clock(&self) -> &MockClock {
        &self.clock
    }

    /// Moves the time forward by `d` (saturating) and wakes the tasks waiting for a deadline.
    /// Never panics (unless a woken task's waker does).
    ///
    /// ```
    /// use twine_core::{Duration, Instant};
    /// use twine_hal::Clock;
    /// use twine_testing::MockPlatform;
    ///
    /// let p = MockPlatform::new();
    /// p.advance(Duration::ms(40));
    /// assert_eq!(p.now(), Instant::from_millis(40));
    /// ```
    pub fn advance(&self, d: Duration) {
        self.clock.advance(d);
        self.wake_timers();
    }

    /// Sets the time (moving backwards is allowed but logged, see [`MockClock::set`]) and
    /// wakes the tasks waiting for a deadline. Never panics (unless a woken task's waker
    /// does).
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::Clock;
    /// use twine_testing::MockPlatform;
    ///
    /// let p = MockPlatform::new();
    /// p.set(Instant::from_millis(1_000));
    /// assert_eq!(p.now(), Instant::from_millis(1_000));
    /// ```
    pub fn set(&self, t: Instant) {
        self.clock.set(t);
        self.wake_timers();
    }

    /// Number of [`wait`](Platform::wait) calls so far (all clones). Never panics.
    ///
    /// ```
    /// use twine_hal::Platform;
    /// use twine_testing::MockPlatform;
    ///
    /// let mut p = MockPlatform::new();
    /// let ui_side = p.clone();
    /// p.wait(None);
    /// assert_eq!(ui_side.waits(), 1); // clones share the records
    /// ```
    #[must_use]
    pub fn waits(&self) -> usize {
        self.state.deadlines.borrow().len()
    }

    /// The deadline of every [`wait`](Platform::wait) so far, in order (all clones).
    /// Allocates the returned `Vec`; never panics.
    ///
    /// ```
    /// use twine_core::Instant;
    /// use twine_hal::Platform;
    /// use twine_testing::MockPlatform;
    ///
    /// let mut p = MockPlatform::new();
    /// p.wait(Some(Instant::from_millis(5)));
    /// p.wait(None);
    /// assert_eq!(p.deadlines(), [Some(Instant::from_millis(5)), None]);
    /// ```
    #[must_use]
    pub fn deadlines(&self) -> Vec<Option<Instant>> {
        self.state.deadlines.borrow().clone()
    }

    /// Number of [`yield_now`](Platform::yield_now) calls so far (all clones). Never panics.
    ///
    /// ```
    /// use twine_hal::Platform;
    /// use twine_testing::MockPlatform;
    /// let mut p = MockPlatform::new();
    /// p.yield_now();
    /// assert_eq!(p.yields(), 1);
    /// assert_eq!(p.waits(), 0);
    /// ```
    #[must_use]
    pub fn yields(&self) -> usize {
        self.state.yields.get()
    }

    /// Notifications so far, process-wide (see [`MockPlatform`]: tests running in parallel
    /// add to it, so compare with `>=`). Never panics.
    ///
    /// ```
    /// use twine_hal::Platform;
    /// use twine_testing::MockPlatform;
    ///
    /// let before = MockPlatform::notifications();
    /// MockPlatform::notify(); // e.g. what a channel send's notify function calls
    /// assert!(MockPlatform::notifications() > before);
    /// ```
    #[must_use]
    pub fn notifications() -> u32 {
        NOTIFIES.load(Ordering::Acquire)
    }

    /// Runs `f` as if in an interrupt handler: [`in_interrupt`](Platform::in_interrupt)
    /// returns `true` on this thread until `f` returns (also when it panics).
    ///
    /// # Panics
    ///
    /// Never by itself; a panic of `f` propagates after the interrupt state is restored.
    ///
    /// ```
    /// use twine_testing::MockPlatform;
    ///
    /// // What an ISR-safe API sees when called from an interrupt handler:
    /// let in_isr = MockPlatform::interrupt(MockPlatform::in_interrupt);
    /// assert!(in_isr);
    /// assert!(!MockPlatform::in_interrupt());
    /// ```
    pub fn interrupt<R>(f: impl FnOnce() -> R) -> R {
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                IN_INTERRUPT.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(IN_INTERRUPT.with(|c| c.replace(true)));
        f()
    }

    /// Whether the calling thread is inside [`interrupt`](Self::interrupt) (the
    /// [`Platform::in_interrupt`] of this platform, callable without naming the trait).
    /// Never panics.
    #[must_use]
    pub fn in_interrupt() -> bool {
        IN_INTERRUPT.with(Cell::get)
    }

    fn wake_timers(&self) {
        let timers = core::mem::take(&mut *self.state.timers.borrow_mut());
        for w in timers {
            w.wake();
        }
    }
}

impl Clock for MockPlatform {
    fn now(&self) -> Instant {
        self.clock.now()
    }
}

impl Platform for MockPlatform {
    fn wait(&mut self, deadline: Option<Instant>) {
        self.state.deadlines.borrow_mut().push(deadline);
        let notified = NOTIFIES.load(Ordering::Acquire);
        if notified != self.state.seen.replace(notified) {
            return;
        }
        if let Some(d) = deadline.filter(|&d| d > self.clock.now()) {
            self.set(d);
        }
    }

    fn yield_now(&mut self) {
        self.state.yields.set(self.state.yields.get() + 1);
    }

    fn notify() {
        NOTIFIES.fetch_add(1, Ordering::AcqRel);
    }

    fn in_interrupt() -> bool {
        MockPlatform::in_interrupt()
    }
}

impl AsyncPlatform for MockPlatform {
    fn poll_wait_until(&mut self, deadline: Instant, cx: &mut Context<'_>) -> Poll<()> {
        if self.clock.now() >= deadline {
            return Poll::Ready(());
        }
        let mut timers = self.state.timers.borrow_mut();
        if !timers.iter().any(|w| w.will_wake(cx.waker())) {
            timers.push(cx.waker().clone());
        }
        Poll::Pending
    }

    fn in_interrupt() -> bool {
        MockPlatform::in_interrupt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_timer_wakes_on_advance() {
        use std::sync::Arc;
        use std::task::Wake;

        struct Flag(std::sync::atomic::AtomicBool);
        impl Wake for Flag {
            fn wake(self: Arc<Self>) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let flag = Arc::new(Flag(std::sync::atomic::AtomicBool::new(false)));
        let waker = Waker::from(flag.clone());
        let mut cx = Context::from_waker(&waker);
        let mut p = MockPlatform::new();
        let deadline = Instant::from_millis(10);
        assert!(p.poll_wait_until(deadline, &mut cx).is_pending());
        p.advance(Duration::ms(5));
        assert!(flag.0.swap(false, Ordering::SeqCst)); // woken to re-check
        assert!(p.poll_wait_until(deadline, &mut cx).is_pending());
        p.advance(Duration::ms(5));
        assert!(p.poll_wait_until(deadline, &mut cx).is_ready());
    }
}
