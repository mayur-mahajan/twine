//! Text properties: [`IntoText`], [`TextProp`] and the zero-allocation [`text!`](crate::text!)
//! macro.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use twine_engine::NodeId;
use twine_reactive::{Memo, ReadSignal, Scope, Signal};
use twine_widgets::label::Label;

use crate::bind::bind_effect;
use crate::build::BuildCx;
use crate::prop::marker;

/// A function writing a text into a formatter.
pub type TextWriter = dyn Fn(&mut dyn Write);

/// A text property value.
pub enum TextProp {
    /// A `'static` text (stored without copying).
    Static(&'static str),
    /// An owned text (copied into the widget once).
    Owned(String),
    /// A closure producing the text (re-run when a signal it reads changes).
    Fn(Box<dyn Fn() -> String>),
    /// A writer formatting the text in place ([`text!`](crate::text!)); no allocation once the
    /// widget's buffers are large enough.
    Write(Box<TextWriter>),
    /// A closure choosing one of several `'static` texts (re-run when a signal it reads
    /// changes); the widget stores the chosen text without copying (e.g. a translation).
    StaticFn(Box<dyn Fn() -> &'static str>),
    /// A text that depends on the scope the widget is built in (resolved once at build time,
    /// e.g. to look up a context such as the translations of `tr!`).
    Scoped(Box<dyn FnOnce(Scope) -> TextProp>),
}

impl core::fmt::Debug for TextProp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TextProp::Static(s) => f.debug_tuple("Static").field(s).finish(),
            TextProp::Owned(s) => f.debug_tuple("Owned").field(s).finish(),
            TextProp::Fn(_) => f.write_str("Fn"),
            TextProp::Write(_) => f.write_str("Write"),
            TextProp::StaticFn(_) => f.write_str("StaticFn"),
            TextProp::Scoped(_) => f.write_str("Scoped"),
        }
    }
}

/// Anything usable as a text: a `&'static str`, a [`Symbol`](twine_text::Symbol), a
/// `String` or a reference to any of these (copied), [`text!`](crate::text!), a closure returning a
/// `String`, a `&'static str` or a `Symbol`, or a `Signal`, `ReadSignal` or `Memo` of any
/// `AsRef<str>` type (`String`, `&'static str`, …).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let name = cx.signal(String::from("Ada"));
/// let n = cx.signal(3);
/// let on = cx.signal(true);
/// let _a = label("static");
/// let _b = label(String::from("owned"));
/// let _c = label(name);
/// let _d = label(move || format!("{} items", n.get()));
/// let _e = label(text!("{} items", n.get())); // formats in place, no allocation
/// let _f = label(move || if on.get() { "On" } else { "Off" }); // stored without copying
/// cx.dispose();
/// ```
///
/// `M` is a marker inferred by the compiler, as for [`IntoProp`](crate::IntoProp): it tells
/// the forms apart (e.g. closures returning a `String` from closures returning a
/// `&'static str`, which a single-parameter trait could not).
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a text",
    label = "not a `&'static str`, `Symbol`, `String`, `text!`, a closure returning a `String`, `&'static str` or `Symbol`, or a signal or memo of a string",
    note = "format other values with `text!` (no allocation) or a closure returning a `String`"
)]
pub trait IntoText<M> {
    /// The text property.
    fn into_text(self) -> TextProp;
}

impl IntoText<marker::Value<&'static str>> for &'static str {
    fn into_text(self) -> TextProp {
        TextProp::Static(self)
    }
}

impl IntoText<marker::Value<twine_text::Symbol>> for twine_text::Symbol {
    /// The symbol's glyph as a static text (no allocation): `label(Symbol::Ok)`.
    #[inline]
    fn into_text(self) -> TextProp {
        TextProp::Static(self.as_str())
    }
}

impl IntoText<marker::Value<String>> for String {
    fn into_text(self) -> TextProp {
        TextProp::Owned(self)
    }
}

impl IntoText<marker::Value<TextFn>> for TextFn {
    fn into_text(self) -> TextProp {
        TextProp::Write(self.0)
    }
}

impl IntoText<marker::Prop> for TextProp {
    fn into_text(self) -> TextProp {
        self
    }
}

/// A reference to a text (e.g. the items of a `&'static [&'static str]`, or `&String`): the
/// referenced value is cloned.
impl<T: IntoText<M> + Clone, M> IntoText<marker::Ref<M>> for &T {
    #[inline]
    fn into_text(self) -> TextProp {
        self.clone().into_text()
    }
}

