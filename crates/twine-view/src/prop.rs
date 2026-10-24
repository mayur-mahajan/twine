//! [`Prop`] and [`IntoProp`]: property values that are constants, signals, memos or closures.

use alloc::boxed::Box;

use twine_engine::{Variant, WidgetClass};
use twine_image::{Image, ImageSource, Symbol};
use twine_reactive::{Memo, ReadSignal, Signal};

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

/// A constant `T::default()` (no binding, no allocation): what `#[prop(default)]` gives a
/// property of a [`#[component]`](macro@crate::component).
///
/// ```
/// use twine_view::prelude::*;
///
/// assert_eq!(Prop::<i32>::default().get(), 0);
/// ```
impl<T: Default> Default for Prop<T> {
    #[inline]
    fn default() -> Self {
        Prop::Static(T::default())
    }
}

impl<T: Clone> Prop<T> {
    /// The current value: a clone of a constant, or the result of a dynamic prop's closure.
    /// Inside a binding, effect or memo the closure's reads are tracked like any other read,
    /// so a component can turn a property it uses in several places into a `Copy` memo:
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// /// Shows `value` twice: as a bar and as a number.
    /// fn meter(cx: Scope, value: Prop<i32>) -> impl View {
    ///     let value = cx.memo(move || value.get()); // `Copy`: read it anywhere
    ///     column((bar(move || value.get()), label(text!("{}", value.get()))))
    /// }
    ///
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
    /// let n = cx.signal(3);
    /// let p: Prop<i32> = n.into_prop();
    /// assert_eq!(p.get(), 3);
    /// let _v = meter(cx, p);
    /// cx.dispose();
    /// ```
    ///
    /// A constant is cloned (no allocation); a dynamic prop runs its closure on every call.
    /// Never panics (unless the closure does).
    #[must_use]
    #[inline]
    pub fn get(&self) -> T {
        match self {
            Prop::Static(v) => v.clone(),
            Prop::Dynamic(f) => f(),
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

/// Anything usable as a property value of type `T`: a value converting into `T` ([`Into`]), or
/// a closure, [`Signal`], [`ReadSignal`] or [`Memo`] whose value converts into `T`.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let n = cx.signal(3);
/// let wide = cx.signal(false);
/// let _v = label("a")
///     .padding(4) // an `i32` constant: `Length::Px(4)`, no binding
///     .width(Length::pct(50))
///     .height(n) // a `Signal<i32>`: one binding converting each value
///     .margin(move || if wide.get() { Length::dp(8) } else { Length::Px(2) });
/// cx.dispose();
/// ```
///
/// Your own types need no declaration: a type is a property of its own type and of every type
/// it converts into through a `From` impl. Color, length, radius, opacity and font properties
/// have a design-value type ([`ColorValue`](twine_style::design::ColorValue), … : a fixed
/// value or a [design element](twine_style::design)), so a type of yours converts into that
/// (conversions do not chain: `From<Brand> for Color` alone is not enough):
///
/// ```
/// use twine_view::prelude::*;
/// use twine_style::design::ColorValue;
///
/// #[derive(Clone, Copy, PartialEq)]
/// pub enum Brand { Primary, Danger }
/// impl From<Brand> for ColorValue {
///     fn from(b: Brand) -> ColorValue {
///         match b {
///             Brand::Primary => design::PRIMARY.into(), // follows the theme
///             Brand::Danger => Color::RED.into(),
///         }
///     }
/// }
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let state = cx.signal(Brand::Primary);
/// let _v = button(label("OK")).bg(Brand::Danger).text_color(state);
/// cx.dispose();
/// ```
///
/// # How the forms are told apart
///
/// The second parameter `M` is a *marker* (see [`marker`]) inferred by the compiler, never
/// written by hand. There is one impl per form: [`marker::Value<U>`] for a constant
/// `U: Into<T>`, [`marker::Closure<U>`] for `F: Fn() -> U`, [`marker::Reactive<U>`] for signals
/// and memos of `U`. Their markers differ, so the impls cannot overlap and one generic impl
/// covers every value type. A function taking a property is generic over the marker:
///
/// ```
/// use twine_view::prelude::*;
///
/// /// A custom component with a `u8` property.
/// fn level<M>(value: impl IntoProp<u8, M>) -> impl View {
///     let value = value.into_prop();
///     label("").op(move |_cx, _node| drop(value))
/// }
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let n = cx.signal(3u8);
/// let _a = level(42);
/// let _b = level(n);
/// let _c = level(move || n.get() / 2);
/// cx.dispose();
/// ```
///
/// Constants are [`Prop::Static`] (applied once, nothing allocated); closures, signals and
/// memos are one boxed closure ([`Prop::Dynamic`]) that converts each value.
///
/// # Integer literals
///
/// An unsuffixed integer literal that several integer types could stand for is an `i32`, so a
/// literal property infers for `i32` and the types converting from it (`Length`, `Radius`,
/// `GridSpan`, …), and for `u8` (the only integer type converting into `u8`). For other
/// integer types (`u16`, `u32`, `usize`, …) several integer types convert and the literal needs
/// a suffix (`3u16`): Twine's own properties use `i32`, `u8` or a unit type for that reason,
/// and so should custom properties that are often written as literals.
///
/// ```compile_fail
/// use twine_view::prelude::*;
/// fn weight<M>(w: impl IntoProp<u16, M>) -> Prop<u16> { w.into_prop() }
/// let _ = weight(3); // error: `u16: From<i32>` is not satisfied
/// ```
///
/// ```
/// use twine_view::prelude::*;
/// fn weight<M>(w: impl IntoProp<u16, M>) -> Prop<u16> { w.into_prop() }
/// let _ = weight(3u16);
/// ```
///
/// For the same reason `Some(5)` is not an `Option<Length>` (there is no
/// `From<Option<i32>> for Option<Length>`): write `Some(Length::Px(5))`, or just
/// `Length::Px(5)` (`From<T> for Option<T>`).
///
/// A closure whose result type would come from the property itself (`move || x.get().into()`)
/// needs a type annotation, as with any `impl Into<T>` parameter: the result may be any type
/// converting into `T`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a property of type `{T}`",
    label = "not convertible into `{T}`, nor a closure, signal or memo of a value convertible into `{T}`",
    note = "a property of type `{T}` accepts every value `V` with `impl From<V> for {T}`, as a constant, a closure result or a signal value"
)]
pub trait IntoProp<T, M> {
    /// The property value.
    fn into_prop(self) -> Prop<T>;
}

/// The markers of the conversion traits' impls, one per form of value: [`IntoProp`],
/// [`IntoText`](crate::IntoText), [`IntoOptions`](crate::IntoOptions) and
/// [`IntoItems`](crate::IntoItems) share them (e.g. `marker::Reactive` marks a signal for all
/// four). They are only type parameters: the
/// compiler infers them; they are never written or created.
pub mod marker {
    use core::marker::PhantomData;

