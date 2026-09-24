//! [`SpanGroup`]: rich text made of [`Span`]s with their own styles (LVGL `lv_spangroup`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::Range;

use twine_core::{Color, Opa, Point, Rect, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx, NodeId, OBJ_FLAGS,
    Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_style::{
    COORD_MAX, Length, Part, PropId, Selector, StyleBuf, StyleProp, StyleValue, TextAlign, TextDecor,
};
use twine_text::{BREAK_CHARS, DOTS, Font, TextDrawFlags, TextFlags, TextLayout, is_wide};

use crate::label::LabelText;
use crate::log_set;

/// The class of [`SpanGroup`]: `"spangroup"`, part `Main`, the base object's flags (LVGL
/// `lv_spangroup_class`).
pub static SPANGROUP_CLASS: WidgetClass = WidgetClass::new("spangroup")
    .parts(&[Part::Main])
    .default_flags(OBJ_FLAGS);

/// The handle of a [`Span`] in its [`SpanGroup`] (stays valid while the span exists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SpanId(u32);

/// How a [`SpanGroup`] sizes itself (LVGL `lv_span_mode_t`, derived from the size styles
/// like LVGL master).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpanMode {
    /// Fixed width and height; text beyond them is clipped (or ellipsized).
    Fixed,
    /// As wide and tall as the text on one line (width and height `Content`).
    #[default]
    Expand,
    /// A fixed width, lines break, the height follows the text.
    Break,
}

/// What a [`SpanGroup`] does with text beyond its height (LVGL `lv_span_overflow_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpanOverflow {
    /// Cut it off.
    #[default]
    Clip,
    /// End the last visible line with "...".
    Ellipsis,
}

/// One run of text with its own text style (LVGL `lv_span_t`). Unset properties fall back
/// to the group's `Main` style (LVGL `lv_span_get_style_*`). Only the text properties are
/// used: font, color, opacity, decoration, letter space.
#[derive(Debug)]
pub struct Span {
    text: LabelText,
    scratch: String,
    style: StyleBuf,
}

