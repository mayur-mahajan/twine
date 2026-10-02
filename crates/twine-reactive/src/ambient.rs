//! The ambient context slot: a scoped, exclusive `&mut dyn Any` that code running further
//! down the call stack of the same UI context can borrow ([`provide_ambient`],
//! [`with_ambient`], [`ambient_is`]).
//!
//! The view layer uses it to reach the engine from event handlers, effects and timer
//! callbacks that do not receive it as a parameter (`NodeRef::with_mut`). The slot lives in
//! the runtime, so it inherits the runtime's confinement to one execution context (a
//! thread-local with `std`, the bound context without it).
//!
//! Borrowing *takes* the value out of the slot for the duration of the borrow, so there is
//! never more than one live `&mut` to it: a nested [`with_ambient`] sees `None`.

use core::any::{Any, TypeId};
use core::ptr::NonNull;

use crate::global::with_runtime;

/// Restores the slot's previous content when a [`provide_ambient`] or [`with_ambient`] frame
/// ends (also on unwinding).
struct Restore(Option<NonNull<dyn Any>>);

impl Drop for Restore {
    fn drop(&mut self) {
        swap_slot(self.0.take());
    }
}

/// Replaces the slot's content with `value` and returns the previous content.
///
/// Deliberately not inlined: [`provide_ambient`] and [`with_ambient`] are generic and
/// instantiated once per closure type (every binding of the view layer has its own), so the
/// runtime access stays in this one out-of-line function and each instantiation is only a
/// few instructions around the call (a direct call, no indirection).
#[inline(never)]
fn swap_slot(value: Option<NonNull<dyn Any>>) -> Option<NonNull<dyn Any>> {
    with_runtime(|rt| core::mem::replace(&mut rt.inner.borrow_mut().ambient, value))
}

/// The type of the value in the slot (`None` when there is none or it is borrowed), read
/// without taking it out of the slot. Out of line like [`swap_slot`].
#[inline(never)]
fn slot_type() -> Option<TypeId> {
    let p = with_runtime(|rt| rt.inner.borrow().ambient)?;
    // SAFETY: `p` was created by `provide_ambient` from a `&mut dyn Any` that is alive while
    // it is in the slot (see `with_ambient`: the providing frame is still active further down
    // this call stack, and the slot is reset before it ends). While the value is in the slot
    // nobody holds a reference to it: `provide_ambient`'s caller gave up its access for the
    // frame, and a `with_ambient` borrow takes the value out of the slot first. So a shared
    // reference for this call is unaliased by any `&mut`; `type_id` only returns a constant
    // and cannot reach the slot, and the reference ends with this statement.
    #[allow(unsafe_code)]
    let ty = unsafe { (*p.as_ptr()).type_id() };
    Some(ty)
}

/// Whether the slot holds a `T` that [`with_ambient`] could borrow right now: `false` outside
/// a [`provide_ambient`], while an enclosing [`with_ambient`] borrows the value, or when it is
/// of another type. It neither takes the value out of the slot nor puts it back (one read of
/// the slot instead of [`with_ambient`]'s two writes), so it is the cheap way to ask before
/// doing work that needs the value.
///
/// ```
/// use twine_reactive::{ambient_is, provide_ambient, with_ambient};
///
/// let mut n = 0u32;
/// assert!(!ambient_is::<u32>());
/// provide_ambient(&mut n, || {
///     assert!(ambient_is::<u32>());
///     assert!(!ambient_is::<i32>());
///     with_ambient(|_| assert!(!ambient_is::<u32>())); // borrowed
/// });
/// assert!(!ambient_is::<u32>());
/// ```
///
/// # Panics
///
/// Only without the `std` feature, before the runtime is bound with
/// [`bind_to_current_context`](crate::bind_to_current_context) (like every function that
/// reaches the runtime). With a bound runtime it never panics: it reads the slot without
/// touching the value.
#[inline]
#[must_use]
pub fn ambient_is<T: Any>() -> bool {
    slot_type() == Some(TypeId::of::<T>())
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
    let _restore = Restore(swap_slot(Some(NonNull::from(value))));
    f()
}

/// Calls `f` with the value of the innermost active [`provide_ambient`] (`None` when there is
/// none, or while it is already borrowed by an enclosing `with_ambient`). The value is taken
/// out of the slot while `f` runs and put back afterwards (also when `f` panics).
///
/// ```
/// use twine_reactive::{provide_ambient, with_ambient};
///
/// struct Engine { frames: u32 }
///
/// fn count_frame() -> bool {
///     // `None` outside `provide_ambient`, or when the value is not an `Engine`.
///     with_ambient(|v| v.and_then(|v| v.downcast_mut::<Engine>()).map(|e| e.frames += 1)).is_some()
/// }
///
/// let mut engine = Engine { frames: 0 };
/// assert!(!count_frame());
/// provide_ambient(&mut engine, || assert!(count_frame()));
/// assert_eq!(engine.frames, 1);
/// ```
///
/// # Panics
///
/// Only without the `std` feature, before the runtime is bound (see [`ambient_is`]), or
/// when `f` panics (the panic propagates after the value was put back).
pub fn with_ambient<R>(f: impl FnOnce(Option<&mut dyn Any>) -> R) -> R {
    let taken = swap_slot(None);
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
    fn ambient_is_follows_provides_and_borrows() {
        let mut a = 0u8;
        let mut b = 0i32;
        assert!(!ambient_is::<u8>());
        provide_ambient(&mut a, || {
            assert!(ambient_is::<u8>());
            provide_ambient(&mut b, || {
                assert!(ambient_is::<i32>());
                assert!(!ambient_is::<u8>()); // shadowed
            });
            assert!(ambient_is::<u8>());
            with_ambient(|v| {
                assert!(!ambient_is::<u8>()); // borrowed
                // Handed on: visible again, as its own type.
                provide_ambient(v.unwrap(), || assert!(ambient_is::<u8>()));
                assert!(!ambient_is::<u8>());
            });
            assert!(ambient_is::<u8>());
        });
        assert!(!ambient_is::<u8>());
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