impl<F: Fn() -> String + 'static> IntoText<marker::Closure<String>> for F {
    fn into_text(self) -> TextProp {
        TextProp::Fn(Box::new(self))
    }
}

/// A closure choosing a `'static` text (e.g. a translation): the widget stores the chosen text
/// without copying.
impl<F: Fn() -> &'static str + 'static> IntoText<marker::Closure<&'static str>> for F {
    fn into_text(self) -> TextProp {
        TextProp::StaticFn(Box::new(self))
    }
}

impl<F: Fn() -> twine_text::Symbol + 'static> IntoText<marker::Closure<twine_text::Symbol>> for F {
    fn into_text(self) -> TextProp {
        TextProp::StaticFn(Box::new(move || self().as_str()))
    }
}

/// The `IntoText` impls of the reactive handles: the text is written from the handle's value
/// in place (`with`, no clone).
macro_rules! reactive_texts {
    ($($s:ident),+) => {$(
        impl<S: AsRef<str> + 'static> IntoText<marker::Reactive<S>> for $s<S> {
            fn into_text(self) -> TextProp {
                TextProp::Write(Box::new(move |w| {
                    self.with(|s| {
                        let _ = w.write_str(s.as_ref());
                    });
                }))
            }
        }
    )+};
}
reactive_texts!(Signal, ReadSignal, Memo);

/// A text writer: the value [`text!`](crate::text!) expands to.
pub struct TextFn(pub Box<TextWriter>);

impl core::fmt::Debug for TextFn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TextFn")
    }
}

impl TextFn {
    /// Wraps a writer.
    #[must_use]
    pub fn new(f: impl Fn(&mut dyn Write) + 'static) -> Self {
        TextFn(Box::new(f))
    }
}

/// A dynamic text formatted like `format!`, re-run only when a signal read in its arguments
/// changes, written in place into the label's own buffers: no allocation after warm-up, and
/// nothing happens when the result equals the current text.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let temp = cx.signal(21.5f32);
/// let _v = label(text!("{:.1} °C", temp.get()));
/// cx.dispose();
/// ```
#[macro_export]
macro_rules! text {
    ($($arg:tt)*) => {
        $crate::text::TextFn($crate::__private::Box::new(move |__w: &mut dyn ::core::fmt::Write| {
            let _ = ::core::write!(__w, $($arg)*);
        }))
    };
}

/// Applies a text property to a [`Label`] node (the label may have been created with the
/// static text already: `Static` is then a no-op).
pub(crate) fn bind_label_text(cx: &mut BuildCx<'_>, node: NodeId, text: TextProp) {
    match text {
        TextProp::Scoped(f) => {
            let t = f(cx.scope());
            bind_label_text(cx, node, t);
        }
        TextProp::StaticFn(f) => {
            let scope = cx.scope();
            cx.provide(|| {
                bind_effect(scope, node, f, |e, n, s: &'static str| {
                    e.with_widget_mut(n, |l: &mut Label, wcx| l.set_text_static(wcx, s));
                });
            });
        }
        TextProp::Static(s) => {
            cx.engine()
                .with_widget_mut(node, |l: &mut Label, wcx| l.set_text_static(wcx, s));
        }
        TextProp::Owned(s) => {
            cx.engine()
                .with_widget_mut(node, |l: &mut Label, wcx| l.set_text(wcx, &s));
        }
        TextProp::Fn(f) => {
            let scope = cx.scope();
            cx.provide(|| {
                bind_effect(scope, node, f, |e, n, s: String| {
                    e.with_widget_mut(n, |l: &mut Label, wcx| l.set_text(wcx, &s));
                });
            });
        }
        TextProp::Write(w) => {
            let scope = cx.scope();
            // The writer runs inside the setter (its reads are tracked by the binding).
            let w: alloc::rc::Rc<TextWriter> = w.into();
            cx.provide(|| {
                bind_effect(
                    scope,
                    node,
                    move || w.clone(),
                    |e, n, w: alloc::rc::Rc<TextWriter>| {
                        e.with_widget_mut(n, |l: &mut Label, wcx| {
                            l.write_text(wcx, |buf| w(buf));
                        });
                    },
                );
            });
        }
    }
}

