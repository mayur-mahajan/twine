//! [`Outbox`]: commands from the UI to a task, a thread or an interrupt-driven driver.

use crate::channel::UiWaker;
use crate::queue::{Overflow, Queue};

/// A bounded, `const`-constructible, allocation-free FIFO **from the UI** to another execution
/// context: a heater task, a motor driver, a network stack. The UI sends with
/// [`try_send`](Self::try_send) (from a click handler, an effect); the consumer polls with
/// [`try_recv`](Self::try_recv) (a bare-metal super-loop, an interrupt handler, an RTOS task
/// woken by the [`waker`](Self::waker)'s notify function) or awaits
/// `recv` (feature `async`; any executor — it registers a
/// [`core::task::Waker`], nothing else).
///
/// A full outbox applies its [`Overflow`] policy ([`on_full`](Self::on_full); default
/// [`Overflow::DropNewest`]: `try_send` returns the command so the UI can show "busy").
/// Lost commands are counted ([`take_dropped`](Self::take_dropped)); what to do about them is
/// the application's decision (the outbox touches no runtime, so it raises no fault itself).
///
/// `Outbox<T, N>` is `Sync` whenever `T` is `Send` (`critical-section` + `portable-atomic`, no
/// compare-and-swap: works on thumbv6m). Sending and receiving allocate nothing.
///
/// ```
/// use twine_reactive::Outbox;
///
/// #[derive(Debug, PartialEq)]
/// enum Heater {
///     On,
///     Off,
/// }
/// static HEATER: Outbox<Heater, 4> = Outbox::new();
///
/// HEATER.try_send(Heater::On).unwrap(); // in the UI, e.g. an effect
/// // In the heater task (or its interrupt handler):
/// assert_eq!(HEATER.try_recv(), Some(Heater::On));
/// assert_eq!(HEATER.try_recv(), None);
/// # let _ = Heater::Off;
/// ```
pub struct Outbox<T, const N: usize> {
    queue: Queue<T, N>,
    waker: UiWaker,
}

impl<T, const N: usize> Default for Outbox<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> core::fmt::Debug for Outbox<T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Outbox")
            .field("len", &self.len())
            .field("capacity", &N)
            .field("on_full", &self.queue.overflow())
            .field("dropped", &self.queue.peek_dropped())
            .finish_non_exhaustive()
    }
}

