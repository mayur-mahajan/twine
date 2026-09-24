//! The core widget views: [`label`], [`button`], [`image`].

use core::cell::Cell;

use alloc::rc::Rc;

use twine_core::{Angle, Color, Opa, Point, Scale};
use twine_engine::{EventCode, EventFilter, EventResult, MeasureCx, State};
use twine_image::ImageSource;
use twine_style::{Align, StyleProp};
use twine_text::LongMode;
use twine_widgets::button::Button;
use twine_widgets::image::{Image, ImageAlign};
use twine_widgets::label::Label;

use crate::access::EngineAccess;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model};
use crate::modifiers::ViewExt;
use crate::prop::IntoProp;
use crate::text::{IntoText, TextProp, bind_label_text};
use crate::view::ViewSeq;

/// A label showing `text` (any [`IntoText`]: a static string, an owned string, a signal, a
/// closure or [`text!`](crate::text!)).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = label("Hello").long_mode(LongMode::Dots).max_lines(2);
/// ```
pub fn label(text: impl IntoText) -> WidgetView<Label> {
    match text.into_text() {
        TextProp::Static(s) => widget_view(move || Label::new(s)),
        other => widget_view(|| Label::new("")).op(move |cx, n| bind_label_text(cx, n, other)),
    }
}

impl WidgetView<Label> {
    /// What happens with text that does not fit ([`LongMode`]).
    #[must_use]
    pub fn long_mode(self, m: impl IntoProp<LongMode>) -> Self {
        self.bind(m, |l: &mut Label, cx, m| l.set_long_mode(cx, m))
    }

    /// Limits the number of lines of a content-sized label (0 = unlimited).
    #[must_use]
    pub fn max_lines(self, n: impl IntoProp<u16>) -> Self {
        self.bind(n, |l: &mut Label, cx, n| l.set_max_lines(cx, n))
    }

    /// Lets the pointer select text: pressing and dragging over the label selects the
    /// characters between the press and the pointer (drawn with the `Selected` part's style).
    #[must_use]
    pub fn selectable(self, on: bool) -> Self {
        if !on {
            return self;
        }
        self.clickable(true).op(|cx, node| {
            let start = Rc::new(Cell::new(0usize));
            let s = start.clone();
            let e = cx.engine();
            e.add_event_handler(node, EventFilter::Code(EventCode::Pressed), move |ecx, _| {
                if let Some(p) = ecx.point() {
                    let e = ecx.engine();
                    let origin = e.content_area(node);
                    let rel = Point::new(p.x - origin.x0, p.y - origin.y0);
                    if let Some(l) = e.widget::<Label>(node) {
                        s.set(l.letter_on(&MeasureCx::new(e, node), rel));
                    }
                }
                EventResult::Continue
            });
            e.add_event_handler(node, EventFilter::Code(EventCode::Pressing), move |ecx, _| {
                if let Some(p) = ecx.point() {
                    let e = ecx.engine_mut();
                    let origin = e.content_area(node);
                    let rel = Point::new(p.x - origin.x0, p.y - origin.y0);
                    let end = e
                        .widget::<Label>(node)
                        .map(|l| l.letter_on(&MeasureCx::new(e, node), rel));
                    if let Some(end) = end {
                        let a = start.get();
                        e.with_widget_mut(node, |l: &mut Label, wcx| {
                            l.set_selection(wcx, a.min(end), a.max(end));
                        });
                    }
                }
                EventResult::Continue
            });
        })
    }
}

/// A button holding `children` (usually one [`label`], which is centered).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = button(label("OK")).on_click(|| {});
/// ```
pub fn button(children: impl ViewSeq) -> WidgetView<Button> {
    widget_view(Button::new)
        .children(children)
        .after_children(|cx, n| {
            let e = cx.engine();
            let mut kids = e.tree().children(n);
            if let (Some(only), None) = (kids.next(), kids.next()) {
                e.set_align(only, Align::Center);
            }
        })
}

impl WidgetView<Button> {
    /// Makes the button toggle its checked state when clicked.
    #[must_use]
    pub fn checkable(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |b: &mut Button, cx, on| b.set_checkable(cx, on))
    }

    /// The checked state: a plain value (the button owns it; see
    /// [`on_change`](Self::on_change)) or a signal kept in sync both ways.
    #[must_use]
    pub fn checked(self, checked: impl IntoModel<bool>) -> Self {
        let model = checked.into_model();
        self.op(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, on| e.set_state(n, State::CHECKED, on),
                |e, n, _ev| {
                    e.tree()
                        .node(n)
                        .is_some_and(|x| x.state().contains(State::CHECKED))
                },
            );
        })
    }

    /// Called with the new checked state whenever the user toggles it.
    #[must_use]
    pub fn on_change(self, mut f: impl FnMut(bool) + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine().add_event_handler(
                node,
                EventFilter::Code(EventCode::ValueChanged),
                move |ecx, ev| {
                    if ev.target == ecx.node() {
                        let on = ecx
                            .engine()
                            .tree()
                            .node(ev.target)
                            .is_some_and(|x| x.state().contains(State::CHECKED));
                        EngineAccess::provide(ecx.engine_mut(), || f(on));
                    }
                    EventResult::Continue
                },
            );
        })
    }
}

/// An image showing `src` (any [`IntoProp<ImageSource>`]).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = image(ImageSource::Symbol("\u{f00c}")).scale(Scale::ONE);
/// ```
pub fn image(src: impl IntoProp<ImageSource>) -> WidgetView<Image> {
    widget_view(Image::new).bind(src, |i: &mut Image, cx, s| i.set_src(cx, s))
}

impl WidgetView<Image> {
    /// Rotation around the pivot.
    #[must_use]
    pub fn rotation(self, a: impl IntoProp<Angle>) -> Self {
        self.bind(a, |i: &mut Image, cx, a| i.set_rotation(cx, a))
    }

    /// Zoom (both axes; 256 = 1.0).
    #[must_use]
    pub fn scale(self, s: impl IntoProp<Scale>) -> Self {
        self.bind(s, |i: &mut Image, cx, s| i.set_scale(cx, s))
    }

    /// Pivot of rotation and zoom, relative to the image.
    #[must_use]
    pub fn pivot(self, p: impl IntoProp<Point>) -> Self {
        self.bind(p, |i: &mut Image, cx, p| i.set_pivot(cx, p))
    }

    /// Anti-aliasing of transformed images.
    #[must_use]
    pub fn antialias(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |i: &mut Image, cx, on| i.set_antialias(cx, on))
    }

    /// Alignment (and stretching, tiling) of the image inside the widget.
    #[must_use]
    pub fn inner_align(self, a: impl IntoProp<ImageAlign>) -> Self {
        self.bind(a, |i: &mut Image, cx, a| i.set_inner_align(cx, a))
    }

    /// Recolors the image pixels (`ImageRecolor` / `ImageRecolorOpa`).
    #[must_use]
    pub fn recolor(self, color: impl IntoProp<Color>, opa: impl IntoProp<Opa>) -> Self {
        self.style_prop(color, StyleProp::ImageRecolor)
            .style_prop(opa, StyleProp::ImageRecolorOpa)
    }
}
