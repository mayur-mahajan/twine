//! [`Latest`]: an interrupt-safe "current value" cell read by the UI.

use core::cell::RefCell;

use critical_section::Mutex;

use crate::channel::{Source, UiWaker};

/// The value and how many times it was set (wrapping).
struct Slot<T> {
    value: T,
    version: u32,
}

/// A latest-value cell from interrupts, tasks, threads or other cores **to the UI**: a
/// sensor reading, a connection state, a battery level — data where only the newest value
/// matters.
///
/// [`set`](Self::set) overwrites the value and wakes the UI; there is no queue, so nothing is
/// ever dropped or reported as an overflow, and a producer faster than the display costs the
/// UI one update per frame, not one per value. The UI follows it with
/// [`Scope::watch`](crate::Scope::watch) (a signal, no closure) or
/// [`Scope::on_latest`](crate::Scope::on_latest) (a handler); several sets between two
/// updates are seen once, with the newest value. Use a [`Channel`](crate::Channel) instead
/// when every value matters.
///
/// **Any `T`.** The value lives in a `critical_section::Mutex`, so `Latest<T>` is `Sync`
/// whenever `T` is `Send` and is never torn: a reader sees one complete value, whatever its
/// size. Reading clones the value inside the critical section ([`get`](Self::get)): for `Copy`
/// types that is a copy; for other types (a `heapless::String`, an enum with data) keep
/// `Clone` cheap and heap-free, since it runs with interrupts masked. Values replaced by
/// [`set`](Self::set) are dropped after the critical section, in the setter's context.
///
/// Only `critical-section` is used (no atomics read-modify-write, no compare-and-swap: works on
/// thumbv6m); setting and reading allocate nothing.
///
/// ```
/// use twine_reactive::Latest;
///
/// /// The newest temperature in tenths of a degree, written by the ADC interrupt.
/// static TEMP: Latest<i16> = Latest::new(0);
///
/// TEMP.set(215); // in the interrupt handler
/// TEMP.set(216); // overwrites: the UI will only see 216
/// assert_eq!(TEMP.get(), 216);
/// assert_eq!(TEMP.version(), 2);
/// ```
pub struct Latest<T> {
    slot: Mutex<RefCell<Slot<T>>>,
    waker: UiWaker,
}

impl<T: Default> Default for Latest<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> core::fmt::Debug for Latest<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Latest")
            .field("version", &self.version())
            .finish_non_exhaustive()
    }
}

impl<T> Latest<T> {
    /// A cell holding `initial` (version 0). `const`, so it can initialise a `static`.
    ///
    /// ```
    /// static LINK_UP: twine_reactive::Latest<bool> = twine_reactive::Latest::new(false);
    /// assert!(!LINK_UP.get());
    /// ```
    #[must_use]
    pub const fn new(initial: T) -> Self {
        Latest {
            slot: Mutex::new(RefCell::new(Slot {
                value: initial,
                version: 0,
            })),
            waker: UiWaker::new(),
        }
    }

    /// Overwrites the value and wakes the UI. Interrupt- and thread-safe; never blocks, never
    /// fails, never allocates. The previous value is dropped after the critical section, in
    /// the caller's context.
    ///
    /// # Panics
    ///
    /// Only if called from the `Clone` implementation of `T` while that clone runs for this
    /// same cell (a re-entrant borrow), or if the waker's notify function or task waker panics.
    ///
    /// ```
    /// use twine_reactive::Latest;
    /// static BATTERY: Latest<u8> = Latest::new(100);
    /// BATTERY.set(87);
    /// assert_eq!(BATTERY.get(), 87);
    /// ```
    #[inline]
    pub fn set(&self, value: T) {
        drop(self.replace(value));
    }

    /// [`set`](Self::set), returning the previous value instead of dropping it.
    ///
    /// # Panics
    ///
    /// As [`set`](Self::set).
    ///
    /// ```
    /// use twine_reactive::Latest;
    /// let mode = Latest::new("idle");
    /// assert_eq!(mode.replace("heating"), "idle");
    /// assert_eq!(mode.get(), "heating");
    /// ```
    #[inline]
    pub fn replace(&self, value: T) -> T {
        let old = critical_section::with(|cs| {
            let mut slot = self.slot.borrow_ref_mut(cs);
            slot.version = slot.version.wrapping_add(1);
            core::mem::replace(&mut slot.value, value)
        });
        self.waker.wake();
        old
    }

    /// The number of [`set`](Self::set)s so far (wrapping at `u32::MAX`). A consumer that polls
    /// instead of registering with the UI compares it with the version it last saw. Never
    /// panics.
    ///
    /// ```
    /// let l = twine_reactive::Latest::new(0u8);
    /// let seen = l.version();
    /// l.set(1);
    /// assert_ne!(l.version(), seen);
    /// ```
    #[must_use]
    pub fn version(&self) -> u32 {
        critical_section::with(|cs| self.slot.borrow_ref(cs).version)
    }

    /// The waker signalled by [`set`](Self::set) (the routing to the UI's waker is set up by
    /// [`Scope::watch`](crate::Scope::watch) / [`Scope::on_latest`](crate::Scope::on_latest)).
    /// A consumer that is not a UI can register its own notify function or task in it.
    /// Never panics.
    ///
    /// ```
    /// use twine_reactive::Latest;
    /// static LINK_UP: Latest<bool> = Latest::new(false);
    /// LINK_UP.set(true);
    /// assert!(LINK_UP.waker().take()); // `set` signalled the waker
    /// assert!(!LINK_UP.waker().is_set());
    /// ```
    #[must_use]
    pub fn waker(&self) -> &UiWaker {
        &self.waker
    }
}

impl<T: Clone> Latest<T> {
    /// A clone of the current value, from any context. The clone runs inside the critical
    /// section (interrupts masked on a single core): a copy for `Copy` types; keep it cheap and
    /// heap-free for others. Never panics (unless `T::clone` does).
    ///
    /// ```
    /// let l = twine_reactive::Latest::new([1u8, 2, 3]);
    /// assert_eq!(l.get(), [1, 2, 3]);
    /// ```
    #[must_use]
    pub fn get(&self) -> T {
        critical_section::with(|cs| self.slot.borrow_ref(cs).value.clone())
    }

    /// The value and its version, read together.
    pub(crate) fn snapshot(&self) -> (T, u32) {
        critical_section::with(|cs| {
            let slot = self.slot.borrow_ref(cs);
            (slot.value.clone(), slot.version)
        })
    }

    /// The value if its version differs from `*seen` (then `*seen` is updated), in one
    /// critical section.
    #[inline]
    pub(crate) fn get_if_newer(&self, seen: &mut u32) -> Option<T> {
        critical_section::with(|cs| {
            let slot = self.slot.borrow_ref(cs);
            if slot.version == *seen {
                None
            } else {
                *seen = slot.version;
                Some(slot.value.clone())
            }
        })
    }
}

impl<T> Source for Latest<T> {
    /// A `Latest` never leaves work behind: a drain takes the newest value, and a later
    /// [`set`](Latest::set) wakes the UI again.
    fn pending(&self) -> bool {
        false
    }

    fn ui_waker(&self) -> &UiWaker {
        &self.waker
    }
}