impl<T, const N: usize> Outbox<T, N> {
    /// An empty outbox with room for `N` commands; a full outbox refuses new ones
    /// ([`Overflow::DropNewest`]; see [`on_full`](Self::on_full)). `const`, for `static`
    /// items; allocates nothing; never panics.
    ///
    /// ```
    /// use twine_reactive::{Outbox, Overflow};
    /// static TO_MOTOR: Outbox<u16, 4> = Outbox::new();
    /// assert!(TO_MOTOR.is_empty());
    /// assert_eq!(TO_MOTOR.overflow(), Overflow::DropNewest);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Outbox {
            queue: Queue::new(Overflow::DropNewest),
            waker: UiWaker::new(),
        }
    }

    /// The outbox with overflow policy `policy` (see [`Overflow`]); `const`.
    ///
    /// ```
    /// use twine_reactive::{Outbox, Overflow};
    /// // Only the newest set-points matter to the motor driver.
    /// static SETPOINT: Outbox<u16, 2> = Outbox::new().on_full(Overflow::DropOldest);
    /// assert_eq!(SETPOINT.overflow(), Overflow::DropOldest);
    /// ```
    #[must_use]
    pub const fn on_full(mut self, policy: Overflow) -> Self {
        self.queue.set_overflow(policy);
        self
    }

    /// The overflow policy (see [`on_full`](Self::on_full)). Never panics.
    ///
    /// ```
    /// use twine_reactive::{Outbox, Overflow};
    /// let log: Outbox<u8, 2> = Outbox::new().on_full(Overflow::DropOldest);
    /// assert_eq!(log.overflow(), Overflow::DropOldest);
    /// ```
    #[must_use]
    pub const fn overflow(&self) -> Overflow {
        self.queue.overflow()
    }

    /// Queues `v` and wakes the consumer (its flag, its registered task, its notify function:
    /// see [`waker`](Self::waker)). Callable from any context; never blocks, never allocates,
    /// never panics (unless the notify function or task waker does).
    ///
    /// # Errors
    ///
    /// `Err(v)` when the outbox is full and its policy is [`Overflow::DropNewest`] (the
    /// consumer is not woken). With [`Overflow::DropOldest`] the oldest command is evicted
    /// (dropped in the caller's context) and `try_send` succeeds. Either way the lost command
    /// is counted.
    ///
    /// ```
    /// let out: twine_reactive::Outbox<u8, 1> = twine_reactive::Outbox::new();
    /// assert_eq!(out.try_send(1), Ok(()));
    /// assert_eq!(out.try_send(2), Err(2));
    /// ```
    #[inline]
    pub fn try_send(&self, v: T) -> Result<(), T> {
        self.queue.push(v)?;
        self.waker.wake();
        Ok(())
    }

    /// Dequeues the oldest command (`None` when empty). Any context; allocation-free; never
    /// panics.
    ///
    /// ```
    /// let out: twine_reactive::Outbox<u8, 2> = twine_reactive::Outbox::new();
    /// out.try_send(3).unwrap();
    /// assert_eq!(out.try_recv(), Some(3));
    /// ```
    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.queue.pop()
    }

    /// Number of queued commands.
    ///
    /// ```
    /// let out: twine_reactive::Outbox<u8, 2> = twine_reactive::Outbox::new();
    /// assert_eq!(out.len(), 0);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether no command is queued.
    ///
    /// ```
    /// let out: twine_reactive::Outbox<u8, 2> = twine_reactive::Outbox::new();
    /// assert!(out.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns and resets the number of commands lost because the outbox was full (refused or
    /// evicted, see [`Overflow`]).
    ///
    /// ```
    /// let out: twine_reactive::Outbox<u8, 1> = twine_reactive::Outbox::new();
    /// out.try_send(1).unwrap();
    /// let _ = out.try_send(2);
    /// assert_eq!(out.take_dropped(), 1);
    /// ```
    pub fn take_dropped(&self) -> u32 {
        self.queue.take_dropped()
    }

    /// The consumer's waker, signalled by every successful [`try_send`](Self::try_send). The
    /// consumer picks how it sleeps: poll the flag ([`UiWaker::take`]) in a super-loop, set a
    /// notify function ([`UiWaker::set_notify`]: an RTOS task notification, an RTIC `pend`,
    /// a `Platform::notify`), or await `recv` (feature `async`), which registers its task.
    ///
    /// ```
    /// use core::sync::atomic::{AtomicBool, Ordering};
    /// use twine_reactive::Outbox;
    ///
    /// static CMD: Outbox<u8, 4> = Outbox::new();
    /// static TASK_READY: AtomicBool = AtomicBool::new(false); // e.g. `xTaskNotifyGive`
    /// CMD.waker().set_notify(|| TASK_READY.store(true, Ordering::Release));
    /// CMD.try_send(1).unwrap();
    /// assert!(TASK_READY.load(Ordering::Acquire));
    /// ```
    #[must_use]
    pub fn waker(&self) -> &UiWaker {
        &self.waker
    }

    /// Waits for the next command: a future that resolves to the oldest queued command,
    /// registering the polling task's [`Waker`](core::task::Waker) in the
    /// [`waker`](Self::waker) while the outbox is empty. Executor-agnostic (embassy, RTIC,
    /// any `block_on`); allocation-free (the registration clones the executor's waker, which
    /// is allocation-free in `no_std` executors). Cancel-safe: dropping the future loses no
    /// command.
    ///
    /// One consumer task at a time: a second task awaiting the same outbox replaces the first
    /// one's registration (both still receive commands when polled).
    ///
    /// ```
    /// # use core::pin::pin;
    /// # use core::task::{Context, Poll, Waker};
    /// use twine_reactive::Outbox;
    /// static CMD: Outbox<u8, 4> = Outbox::new();
    ///
    /// async fn consumer() -> u8 {
    ///     CMD.recv().await
    /// }
    ///
    /// CMD.try_send(9).unwrap();
    /// let mut fut = pin!(consumer());
    /// let mut cx = Context::from_waker(Waker::noop());
    /// assert_eq!(fut.as_mut().poll(&mut cx), Poll::Ready(9));
    /// ```
    #[cfg(feature = "async")]
    pub fn recv(&self) -> Recv<'_, T, N> {
        Recv { outbox: self }
    }
}

/// The future of [`Outbox::recv`] (feature `async`).
#[cfg(feature = "async")]
#[must_use = "futures do nothing unless awaited"]
pub struct Recv<'a, T, const N: usize> {
    outbox: &'a Outbox<T, N>,
}

#[cfg(feature = "async")]
impl<T, const N: usize> core::fmt::Debug for Recv<'_, T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Recv").finish_non_exhaustive()
    }
}

#[cfg(feature = "async")]
impl<T, const N: usize> core::future::Future for Recv<'_, T, N> {
    type Output = T;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<T> {
        let outbox = self.outbox;
        if let Some(v) = outbox.try_recv() {
            return core::task::Poll::Ready(v);
        }
        outbox.waker.register(cx.waker());
        // A send between the first check and the registration woke no task: check again.
        match outbox.try_recv() {
            Some(v) => core::task::Poll::Ready(v),
            None => core::task::Poll::Pending,
        }
    }
}
