//! Rich text: [`spangroup`] and its [`span`]s.

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_engine::{Engine, NodeId, fmt_node_id};
use twine_style::design::{ColorValue, FontValue, OpacityValue};
use twine_style::{Selector, StyleProp};
use twine_text::TextDecor;
use twine_widgets::label::Label;
use twine_widgets::spangroup::{SpanGroup, SpanId, SpanMode, SpanOverflow};

use crate::bind::bind_node;
use crate::build::{BuildCx, WidgetView, widget_view};
use crate::prop::IntoProp;
use crate::text::{IntoText, TextProp, TextRef, bind_str};
use crate::view::{View, ViewSeq};

/// A paragraph of [`span`]s with their own text styles, wrapped as one text.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let name = cx.signal(String::from("Ada"));
/// let _v = spangroup((
///     span("Hello, "),
///     span(name).text_color(Color::hex(0x21_96_F3)).text_decoration(TextDecor::UNDERLINE),
///     span("!"),
/// ))
/// .mode(SpanMode::Break)
/// .width(200);
/// cx.dispose();
/// ```
pub fn spangroup(spans: impl ViewSeq) -> WidgetView<SpanGroup> {
    widget_view(SpanGroup::new).children(spans)
}

impl WidgetView<SpanGroup> {
    /// Fixed size, content size on one line (`Expand`), or wrapping in a fixed width
    /// (`Break`). Sets the size styles like LVGL.
    #[must_use]
    pub fn mode<M>(self, m: impl IntoProp<SpanMode, M>) -> Self {
        self.bind(m, |g: &mut SpanGroup, cx, m| g.set_mode(cx, m))
    }

    /// Clip the text beyond the height, or end the last line with "...".
    #[must_use]
    pub fn overflow<M>(self, o: impl IntoProp<SpanOverflow, M>) -> Self {
        self.bind(o, |g: &mut SpanGroup, cx, o| g.set_overflow(cx, o))
    }

    /// Indent of the first line in px.
    #[must_use]
    pub fn indent<M>(self, px: impl IntoProp<i32, M>) -> Self {
        self.bind(px, |g: &mut SpanGroup, cx, px| g.set_indent(cx, px))
    }

    /// The maximum number of lines (`Break` mode; negative = unlimited).
    #[must_use]
    pub fn max_lines<M>(self, n: impl IntoProp<i32, M>) -> Self {
        self.bind(n, |g: &mut SpanGroup, cx, n| g.set_max_lines(cx, n))
    }
}

/// A span's style binding: applied to the span in a group, or to the label fallback.
type SpanStyleOp = Box<dyn FnOnce(&mut BuildCx<'_>, NodeId, Option<SpanId>)>;

/// One run of text inside a [`spangroup`] (see [`span`]). It creates no node of its own: it
/// adds a span to the parent group, and its text and style modifiers bind to that span.
/// Unset style properties fall back to the group's.
pub struct SpanView {
    text: TextProp,
    styles: Vec<SpanStyleOp>,
}

impl core::fmt::Debug for SpanView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpanView")
            .field("text", &self.text)
            .field("styles", &self.styles.len())
            .finish()
    }
}

/// A span showing `text` (any [`IntoText`]; dynamic texts update only this span, and the
/// group redraws its own area). Only valid as a child of a [`spangroup`]: elsewhere it logs a
/// warning and builds a [`label`](crate::label) instead.
pub fn span<MT>(text: impl IntoText<MT>) -> SpanView {
    SpanView {
        text: text.into_text(),
        styles: Vec::new(),
    }
}

/// Sets a style property on span `id` of group `n`, or on the fallback label `n`.
fn apply_style(e: &mut Engine, n: NodeId, id: Option<SpanId>, p: StyleProp) {
    match id {
        Some(id) => {
            e.with_widget_mut(n, |g: &mut SpanGroup, wcx| g.set_span_style(wcx, id, p));
        }
        None => e.set_local_prop(n, Selector::MAIN, p),
    }
}

impl SpanView {
    /// Binds a text style property of the span.
    #[must_use]
    pub fn style_prop<T: 'static, M>(
        mut self,
        v: impl IntoProp<T, M>,
        make: impl Fn(T) -> StyleProp + 'static,
    ) -> Self {
        let p = v.into_prop();
        self.styles.push(Box::new(move |cx, n, id| {
            bind_node(cx, n, p, move |e, n, v| apply_style(e, n, id, make(v)));
        }));
        self
    }

    /// The span's font: a `&'static Font` or a [design element](twine_style::design) such as
    /// `design::FONT_LARGE`.
    #[must_use]
    pub fn font<M>(self, f: impl IntoProp<FontValue, M>) -> Self {
        self.style_prop(f, StyleProp::Font)
    }

    /// The span's text color: a `Color` or a [design element](twine_style::design) such as
    /// `design::PRIMARY`.
    #[must_use]
    pub fn text_color<M>(self, c: impl IntoProp<ColorValue, M>) -> Self {
        self.style_prop(c, StyleProp::TextColor)
    }

    /// The span's text opacity: an `Opa` or an [opacity element](twine_style::design).
    #[must_use]
    pub fn text_opacity<M>(self, o: impl IntoProp<OpacityValue, M>) -> Self {
        self.style_prop(o, StyleProp::TextOpacity)
    }

    /// Underline and/or strikethrough.
    #[must_use]
    pub fn text_decoration<M>(self, d: impl IntoProp<TextDecor, M>) -> Self {
        self.style_prop(d, StyleProp::TextDecoration)
    }

    /// Extra space between the span's letters.
    #[must_use]
    pub fn letter_spacing<M>(self, px: impl IntoProp<i32, M>) -> Self {
        self.style_prop(px, StyleProp::LetterSpacing)
    }
}

impl View for SpanView {
    /// Adds the span to the parent group and returns the group's node.
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let group = cx.parent();
        if group == twine_engine::DEAD_NODE {
            return group; // the parent failed (reported)
        }
        let id = cx
            .engine()
            .with_widget_mut(group, |g: &mut SpanGroup, wcx| g.add_span(wcx));
        let Some(id) = id else {
            twine_core::warn!(
                target: "twine::view",
                "span outside a spangroup (parent {}); building a label instead",
                fmt_node_id(group)
            );
            let n = cx.create(Label::new(""));
            crate::text::bind_label_text(cx, n, self.text);
            for op in self.styles {
                op(cx, n, None);
            }
            return n;
        };
        bind_str(cx, group, self.text, move |e, n, s| {
            e.with_widget_mut(n, |g: &mut SpanGroup, wcx| match s {
                TextRef::Static(s) => g.set_span_text_static(wcx, id, s),
                TextRef::Borrowed(s) => g.set_span_text(wcx, id, s),
            });
        });
        for op in self.styles {
            op(cx, group, Some(id));
        }
        group
    }
}
