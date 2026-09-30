//! [`Prop`] and [`IntoProp`]: property values that are constants, signals, memos or closures.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

use twine_core::{Angle, Color, Duration, Insets, Opa, Point, Rect, Scale, Size};
use twine_image::{ImageSource, Symbol};
use twine_reactive::{Memo, ReadSignal, Signal};
use twine_render::BlendMode;
use twine_style::{
    Align, Anchor, Axis, BaseDir, CrossAlign, FlexDirection, FlexFlow, GridAlign, GridSpan, Length,
    MainAlign, ScrollSnap, ScrollbarMode, Side, Sides,
};
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

/// Anything usable as a property value of type `T`: the value itself (for every
/// [`PropValue`]), a [`Signal`], [`ReadSignal`] or [`Memo`] of it, or a closure `Fn() -> T`.
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
/// compiler knows that the constant types are not closures. That is also why a constant is a
/// property through one `impl IntoProp<T> for T` per type (written by [`prop_value!`](crate::prop_value!)) and not
/// through a blanket `impl<T: PropValue> IntoProp<T> for T`, which would overlap the closure
/// impl (a type could be both a `PropValue` and a closure returning itself).
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a property of type `{T}`",
    label = "not a `{T}`, a signal, a memo or a closure returning one",
    note = "declare your own property types with `twine_view::prop_value!(MyType);`"
)]
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

/// A type usable as a constant property value: `value` in `.setter(value)` is applied once, at
/// build time, without a binding ([`Prop::Static`]).
///
/// Declare your own types with the one-liner [`prop_value!`](crate::prop_value!), which implements this trait and
/// [`IntoProp<T>`] for `T`:
///
/// ```
/// use twine_view::prelude::*;
///
/// #[derive(Clone, Copy, PartialEq)]
/// pub struct Speed(pub u16);
/// twine_view::prop_value!(Speed);
///
/// fn take<T: 'static>(p: impl IntoProp<T>) -> Prop<T> { p.into_prop() }
/// let cx = twine_reactive::create_root();
/// let s = cx.signal(Speed(3));
/// assert!(matches!(take(Speed(1)), Prop::Static(Speed(1))));
/// assert!(matches!(take(s), Prop::Dynamic(_)));
/// cx.dispose();
/// ```
///
/// Containers of property values are property values too: `Option<T>`, `Vec<T>`,
/// `&'static [T]`, `RangeInclusive<T>` and pairs `(A, B)`.
///
/// `IntoProp<Self>` is a supertrait (see [`IntoProp`] for why it cannot be a blanket impl): a
/// bare `impl PropValue for MyType {}` does not compile; use the macro.
pub trait PropValue: IntoProp<Self> + Sized + 'static {}

/// Declares property value types: implements [`PropValue`] and [`IntoProp<T>`] for `T` (a
/// constant, [`Prop::Static`]) for each listed type.
///
/// ```
/// #[derive(Clone, Copy)]
/// pub struct Speed(pub u16);
/// #[derive(Clone, Copy)]
/// pub enum Gear { Low, High }
/// twine_view::prop_value!(Speed, Gear);
/// ```
#[macro_export]
macro_rules! prop_value {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::PropValue for $t {}
            impl $crate::IntoProp<$t> for $t {
                #[inline]
                fn into_prop(self) -> $crate::Prop<$t> {
                    $crate::Prop::Static(self)
                }
            }
        )+
    };
}

prop_value!(
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
    twine_style::Radius,
    twine_style::DurationMs,
    twine_core::Fraction,
    twine_core::AngularSpeed,
    Align,
    Anchor,
    MainAlign,
    CrossAlign,
    FlexDirection,
    FlexFlow,
    GridSpan,
    GridAlign,
    TextAlign,
    TextDecor,
    BlendMode,
    Axis,
    Side,
    Sides,
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
    twine_style::GradDir,
    twine_style::BlurQuality,
    twine_style::TextLeadingTrim,
    twine_style::LayoutKind,
    twine_style::GridTrack,
    &'static twine_style::ImageColorkey,
    &'static twine_style::ColorFilter,
    &'static twine_anim::AnimTemplate,
    twine_widgets::image::ImageAlign,
    twine_widgets::Orientation,
    twine_widgets::bar::BarMode,
    twine_widgets::arc::ArcMode,
    twine_widgets::keyboard::KeyboardMode,
    twine_widgets::spangroup::SpanMode,
    twine_widgets::spangroup::SpanOverflow,
    twine_anim::Repeat,
    twine_widgets_ext::roller::RollerMode,
    twine_widgets_ext::menu::MenuHeaderMode,
);