    /// A constant `U: Into<T>`.
    pub struct Value<U>(PhantomData<fn() -> U>);
    /// A closure `Fn() -> U` with `U: Into<T>`.
    pub struct Closure<U>(PhantomData<fn() -> U>);
    /// A [`Signal`](twine_reactive::Signal), [`ReadSignal`](twine_reactive::ReadSignal) or
    /// [`Memo`](twine_reactive::Memo) of `U: Into<T>`.
    pub struct Reactive<U>(PhantomData<fn() -> U>);
    /// A `&'static` slice or array, stored by reference (e.g. a `&'static [T; N]` as a
    /// `&'static [T]`, an unsizing coercion, which has no `From`).
    pub struct Slice(());
    /// A [`Prop`](crate::Prop) (or a [`TextProp`](crate::TextProp)) passed on as is.
    pub struct Prop(());
    /// A reference to a text, cloned ([`IntoText`](crate::IntoText)).
    pub struct Ref<M>(PhantomData<fn() -> M>);
    #[cfg(feature = "display-integrity")]
    /// A [`Validity`](twine_core::validity::Validity) (or a signal or memo of one) shown as a
    /// text ([`IntoText`](crate::IntoText)): `S` is its source.
    pub struct Valid<S>(PhantomData<fn() -> S>);
}

impl<T: 'static, U: Into<T>> IntoProp<T, marker::Value<U>> for U {
    #[inline]
    fn into_prop(self) -> Prop<T> {
        Prop::Static(self.into())
    }
}

impl<T: 'static, U: Into<T>, F: Fn() -> U + 'static> IntoProp<T, marker::Closure<U>> for F {
    #[inline]
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Box::new(move || self().into()))
    }
}

/// The `IntoProp` impls of the reactive handles: one boxed closure that reads and converts.
macro_rules! reactive_props {
    ($($s:ident),+) => {$(
        impl<T: 'static, U: Into<T> + Clone + 'static> IntoProp<T, marker::Reactive<U>> for $s<U> {
            #[inline]
            fn into_prop(self) -> Prop<T> {
                Prop::Dynamic(Box::new(move || self.get().into()))
            }
        }
    )+};
}
reactive_props!(Signal, ReadSignal, Memo);

// ---- Variants ---------------------------------------------------------------------------------

