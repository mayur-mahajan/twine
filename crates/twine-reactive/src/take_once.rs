//! [`TakeOnce`]: a `static` value handed out once as `&'static mut`.

use core::cell::UnsafeCell;

use portable_atomic::{AtomicBool, Ordering};

/// A value stored in a `static` and handed out **once** as `&'static mut T`, without
/// `static mut` and without `unsafe` in the application.
///
/// This is how firmware obtains `'static` memory that must be exclusively owned, such as draw
/// buffers a DMA engine reads after the call that started the transfer returned (see
/// `twine_hal::buffer`). The value is constant-initialised, so a zeroed buffer lands in
/// `.bss` and costs no flash. `twine::draw_buffers!` declares draw buffers with it.
///
/// [`take`](Self::take) returns `Some` the first time and `None` afterwards, from any
/// execution context (interrupts, tasks, cores, threads): the check and the claim are one
/// critical section (`critical-section` crate), so no compare-and-swap is needed and it builds
/// for `thumbv6m`. The claim is never released.
///
/// ```
/// use twine_reactive::TakeOnce;
///
/// static COUNTERS: TakeOnce<[u32; 4]> = TakeOnce::new([0; 4]);
///
/// let counters: &'static mut [u32; 4] = COUNTERS.take().expect("taken once");
/// counters[0] += 1;
/// assert!(COUNTERS.take().is_none()); // the second take fails
/// assert!(COUNTERS.is_taken());
/// ```
pub struct TakeOnce<T> {
    taken: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the only access to `value` is the `&'static mut T` that `take` hands out at most once
// (the flag is checked and set inside one critical section, so two contexts cannot both see
// it unset). That reference may be used in another context than the one that declared the
// `static`, which is sound when `T: Send`.
#[allow(unsafe_code)]
unsafe impl<T: Send> Sync for TakeOnce<T> {}

impl<T> TakeOnce<T> {
    /// A cell holding `value`, not yet taken. `const`, for `static` items; allocates
    /// nothing; never panics.
    ///
    /// ```
    /// use twine_reactive::TakeOnce;
    /// static DMA_BUF: TakeOnce<[u8; 64]> = TakeOnce::new([0; 64]);
    /// let buf: &'static mut [u8; 64] = DMA_BUF.take().unwrap();
    /// buf[0] = 1;
    /// assert!(DMA_BUF.take().is_none()); // handed out once
    /// ```
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self {
            taken: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// The value as `&'static mut T` the first time, `None` on every later call (from any
    /// context).
    ///
    /// Allocates nothing; never panics; takes one critical section.
    ///
    /// ```
    /// use twine_reactive::TakeOnce;
    ///
    /// static BUF: TakeOnce<[u8; 16]> = TakeOnce::new([0; 16]);
    /// let buf = BUF.take().unwrap();
    /// buf[0] = 7;
    /// assert!(BUF.take().is_none());
    /// ```
    #[must_use]
    #[allow(clippy::mut_from_ref)] // the point of the type: the claim makes the reference unique
    pub fn take(&'static self) -> Option<&'static mut T> {
        let first = critical_section::with(|_| {
            if self.taken.load(Ordering::Relaxed) {
                false
            } else {
                self.taken.store(true, Ordering::Relaxed);
                true
            }
        });
        if !first {
            return None;
        }
        // SAFETY: `first` is `true` exactly once over the lifetime of the `static` (see the
        // critical section above and the `Sync` impl), so this is the only reference to the
        // value ever created; the value lives forever (`&'static self`).
        #[allow(unsafe_code)]
        let value = unsafe { &mut *self.value.get() };
        Some(value)
    }

    /// Whether [`take`](Self::take) has handed the value out.
    ///
    /// ```
    /// use twine_reactive::TakeOnce;
    ///
    /// static X: TakeOnce<u8> = TakeOnce::new(0);
    /// assert!(!X.is_taken());
    /// let _ = X.take();
    /// assert!(X.is_taken());
    /// ```
    #[must_use]
    pub fn is_taken(&self) -> bool {
        self.taken.load(Ordering::Relaxed)
    }
}

impl<const N: usize> TakeOnce<[u8; N]> {
    /// A cell holding `N` zero bytes, not yet taken: a byte buffer for a `static` (placed in
    /// `.bss`, so it costs RAM, not flash), e.g. the engine's layer buffer
    /// (`twine::view::LayerBuffer`). `const`.
    ///
    /// ```
    /// use twine_reactive::TakeOnce;
    ///
    /// static BUF: TakeOnce<[u8; 4096]> = TakeOnce::zeroed();
    /// assert!(BUF.take().unwrap().iter().all(|b| *b == 0));
    /// ```
    #[must_use]
    pub const fn zeroed() -> Self {
        Self::new([0; N])
    }
}

impl<T> core::fmt::Debug for TakeOnce<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never shows the value: it may be borrowed mutably elsewhere.
        f.debug_struct("TakeOnce")
            .field("taken", &self.is_taken())
            .finish_non_exhaustive()
    }
}