/// An [`Align`] as an anchor inside the base: `.align_to(base, Align::Center, 0, 0)`.
impl IntoProp<Anchor> for Align {
    #[inline]
    fn into_prop(self) -> Prop<Anchor> {
        Prop::Static(Anchor::Inside(self))
    }
}

/// A grid item's columns or rows ([`GridSpan`]) for
/// [`grid_col`](crate::ViewExt::grid_col) / [`grid_row`](crate::ViewExt::grid_row): an index
/// (`1`), a range (`0..2`, `0..=1`), or any [`IntoProp<GridSpan>`] (a `GridSpan`, a signal or a
/// closure returning one).
///
/// A trait of its own rather than `IntoProp<GridSpan>` impls for the integers: those would make
/// `i32` a property of two types and break the inference of `.width(w)` with an `i32` `w`.
///
/// ```
/// use twine_view::prelude::*;
/// let cx = twine_reactive::create_root();
/// let col = cx.signal(GridSpan::new(0, 2));
/// let _v = label("a").grid_col(col).grid_row(1);
/// let _w = label("b").grid_col(0..2).grid_row(0..=1);
/// cx.dispose();
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a grid column/row",
    label = "use an index (`1`), a range (`0..2`) or a `GridSpan`"
)]
pub trait IntoGridSpan {
    /// The span as a property.
    fn into_grid_span(self) -> Prop<GridSpan>;
}

impl<P: IntoProp<GridSpan>> IntoGridSpan for P {
    #[inline]
    fn into_grid_span(self) -> Prop<GridSpan> {
        self.into_prop()
    }
}

macro_rules! grid_span_args {
    ($($t:ty),*) => {$(
        impl IntoGridSpan for $t {
            #[inline]
            fn into_grid_span(self) -> Prop<GridSpan> {
                Prop::Static(GridSpan::from(self))
            }
        }
        impl IntoGridSpan for core::ops::Range<$t> {
            #[inline]
            fn into_grid_span(self) -> Prop<GridSpan> {
                Prop::Static(GridSpan::from(self))
            }
        }
        impl IntoGridSpan for RangeInclusive<$t> {
            #[inline]
            fn into_grid_span(self) -> Prop<GridSpan> {
                Prop::Static(GridSpan::from(self))
            }
        }
    )*};
}

grid_span_args!(i32, u8, u16, u32, usize);

impl<T: 'static> IntoProp<Option<T>> for Option<T> {
    #[inline]
    fn into_prop(self) -> Prop<Option<T>> {
        Prop::Static(self)
    }
}
impl<T: PropValue> PropValue for Option<T> {}

impl<T: PropValue> IntoProp<Vec<T>> for Vec<T> {
    #[inline]
    fn into_prop(self) -> Prop<Vec<T>> {
        Prop::Static(self)
    }
}
impl<T: PropValue> PropValue for Vec<T> {}

impl<T: PropValue> IntoProp<&'static [T]> for &'static [T] {
    #[inline]
    fn into_prop(self) -> Prop<&'static [T]> {
        Prop::Static(self)
    }
}
impl<T: PropValue> PropValue for &'static [T] {}

/// A `'static` array is a slice property (e.g. `animimg(&FRAMES, ..)`).
impl<T: PropValue, const N: usize> IntoProp<&'static [T]> for &'static [T; N] {
    #[inline]
    fn into_prop(self) -> Prop<&'static [T]> {
        Prop::Static(self)
    }
}

impl<T: PropValue> IntoProp<RangeInclusive<T>> for RangeInclusive<T> {
    #[inline]
    fn into_prop(self) -> Prop<RangeInclusive<T>> {
        Prop::Static(self)
    }
}
impl<T: PropValue> PropValue for RangeInclusive<T> {}