impl Span {
    fn new() -> Self {
        Self {
            text: LabelText::Static(""),
            scratch: String::new(),
            style: StyleBuf::new(),
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Sets the text (copied, reusing the buffer); returns whether it changed. Call
    /// [`SpanGroup::refresh`] afterwards (or use [`SpanGroup::set_span_text`]).
    pub fn set_text(&mut self, s: &str) -> bool {
        if self.text() == s {
            return false;
        }
        match &mut self.text {
            LabelText::Owned(b) => {
                b.clear();
                b.push_str(s);
            }
            LabelText::Static(_) => {
                let mut b = core::mem::take(&mut self.scratch);
                b.clear();
                b.push_str(s);
                self.text = LabelText::Owned(b);
            }
        }
        true
    }

    /// Sets a `'static` text without copying; returns whether it changed.
    pub fn set_text_static(&mut self, s: &'static str) -> bool {
        let changed = self.text() != s;
        if let LabelText::Owned(b) = core::mem::replace(&mut self.text, LabelText::Static(s)) {
            self.scratch = b;
        }
        changed
    }

    /// The span's own style.
    #[must_use]
    pub fn style(&self) -> &StyleBuf {
        &self.style
    }

    /// Sets a style property of the span (text properties only are used); returns whether
    /// it changed.
    pub fn set_style(&mut self, p: StyleProp) -> bool {
        if self.style.get(p.id()) == Some(p.value()) {
            return false;
        }
        self.style.set(p);
        true
    }
}

/// One piece of a span on one line.
#[derive(Clone, Debug)]
struct Snippet {
    span: usize,
    range: Range<usize>,
    /// Width (trailing spaces included, the last letter space excluded).
    w: i32,
    font: &'static Font,
    letter_space: i32,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LineInfo {
    snippets: Range<usize>,
    /// Top of the line relative to the content area.
    y: i32,
    /// The tallest snippet's line height plus the line space.
    h: i32,
    /// The base line of the tallest font.
    base_line: i32,
    /// Where the first snippet starts (the indent on the first line).
    x: i32,
}

/// The cached line layout of a group: rebuilt only when the width, the spans or the styles
/// change, so redrawing allocates nothing.
#[derive(Debug, Default)]
struct SpanLayoutCache {
    key: Option<(i32, u32)>,
    snippets: Vec<Snippet>,
    lines: Vec<LineInfo>,
    /// Height of all lines (without the last line space).
    height: i32,
}

/// A line of a [`SpanGroup`]'s layout (see [`SpanGroup::line_layout`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanLine {
    /// Top of the line relative to the content area.
    pub y: i32,
    /// The line height (the tallest span's line height plus the line space).
    pub height: i32,
    /// The pieces on the line: span, byte range of its text, x (before alignment) and width.
    pub pieces: Vec<(SpanId, Range<usize>, i32, i32)>,
}

/// Rich text (LVGL `lv_spangroup`): [`Span`]s, each with its own font, color, opacity,
/// decoration and letter space, flowed together like one paragraph.
///
/// - **Layout** (LVGL `lv_draw_span`): the spans are broken into lines across their
///   boundaries; a line is as tall as its tallest span and the spans sit on a common
///   baseline; the first line starts at the [indent](Self::set_indent).
/// - **Modes** ([`SpanMode`]): `Expand` sizes the group to the text on one line, `Break`
///   wraps at the group's width with the height of the text (at most
///   [`max_lines`](Self::set_max_lines)), `Fixed` keeps the set size and clips.
/// - **Overflow** ([`SpanOverflow`]): with `Ellipsis` the last visible line ends with "...".
/// - The line layout is cached (keyed by the width and a change counter): redrawing
///   allocates nothing.
///
/// ```
/// use twine_core::Color;
/// use twine_style::StyleProp;
/// use twine_testing::EngineHarness;
/// use twine_widgets::spangroup::{self, SpanGroup};
///
/// let mut h = EngineHarness::new(200, 60);
/// let screen = h.screen();
/// let g = spangroup::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(g, |w: &mut SpanGroup, cx| {
///     let a = w.add_span(cx);
///     w.set_span_text(cx, a, "Hello, ");
///     let b = w.add_span(cx);
///     w.set_span_text(cx, b, "world");
///     w.set_span_style(cx, b, StyleProp::TextColor(Color::RED));
/// });
/// h.run_until_idle();
/// assert_eq!(h.engine().widget::<SpanGroup>(g).unwrap().span_count(), 2);
/// ```
#[derive(Debug)]
pub struct SpanGroup {
    spans: Vec<(SpanId, Span)>,
    next_id: u32,
    overflow: SpanOverflow,
    indent: i32,
    max_lines: i32,
    generation: u32,
    cache: RefCell<SpanLayoutCache>,
}

impl Default for SpanGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// The style of a span with the group's `Main` style as fallback.
struct SpanStyle<'a> {
    m: &'a MeasureCx<'a>,
    span: &'a Span,
}

impl SpanStyle<'_> {
    fn get(&self, p: PropId) -> StyleValue {
        self.span
            .style
            .get(p)
            .unwrap_or_else(|| self.m.style(Part::Main, p))
    }

    fn font(&self) -> &'static Font {
        match self.span.style.get(PropId::TextFont) {
            Some(v) => v
                .get::<&'static Font>()
                .unwrap_or_else(|| self.m.font(Part::Main)),
            None => self.m.font(Part::Main),
        }
    }

    fn i32(&self, p: PropId) -> i32 {
        match self.span.style.get(p) {
            Some(v) => v.as_i32().unwrap_or(0),
            None => self.m.style_i32(Part::Main, p),
        }
    }

    fn color(&self) -> Color {
        self.get(PropId::TextColor).as_color().unwrap_or(Color::BLACK)
    }

    fn opa(&self) -> Opa {
        self.get(PropId::TextOpa).as_opa().unwrap_or(Opa::COVER)
    }

    fn decor(&self) -> TextDecor {
        self.get(PropId::TextDecor).get::<TextDecor>().unwrap_or_default()
    }
}

