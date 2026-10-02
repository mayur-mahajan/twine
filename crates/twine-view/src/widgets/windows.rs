//! Windows and message boxes: [`window`], [`window_button`] and [`msgbox`].

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_engine::{EventCode, EventFilter, EventResult, NodeId, ObjFlags};
use twine_image::{ImageSource, Symbol};
use twine_style::{Align, Length, Selector, StyleProp};
use twine_widgets::button::Button;
use twine_widgets::image::Image;
use twine_widgets::label::Label;
use twine_widgets_ext::msgbox::{self, Msgbox};
use twine_widgets_ext::window::{self, Window};

use crate::access::EngineAccess;
use crate::bind::{bind_node, bind_prop};
use crate::build::{WidgetView, widget_view};
use crate::nav::ModalHandle;
use crate::prop::{IntoProp, Prop};
use crate::style_ext::StyleExt;
use crate::text::{IntoText, TextProp, bind_label_text};
use crate::view::ViewSeq;

// ---- Window ---------------------------------------------------------------------------------

/// A window: a header with `title` (cut with dots when too long) and `header_buttons`
/// (usually [`window_button`]s), above `content`, which scrolls.
///
/// ```
/// use twine_view::prelude::*;
///
/// let _v = window(
///     "Settings",
///     window_button(Symbol::Close, 40).on_click(|| {}),
///     column((label("Volume"), slider(50))),
/// );
/// ```
pub fn window<MT>(
    title: impl IntoText<MT>,
    header_buttons: impl ViewSeq,
    content: impl ViewSeq,
) -> WidgetView<Window> {
    let title = title.into_text();
    widget_view(Window::new).op(move |cx, node| {
        let Some((header, body)) = cx
            .engine()
            .widget::<Window>(node)
            .map(|w| (w.header(), w.content()))
        else {
            return;
        };
        let t = cx.with_parent(header, |cx| cx.create(Label::new("")));
        bind_label_text(cx, t, title);
        window::init_title(cx.engine(), t);
        cx.with_parent(header, |cx| header_buttons.build_seq(cx));
        cx.with_parent(body, |cx| content.build_seq(cx));
    })
}

impl WidgetView<Window> {
    /// The header height (default: the display's DPI / 2).
    #[must_use]
    pub fn header_height<M>(self, h: impl IntoProp<i32, M>) -> Self {
        self.bind(h, |w: &mut Window, cx, h| w.set_header_height(cx, h))
    }

    /// The padding of the content area on all sides (the theme's by default), e.g. 0 for a
    /// tabview filling the window.
    #[must_use]
    pub fn content_padding<M>(self, pad: impl IntoProp<i32, M>) -> Self {
        let pad = pad.into_prop();
        self.op(move |cx, node| {
            let Some(c) = cx.engine().widget::<Window>(node).map(Window::content) else {
                return;
            };
            bind_node(cx, c, pad, |e, c, pad| {
                for p in [
                    StyleProp::PaddingTop(Length::Px(pad).into()),
                    StyleProp::PaddingBottom(Length::Px(pad).into()),
                    StyleProp::PaddingLeft(Length::Px(pad).into()),
                    StyleProp::PaddingRight(Length::Px(pad).into()),
                ] {
                    e.set_local_prop(c, Selector::MAIN, p);
                }
            });
        })
    }
}

/// A header button of a [`window()`]: `width` px wide, as high as the header, `icon` (a
/// [`Symbol`] or an image) centered.
/// Unlike the other icon setters (which take any [`Icon`](crate::Icon), so also `()` for
/// none), `icon` is an [`ImageSource`]: a header button always shows one.
///
/// ```
/// use twine_view::prelude::*;
/// let _v = window(
///     "Settings",
///     (window_button(Symbol::Close, 40).on_click(|| {}),),
///     label("content"),
/// );
/// ```
pub fn window_button<M1, M2>(
    icon: impl IntoProp<ImageSource, M1>,
    width: impl IntoProp<i32, M2>,
) -> WidgetView<Button> {
    let icon = icon.into_prop();
    widget_view(Button::new)
        .style_prop(width, |w: i32| StyleProp::Width(Length::Px(w).into()))
        .op(move |cx, b| {
            cx.engine()
                .set_local_prop(b, Selector::MAIN, StyleProp::Height(Length::pct(100).into()));
            let img = cx.with_parent(b, |cx| cx.create(Image::new()));
            cx.engine().align(img, Align::Center, 0, 0);
            bind_prop(cx, img, icon, |i: &mut Image, wcx, src| i.set_src(wcx, src));
        })
}

// ---- Msgbox ---------------------------------------------------------------------------------

/// A footer button callback of a [`msgbox`].
type ButtonFn = Box<dyn FnMut(usize)>;

/// The builder settings of a [`msgbox`] view.
#[derive(Default)]
struct MsgboxSettings {
    buttons: RefCell<Vec<TextProp>>,
    close_button: RefCell<Option<Prop<bool>>>,
    on_button: RefCell<Option<ButtonFn>>,
    on_close: RefCell<Option<Box<dyn FnMut()>>>,
}

