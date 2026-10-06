//! [`IntoItems`] and [`Items`]: the item list of a [`for_each`](crate::for_each) or a
//! [`virtual_list`](crate::virtual_list).

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_reactive::{Memo, ReadSignal, Signal};

use crate::prop::marker;

/// Lends the items to a visitor (`with`: no copy of the list).
type ReadItems<T> = dyn Fn(&mut dyn FnMut(&[T]));

/// The items of a keyed collection ([`for_each`](crate::for_each),
/// [`virtual_list`](crate::virtual_list)), made by [`IntoItems`].
///
/// It lends the items as a slice: a signal or memo of a `Vec` is read in place with `with`
/// (the list is never cloned), a closure's `Vec` is borrowed for the time of the read, a
/// constant list is stored once.
pub struct Items<T: 'static>(Box<ReadItems<T>>);

impl<T: 'static> Items<T> {
    /// Items read by `read`, which calls its argument with the current items (the reads it
    /// makes are tracked by the collection's effect).
    ///
    /// ```
    /// use twine_view::Items;
    ///
    /// static DAYS: [&str; 3] = ["Mon", "Tue", "Wed"];
    /// let items: Items<&'static str> = Items::new(|f| f(&DAYS));
    /// let mut n = 0;
    /// items.read(|days| n = days.len());
    /// assert_eq!(n, 3);
    /// ```
    pub fn new(read: impl Fn(&mut dyn FnMut(&[T])) + 'static) -> Self {
        Items(Box::new(read))
    }

    /// Calls `f` with the current items (tracked when called inside an effect or memo).
    /// Never panics (beyond what the source panics with, e.g. a disposed signal).
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let v = cx.signal(vec![1, 2, 3]);
    /// let items = v.into_items();
    /// let mut sum = 0;
    /// items.read(|s| sum = s.iter().sum());
    /// assert_eq!(sum, 6);
    /// cx.dispose();
    /// ```
    pub fn read(&self, mut f: impl FnMut(&[T])) {
        (self.0)(&mut f);
    }
}

impl<T> core::fmt::Debug for Items<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Items")
    }
}

/// The items of a [`for_each`](crate::for_each) or a [`virtual_list`](crate::virtual_list): a
/// `Signal`, `ReadSignal` or `Memo` of a `Vec<T>` (read in place with `with`: no clone of the
/// list when it changes), a closure returning a `Vec<T>` (re-run when a signal it reads
/// changes), or a constant `Vec<T>` or `&'static [T]` (e.g. a table in flash). `M` is an
/// inferred marker, as for [`IntoProp`](crate::IntoProp).
///
/// Prefer a signal or memo to a closure: a closure builds a new `Vec` on every change, a
/// signal is only borrowed.
///
/// ```
/// use twine_view::prelude::*;
///
/// static CITIES: [&str; 3] = ["Berlin", "Paris", "Rome"];
///
/// fn lists(cx: Scope) -> impl View {
///     let ids = cx.signal(vec![1u32, 2, 3]);
///     let evens = cx.memo(move || ids.with(|v| v.iter().copied().filter(|i| i % 2 == 0).collect::<Vec<_>>()));
///     column((
///         for_each(ids, |i| *i, |_, i| label(text!("#{}", i.get()))),
///         for_each(evens, |i| *i, |_, i| label(text!("even {}", i.get()))),
///         for_each(move || ids.get().into_iter().rev().collect::<Vec<_>>(), |i| *i, |_, i| label(text!("{}", i.get()))),
///         for_each(&CITIES[..], |c| *c, |_, c| label(c)),
///     ))
/// }
/// # let _ = lists;
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as the items of a keyed collection",
    note = "use a `Signal`, `ReadSignal` or `Memo` of a `Vec`, a closure returning a `Vec`, a `Vec` or a `&'static` slice"
)]
pub trait IntoItems<T: 'static, M> {
    /// The items.
    fn into_items(self) -> Items<T>;
}

/// The `IntoItems` impls of the reactive handles: the list is lent with `with`.
macro_rules! reactive_items {
    ($($s:ident),+) => {$(
        impl<T: 'static> IntoItems<T, marker::Reactive<T>> for $s<Vec<T>> {
            fn into_items(self) -> Items<T> {
                Items::new(move |f| self.with(|v| f(v)))
            }
        }
    )+};
}
reactive_items!(Signal, ReadSignal, Memo);

impl<T: 'static, F: Fn() -> Vec<T> + 'static> IntoItems<T, marker::Closure<T>> for F {
    fn into_items(self) -> Items<T> {
        Items::new(move |f| f(&self()))
    }
}

impl<T: 'static> IntoItems<T, marker::Value<T>> for Vec<T> {
    fn into_items(self) -> Items<T> {
        Items::new(move |f| f(&self))
    }
}

impl<T: 'static> IntoItems<T, marker::Slice> for &'static [T] {
    fn into_items(self) -> Items<T> {
        Items::new(move |f| f(self))
    }
}

impl<T: 'static, const N: usize> IntoItems<T, marker::Slice> for &'static [T; N] {
    fn into_items(self) -> Items<T> {
        Items::new(move |f| f(self))
    }
}

impl<T: 'static> IntoItems<T, marker::Prop> for Items<T> {
    #[inline]
    fn into_items(self) -> Items<T> {
        self
    }
}
