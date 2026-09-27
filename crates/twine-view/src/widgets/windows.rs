//! Windows and message boxes: [`window`], [`window_button`] and [`msgbox`].

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use twine_engine::{EventCode, EventFilter, EventResult, NodeId};
use twine_image::ImageSource;
use twine_style::{Align, Length, Selector, StyleProp};
use twine_widgets::button::Button;
use twine_widgets::image::Image;
use twine_widgets::label::Label;
use twine_widgets_ext::msgbox::{self, Msgbox};
use twine_widgets_ext::window::{self, Window};

use crate::access::EngineAccess;
use crate::build::{WidgetView, widget_view};
use crate::nav::ModalHandle;
use crate::prop::IntoProp;
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
///     window_button(ImageSource::Symbol(symbols::CLOSE), 40).on_click(|| {}),
///     column((label("Volume"), slider(50))),
/// );
/// ```
pub fn window(
    title: impl IntoText,
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
    pub fn header_height(self, h: impl IntoProp<i32>) -> Self {
        self.bind(h, |w: &mut Window, cx, h| w.set_header_height(cx, h))
    }

    /// The padding of the content area on all sides (the theme's by default), e.g. 0 for a
    /// tabview filling the window.
    #[must_use]
    pub fn content_padding(self, pad: i32) -> Self {
        self.op(move |cx, node| {
            let Some(c) = cx.engine().widget::<Window>(node).map(Window::content) else {
                return;
            };
            for p in [
                StyleProp::PadTop(pad),
                StyleProp::PadBottom(pad),
                StyleProp::PadLeft(pad),
                StyleProp::PadRight(pad),
            ] {
                cx.engine().set_local_prop(c, Selector::MAIN, p);
            }
        })
    }
}

/// A header button of a [`window()`]: `width` px wide, as high as the header, `icon` centered.
pub fn window_button(icon: ImageSource, width: i32) -> WidgetView<Button> {
    widget_view(Button::new).op(move |cx, b| {
        cx.engine().set_size(b, width, Length::pct(100));
        let img = cx.with_parent(b, |cx| cx.create(Image::new()));
        let e = cx.engine();
        e.with_widget_mut(img, |i: &mut Image, wcx| i.set_src(wcx, icon));
        e.align(img, Align::Center, 0, 0);
    })
}

// ---- Msgbox ---------------------------------------------------------------------------------

/// A footer button callback of a [`msgbox`].
type ButtonFn = Box<dyn FnMut(usize)>;

/// The builder settings of a [`msgbox`] view.
#[derive(Default)]
struct MsgboxSettings {
    buttons: Cell<&'static [&'static str]>,
    close_button: Cell<bool>,
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
///         let m = cx.show_modal(|_| {
///             msgbox("Delete?", "This cannot be undone.").buttons(&["Yes", "No"]).close_button(true)
///         });
///         let _ = m;
///     })
/// }
/// # let _ = app;
/// ```
pub fn msgbox(title: impl IntoText, text: impl IntoText) -> WidgetView<Msgbox> {
    let title = title.into_text();
    let text = text.into_text();
    let mut v = widget_view(Msgbox::new);
    let settings = v.shared::<MsgboxSettings>();
    v.after_children(move |cx, node| {
        let modal = cx.scope().use_context::<ModalHandle>();
        let has_header = settings.close_button.get() || !matches!(title, TextProp::Static(""));
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
        let e = cx.engine();
        let mut buttons: Vec<NodeId> = Vec::new();
        for label in settings.buttons.get() {
            if let Some(b) = e
                .with_widget_mut(node, |m: &mut Msgbox, wcx| m.add_footer_button(wcx, label))
                .flatten()
            {
                buttons.push(b);
            }
        }
        for (idx, b) in buttons.into_iter().enumerate() {
            let s = settings.clone();
            e.add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                if ev.target == ev.current_target {
                    twine_core::debug!(target: "twine::view", "msgbox button {}", idx);
                    if let Some(f) = s.on_button.borrow_mut().as_mut() {
                        EngineAccess::provide(ecx.engine_mut(), || f(idx));
                    }
                }
                EventResult::Continue
            });
        }
        if settings.close_button.get() {
            let close = e
                .with_widget_mut(node, |m: &mut Msgbox, wcx| {
                    m.add_header_button(wcx, Some(ImageSource::Symbol(twine_text::symbols::CLOSE)))
                })
                .flatten();
            if let Some(c) = close {
                let s = settings.clone();
                e.add_event_handler(c, EventFilter::Code(EventCode::Clicked), move |ecx, ev| {
                    if ev.target == ev.current_target {
                        if let Some(f) = s.on_close.borrow_mut().as_mut() {
                            EngineAccess::provide(ecx.engine_mut(), &mut *f);
                        }
                        match &modal {
                            Some(m) => EngineAccess::provide(ecx.engine_mut(), || m.close()),
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
    /// The footer buttons' texts.
    #[must_use]
    pub fn buttons(mut self, texts: &'static [&'static str]) -> Self {
        self.shared::<MsgboxSettings>().buttons.set(texts);
        self
    }

    /// A close button in the header.
    #[must_use]
    pub fn close_button(mut self, on: bool) -> Self {
        self.shared::<MsgboxSettings>().close_button.set(on);
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
