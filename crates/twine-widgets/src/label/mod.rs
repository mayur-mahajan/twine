//! [`Label`]: text (LVGL `lv_label`) with every long mode, selection and in-place text
//! storage.

mod long;

use alloc::boxed::Box;
use alloc::string::String;
use core::cell::Cell;

use twine_core::{Point, Rect, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx, NodeId, OBJ_FLAGS,
    ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_style::{COORD_MAX, Length, Part, PropId, TextAlign};
use twine_text::{LongMode, TextDrawFlags, TextFlags, TextLayout};

use crate::log_set;
pub use long::{ANIM_OFS_X, ANIM_OFS_Y, LABEL_DEF_SCROLL_SPEED, LABEL_SCROLL_DELAY};
use long::{DotState, ScrollState};

/// LVGL `LV_LABEL_DOT_NUM`: the number of dots of the `Dots` long mode.
pub const LABEL_DOT_NUM: usize = 3;
/// LVGL `LV_LABEL_WAIT_CHAR_COUNT`: spaces between the two copies of a circular scroll.
pub const LABEL_WAIT_CHAR_COUNT: i32 = 3;
/// LVGL `LV_LABEL_DEFAULT_TEXT`.
pub const LABEL_DEFAULT_TEXT: &str = "Text";

/// The class of [`Label`]: `"label"`, parts `Main`, `Scrollbar` and `Selected`, the base
/// object's flags without `CLICKABLE` (LVGL `lv_label_constructor`).
pub static LABEL_CLASS: WidgetClass = WidgetClass::new("label")
    .parts(&[Part::Main, Part::Scrollbar, Part::Selected])
    .default_flags(OBJ_FLAGS.difference(ObjFlags::CLICKABLE));

/// Where a label's text lives: in flash (no allocation) or in an owned buffer whose
/// capacity is reused by later texts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LabelText {
    /// A `'static` string (LVGL `lv_label_set_text_static`).
    Static(&'static str),
    /// An owned copy (LVGL `lv_label_set_text`).
    Owned(String),
}

impl LabelText {
    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            LabelText::Static(s) => s,
            LabelText::Owned(s) => s,
        }
    }
}

/// Everything the text layout of a label depends on (the key of the size cache).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayoutKey {
    hash: u32,
    len: usize,
    max_w: i32,
    font: usize,
    letter_space: i32,
    line_space: i32,
    expand: bool,
    max_lines: u16,
}

/// One cached measurement: the key and the resulting content size.
#[derive(Clone, Copy, Debug)]
struct LayoutCache {
    key: LayoutKey,
    size: Size,
}