/// A text handed to a widget text setter by [`bind_str`]: a `'static` text the widget can
/// store without copying, or a borrowed one it copies.
#[derive(Clone, Copy)]
pub(crate) enum TextRef<'a> {
    /// A `'static` text (store the reference).
    Static(&'static str),
    /// A borrowed text (copy it; ideally only when it differs from the current one).
    Borrowed(&'a str),
}

/// Applies a text property to any widget text through `set` (which chooses the widget's
/// no-copy setter for [`TextRef::Static`]). A constant text is set at once; a dynamic one
/// becomes a binding; [`text!`](crate::text!) and signal texts format into a scratch string
/// owned by the binding (no allocation once it is large enough). `set` should be idempotent.
///
/// Only the constant case and one binding are generic (per call site): the dynamic text
/// sources are evaluated by the shared, non-generic [`DynText::eval`] (a direct call).
pub(crate) fn bind_str(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    text: TextProp,
    set: impl Fn(&mut twine_engine::Engine, NodeId, TextRef<'_>) + 'static,
) {
    let src = match text.resolve(cx.scope()) {
        TextProp::Static(s) => return set(cx.engine(), node, TextRef::Static(s)),
        TextProp::Owned(s) => return set(cx.engine(), node, TextRef::Borrowed(&s)),
        TextProp::StaticFn(f) => DynText::StaticFn(f),
        TextProp::Fn(f) => DynText::Fn(f),
        TextProp::Write(w) => DynText::Write(w, alloc::rc::Rc::default()),
        // `resolve` resolved it.
        TextProp::Scoped(_) => return,
    };
    let scope = cx.scope();
    cx.provide(|| {
        bind_effect(
            scope,
            node,
            move || src.eval(),
            move |e, n, v: TextValue| match v {
                TextValue::Static(s) => set(e, n, TextRef::Static(s)),
                TextValue::Owned(s) => set(e, n, TextRef::Borrowed(&s)),
                TextValue::Scratch(b) => set(e, n, TextRef::Borrowed(&b.borrow())),
            },
        );
    });
}

/// The source of a dynamic text bound by [`bind_str`].
enum DynText {
    /// [`TextProp::StaticFn`].
    StaticFn(Box<dyn Fn() -> &'static str>),
    /// [`TextProp::Fn`].
    Fn(Box<dyn Fn() -> String>),
    /// [`TextProp::Write`] and the binding's reused scratch string.
    Write(Box<TextWriter>, alloc::rc::Rc<core::cell::RefCell<String>>),
}

/// One evaluation of a [`DynText`].
enum TextValue {
    Static(&'static str),
    Owned(String),
    Scratch(alloc::rc::Rc<core::cell::RefCell<String>>),
}

impl DynText {
    /// Evaluates the text (the signal reads are tracked by the running binding).
    #[inline(never)]
    fn eval(&self) -> TextValue {
        match self {
            DynText::StaticFn(f) => TextValue::Static(f()),
            DynText::Fn(f) => TextValue::Owned(f()),
            DynText::Write(w, scratch) => {
                {
                    let mut b = scratch.borrow_mut();
                    b.clear();
                    w(&mut *b);
                }
                TextValue::Scratch(scratch.clone())
            }
        }
    }
}

impl TextProp {
    /// Resolves a [`TextProp::Scoped`] text (repeatedly) in `scope`; other texts are returned
    /// as they are.
    pub(crate) fn resolve(self, scope: Scope) -> TextProp {
        let mut t = self;
        while let TextProp::Scoped(f) = t {
            t = f(scope);
        }
        t
    }

    /// Whether the text never changes (no binding needed).
    pub(crate) fn is_constant(&self) -> bool {
        matches!(self, TextProp::Static(_) | TextProp::Owned(_))
    }

    /// Writes the current text into `w` (a [`TextProp::Scoped`] text, not resolved, writes
    /// nothing). Reads of signals are tracked by the running binding.
    pub(crate) fn write_to(&self, w: &mut dyn Write) {
        let _ = match self {
            TextProp::Static(s) => w.write_str(s),
            TextProp::Owned(s) => w.write_str(s),
            TextProp::StaticFn(f) => w.write_str(f()),
            TextProp::Fn(f) => w.write_str(&f()),
            TextProp::Write(f) => {
                f(w);
                Ok(())
            }
            TextProp::Scoped(_) => Ok(()),
        };
    }
}

/// Joins texts with `'\n'` into one text: an owned text when every item is constant (joined
/// once), else a writer re-run as one binding (the items' reads are tracked; no allocation
/// once the binding's buffer is large enough, unless an item is a `String` closure).
/// [`TextProp::Scoped`] items are resolved when the text is applied.
pub(crate) fn join_texts(items: Vec<TextProp>) -> TextProp {
    if items.iter().any(|t| matches!(t, TextProp::Scoped(_))) {
        return TextProp::Scoped(Box::new(move |cx| {
            join_texts(items.into_iter().map(|t| t.resolve(cx)).collect())
        }));
    }
    let write_all = |items: &[TextProp], w: &mut dyn Write| {
        for (i, t) in items.iter().enumerate() {
            if i > 0 {
                let _ = w.write_char('\n');
            }
            t.write_to(w);
        }
    };
    match items.as_slice() {
        [] => TextProp::Static(""),
        [TextProp::Static(s)] => TextProp::Static(s),
        _ if items.iter().all(TextProp::is_constant) => {
            let mut s = String::new();
            write_all(&items, &mut s);
            TextProp::Owned(s)
        }
        _ => TextProp::Write(Box::new(move |w| write_all(&items, w))),
    }
}

/// Writes `items` joined with `'\n'`.
fn write_joined<S: AsRef<str>>(items: &[S], w: &mut dyn Write) {
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            let _ = w.write_char('\n');
        }
        let _ = w.write_str(s.as_ref());
    }
}

/// The options of a [`dropdown`](crate::dropdown) or [`roller`](crate::roller): a fixed list
/// of texts (anything iterable — an array, a `Vec`, a `&'static` slice, an iterator — of
/// anything [`IntoText`]; each item may itself be dynamic, e.g. a translation) or a reactive
/// list (a `Signal`, `ReadSignal` or `Memo` of a `Vec` of strings, a closure returning one, or
/// a [`Prop`](crate::Prop)). `M` is an inferred marker, as for [`IntoText`].
///
/// The widget stores the options as one `'\n'`-joined text (an option must not contain
/// `'\n'`): a fixed list of constant texts is joined once when the view is built; dynamic
/// items and reactive lists are joined by one binding into a reused buffer (no allocation
/// once it is large enough), and the widget copies the result only when it changed.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn pickers(cx: Scope) -> impl View {
///     let sel = cx.signal(0usize);
///     let names = cx.signal(vec![String::from("Ada"), String::from("Grace")]);
///     column((
///         dropdown(["Low", "Medium", "High"], sel),
///         dropdown(names, 0usize),
///         roller((0..24).map(|h| format!("{h:02}")), 7),
///         roller((1..=4).filter(|q| q % 2 == 0).map(|q| format!("{q} h")), 0),
///         dropdown(vec![text!("{} items", sel.get()), text!("none")], 0usize),
///     ))
/// }
/// # let _ = pickers;
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as the options of a dropdown or roller",
    note = "use an array, a `Vec`, a slice or an iterator of texts, or a signal, memo or closure of a `Vec` of strings"
)]
pub trait IntoOptions<M> {
    /// The options as one `'\n'`-joined text.
    fn into_options(self) -> TextProp;
}