impl<A: PropValue, B: PropValue> IntoProp<(A, B)> for (A, B) {
    #[inline]
    fn into_prop(self) -> Prop<(A, B)> {
        Prop::Static(self)
    }
}
impl<A: PropValue, B: PropValue> PropValue for (A, B) {}

// ---- Symbols and icons ------------------------------------------------------------------------

impl IntoProp<ImageSource> for Symbol {
    /// A built-in symbol as a constant image source: `image(Symbol::Ok)`.
    #[inline]
    fn into_prop(self) -> Prop<ImageSource> {
        Prop::Static(ImageSource::symbol(self))
    }
}

/// Reactive symbols as image sources (`Signal<Symbol>`, `ReadSignal<Symbol>`, `Memo<Symbol>`).
macro_rules! symbol_signal_props {
    ($($s:ident),+) => {
        $(
            impl IntoProp<ImageSource> for $s<Symbol> {
                fn into_prop(self) -> Prop<ImageSource> {
                    Prop::Dynamic(Box::new(move || ImageSource::symbol(self.get())))
                }
            }
            impl IntoIcon for $s<Symbol> {
                fn into_icon(self) -> Prop<Option<ImageSource>> {
                    Prop::Dynamic(Box::new(move || Some(ImageSource::symbol(self.get()))))
                }
            }
            impl IntoIcon for $s<Option<Symbol>> {
                fn into_icon(self) -> Prop<Option<ImageSource>> {
                    Prop::Dynamic(Box::new(move || self.get().map(ImageSource::symbol)))
                }
            }
        )+
    };
}
symbol_signal_props!(Signal, ReadSignal, Memo);

/// An optional icon of a widget (a list button, a dropdown arrow, the side images of an image
/// button…): `()` for none, a [`Symbol`], an [`ImageSource`], an `Option` of either, or
/// anything that is an [`IntoProp<Option<ImageSource>>`] (a signal, a memo or a closure
/// returning `Option<ImageSource>`: a dynamic `None` hides the icon).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn icon(i: impl IntoIcon) -> Prop<Option<ImageSource>> { i.into_icon() }
///
/// let cx = twine_reactive::create_root();
/// assert!(matches!(icon(()), Prop::Static(None)));
/// assert!(matches!(icon(Symbol::Save), Prop::Static(Some(ImageSource::Symbol("\u{F0C7}")))));
/// assert!(matches!(icon(Some(Symbol::Save)), Prop::Static(Some(_))));
/// let shown = cx.signal(true);
/// assert!(matches!(icon(move || shown.get().then(|| Symbol::Ok.into())), Prop::Dynamic(_)));
/// let _v = list((
///     list_button((), "No icon"),
///     list_button(Symbol::File, "Open"),
/// ));
/// cx.dispose();
/// ```
///
/// Constants are [`Prop::Static`]: no binding and no allocation.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an icon",
    label = "not `()`, a `Symbol`, an `ImageSource`, an `Option` of one, or a signal/closure of `Option<ImageSource>`"
)]
pub trait IntoIcon {
    /// The icon property (`Prop::Static(None)` for no icon).
    fn into_icon(self) -> Prop<Option<ImageSource>>;
}

impl IntoIcon for () {
    #[inline]
    fn into_icon(self) -> Prop<Option<ImageSource>> {
        Prop::Static(None)
    }
}

impl IntoIcon for Symbol {
    #[inline]
    fn into_icon(self) -> Prop<Option<ImageSource>> {
        Prop::Static(Some(ImageSource::symbol(self)))
    }
}

impl IntoIcon for Option<Symbol> {
    #[inline]
    fn into_icon(self) -> Prop<Option<ImageSource>> {
        Prop::Static(self.map(ImageSource::symbol))
    }
}

impl IntoIcon for ImageSource {
    #[inline]
    fn into_icon(self) -> Prop<Option<ImageSource>> {
        Prop::Static(Some(self))
    }
}

impl<T: IntoProp<Option<ImageSource>>> IntoIcon for T {
    #[inline]
    fn into_icon(self) -> Prop<Option<ImageSource>> {
        self.into_prop()
    }
}
