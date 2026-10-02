//! The core widget views: [`label`], [`button`], [`image`].

use core::cell::Cell;

use alloc::rc::Rc;

use twine_core::{Angle, Point, Scale};
use twine_engine::{EventCode, EventFilter, EventResult, MeasureCx, ObjFlags, State};
use twine_image::ImageSource;
use twine_style::Align;
use twine_text::LongMode;
use twine_widgets::button::Button;
use twine_widgets::image::{Image, ImageAlign};
use twine_widgets::label::Label;

use crate::access::EngineAccess;
use crate::bind::bind_node;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model};
use crate::prop::{IntoProp, Prop};
use crate::text::{IntoText, TextProp, bind_label_text};
use crate::view::ViewSeq;

/// A label showing `text` (any [`IntoText`]: a static string, an owned string, a signal, a
/// closure or [`text!`](crate::text!)).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = label("Hello").long_mode(LongMode::Dots).max_lines(2);
/// ```
pub fn label<MT>(text: impl IntoText<MT>) -> WidgetView<Label> {
    match text.into_text() {
        TextProp::Static(s) => widget_view(move || Label::new(s)),
        other => widget_view(|| Label::new("")).op(move |cx, n| bind_label_text(cx, n, other)),
    }
}

impl WidgetView<Label> {
    /// What happens with text that does not fit ([`LongMode`]).
    #[must_use]
    pub fn long_mode<M>(self, m: impl IntoProp<LongMode, M>) -> Self {
        self.bind(m, |l: &mut Label, cx, m| l.set_long_mode(cx, m))
    }

    /// Limits the number of lines of a content-sized label (0 = unlimited). An `i32` like every
    /// count of the view API, so that literals infer (see [`IntoProp`] § Integer literals); a
    /// value outside `0..=u16::MAX` is warned about and clamped.
    #[must_use]
    pub fn max_lines<M>(self, n: impl IntoProp<i32, M>) -> Self {
        self.bind(n, |l: &mut Label, cx, n: i32| {
            l.set_max_lines(cx, super::count_u16("max_lines", n));
        })
    }

    /// Lets the pointer select text: pressing and dragging over the label selects the
    /// characters between the press and the pointer (drawn with the `Selected` part's style).
    /// Turning it off clears the selection.
    #[must_use]
    pub fn selectable<M>(self, on: impl IntoProp<bool, M>) -> Self {
        let on = on.into_prop();
        if matches!(on, Prop::Static(false)) {
            return self;
        }
        self.op(move |cx, node| {
            let enabled = Rc::new(Cell::new(false));
            let start = Rc::new(Cell::new(0usize));
            let (en, s) = (enabled.clone(), start.clone());
            let e = cx.engine();
            e.add_event_handler(node, EventFilter::Code(EventCode::Pressed), move |ecx, _| {
                if let (true, Some(p)) = (en.get(), ecx.point()) {
                    let e = ecx.engine();
                    let origin = e.content_area(node);
                    let rel = Point::new(p.x - origin.x0, p.y - origin.y0);
                    if let Some(l) = e.widget::<Label>(node) {
                        s.set(l.letter_on(&MeasureCx::new(e, node), rel));
                    }
                }
                EventResult::Continue
            });
            let en = enabled.clone();
            e.add_event_handler(node, EventFilter::Code(EventCode::Pressing), move |ecx, _| {
                if let (true, Some(p)) = (en.get(), ecx.point()) {
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
            bind_node(cx, node, on, move |e, n, on| {
                enabled.set(on);
                e.set_flag(n, ObjFlags::CLICKABLE, on);
                if !on {
                    e.with_widget_mut(n, |l: &mut Label, wcx| l.clear_selection(wcx));
                }
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
    pub fn checkable<M>(self, on: impl IntoProp<bool, M>) -> Self {
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
            let rt = cx.runtime();
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
                        EngineAccess::provide(rt, ecx.engine_mut(), || f(on));
                    }
                    EventResult::Continue
                },
            );
        })
    }
}

/// An image showing `src` (any [`IntoProp<ImageSource, _>`](IntoProp): an image source, a
/// [`Symbol`](twine_text::Symbol) drawn with the symbol font, a `&'static` image, or a closure,
/// signal or memo of one).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = image(Symbol::Ok).scale(Scale::ONE);
/// ```
pub fn image<M>(src: impl IntoProp<ImageSource, M>) -> WidgetView<Image> {
    widget_view(Image::new).bind(src, |i: &mut Image, cx, s| i.set_src(cx, s))
}

impl WidgetView<Image> {
    /// Rotation around the pivot.
    #[must_use]
    pub fn rotation<M>(self, a: impl IntoProp<Angle, M>) -> Self {
        self.bind(a, |i: &mut Image, cx, a| i.set_rotation(cx, a))
    }

    /// Zoom (both axes; 256 = 1.0).
    #[must_use]
    pub fn scale<M>(self, s: impl IntoProp<Scale, M>) -> Self {
        self.bind(s, |i: &mut Image, cx, s| i.set_scale(cx, s))
    }

    /// Pivot of rotation and zoom, relative to the image.
    #[must_use]
    pub fn pivot<M>(self, p: impl IntoProp<Point, M>) -> Self {
        self.bind(p, |i: &mut Image, cx, p| i.set_pivot(cx, p))
    }

    /// Anti-aliasing of transformed images.
    #[must_use]
    pub fn antialias<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |i: &mut Image, cx, on| i.set_antialias(cx, on))
    }

    /// Alignment (and stretching, tiling) of the image inside the widget.
    #[must_use]
    pub fn inner_align<M>(self, a: impl IntoProp<ImageAlign, M>) -> Self {
        self.bind(a, |i: &mut Image, cx, a| i.set_inner_align(cx, a))
    }

    /// Draws an SVG source from a raster cached once instead of from its vector paths
    /// (feature `svg`; faster redraws of static icons).
    #[cfg(feature = "svg")]
    #[must_use]
    pub fn svg_cache<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |i: &mut Image, cx, on| i.set_svg_cache(cx, on))
    }
}