/// LVGL `lv_text_get_snippet`: the first line of `txt` within `max_w`, its end offset (after
/// a newline), its width, and whether it filled the line (more text follows on another line
/// or it ended with a line break).
fn snippet(txt: &str, font: &'static Font, ls: i32, max_w: i32, flags: TextFlags) -> (usize, i32, bool) {
    if txt.is_empty() {
        return (0, 0, false);
    }
    let mut l = TextLayout::new(txt, font);
    l.letter_space = ls;
    l.max_width = max_w.max(0);
    l.flags = flags;
    let mut it = l.lines();
    let Some(line) = it.next() else {
        return (0, 0, false);
    };
    let end = it.next_start().max(line.range.end).min(txt.len());
    let end = if end == 0 {
        txt.chars().next().map_or(0, char::len_utf8)
    } else {
        end
    };
    let w = l.line_width(line.range.start..line.range.end);
    let newline = txt[..end].ends_with(['\n', '\r']);
    let fill = end < txt.len() || newline || w > max_w;
    (end, w, fill)
}

impl SpanGroup {
    /// An empty group (clip overflow, no indent, no line limit).
    #[must_use]
    pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            next_id: 0,
            overflow: SpanOverflow::Clip,
            indent: 0,
            max_lines: -1,
            generation: 0,
            cache: RefCell::new(SpanLayoutCache::default()),
        }
    }

    /// The number of spans.
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    /// The span ids in order.
    pub fn span_ids(&self) -> impl Iterator<Item = SpanId> + '_ {
        self.spans.iter().map(|(id, _)| *id)
    }

    /// A span.
    #[must_use]
    pub fn span(&self, id: SpanId) -> Option<&Span> {
        self.spans.iter().find(|(i, _)| *i == id).map(|(_, s)| s)
    }

    /// A span, mutably: call [`refresh`](Self::refresh) after changing it.
    pub fn span_mut(&mut self, id: SpanId) -> Option<&mut Span> {
        self.spans.iter_mut().find(|(i, _)| *i == id).map(|(_, s)| s)
    }

    /// The overflow handling.
    #[must_use]
    pub fn overflow(&self) -> SpanOverflow {
        self.overflow
    }

    /// The first line's indent in px.
    #[must_use]
    pub fn indent(&self) -> i32 {
        self.indent
    }

    /// The line limit of `Break` mode (-1: none).
    #[must_use]
    pub fn max_lines(&self) -> i32 {
        self.max_lines
    }

    /// The mode, derived from the size styles like LVGL master (`lv_spangroup_get_mode`).
    #[must_use]
    pub fn mode(&self, cx: &MeasureCx<'_>) -> SpanMode {
        let content = |p| cx.style(Part::Main, p).as_length() == Some(Length::Content);
        if content(PropId::Width) {
            SpanMode::Expand
        } else if content(PropId::Height) {
            SpanMode::Break
        } else {
            SpanMode::Fixed
        }
    }

    /// Appends an empty span (LVGL `lv_spangroup_add_span`).
    pub fn add_span(&mut self, cx: &mut WidgetCx<'_>) -> SpanId {
        let id = SpanId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.spans.push((id, Span::new()));
        log_set(SPANGROUP_CLASS.name, cx.node(), "add_span");
        self.refresh(cx);
        id
    }

    /// Removes a span (LVGL `lv_spangroup_delete_span`). Unknown ids are ignored.
    pub fn delete_span(&mut self, cx: &mut WidgetCx<'_>, id: SpanId) {
        let before = self.spans.len();
        self.spans.retain(|(i, _)| *i != id);
        if self.spans.len() != before {
            log_set(SPANGROUP_CLASS.name, cx.node(), "delete_span");
            self.refresh(cx);
        }
    }

    /// Sets the text of a span (LVGL `lv_spangroup_set_span_text`). Idempotent.
    pub fn set_span_text(&mut self, cx: &mut WidgetCx<'_>, id: SpanId, s: &str) {
        if self.span_mut(id).is_some_and(|sp| sp.set_text(s)) {
            log_set(SPANGROUP_CLASS.name, cx.node(), "span_text");
            self.refresh(cx);
        }
    }

    /// Sets a `'static` span text without copying. Idempotent.
    pub fn set_span_text_static(&mut self, cx: &mut WidgetCx<'_>, id: SpanId, s: &'static str) {
        if self.span_mut(id).is_some_and(|sp| sp.set_text_static(s)) {
            log_set(SPANGROUP_CLASS.name, cx.node(), "span_text");
            self.refresh(cx);
        }
    }

    /// Sets a style property of a span (LVGL `lv_spangroup_set_span_style`). Idempotent.
    pub fn set_span_style(&mut self, cx: &mut WidgetCx<'_>, id: SpanId, p: StyleProp) {
        if self.span_mut(id).is_some_and(|sp| sp.set_style(p)) {
            log_set(SPANGROUP_CLASS.name, cx.node(), "span_style");
            self.refresh(cx);
        }
    }

    /// Sets the mode (LVGL `lv_spangroup_set_mode`, which sets the size styles: `Expand`
    /// makes both `Content`, `Break` gives a content width a fixed 100 px and a `Content`
    /// height, `Fixed` gives `Content` sizes 100 px).
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: SpanMode) {
        if self.mode(&cx.measure()) == mode {
            return;
        }
        log_set(SPANGROUP_CLASS.name, cx.node(), "mode");
        let id = cx.node();
        let is_content = |cx: &WidgetCx<'_>, p| cx.style(Part::Main, p).as_length() == Some(Length::Content);
        let (wc, hc) = (is_content(cx, PropId::Width), is_content(cx, PropId::Height));
        let e = cx.engine_mut();
        match mode {
            SpanMode::Expand => e.set_size(id, Length::Content, Length::Content),
            SpanMode::Break => {
                if wc {
                    e.set_width(id, 100);
                }
                e.set_height(id, Length::Content);
            }
            SpanMode::Fixed => {
                if wc {
                    e.set_width(id, 100);
                }
                if hc {
                    e.set_height(id, 100);
                }
            }
        }
        self.refresh(cx);
    }

    /// Sets the overflow handling (LVGL `lv_spangroup_set_overflow`). Idempotent.
    pub fn set_overflow(&mut self, cx: &mut WidgetCx<'_>, o: SpanOverflow) {
        if self.overflow != o {
            log_set(SPANGROUP_CLASS.name, cx.node(), "overflow");
            self.overflow = o;
            cx.invalidate_for("spangroup.overflow");
        }
    }

    /// Indents the first line by `px` (LVGL `lv_spangroup_set_indent`). Idempotent.
    pub fn set_indent(&mut self, cx: &mut WidgetCx<'_>, px: i32) {
        if self.indent != px {
            log_set(SPANGROUP_CLASS.name, cx.node(), "indent");
            self.indent = px;
            self.refresh(cx);
        }
    }

    /// Limits `Break` mode to `n` lines (-1: no limit; LVGL `lv_spangroup_set_max_lines`).
    /// Idempotent.
    pub fn set_max_lines(&mut self, cx: &mut WidgetCx<'_>, n: i32) {
        if self.max_lines != n {
            log_set(SPANGROUP_CLASS.name, cx.node(), "max_lines");
            self.max_lines = n;
            self.refresh(cx);
        }
    }

    /// Sets the text alignment (the `TextAlign` style, like LVGL's deprecated
    /// `lv_spangroup_set_align`).
    pub fn set_align(&mut self, cx: &mut WidgetCx<'_>, align: TextAlign) {
        let id = cx.node();
        cx.engine_mut()
            .set_local_prop(id, Selector::MAIN, StyleProp::TextAlign(align));
    }

    /// Re-lays out and redraws the group after spans changed (LVGL
    /// `lv_spangroup_refresh`).
    pub fn refresh(&mut self, cx: &mut WidgetCx<'_>) {
        self.generation = self.generation.wrapping_add(1);
        cx.invalidate_for("spangroup");
        cx.mark_layout();
    }

    /// LVGL `lv_spangroup_get_expand_width`: the width of all spans on one line (plus the
    /// indent), at most `max` (0: no limit).
    #[must_use]
    pub fn expand_width(&self, cx: &MeasureCx<'_>, max: i32) -> i32 {
        if self.spans.is_empty() {
            return 0;
        }
        let mut width = self.indent.max(0);
        let mut last_ls = 0;
        for (_, sp) in &self.spans {
            let st = SpanStyle { m: cx, span: sp };
            let font = st.font();
            let ls = st.i32(PropId::TextLetterSpace);
            last_ls = ls;
            let t = sp.text();
            let mut it = t.chars().peekable();
            while let Some(c) = it.next() {
                if max > 0 && width >= max {
                    return max;
                }
                width += font.advance_px(c, it.peek().copied()) + ls;
            }
        }
        width - last_ls
    }

    /// The tallest line height of the spans (LVGL `lv_spangroup_get_max_line_height`).
    #[must_use]
    pub fn max_line_height(&self, cx: &MeasureCx<'_>) -> i32 {
        self.spans
            .iter()
            .map(|(_, sp)| i32::from(SpanStyle { m: cx, span: sp }.font().line_height))
            .max()
            .unwrap_or(0)
    }

    /// LVGL `lv_spangroup_get_expand_height`: the height of the text wrapped at `width`
    /// (at most [`max_lines`](Self::max_lines) lines).
    #[must_use]
    pub fn expand_height(&self, cx: &MeasureCx<'_>, width: i32) -> i32 {
        if self.spans.is_empty() || width <= 0 {
            return 0;
        }
        let mut c = SpanLayoutCache::default();
        self.build(cx, width, TextFlags::empty(), &mut c);
        c.height
    }

    /// The line layout at the current width (for inspection; allocates).
    #[must_use]
    pub fn line_layout(&self, cx: &MeasureCx<'_>) -> Vec<SpanLine> {
        self.ensure_layout(cx);
        let c = self.cache.borrow();
        c.lines
            .iter()
            .map(|l| {
                let mut x = l.x;
                let pieces = c.snippets[l.snippets.clone()]
                    .iter()
                    .map(|s| {
                        let p = (self.spans[s.span].0, s.range.clone(), x, s.w);
                        x += s.w + s.letter_space;
                        p
                    })
                    .collect();
                SpanLine {
                    y: l.y,
                    height: l.h,
                    pieces,
                }
            })
            .collect()
    }

    /// The span at the absolute point `p` (LVGL `lv_spangroup_get_span_by_point`; here the
    /// drawn pieces are hit exactly).
    #[must_use]
    pub fn span_by_point(&self, cx: &MeasureCx<'_>, p: Point) -> Option<SpanId> {
        self.ensure_layout(cx);
        let content = cx.content_area();
        let c = self.cache.borrow();
        let max_w = content.width();
        let align = Self::align(cx);
        for l in &c.lines {
            let (mut x, w) = (l.x, Self::line_width(&c, l));
            x += Self::align_ofs(align, max_w, w);
            for s in &c.snippets[l.snippets.clone()] {
                let r = Rect::new(
                    content.x0 + x,
                    content.y0 + l.y,
                    content.x0 + x + s.w + s.letter_space,
                    content.y0 + l.y + l.h,
                );
                if r.contains(p) {
                    return Some(self.spans[s.span].0);
                }
                x += s.w + s.letter_space;
            }
        }
        None
    }

    fn align(cx: &MeasureCx<'_>) -> TextAlign {
        cx.style(Part::Main, PropId::TextAlign)
            .get::<TextAlign>()
            .unwrap_or(TextAlign::Auto)
    }

    fn line_width(c: &SpanLayoutCache, l: &LineInfo) -> i32 {
        let sn = &c.snippets[l.snippets.clone()];
        let w: i32 = sn.iter().map(|s| s.w + s.letter_space).sum();
        l.x + w - sn.last().map_or(0, |s| s.letter_space)
    }

    fn align_ofs(align: TextAlign, max_w: i32, w: i32) -> i32 {
        let ofs = (max_w - w).max(0);
        match align {
            TextAlign::Center => ofs >> 1,
            TextAlign::Right => ofs,
            TextAlign::Left | TextAlign::Auto => 0,
        }
    }

    /// The width the text wraps at: unlimited in `Expand` mode, else the content width.
    fn wrap_width(&self, cx: &MeasureCx<'_>) -> i32 {
        match self.mode(cx) {
            SpanMode::Expand => COORD_MAX,
            _ => avail_width(cx),
        }
    }

    /// Rebuilds the cached layout when the width or the spans changed.
    fn ensure_layout(&self, cx: &MeasureCx<'_>) {
        let w = self.wrap_width(cx);
        let key = (w, self.generation);
        let mut c = self.cache.borrow_mut();
        if c.key == Some(key) {
            return;
        }
        self.build(cx, w, TextFlags::empty(), &mut c);
        c.key = Some(key);
    }

    /// LVGL's line filling (`lv_draw_span` / `lv_spangroup_get_expand_height`).
    fn build(&self, cx: &MeasureCx<'_>, max_width: i32, flags: TextFlags, c: &mut SpanLayoutCache) {
        c.snippets.clear();
        c.lines.clear();
        c.height = 0;
        let line_space = cx.style_i32(Part::Main, PropId::TextLineSpace);
        let indent = self.indent;
        let lines_limit = if self.max_lines < 0 {
            usize::MAX
        } else {
            usize::try_from(self.max_lines).unwrap_or(0)
        };
        let mut max_w = max_width.saturating_sub(indent);
        let mut y = 0;
        let mut span_i = 0;
        let mut ofs = 0;
        let mut first = true;
        while span_i < self.spans.len() {
            let start = c.snippets.len();
            let mut max_line_h = 0;
            let mut base_line = 0;
            while let Some((_, sp)) = self.spans.get(span_i) {
                let txt = sp.text();
                if ofs >= txt.len() {
                    span_i += 1;
                    ofs = 0;
                    continue;
                }
                let st = SpanStyle { m: cx, span: sp };
                let font = st.font();
                let ls = st.i32(PropId::TextLetterSpace);
                let line_h = i32::from(font.line_height) + line_space;
                let (next, w, fill) = snippet(&txt[ofs..], font, ls, max_w, flags);
                if fill && next > 0 && c.snippets.len() > start {
                    // Do not break a word that started in an earlier span of this line.
                    let drawn = if span_i + 1 == self.spans.len() { w - ls } else { w };
                    if max_w < drawn {
                        break;
                    }
                    let letter = txt[ofs..ofs + next].chars().next_back();
                    let letter_next = txt[ofs + next..].chars().next();
                    let brk = |c: Option<char>| match c {
                        None | Some('\n' | '\r') => true,
                        Some(c) => BREAK_CHARS.contains(&c),
                    };
                    let word = |c: Option<char>| c.is_some_and(is_wide);
                    if !(brk(letter) || word(letter) || word(letter_next) || brk(letter_next)) {
                        break;
                    }
                }
                c.snippets.push(Snippet {
                    span: span_i,
                    range: ofs..ofs + next,
                    w,
                    font,
                    letter_space: ls,
                });
                ofs += next;
                if max_line_h < line_h {
                    max_line_h = line_h;
                    base_line = i32::from(font.base_line);
                }
                max_w -= w + ls;
                if fill || max_w <= 0 {
                    break;
                }
            }
            if c.snippets.len() == start {
                break;
            }
            c.lines.push(LineInfo {
                snippets: start..c.snippets.len(),
                y,
                h: max_line_h,
                base_line,
                x: if first { indent } else { 0 },
            });
            first = false;
            y += max_line_h;
            max_w = max_width;
            if c.lines.len() >= lines_limit {
                break;
            }
        }
        c.height = (y - line_space).max(0);
    }

    /// Draws the lines (LVGL `lv_draw_span`).
    fn draw_spans(&self, cx: &mut DrawCx<'_, '_>) {
        let content = cx.content_area();
        let Some(clip) = content.intersection(&cx.clip()) else {
            return;
        };
        let m = MeasureCx::new(cx.engine(), cx.node());
        self.ensure_layout(&m);
        let c = self.cache.borrow();
        let max_w = content.width();
        let align = Self::align(&m);
        let line_space = m.style_i32(Part::Main, PropId::TextLineSpace);
        let base = cx.text_dsc(Part::Main);
        let obj_opa = cx.opa();
        let n_lines = c.lines.len();
        let _ = cx.with_clip(clip, |cx| {
            for (li, l) in c.lines.iter().enumerate() {
                let top = content.y0 + l.y;
                if top >= clip.y1 {
                    break;
                }
                // The end line: the next one would not fit (LVGL's overflow check).
                let next_h = c.lines.get(li + 1).map_or(0, |n| n.h);
                let end_line = top + l.h + next_h - line_space > content.y1;
                let more = li + 1 < n_lines;
                let ellipsis = end_line && more && self.overflow == SpanOverflow::Ellipsis;
                if top + l.h < clip.y0 {
                    if end_line {
                        break;
                    }
                    continue;
                }
                let lw = Self::line_width(&c, l);
                let mut x = content.x0 + l.x + Self::align_ofs(align, max_w, lw);
                let sn = &c.snippets[l.snippets.clone()];
                for (i, s) in sn.iter().enumerate() {
                    let (_, sp) = &self.spans[s.span];
                    let st = SpanStyle { m: &m, span: sp };
                    let mut d = base;
                    d.font = s.font;
                    d.color = st.color();
                    d.opa = st.opa().mul(obj_opa);
                    d.decor = st.decor();
                    d.letter_space = s.letter_space;
                    d.line_space = line_space;
                    d.align = TextAlign::Left;
                    d.flags |= TextDrawFlags::EXPAND;
                    let y = top + l.h
                        - (i32::from(s.font.line_height) + line_space)
                        - (l.base_line - i32::from(s.font.base_line));
                    let txt = sp.text()[s.range.clone()].trim_end_matches(['\n', '\r']);
                    let h = i32::from(s.font.line_height);
                    if ellipsis && i + 1 == sn.len() {
                        let dots = TextLayout::new(DOTS, s.font).line_width(0..DOTS.len());
                        let avail = (content.x1 - x - dots).max(0);
                        let (end, _, _) = snippet(txt, s.font, s.letter_space, avail, TextFlags::BREAK_ALL);
                        // Trailing spaces may overhang a line: drop them before the dots.
                        let keep = txt[..end.min(txt.len())].trim_end_matches(' ');
                        let mut kl = TextLayout::new(keep, s.font);
                        kl.letter_space = s.letter_space;
                        let w = kl.line_width(0..keep.len());
                        cx.draw_text(Rect::new(x, y, x + w + 1, y + h), keep, &d);
                        let dx = x + w + s.letter_space;
                        cx.draw_text(Rect::new(dx, y, dx + dots + 1, y + h), DOTS, &d);
                        break;
                    }
                    cx.draw_text(Rect::new(x, y, x + s.w + 1, y + h), txt, &d);
                    x += s.w + s.letter_space;
                }
                if end_line {
                    break;
                }
            }
        });
    }
}

