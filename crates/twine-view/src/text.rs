//! Text properties: [`IntoText`], [`TextProp`] and the zero-allocation [`text!`](crate::text!)
//! macro.

use alloc::boxed::Box;
use alloc::string::String;
use core::fmt::Write;

use twine_engine::NodeId;
use twine_reactive::{Memo, ReadSignal, Signal};
use twine_widgets::label::Label;

use crate::bind::bind_effect;
use crate::build::BuildCx;

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
}

impl core::fmt::Debug for TextProp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TextProp::Static(s) => f.debug_tuple("Static").field(s).finish(),
            TextProp::Owned(s) => f.debug_tuple("Owned").field(s).finish(),
            TextProp::Fn(_) => f.write_str("Fn"),
            TextProp::Write(_) => f.write_str("Write"),
        }
    }
}

/// Anything usable as a text: `&'static str`, `String`, [`text!`](crate::text!),
/// `Signal<String>`, `ReadSignal<String>`, `Memo<String>` and `F: Fn() -> String`.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let name = cx.signal(String::from("Ada"));
/// let n = cx.signal(3);
/// let _a = label("static");
/// let _b = label(String::from("owned"));
/// let _c = label(name);
/// let _d = label(move || format!("{} items", n.get()));
/// let _e = label(text!("{} items", n.get())); // formats in place, no allocation
/// cx.dispose();
/// ```
pub trait IntoText {
    /// The text property.
    fn into_text(self) -> TextProp;
}

impl IntoText for &'static str {
    fn into_text(self) -> TextProp {
        TextProp::Static(self)
    }
}

impl IntoText for String {
    fn into_text(self) -> TextProp {
        TextProp::Owned(self)
    }
}

impl IntoText for TextProp {
    fn into_text(self) -> TextProp {
        self
    }
}

impl<F: Fn() -> String + 'static> IntoText for F {
    fn into_text(self) -> TextProp {
        TextProp::Fn(Box::new(self))
    }
}

impl IntoText for Signal<String> {
    fn into_text(self) -> TextProp {
        TextProp::Write(Box::new(move |w| {
            self.with(|s| {
                let _ = w.write_str(s);
            });
        }))
    }
}

impl IntoText for ReadSignal<String> {
    fn into_text(self) -> TextProp {
        TextProp::Write(Box::new(move |w| {
            self.with(|s| {
                let _ = w.write_str(s);
            });
        }))
    }
}

impl IntoText for Memo<String> {
    fn into_text(self) -> TextProp {
        TextProp::Write(Box::new(move |w| {
            self.with(|s| {
                let _ = w.write_str(s);
            });
        }))
    }
}

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

impl IntoText for TextFn {
    fn into_text(self) -> TextProp {
        TextProp::Write(self.0)
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