impl<I: IntoIterator, M> IntoOptions<marker::Value<M>> for I
where
    I::Item: IntoText<M>,
{
    fn into_options(self) -> TextProp {
        join_texts(self.into_iter().map(IntoText::into_text).collect())
    }
}

/// The `IntoOptions` impls of the reactive handles: one binding writes the joined list from
/// the handle's value in place (`with`, no clone).
macro_rules! reactive_options {
    ($($s:ident),+) => {$(
        impl<S: AsRef<str> + 'static> IntoOptions<marker::Reactive<S>> for $s<Vec<S>> {
            fn into_options(self) -> TextProp {
                TextProp::Write(Box::new(move |w| self.with(|v| write_joined(v, w))))
            }
        }
    )+};
}
reactive_options!(Signal, ReadSignal, Memo);

impl<S: AsRef<str> + 'static, F: Fn() -> Vec<S> + 'static> IntoOptions<marker::Closure<S>> for F {
    fn into_options(self) -> TextProp {
        TextProp::Write(Box::new(move |w| write_joined(&self(), w)))
    }
}

impl<S: AsRef<str> + 'static> IntoOptions<marker::Prop> for crate::Prop<Vec<S>> {
    fn into_options(self) -> TextProp {
        match self {
            crate::Prop::Static(v) => {
                let mut s = String::new();
                write_joined(&v, &mut s);
                TextProp::Owned(s)
            }
            crate::Prop::Dynamic(f) => TextProp::Write(Box::new(move |w| write_joined(&f(), w))),
        }
    }
}