/// A variant of the widget type `W` for the views' [`variant`](crate::ViewExt::variant): a
/// [`Variant`] whose `Widget` is `W` (e.g. a `ButtonVariant` for a button), or a closure,
/// [`Signal`], [`ReadSignal`] or [`Memo`] of one. Like [`IntoProp`] (whose markers it shares),
/// but the widget type picks the variant type, so `.variant(ButtonVariant::Plain)` needs no
/// annotation and a variant of another widget does not compile.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn class<M>(v: impl IntoVariant<Button, M>) -> Prop<&'static WidgetClass> {
///     v.into_variant()
/// }
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let danger = cx.signal(false);
/// assert!(class(ButtonVariant::Filled).get().is(&twine_widgets::button::BUTTON_CLASS));
/// let p = class(move || if danger.get() { ButtonVariant::Destructive } else { ButtonVariant::Gray });
/// assert_eq!(p.get().name, "button.gray");
/// cx.dispose();
/// ```
///
/// Constants are [`Prop::Static`] (no binding); closures and reactive handles one boxed
/// closure.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a variant of `{W}`",
    label = "not a `Variant` with `Widget = {W}`, nor a closure, signal or memo of one",
    note = "a widget's variants are its theme classes: e.g. `ButtonVariant` for a button"
)]
pub trait IntoVariant<W, M> {
    /// The variant's theme class, as a property.
    fn into_variant(self) -> Prop<&'static WidgetClass>;
}

impl<V: Variant> IntoVariant<V::Widget, marker::Value<V>> for V {
    #[inline]
    fn into_variant(self) -> Prop<&'static WidgetClass> {
        Prop::Static(self.class())
    }
}

impl<V: Variant, F: Fn() -> V + 'static> IntoVariant<V::Widget, marker::Closure<V>> for F {
    #[inline]
    fn into_variant(self) -> Prop<&'static WidgetClass> {
        Prop::Dynamic(Box::new(move || self().class()))
    }
}

/// The `IntoVariant` impls of the reactive handles.
macro_rules! reactive_variants {
    ($($s:ident),+) => {$(
        impl<V: Variant> IntoVariant<V::Widget, marker::Reactive<V>> for $s<V> {
            #[inline]
            fn into_variant(self) -> Prop<&'static WidgetClass> {
                Prop::Dynamic(Box::new(move || self.get().class()))
            }
        }
    )+};
}
reactive_variants!(Signal, ReadSignal, Memo);

/// A `'static` array is a slice property (e.g. `animimg(&FRAMES, ..)`).
impl<T: 'static, const N: usize> IntoProp<&'static [T], marker::Slice> for &'static [T; N] {
    #[inline]
    fn into_prop(self) -> Prop<&'static [T]> {
        Prop::Static(self)
    }
}

impl<T> IntoProp<T, marker::Prop> for Prop<T> {
    #[inline]
    fn into_prop(self) -> Prop<T> {
        self
    }
}

// ---- Icons ------------------------------------------------------------------------------------

/// An optional icon of a widget (a list button, a dropdown arrow, the side images of an image
/// button…): `()` for none, a [`Symbol`], an [`ImageSource`], a `&'static` [`Image`], or an
/// `Option` of a symbol or an image source. Icon parameters are properties
/// (`impl IntoProp<Icon, M>`), so a closure, signal or memo of any of these is a reactive
/// icon (a dynamic `None` hides it).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let shown = cx.signal(true);
/// let sym = cx.signal(Symbol::Ok);
/// let _v = list((
///     list_button((), "No icon"),
///     list_button(Symbol::File, "Open"),
///     list_button(sym, "Reactive"),
///     list_button(move || shown.get().then_some(Symbol::Ok), "Shown or hidden"),
/// ));
/// assert_eq!(Icon::from(Symbol::Ok).0, Some(ImageSource::symbol(Symbol::Ok)));
/// cx.dispose();
/// ```
///
/// Constants are [`Prop::Static`]: no binding and no allocation (`()` creates no image node).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Icon(pub Option<ImageSource>);

impl Icon {
    /// No icon.
    pub const NONE: Icon = Icon(None);
}

impl From<()> for Icon {
    /// No icon.
    #[inline]
    fn from((): ()) -> Self {
        Icon(None)
    }
}

impl From<Symbol> for Icon {
    #[inline]
    fn from(s: Symbol) -> Self {
        Icon(Some(ImageSource::symbol(s)))
    }
}

impl From<ImageSource> for Icon {
    #[inline]
    fn from(s: ImageSource) -> Self {
        Icon(Some(s))
    }
}

impl From<&'static Image> for Icon {
    #[inline]
    fn from(img: &'static Image) -> Self {
        Icon(Some(ImageSource::Static(img)))
    }
}

impl From<Option<Symbol>> for Icon {
    #[inline]
    fn from(s: Option<Symbol>) -> Self {
        Icon(s.map(ImageSource::symbol))
    }
}

impl From<Option<ImageSource>> for Icon {
    #[inline]
    fn from(s: Option<ImageSource>) -> Self {
        Icon(s)
    }
}

impl From<Icon> for Option<ImageSource> {
    #[inline]
    fn from(i: Icon) -> Self {
        i.0
    }
}
