//! [`Overflow`] and the bounded FIFO shared by [`Channel`](crate::Channel) and
//! [`Outbox`](crate::Outbox).

use core::cell::RefCell;

use critical_section::Mutex;

/// What a full [`Channel`](crate::Channel) or [`Outbox`](crate::Outbox) does with a value sent
/// to it. Either way the lost value is counted (`take_dropped`), and for a `Channel` with an
/// `on_message` handler it is reported as a
/// [`FaultKind::ChannelOverflow`](crate::FaultKind::ChannelOverflow) fault at the next drain.
///
/// Chosen once, in the `const` initialiser (`Channel::new().on_full(..)`), so a `static` keeps
/// its policy in flash. The policy is consulted only when the queue is full: the send path of a
/// queue with room is the same for both.
///
/// ```
/// use twine_reactive::{Channel, Overflow};
///
/// // Telemetry: keep the newest readings, lose the oldest.
/// static SAMPLES: Channel<u16, 2> = Channel::new().on_full(Overflow::DropOldest);
/// SAMPLES.try_send(1).unwrap();
/// SAMPLES.try_send(2).unwrap();
/// SAMPLES.try_send(3).unwrap(); // evicts 1
/// assert_eq!(SAMPLES.take_dropped(), 1);
/// assert_eq!(SAMPLES.try_recv(), Some(2));
/// assert_eq!(SAMPLES.try_recv(), Some(3));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Overflow {
    /// Refuse the new value: `try_send` returns it as `Err` (the default). Right for commands
    /// and events where the queued ones must be handled in order and the sender can react.
    #[default]
    DropNewest,
    /// Evict the oldest queued value and queue the new one: `try_send` always succeeds. Right for telemetry, where the newest data matters most.
    DropOldest,
}

/// What a push did (decided inside the critical section).
enum Pushed<T> {
    /// Queued.
    Queued,
    /// Queued after evicting the oldest value (dropped by the caller, outside the critical
    /// section).
    Evicted(T),
    /// Not queued.
    Refused(T),
}

/// The state behind the critical section: the values and the number lost to [`Overflow`].
struct State<T, const N: usize> {
    deque: heapless::Deque<T, N>,
    /// Values lost since the last `take_dropped` (saturating).
    dropped: u32,
}

/// A bounded, allocation-free FIFO usable from any context: a `heapless::Deque` and a lost-value
/// counter in one `critical_section::Mutex`. No atomics: every operation is one critical
/// section, so the counter costs nothing extra on chips without atomic read-modify-write
/// (thumbv6m) and is exact under any interleaving.
pub(crate) struct Queue<T, const N: usize> {
    state: Mutex<RefCell<State<T, N>>>,
    overflow: Overflow,
}

impl<T, const N: usize> Queue<T, N> {
    pub(crate) const fn new(overflow: Overflow) -> Self {
        Queue {
            state: Mutex::new(RefCell::new(State {
                deque: heapless::Deque::new(),
                dropped: 0,
            })),
            overflow,
        }
    }

    pub(crate) const fn overflow(&self) -> Overflow {
        self.overflow
    }

    pub(crate) const fn set_overflow(&mut self, overflow: Overflow) {
        self.overflow = overflow;
    }

    /// Queues `v`: `Ok(())` when `v` was queued (an evicted value is dropped here, after the
    /// critical section), `Err(v)` when it was refused. Counts every lost value.
    #[inline]
    pub(crate) fn push(&self, v: T) -> Result<(), T> {
        let pushed = critical_section::with(|cs| {
            let mut st = self.state.borrow_ref_mut(cs);
            match st.deque.push_back(v) {
                Ok(()) => Pushed::Queued,
                Err(v) => overflow(&mut st, v, self.overflow),
            }
        });
        match pushed {
            Pushed::Queued => Ok(()),
            Pushed::Evicted(old) => {
                drop(old);
                Ok(())
            }
            Pushed::Refused(v) => Err(v),
        }
    }

    #[inline]
    pub(crate) fn pop(&self) -> Option<T> {
        critical_section::with(|cs| self.state.borrow_ref_mut(cs).deque.pop_front())
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        critical_section::with(|cs| self.state.borrow_ref(cs).deque.len())
    }

    #[inline]
    pub(crate) fn take_dropped(&self) -> u32 {
        critical_section::with(|cs| core::mem::take(&mut self.state.borrow_ref_mut(cs).dropped))
    }

    pub(crate) fn peek_dropped(&self) -> u32 {
        critical_section::with(|cs| self.state.borrow_ref(cs).dropped)
    }
}

/// The full-queue path of [`Queue::push`] (inside its critical section): applies `policy` and
/// counts the lost value.
#[cold]
#[inline(never)]
fn overflow<T, const N: usize>(st: &mut State<T, N>, v: T, policy: Overflow) -> Pushed<T> {
    st.dropped = st.dropped.saturating_add(1);
    match policy {
        Overflow::DropNewest => Pushed::Refused(v),
        Overflow::DropOldest => {
            // `heapless` rejects a capacity of 0 at compile time, so a full queue has an oldest
            // value and room after removing it; the fallbacks only avoid a panic path.
            let Some(old) = st.deque.pop_front() else {
                return Pushed::Refused(v);
            };
            match st.deque.push_back(v) {
                Ok(()) => Pushed::Evicted(old),
                Err(v) => {
                    let _ = st.deque.push_front(old);
                    Pushed::Refused(v)
                }
            }
        }
    }
}
