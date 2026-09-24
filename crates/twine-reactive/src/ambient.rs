//! The ambient context slot: a scoped, exclusive `&mut dyn Any` that code running further
//! down the call stack of the same UI context can borrow ([`provide_ambient`],
//! [`with_ambient`]).
//!
//! The view layer uses it to reach the engine from event handlers, effects and timer
//! callbacks that do not receive it as a parameter (`NodeRef::with_mut`). The slot lives in
//! the runtime, so it inherits the runtime's confinement to one execution context (a
//! thread-local with `std`, the bound context without it).
//!
//! Borrowing *takes* the value out of the slot for the duration of the borrow, so there is
//! never more than one live `&mut` to it: a nested [`with_ambient`] sees `None`.

use core::any::Any;
use core::ptr::NonNull;

use crate::global::with_runtime;

/// Restores the slot's previous content when a [`provide_ambient`] or [`with_ambient`] frame
/// ends (also on unwinding).
struct Restore(Option<NonNull<dyn Any>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let prev = self.0.take();
        with_runtime(|rt| rt.inner.borrow_mut().ambient = prev);
    }
}

/// Makes `value` available to [`with_ambient`] calls made (directly or indirectly) by `f`,
/// then restores the previous ambient value (also when `f` panics). Nested calls shadow the
/// outer value until they return.
///
/// ```
/// use twine_reactive::{provide_ambient, with_ambient};
///
/// let mut counter = 0u32;
/// provide_ambient(&mut counter, || {
///     with_ambient(|v| *v.unwrap().downcast_mut::<u32>().unwrap() += 1);
///     // Borrowed: a nested borrow sees nothing.
///     with_ambient(|outer| {
///         assert!(outer.is_some());
///         with_ambient(|inner| assert!(inner.is_none()));
///     });
/// });
/// assert_eq!(counter, 1);
/// with_ambient(|v| assert!(v.is_none())); // gone once `provide_ambient` returned
/// ```
pub fn provide_ambient<R>(value: &mut dyn Any, f: impl FnOnce() -> R) -> R {
    let ptr = NonNull::from(value);
    let prev = with_runtime(|rt| rt.inner.borrow_mut().ambient.replace(ptr));
    let _restore = Restore(prev);
    f()
}

/// Calls `f` with the value of the innermost active [`provide_ambient`] (`None` when there is
/// none, or while it is already borrowed by an enclosing `with_ambient`). The value is taken
/// out of the slot while `f` runs and put back afterwards.
pub fn with_ambient<R>(f: impl FnOnce(Option<&mut dyn Any>) -> R) -> R {
    let taken = with_runtime(|rt| rt.inner.borrow_mut().ambient.take());
    let _restore = Restore(taken);
    let value = taken.map(|p| {
        // SAFETY: `p` was created by `provide_ambient` from a `&mut dyn Any` whose borrow
        // lasts for that function's whole frame, and the slot is reset to the previous value
        // before the frame ends (`Restore`, also on unwinding). The slot lives in the reactive
        // runtime, which is confined to one execution context (thread-local with `std`, the
        // bound context otherwise), so the providing frame is still active further down this
        // very call stack while we run: the pointee is alive. Nobody else can access it:
        // `provide_ambient` gives up its access to `value` for its whole frame (the caller's
        // `&mut` is reborrowed into `p` and not used until it returns), and we just took `p`
        // out of the slot, so any nested `with_ambient` sees `None` until `Restore` puts it
        // back after `f` returned. The `&mut` handed to `f` cannot escape `f` (it is only
        // valid for the higher-ranked lifetime of the closure argument).
        #[allow(unsafe_code)]
        let r = unsafe { &mut *p.as_ptr() };
        r
    });
    f(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_provides_shadow_and_restore() {
        let mut a = 1u8;
        let mut b = 2u8;
        provide_ambient(&mut a, || {
            provide_ambient(&mut b, || {
                with_ambient(|v| *v.unwrap().downcast_mut::<u8>().unwrap() += 10);
            });
            with_ambient(|v| *v.unwrap().downcast_mut::<u8>().unwrap() += 100);
        });
        assert_eq!((a, b), (101, 12));
    }

    #[test]
    fn reprovide_from_a_borrow() {
        let mut a = 0i32;
        provide_ambient(&mut a, || {
            with_ambient(|v| {
                let v = v.unwrap();
                // Hand the borrowed value on to nested code.
                provide_ambient(v, || {
                    with_ambient(|w| *w.unwrap().downcast_mut::<i32>().unwrap() += 1);
                });
                *v.downcast_mut::<i32>().unwrap() += 1;
            });
        });
        assert_eq!(a, 2);
    }

    #[test]
    fn restored_after_panic() {
        let mut a = 0i32;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            provide_ambient(&mut a, || with_ambient(|_| panic!("boom")))
        }));
        assert!(r.is_err());
        with_ambient(|v| assert!(v.is_none()));
    }
}