/// A message box with `title` (no header when it is `""` and there is no close button) and
/// `text`; show it with [`show_modal`](crate::ScopeExt::show_modal).
///
/// Clicking a footer button calls [`on_button`](WidgetView::on_button) with its index; the
/// application decides whether to close (through the [`ModalHandle`]). The close button
/// closes the modal (or deletes the box outside one) and calls
/// [`on_close`](WidgetView::on_close).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     button(label("Delete")).on_click(move || {
///         let m = cx.show_modal(|_, _| {
///             msgbox("Delete?", "This cannot be undone.").buttons(["Yes", "No"]).close_button(true)
///         });
///         let _ = m;
///     })
/// }
/// # let _ = app;
/// ```
pub fn msgbox<MT1, MT2>(title: impl IntoText<MT1>, text: impl IntoText<MT2>) -> WidgetView<Msgbox> {
    msgbox_view(title.into_text(), text.into_text())
}

/// [`msgbox`] after the conversions (not generic: one copy whatever the argument types).
fn msgbox_view(title: TextProp, text: TextProp) -> WidgetView<Msgbox> {
    let mut v = widget_view(Msgbox::new);
    let settings = v.shared::<MsgboxSettings>();
    v.after_children(move |cx, node| {
        let rt = cx.runtime();
        let modal = cx.scope().use_context::<ModalHandle>();
        // No close button for `false` (or none given); a hidden one for a dynamic value.
        let close_button = match settings.close_button.borrow_mut().take() {
            None | Some(Prop::Static(false)) => None,
            Some(p) => Some(p),
        };
        let has_header = close_button.is_some() || !matches!(title, TextProp::Static(""));
        let e = cx.engine();
        if has_header {
            let t = e
                .with_widget_mut(node, |m: &mut Msgbox, wcx| m.add_title(wcx, ""))
                .flatten();
            if let Some(t) = t {
                bind_label_text(cx, t, title);
            }
        }
        let l = cx
            .engine()
            .with_widget_mut(node, |m: &mut Msgbox, wcx| m.add_text(wcx, ""))
            .flatten();
        if let Some(l) = l {
            bind_label_text(cx, l, text);
        }
        let mut buttons: Vec<NodeId> = Vec::new();
        for text in settings.buttons.borrow_mut().drain(..) {
            let Some(b) = cx
                .engine()
                .with_widget_mut(node, |m: &mut Msgbox, wcx| m.add_footer_button(wcx, ""))
                .flatten()
            else {
                continue;
            };
            let l = cx.with_parent(b, |cx| cx.create(Label::new("")));
            cx.engine().align(l, Align::Center, 0, 0);
            bind_label_text(cx, l, text);
            buttons.push(b);
        }
        let e = cx.engine();
        for (idx, b) in buttons.into_iter().enumerate() {
            let s = settings.clone();
            e.add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                if ev.target == ev.current_target {
                    twine_core::debug!(target: "twine::view", "msgbox button {}", idx);
                    if let Some(f) = s.on_button.borrow_mut().as_mut() {
                        EngineAccess::provide(rt, ecx.engine_mut(), || f(idx));
                    }
                }
                EventResult::Continue
            });
        }
        if let Some(shown) = close_button {
            let close = e
                .with_widget_mut(node, |m: &mut Msgbox, wcx| {
                    m.add_header_button(wcx, Some(ImageSource::symbol(Symbol::Close)))
                })
                .flatten();
            if let Some(c) = close {
                if let Prop::Dynamic(_) = shown {
                    bind_node(cx, c, shown, |e, c, on| e.set_flag(c, ObjFlags::HIDDEN, !on));
                }
                let e = cx.engine();
                let s = settings.clone();
                e.add_event_handler(c, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                    if ev.target == ev.current_target {
                        if let Some(f) = s.on_close.borrow_mut().as_mut() {
                            EngineAccess::provide(rt, ecx.engine_mut(), &mut *f);
                        }
                        match &modal {
                            Some(m) => EngineAccess::provide(rt, ecx.engine_mut(), || m.close()),
                            // The box itself is busy only inside its own event, not here.
                            None => msgbox::close_async(ecx.engine_mut(), node),
                        }
                    }
                    EventResult::Continue
                });
            }
        }
    })
}

impl WidgetView<Msgbox> {
    /// The footer buttons' texts (each any [`IntoText`], e.g. a translation).
    #[must_use]
    pub fn buttons<T: IntoText<MT>, MT>(mut self, texts: impl IntoIterator<Item = T>) -> Self {
        *self.shared::<MsgboxSettings>().buttons.borrow_mut() =
            texts.into_iter().map(IntoText::into_text).collect();
        self
    }

    /// A close button in the header (a dynamic value creates the button and hides it while
    /// `false`).
    #[must_use]
    pub fn close_button<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        *self.shared::<MsgboxSettings>().close_button.borrow_mut() = Some(on.into_prop());
        self
    }

    /// Called with the index of the footer button clicked.
    #[must_use]
    pub fn on_button(mut self, f: impl FnMut(usize) + 'static) -> Self {
        *self.shared::<MsgboxSettings>().on_button.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Called when the close button closes the box.
    #[must_use]
    pub fn on_close<R>(mut self, mut f: impl FnMut() -> R + 'static) -> Self {
        *self.shared::<MsgboxSettings>().on_close.borrow_mut() = Some(Box::new(move || {
            let _ = f();
        }));
        self
    }
}
