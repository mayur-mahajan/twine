//! [`View`], [`ViewSeq`], [`AnyView`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_engine::NodeId;

use crate::build::BuildCx;

/// A description of a piece of UI, consumed once to create its node(s).
///
/// Views are built exactly once; everything that changes later is a binding created while
/// building (see [`IntoProp`](crate::IntoProp)).
pub trait View: 'static {
    /// Creates this view's node(s) under `cx.parent()` and returns the root node.
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId;
}

/// A sequence of child views: every `V: View` (one child), tuples of 1 to 16 views, `Vec<V>`,
/// `[V; N]`, `Option<V>` and `()`.
pub trait ViewSeq: 'static {
    /// Builds every view of the sequence under `cx.parent()`, in order.
    fn build_seq(self, cx: &mut BuildCx<'_>);
}

impl<V: View> ViewSeq for V {
    fn build_seq(self, cx: &mut BuildCx<'_>) {
        self.build(cx);
    }
}

impl ViewSeq for () {
    fn build_seq(self, _cx: &mut BuildCx<'_>) {}
}

impl<V: View> ViewSeq for Option<V> {
    fn build_seq(self, cx: &mut BuildCx<'_>) {
        if let Some(v) = self {
            v.build(cx);
        }
    }
}

impl<V: View> ViewSeq for Vec<V> {
    fn build_seq(self, cx: &mut BuildCx<'_>) {
        for v in self {
            v.build(cx);
        }
    }
}

impl<V: View, const N: usize> ViewSeq for [V; N] {
    fn build_seq(self, cx: &mut BuildCx<'_>) {
        for v in self {
            v.build(cx);
        }
    }
}

macro_rules! tuple_seq {
    ($($v:ident),+) => {
        impl<$($v: ViewSeq),+> ViewSeq for ($($v,)+) {
            #[allow(non_snake_case)]
            fn build_seq(self, cx: &mut BuildCx<'_>) {
                let ($($v,)+) = self;
                $($v.build_seq(cx);)+
            }
        }
    };
}

tuple_seq!(A);
tuple_seq!(A, B);
tuple_seq!(A, B, C);
tuple_seq!(A, B, C, D);
tuple_seq!(A, B, C, D, E);
tuple_seq!(A, B, C, D, E, F);
tuple_seq!(A, B, C, D, E, F, G);
tuple_seq!(A, B, C, D, E, F, G, H);
tuple_seq!(A, B, C, D, E, F, G, H, I);
tuple_seq!(A, B, C, D, E, F, G, H, I, J);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K, L);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
tuple_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

/// A type-erased view (`Box<dyn FnOnce(&mut BuildCx) -> NodeId>`), for returning different
/// view types from one function (e.g. the branches of [`dynamic`](crate::dynamic)).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn badge(n: u32) -> AnyView {
///     if n == 0 { label("none").into_any() } else { button(label("some")).into_any() }
/// }
/// # let _ = badge(1);
/// ```
pub struct AnyView(Box<dyn FnOnce(&mut BuildCx<'_>) -> NodeId>);

impl core::fmt::Debug for AnyView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AnyView")
    }
}

impl AnyView {
    /// Wraps a build function.
    #[must_use]
    pub fn new(build: impl FnOnce(&mut BuildCx<'_>) -> NodeId + 'static) -> Self {
        AnyView(Box::new(build))
    }
}

impl View for AnyView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        (self.0)(cx)
    }
}

/// Conversion into [`AnyView`], implemented for every view. (A blanket
/// `From<V: View> for AnyView` is impossible: `AnyView` is a view itself, so it would overlap
/// with the reflexive `From<T> for T`.)
pub trait IntoAnyView {
    /// Erases the view's type.
    fn into_any(self) -> AnyView;
}

impl<V: View> IntoAnyView for V {
    fn into_any(self) -> AnyView {
        AnyView(Box::new(move |cx| self.build(cx)))
    }
}