/// FNV-1a over the text (cache key; collisions only cost a stale size for one frame
/// until the text length or another key field differs, and a collision needs the same length).
fn text_hash(s: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// A text label (LVGL `lv_label`).
///
/// - **Text** is stored as [`LabelText`]: [`set_text`](Self::set_text) copies into an owned
///   buffer reusing its capacity (no allocation once it is large enough),
///   [`set_text_static`](Self::set_text_static) stores a `'static` string without copying, and
///   [`write_text`](Self::write_text) formats in place (for `text!` bindings).
/// - **Size**: `Content` by default; the text is measured with the resolved font, letter and
///   line spacing and wrapped at the label's width when that is not `Content`.
/// - **Long modes** ([`LongMode`]): `Wrap` (default, grows in height), `Dots` (cut with
///   "..."; the stored text is not modified), `Scroll` (back and forth, 40 px/s, 300 ms
///   pauses), `ScrollCircular` (endless, the text drawn twice) and `Clip` (single line,
///   clipped) — LVGL's `lv_label_refr_text`.
/// - **Selection** of a byte range is drawn with the `Selected` part's text and background
///   colors.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::label::{self, Label};
///
/// let mut h = EngineHarness::new(120, 40);
/// let screen = h.screen();
/// let l = label::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, "Hello"));
/// h.run_until_idle();
/// let w = h.engine().widget::<Label>(l).unwrap();
/// assert_eq!(w.text(), "Hello");
/// assert!(h.engine().coords(l).width() > 20);
/// ```
#[derive(Debug)]
pub struct Label {
    text: LabelText,
    /// Second buffer of [`write_text`](Self::write_text) (swapped with the text).
    scratch: String,
    long_mode: LongMode,
    max_lines: u16,
    sel: Option<(usize, usize)>,
    dot: DotState,
    scroll: ScrollState,
    /// Size of the text laid out in the content area (LVGL `text_size`).
    text_size: Size,
    layout_cache: Cell<Option<LayoutCache>>,
    /// Password masking (see [`set_mask`](Self::set_mask)).
    mask: Option<Mask>,
}

/// The masked form of a label's text (a textarea in password mode).
#[derive(Debug, Default)]
struct Mask {
    /// Drawn instead of every character.
    bullet: String,
    /// The character index shown in clear (the last typed one), if any.
    reveal: Option<usize>,
    /// The text as drawn.
    shown: String,
}

impl Mask {
    /// Rebuilds `shown` from `text` (reusing its capacity).
    fn update(&mut self, text: &str) {
        self.shown.clear();
        for (i, c) in text.chars().enumerate() {
            if Some(i) == self.reveal || c == '\n' {
                self.shown.push(c);
            } else {
                self.shown.push_str(&self.bullet);
            }
        }
    }
}

impl Default for Label {
    fn default() -> Self {
        Self::new(LABEL_DEFAULT_TEXT)
    }
}

impl Label {
    /// A label showing `text` (a `'static` string, stored without copying).
    #[must_use]
    pub fn new(text: &'static str) -> Self {
        Self {
            text: LabelText::Static(text),
            scratch: String::new(),
            long_mode: LongMode::Wrap,
            max_lines: 0,
            sel: None,
            dot: DotState::default(),
            scroll: ScrollState::default(),
            text_size: Size::ZERO,
            layout_cache: Cell::new(None),
            mask: None,
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// The text storage.
    #[must_use]
    pub fn text_storage(&self) -> &LabelText {
        &self.text
    }

    /// The text as drawn: the [`text`](Self::text), or with a [mask](Self::set_mask) the
    /// bullets. Byte indices of [`letter_pos`](Self::letter_pos),
    /// [`letter_on`](Self::letter_on) and the selection refer to this text.
    #[must_use]
    pub fn shown_text(&self) -> &str {
        match &self.mask {
            Some(m) => &m.shown,
            None => self.text.as_str(),
        }
    }

    /// Whether the text is masked.
    #[must_use]
    pub fn is_masked(&self) -> bool {
        self.mask.is_some()
    }

    /// Masks the text (a Twine extension used by the textarea's password mode): every
    /// character is drawn as `bullet`, except the character at index `reveal` (and line
    /// breaks). `None` shows the text. [`text`](Self::text) still returns the real text.
    /// Idempotent.
    pub fn set_mask(&mut self, cx: &mut WidgetCx<'_>, bullet: Option<&str>, reveal: Option<usize>) {
        let same = match (&self.mask, bullet) {
            (None, None) => true,
            (Some(m), Some(b)) => m.bullet == b && m.reveal == reveal,
            _ => false,
        };
        if same {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "mask");
        match bullet {
            Some(b) => {
                let m = self.mask.get_or_insert_with(Mask::default);
                m.bullet.clear();
                m.bullet.push_str(b);
                m.reveal = reveal;
                m.update(self.text.as_str());
            }
            None => self.mask = None,
        }
        self.sel = None;
        self.text_changed(cx);
    }

    /// Edits the text in place with `f` (a Twine extension for the textarea: inserting or
    /// removing characters reuses the buffer's capacity). A `'static` text is copied into
    /// the owned buffer first. Always redraws and re-measures.
    pub fn edit_text(&mut self, cx: &mut WidgetCx<'_>, f: impl FnOnce(&mut String)) {
        if let LabelText::Static(s) = self.text {
            let mut buf = core::mem::take(&mut self.scratch);
            buf.clear();
            buf.push_str(s);
            self.text = LabelText::Owned(buf);
        }
        if let LabelText::Owned(buf) = &mut self.text {
            f(buf);
        }
        log_set(LABEL_CLASS.name, cx.node(), "text");
        self.text_changed(cx);
    }

    /// The long mode.
    #[must_use]
    pub fn long_mode(&self) -> LongMode {
        self.long_mode
    }

    /// The line limit (0 = unlimited).
    #[must_use]
    pub fn max_lines(&self) -> u16 {
        self.max_lines
    }

    /// The selected byte range (start ≤ end), if any.
    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.sel
    }

    /// The offset the text is drawn at (scrolling long modes).
    #[must_use]
    pub fn scroll_offset(&self) -> Point {
        self.scroll.ofs
    }

    /// In `Dots` mode when the text is cut: the byte index where the visible text ends and
    /// `"..."` follows.
    #[must_use]
    pub fn dots_end(&self) -> Option<usize> {
        self.dot.end
    }

    /// Size of the laid-out text as of the last refresh (LVGL `text_size`).
    #[must_use]
    pub fn text_size(&self) -> Size {
        self.text_size
    }

    /// Sets the text, copying it into the label's own buffer. Idempotent: an equal text does
    /// nothing. A different text overwrites the buffer in place (`clear` + `push_str`, so no
    /// allocation when it fits), invalidates the label and marks its layout if it is
    /// content-sized.
    pub fn set_text(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        if self.text() == s {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "text");
        match &mut self.text {
            LabelText::Owned(buf) => {
                buf.clear();
                buf.push_str(s);
            }
            LabelText::Static(_) => {
                // Reuse the scratch buffer's capacity when there is one.
                let mut buf = core::mem::take(&mut self.scratch);
                buf.clear();
                buf.push_str(s);
                self.text = LabelText::Owned(buf);
            }
        }
        self.text_changed(cx);
    }

    /// Sets a `'static` text without copying (LVGL `lv_label_set_text_static`). Idempotent.
    /// An owned buffer is kept for later [`set_text`](Self::set_text) calls.
    pub fn set_text_static(&mut self, cx: &mut WidgetCx<'_>, s: &'static str) {
        if let LabelText::Static(cur) = self.text {
            if core::ptr::eq(cur, s) || cur == s {
                if !core::ptr::eq(cur, s) {
                    self.text = LabelText::Static(s);
                }
                return;
            }
        } else if self.text() == s {
            // Same content: switch storage silently (nothing to redraw).
            if let LabelText::Owned(buf) = core::mem::replace(&mut self.text, LabelText::Static(s)) {
                self.scratch = buf;
            }
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "text_static");
        if let LabelText::Owned(buf) = core::mem::replace(&mut self.text, LabelText::Static(s)) {
            self.scratch = buf;
        }
        self.text_changed(cx);
    }

    /// Builds the text in place with `f` (which receives an empty buffer) and applies it if
    /// it differs from the current text; returns whether it changed. Two buffers are
    /// swapped, so formatting a text of a stable length never allocates.
    ///
    /// ```
    /// use core::fmt::Write;
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::label::{self, Label};
    ///
    /// let mut h = EngineHarness::new(80, 30);
    /// let screen = h.screen();
    /// let l = label::create(h.engine_mut(), screen).unwrap();
    /// let changed = h.engine_mut().with_widget_mut(l, |w: &mut Label, cx| {
    ///     w.write_text(cx, |s| { let _ = write!(s, "{} °C", 21); })
    /// });
    /// assert_eq!(changed, Some(true));
    /// assert_eq!(h.engine().widget::<Label>(l).unwrap().text(), "21 °C");
    /// ```
    pub fn write_text(&mut self, cx: &mut WidgetCx<'_>, f: impl FnOnce(&mut String)) -> bool {
        self.scratch.clear();
        f(&mut self.scratch);
        if self.scratch == self.text() {
            return false;
        }
        log_set(LABEL_CLASS.name, cx.node(), "text");
        match &mut self.text {
            LabelText::Owned(buf) => core::mem::swap(buf, &mut self.scratch),
            LabelText::Static(_) => {
                self.text = LabelText::Owned(core::mem::take(&mut self.scratch));
            }
        }
        self.text_changed(cx);
        true
    }

    /// Sets the long mode. Idempotent. Stops the scroll animations and resets the offset
    /// (LVGL `lv_label_set_long_mode`).
    pub fn set_long_mode(&mut self, cx: &mut WidgetCx<'_>, m: LongMode) {
        if self.long_mode == m {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "long_mode");
        self.scroll.stop(cx);
        self.long_mode = m;
        self.mark_need_refr(cx);
    }

    /// Limits the number of lines of a content-sized label (0 = unlimited). Idempotent.
    pub fn set_max_lines(&mut self, cx: &mut WidgetCx<'_>, n: u16) {
        if self.max_lines == n {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "max_lines");
        self.max_lines = n;
        self.mark_need_refr(cx);
    }

    /// Selects the bytes `start..end` (in any order; clamped to the text and rounded down to
    /// character boundaries). Idempotent; an empty range clears the selection.
    pub fn set_selection(&mut self, cx: &mut WidgetCx<'_>, start: usize, end: usize) {
        let t = self.shown_text();
        let fb = |i: usize| {
            let mut i = i.min(t.len());
            while !t.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        let (a, b) = (fb(start.min(end)), fb(start.max(end)));
        let sel = (a != b).then_some((a, b));
        if self.sel == sel {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "selection");
        self.sel = sel;
        cx.invalidate_for("label.set_selection");
    }

    /// Removes the selection. Idempotent.
    pub fn clear_selection(&mut self, cx: &mut WidgetCx<'_>) {
        if self.sel.is_none() {
            return;
        }
        log_set(LABEL_CLASS.name, cx.node(), "selection");
        self.sel = None;
        cx.invalidate_for("label.clear_selection");
    }

    /// Position of the character at byte `byte_idx` relative to the top-left corner of the
    /// content area (LVGL `lv_label_get_letter_pos`).
    #[must_use]
    pub fn letter_pos(&self, cx: &MeasureCx<'_>, byte_idx: usize) -> Point {
        let (layout, align, w) = self.draw_layout(cx);
        layout.pos_of(byte_idx, align, w)
    }

    /// Byte index of the character at `p` (relative to the top-left corner of the content
    /// area; LVGL `lv_label_get_letter_on`).
    #[must_use]
    pub fn letter_on(&self, cx: &MeasureCx<'_>, p: Point) -> usize {
        let (layout, align, w) = self.draw_layout(cx);
        layout.char_at(p, align, w)
    }

    /// Whether the text is laid out on one line only (no wrapping): `Scroll`,
    /// `ScrollCircular` and `Clip` (LVGL `expand`).
    fn expand(&self) -> bool {
        matches!(
            self.long_mode,
            LongMode::Scroll | LongMode::ScrollCircular | LongMode::Clip
        )
    }

    /// The layout as drawn in the current content area, with the alignment.
    fn draw_layout(&self, cx: &MeasureCx<'_>) -> (TextLayout<'_>, TextAlign, i32) {
        let c = cx.content_area();
        let d = cx.text_dsc(Part::Main);
        let mut layout = TextLayout::new(self.shown_text(), d.font);
        layout.letter_space = d.letter_space;
        layout.line_space = d.line_space;
        layout.max_width = c.width();
        if self.expand() {
            layout.flags |= TextFlags::EXPAND;
        }
        (layout, self.draw_align(d.align, c.width()), c.width())
    }

    /// LVGL: in the scrolling modes, center and right alignment make no sense for text wider
    /// than the area, so it is drawn left-aligned.
    fn draw_align(&self, align: TextAlign, w: i32) -> TextAlign {
        let align = match align {
            TextAlign::Auto => TextAlign::Left,
            a => a,
        };
        if matches!(self.long_mode, LongMode::Scroll | LongMode::ScrollCircular)
            && matches!(align, TextAlign::Center | TextAlign::Right)
            && self.text_size.w > w
        {
            TextAlign::Left
        } else {
            align
        }
    }

    /// After a text change: redraw, relayout when content-sized, refresh the long mode.
    fn text_changed(&mut self, cx: &mut WidgetCx<'_>) {
        if let Some(m) = &mut self.mask {
            m.update(self.text.as_str());
        }
        if let Some((_, b)) = self.sel {
            if b > self.shown_text().len() {
                self.sel = None;
            }
        }
        self.mark_need_refr(cx);
    }

    /// LVGL `lv_label_mark_need_refr_text`: invalidates (old area), marks the layout of a
    /// content-sized label and refreshes the long mode state.
    fn mark_need_refr(&mut self, cx: &mut WidgetCx<'_>) {
        cx.invalidate_for("label");
        let content_sized = [PropId::Width, PropId::Height]
            .iter()
            .any(|&p| cx.style(Part::Main, p).as_length() == Some(Length::Content));
        if content_sized {
            cx.mark_layout();
        }
        self.refr_text(cx);
    }
}

/// Creates a label showing "Text" as the last child of `parent` (LVGL `lv_label_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Label::default()))
}

/// Creates a label showing `text` (stored without copying).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create_with(engine: &mut Engine, parent: NodeId, text: &'static str) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Label::new(text)))
}

impl Widget for Label {
    fn class(&self) -> &'static WidgetClass {
        &LABEL_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let ext = Self::ext_for(&cx.measure());
        cx.refresh_ext_draw_with(ext);
        self.refr_text(cx);
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE` of the label: the text measured with the label's width
    /// (unlimited when the width is `Content`), limited by `max_lines` and `MaxHeight`.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        let m = Part::Main;
        let font = cx.font(m);
        let letter_space = cx.style_i32(m, PropId::TextLetterSpace);
        let line_space = cx.style_i32(m, PropId::TextLineSpace);
        let max_w = Self::max_width(cx);
        let key = LayoutKey {
            hash: text_hash(self.shown_text()),
            len: self.shown_text().len(),
            max_w,
            font: core::ptr::from_ref(font) as usize,
            letter_space,
            line_space,
            expand: self.expand(),
            max_lines: self.max_lines,
        };
        if let Some(c) = self.layout_cache.get() {
            if c.key == key {
                return c.size;
            }
        }
        let mut layout = TextLayout::new(self.shown_text(), font);
        layout.letter_space = letter_space;
        layout.line_space = line_space;
        layout.max_width = max_w;
        if self.expand() {
            layout.flags |= TextFlags::EXPAND;
        }
        let mut size = layout.measure();
        if self.max_lines > 0 {
            let n = i32::from(self.max_lines);
            size.h = size.h.min(i32::from(font.line_height) * n + line_space * (n - 1));
        }
        if let Some(Length::Px(mh)) = cx.style(m, PropId::MaxHeight).as_length() {
            size.h = size.h.min(mh);
        }
        self.layout_cache.set(Some(LayoutCache { key, size }));
        size
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let content = cx.content_area();
        if content.is_empty() && self.long_mode != LongMode::Wrap {
            return;
        }
        let mut dsc = cx.text_dsc(Part::Main);
        dsc.align = self.draw_align(dsc.align, content.width());
        if self.expand() {
            dsc.flags |= TextDrawFlags::EXPAND;
        }
        dsc.ofs = self.scroll.ofs;
        if let Some((a, b)) = self.sel {
            dsc.sel_start = Some(a);
            dsc.sel_end = Some(b);
            let sel_text = cx.text_dsc(Part::Selected);
            dsc.sel_color = sel_text.color;
            dsc.sel_bg_color = cx.rect_dsc(Part::Selected).base.bg_color;
        }
        if self.long_mode == LongMode::Dots {
            dsc.ellipsis_lines = self.dot.lines.map(usize::from);
        }
        let clip = cx.clip();
        // Scrolling and clipping modes clip to the content area; the others only at the
        // bottom (glyphs may overhang sideways, LVGL `draw_main`).
        let text_clip = if self.expand() {
            content.intersection(&clip)
        } else {
            Some(Rect::new(clip.x0, clip.y0, clip.x1, clip.y1.min(content.y1))).filter(|r| !r.is_empty())
        };
        let Some(text_clip) = text_clip else {
            return;
        };
        let text = self.shown_text();
        // The text area reaches the bottom of the label in wrap mode (LVGL).
        let area = if self.long_mode == LongMode::Wrap {
            Rect::new(content.x0, content.y0, content.x1, cx.coords().y1.max(content.y1))
        } else {
            content
        };
        let (size, circular) = (self.text_size, self.long_mode == LongMode::ScrollCircular);
        let ofs = self.scroll.ofs;
        cx.with_clip(text_clip, |cx| {
            cx.draw_text(area, text, &dsc);
            if circular {
                // The second copy, one gap after the first (LVGL `draw_main`).
                if size.w > content.width() {
                    let mut d2 = dsc;
                    d2.ofs = Point::new(ofs.x + size.w + long::gap_width(dsc.font), ofs.y);
                    cx.draw_text(area, text, &d2);
                }
                if size.h > content.height() {
                    let mut d2 = dsc;
                    d2.ofs = Point::new(ofs.x, ofs.y + size.h + i32::from(dsc.font.line_height));
                    cx.draw_text(area, text, &d2);
                }
            }
        });
    }

    /// LVGL: labels draw `line_height / 4` outside their area (italic and other fonts whose
    /// glyphs overhang).
    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        Self::ext_for(cx)
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && matches!(ev.code, EventCode::StyleChanged | EventCode::SizeChanged) {
            let mut wcx = cx.widget_cx();
            if ev.code == EventCode::StyleChanged {
                let ext = Self::ext_for(&wcx.measure());
                if wcx.refresh_ext_draw_with(ext) {
                    wcx.invalidate();
                }
            }
            self.mark_need_refr(&mut wcx);
        }
        EventResult::Continue
    }

    /// The text as drawn (masked text shows its bullets).
    fn text(&self) -> Option<&str> {
        Some(self.shown_text())
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        self.scroll.apply_anim(cx, id, v);
    }
}

impl Label {
    fn ext_for(cx: &MeasureCx<'_>) -> u16 {
        let h = i32::from(cx.font(Part::Main).line_height);
        u16::try_from(h / 4).unwrap_or(0)
    }

    /// The width the text may use for measuring (LVGL `GET_SELF_SIZE`): unlimited for a
    /// content-sized label (then `MaxWidth` applies), else the resolved width minus padding
    /// and border (flex-grown labels use their current content width).
    fn max_width(cx: &MeasureCx<'_>) -> i32 {
        let m = Part::Main;
        let pad = cx.padding(m);
        let border = cx.style_i32(m, PropId::BorderWidth).max(0);
        let spaces = pad.left + pad.right + 2 * border;
        let grow = cx.style_i32(m, PropId::FlexGrow) > 0;
        let w = match cx.style(m, PropId::Width).as_length() {
            _ if grow => cx.content_area().width(),
            Some(Length::Px(w)) => w - spaces,
            Some(Length::Pct(p)) => {
                let parent = cx
                    .engine()
                    .tree()
                    .parent(cx.node())
                    .map_or(0, |p| cx.engine().content_area(p).width());
                (i64::from(parent) * i64::from(p) / 100) as i32 - spaces
            }
            _ => COORD_MAX,
        };
        let max = match cx.style(m, PropId::MaxWidth).as_length() {
            Some(Length::Px(v)) if v < COORD_MAX => v - spaces,
            _ => COORD_MAX,
        };
        w.min(max).max(0)
    }
}