/// The width available to the text for measuring (like the label's): the resolved width
/// minus padding and border; unlimited for `Content`.
fn avail_width(cx: &MeasureCx<'_>) -> i32 {
    let m = Part::Main;
    let pad = cx.padding(m);
    let border = cx.style_i32(m, PropId::BorderWidth).max(0);
    let spaces = pad.left + pad.right + 2 * border;
    if cx.style_i32(m, PropId::FlexGrow) > 0 {
        return cx.content_area().width();
    }
    match cx.style(m, PropId::Width).as_length() {
        Some(Length::Px(w)) => (w - spaces).max(0),
        Some(Length::Pct(p)) => {
            let parent = cx
                .engine()
                .tree()
                .parent(cx.node())
                .map_or(0, |p| cx.engine().content_area(p).width());
            ((i64::from(parent) * i64::from(p) / 100) as i32 - spaces).max(0)
        }
        _ => COORD_MAX,
    }
}

/// Creates an empty span group (sized to its text) as the last child of `parent` (LVGL
/// `lv_spangroup_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(SpanGroup::new()))
}

impl Widget for SpanGroup {
    fn class(&self) -> &'static WidgetClass {
        &SPANGROUP_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        // LVGL `width_def` / `height_def`: `LV_SIZE_CONTENT` (the `Expand` mode).
        cx.engine_mut().set_size(id, Length::Content, Length::Content);
        twine_core::trace!(target: "twine::engine", "spangroup#{} created", fmt_node_id(id));
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE` of the span group.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        match self.mode(cx) {
            SpanMode::Expand => Size::new(self.expand_width(cx, 0), self.max_line_height(cx)),
            SpanMode::Break => {
                self.ensure_layout(cx);
                Size::new(avail_width(cx), self.cache.borrow().height)
            }
            SpanMode::Fixed => {
                let c = cx.content_area();
                Size::new(c.width(), c.height())
            }
        }
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        if !self.spans.is_empty() {
            self.draw_spans(cx);
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && matches!(ev.code, EventCode::StyleChanged | EventCode::SizeChanged) {
            // The cache key has the width; styles (fonts, spacing) need a new generation.
            if ev.code == EventCode::StyleChanged {
                self.refresh(&mut cx.widget_cx());
            } else {
                cx.widget_cx().invalidate();
            }
        }
        EventResult::Continue
    }
}
