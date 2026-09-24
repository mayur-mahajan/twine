//! [`Prop`] and [`IntoProp`]: property values that are constants, signals, memos or closures.

use alloc::boxed::Box;
use alloc::string::String;

use twine_core::{Angle, Color, Duration, Insets, Opa, Point, Rect, Scale, Size};
use twine_image::ImageSource;
use twine_reactive::{Memo, ReadSignal, Signal};
use twine_render::BlendMode;
use twine_style::{Align, BaseDir, Dir, FlexAlign, FlexFlow, GridAlign, Length, ScrollSnap, ScrollbarMode};
use twine_text::{Font, LongMode, TextAlign, TextDecor};

/// A property value: fixed, or computed by a closure that is re-run (as a binding) whenever a
/// signal it reads changes.
pub enum Prop<T> {
    /// Applied once, at build time (no binding is created).
    Static(T),
    /// Re-evaluated by a binding effect.
    Dynamic(Box<dyn Fn() -> T>),
}

impl<T> core::fmt::Debug for Prop<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Prop::Static(_) => f.write_str("Prop::Static"),
            Prop::Dynamic(_) => f.write_str("Prop::Dynamic"),
        }
    }
}

impl<T: 'static> Prop<T> {
    /// Maps the value (lazily for dynamic props).
    #[must_use]
    pub fn map<U: 'static>(self, f: impl Fn(T) -> U + 'static) -> Prop<U> {
        match self {
            Prop::Static(v) => Prop::Static(f(v)),
            Prop::Dynamic(g) => Prop::Dynamic(Box::new(move || f(g()))),
        }
    }
}

/// Anything usable as a property value of type `T`: the value itself (for every type used as
/// a property), a [`Signal`], [`ReadSignal`] or [`Memo`] of it, or a closure `Fn() -> T`.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn take<T: 'static>(p: impl IntoProp<T>) -> Prop<T> { p.into_prop() }
///
/// let cx = twine_reactive::create_root();
/// let n = cx.signal(3);
/// assert!(matches!(take(5), Prop::Static(5)));
/// assert!(matches!(take(n), Prop::Dynamic(_)));
/// assert!(matches!(take(move || n.get() * 2), Prop::Dynamic(_)));
/// cx.dispose();
/// ```
///
/// Closures and constants do not overlap because the `Fn*` traits are `#[fundamental]`: the
/// compiler knows that the constant types listed here are not closures.
pub trait IntoProp<T> {
    /// The property value.
    fn into_prop(self) -> Prop<T>;
}

impl<T, F: Fn() -> T + 'static> IntoProp<T> for F {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Box::new(self))
    }
}

impl<T: Clone + 'static> IntoProp<T> for Signal<T> {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Box::new(move || self.get()))
    }
}

impl<T: Clone + 'static> IntoProp<T> for ReadSignal<T> {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Box::new(move || self.get()))
    }
}

impl<T: Clone + 'static> IntoProp<T> for Memo<T> {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Box::new(move || self.get()))
    }
}

impl<T> IntoProp<T> for Prop<T> {
    fn into_prop(self) -> Prop<T> {
        self
    }
}

/// Implements [`IntoProp<T>`] for `T` itself (a constant property) for each listed type.
/// Use it for your own property types:
///
/// ```
/// #[derive(Clone, Copy)]
/// pub struct Speed(pub u16);
/// twine_view::impl_into_prop!(Speed);
/// ```
#[macro_export]
macro_rules! impl_into_prop {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::IntoProp<$t> for $t {
                fn into_prop(self) -> $crate::Prop<$t> {
                    $crate::Prop::Static(self)
                }
            }
        )*
    };
}

impl_into_prop!(
    bool,
    u8,
    u16,
    u32,
    u64,
    usize,
    i8,
    i16,
    i32,
    i64,
    char,
    Color,
    Opa,
    Angle,
    Scale,
    Length,
    Align,
    FlexAlign,
    FlexFlow,
    GridAlign,
    TextAlign,
    TextDecor,
    BlendMode,
    Dir,
    ScrollbarMode,
    ScrollSnap,
    BaseDir,
    Point,
    Size,
    Insets,
    ImageSource,
    LongMode,
    Duration,
    &'static Font,
    &'static str,
    String,
    Rect,
    &'static ImageSource,
    &'static twine_render::Gradient,
    &'static twine_style::TransitionDsc,
    twine_render::BorderSide,
    twine_render::ShadowDsc,
    twine_widgets::image::ImageAlign,
    twine_widgets::Orientation,
    twine_widgets::bar::BarMode,
    twine_widgets::arc::ArcMode,
    twine_widgets::keyboard::KeyboardMode,
    twine_widgets::spangroup::SpanMode,
    twine_widgets::spangroup::SpanOverflow,
    twine_anim::Repeat,
    core::ops::RangeInclusive<i32>,
    alloc::vec::Vec<Point>,
);

impl<T: 'static> IntoProp<Option<T>> for Option<T> {
    fn into_prop(self) -> Prop<Option<T>> {
        Prop::Static(self)
    }
}
