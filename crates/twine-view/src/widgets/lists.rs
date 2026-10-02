//! Lists: [`list`], [`list_text`] and [`list_button`].

use twine_engine::ObjFlags;
use twine_widgets::button::Button;
use twine_widgets::image::Image;
use twine_widgets::label::Label;
use twine_widgets_ext::list::{self, LIST_BUTTON_CLASS, List};

use crate::bind::bind_node;
use crate::build::{WidgetView, widget_view};
use crate::prop::{Icon, IntoProp, Prop};
use crate::text::{IntoText, TextProp, bind_label_text};
use crate::view::ViewSeq;

/// A list: a scrollable card whose `items` (usually [`list_text`] headers and [`list_button`]s,
/// also built by [`for_each`](crate::for_each)) stack in a column.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn files(cx: Scope) -> impl View {
///     let names = cx.signal(vec!["a.txt".to_string(), "b.txt".to_string()]);
///     list((
///         list_text("Files"),
///         for_each(move || names.get(), |n| n.clone(), |_, n| {
///             list_button(Symbol::File, n)
///         }),
///     ))
/// }
/// # let _ = files;
/// ```
pub fn list(items: impl ViewSeq) -> WidgetView<List> {
    widget_view(|| List).children(items)
}

/// A section header of a [`list()`]: a label of class `"list_text"`, 100 % wide, scrolling
/// circularly when too long. All label modifiers apply.
pub fn list_text<MT>(text: impl IntoText<MT>) -> WidgetView<Label> {
    let v = match text.into_text() {
        TextProp::Static(s) => widget_view(move || list::text_label(s)),
        other => widget_view(|| list::text_label("")).op(move |cx, n| bind_label_text(cx, n, other)),
    };
    v.op(|cx, n| list::init_text(cx.engine(), n))
}

/// A button of a [`list()`]: an optional `icon` (any [`Icon`]: `()` for none, a
/// [`Symbol`](twine_text::Symbol), an image, or a signal/closure of one, which hides the image
/// while `None`) and `text` in a row, the text scrolling circularly when too long.
/// All button modifiers apply (`on_click`, `checkable`…).
/// ```
/// use twine_view::prelude::*;
/// let _v = list((
///     list_text("Settings"),
///     list_button(Symbol::Wifi, "Network").on_click(|| {}),
///     list_button((), "About"), // no icon
/// ));
/// ```
pub fn list_button<M, MT>(icon: impl IntoProp<Icon, M>, text: impl IntoText<MT>) -> WidgetView<Button> {
    list_button_view(icon.into_prop(), text.into_text())
}

/// [`list_button`] after the conversions (not generic: one copy whatever the argument types).
fn list_button_view(icon: Prop<Icon>, text: TextProp) -> WidgetView<Button> {
    widget_view(|| Button::with_class(&LIST_BUTTON_CLASS)).op(move |cx, btn| {
        list::init_button(cx.engine(), btn);
        if !matches!(icon, Prop::Static(Icon(None))) {
            let img = cx.with_parent(btn, |cx| cx.create(Image::new()));
            bind_node(cx, img, icon, |e, img, Icon(src): Icon| {
                e.set_flag(img, ObjFlags::HIDDEN, src.is_none());
                if let Some(src) = src {
                    e.with_widget_mut(img, |i: &mut Image, wcx| i.set_src(wcx, src));
                }
            });
        }
        let lbl = cx.with_parent(btn, |cx| cx.create(Label::new("")));
        bind_label_text(cx, lbl, text);
        list::init_button_label(cx.engine(), lbl);
    })
}
