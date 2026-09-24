//! The editing model of the [`Textarea`]: inserting and deleting at the cursor, constraints,
//! the insert filter and cursor moves (LVGL `lv_textarea_add_char`, `add_text`,
//! `delete_char`, `set_text`, `set_cursor_pos`, `cursor_up/down/left/right`).

use alloc::boxed::Box;
use alloc::string::String;

use twine_core::Point;
use twine_engine::{Engine, EventCode, EventParam, Key, MeasureCx, WidgetCx, fmt_node_id};
use twine_style::{Part, PropId};

use super::{CURSOR_LAST, DELETE_TEXT, Textarea, byte_of};
use crate::label::Label;

/// A filter for text about to be inserted into a [`Textarea`] (see
/// [`Textarea::set_insert_filter`]).
pub type InsertFilter = Box<dyn FnMut(&mut InsertCx<'_>)>;

/// What an [`InsertFilter`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Keep,
    Replace,
    Cancel,
}

/// The text about to be inserted, for an [`InsertFilter`] (LVGL `LV_EVENT_INSERT` with
/// `lv_textarea_set_insert_replace`).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::textarea::{self, InsertCx, Textarea};
///
/// let mut h = EngineHarness::new(160, 80);
/// let screen = h.screen();
/// let ta = textarea::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(ta, |t: &mut Textarea, cx| {
///     // Upper-case everything, drop digits.
///     t.set_insert_filter(Some(Box::new(|ins: &mut InsertCx<'_>| {
///         if ins.text().chars().any(|c| c.is_ascii_digit()) {
///             ins.cancel();
///         } else if ins.text().chars().any(|c| c.is_lowercase()) {
///             let up = ins.text().to_uppercase();
///             ins.replace(&up);
///         }
///     })));
///     t.add_text(cx, "ab");
///     t.add_char(cx, '7');
/// });
/// assert_eq!(textarea::text_of(h.engine(), ta), Some("AB"));
/// ```
#[derive(Debug)]
pub struct InsertCx<'a> {
    text: &'a str,
    replacement: &'a mut String,
    action: Action,
}

impl InsertCx<'_> {
    /// The text to insert ([`DELETE_TEXT`] before a deletion).
    #[must_use]
    pub fn text(&self) -> &str {
        self.text
    }

    /// Whether a character is about to be deleted.
    #[must_use]
    pub fn is_delete(&self) -> bool {
        self.text == DELETE_TEXT
    }

    /// Inserts `s` instead (an empty `s` cancels, LVGL).
    pub fn replace(&mut self, s: &str) {
        if s.is_empty() {
            self.cancel();
            return;
        }
        self.replacement.clear();
        self.replacement.push_str(s);
        self.action = Action::Replace;
    }

    /// Inserts (or deletes) nothing.
    pub fn cancel(&mut self) {
        self.action = Action::Cancel;
    }
}

impl Textarea {
    /// The label's text.
    fn label_text<'e>(&self, e: &'e Engine) -> &'e str {
        e.widget::<Label>(self.label).map_or("", Label::text)
    }

    /// The number of characters.
    fn char_count(&self, e: &Engine) -> usize {
        self.label_text(e).chars().count()
    }

    /// Sends `ValueChanged` (unless a spinbox handles it).
    fn value_changed(&self, cx: &mut WidgetCx<'_>) {
        if self.quiet {
            return;
        }
        let n = cx.node();
        cx.engine_mut()
            .send_event(n, EventCode::ValueChanged, EventParam::None);
    }

    fn log_edit(&self, cx: &WidgetCx<'_>, what: &str) {
        twine_core::trace!(
            target: "twine::engine",
            "textarea#{} {} pos {} len {}",
            fmt_node_id(cx.node()),
            what,
            self.cursor.pos,
            self.label_text(cx.engine()).len()
        );
        let _ = what;
    }

    /// LVGL `insert_handler`: sends `Insert` and runs the filter. `false` when the text was
    /// cancelled or replaced (the replacement is inserted here).
    fn insert_handler(&mut self, cx: &mut WidgetCx<'_>, txt: &str) -> bool {
        let n = cx.node();
        let mut chars = txt.chars();
        let param = match (chars.next(), chars.next()) {
            _ if txt == DELETE_TEXT => EventParam::Key(Key::Backspace),
            (Some(c), None) => EventParam::Key(Key::Char(c)),
            _ => EventParam::None,
        };
        cx.engine_mut().send_event(n, EventCode::Insert, param);
        if self.in_replace {
            return true;
        }
        let Some(f) = self.insert_filter.as_mut() else {
            return true;
        };
        let mut ins = InsertCx {
            text: txt,
            replacement: &mut self.replace_buf,
            action: Action::Keep,
        };
        f(&mut ins);
        match ins.action {
            Action::Keep => true,
            Action::Cancel => false,
            Action::Replace if self.replace_buf == txt => true,
            Action::Replace => {
                let r = core::mem::take(&mut self.replace_buf);
                // LVGL adds the replacement instead (also for a deletion).
                self.in_replace = true;
                self.add_text(cx, &r);
                self.in_replace = false;
                self.replace_buf = r;
                false
            }
        }
    }

    /// LVGL `char_is_accepted`: the length limit and the accepted list. `extra` characters
    /// are about to be removed (a selection being replaced).
    fn char_is_accepted(&self, e: &Engine, c: char, extra: usize) -> bool {
        if self.max_length > 0 {
            let len = self.char_count(e).saturating_sub(extra);
            if len >= self.max_length as usize {
                return false;
            }
        }
        self.accepted.as_ref().is_none_or(|a| a.accepts(c))
    }

    /// The selected character range, if any.
    fn sel_range(&self, e: &Engine) -> Option<(usize, usize)> {
        let l = e.widget::<Label>(self.label)?;
        let (a, b) = l.selection()?;
        let s = l.shown_text();
        Some((super::char_of(s, a), super::char_of(s, b)))
    }

    /// Deletes the selected text and puts the cursor at its start. `true` if there was one.
    fn delete_selection(&mut self, cx: &mut WidgetCx<'_>) -> bool {
        let Some((a, b)) = self.sel_range(cx.engine()) else {
            return false;
        };
        let text = self.label_text(cx.engine());
        let (ba, bb) = (byte_of(text, a), byte_of(text, b));
        let label = self.label;
        cx.engine_mut().with_widget_mut(label, |l: &mut Label, lcx| {
            l.clear_selection(lcx);
            l.edit_text(lcx, |s| {
                s.replace_range(ba..bb, "");
            });
        });
        self.sel.end = None;
        self.cursor.pos = usize::MAX; // force the move below
        self.set_cursor_pos(cx, i32::try_from(a).unwrap_or(CURSOR_LAST));
        self.log_edit(cx, "delete selection");
        true
    }

    /// Redraws the placeholder area when the text is (or becomes) empty.
    fn placeholder_refresh(&self, cx: &mut WidgetCx<'_>) {
        if self.placeholder.is_some() && self.label_text(cx.engine()).is_empty() {
            cx.invalidate_for("textarea.placeholder");
        }
    }

    /// Inserts `s` at the cursor in the label's buffer (in place).
    fn insert_at_cursor(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        let b = self.cursor.byte;
        let label = self.label;
        cx.engine_mut().with_widget_mut(label, |l: &mut Label, lcx| {
            l.edit_text(lcx, |t| {
                let b = b.min(t.len());
                t.insert_str(b, s);
            });
        });
    }

    /// LVGL's static `add_char`: inserts `c` at the cursor without moving it. `false` when
    /// rejected (one-line line break, filter, constraints).
    fn add_char_inner(&mut self, cx: &mut WidgetCx<'_>, c: char) -> bool {
        if self.one_line && (c == '\n' || c == '\r') {
            twine_core::debug!(target: "twine::engine", "textarea: line break ignored in one-line mode");
            return false;
        }
        let mut buf = [0u8; 4];
        let s: &str = c.encode_utf8(&mut buf);
        if !self.insert_handler(cx, s) {
            return false;
        }
        let sel = self.sel_range(cx.engine()).map_or(0, |(a, b)| b - a);
        if !self.char_is_accepted(cx.engine(), c, sel) {
            twine_core::debug!(target: "twine::engine", "textarea: {:?} not accepted (length or list)", c);
            return false;
        }
        self.delete_selection(cx);
        self.placeholder_refresh(cx);
        self.insert_at_cursor(cx, s);
        self.clear_selection(cx);
        if self.pwd.on {
            self.auto_hide(cx, self.cursor.pos);
        }
        self.log_edit(cx, "add");
        true
    }

    /// Inserts `c` at the cursor and moves the cursor after it (LVGL
    /// `lv_textarea_add_char`). Rejected characters (see the constraints) are ignored.
    pub fn add_char(&mut self, cx: &mut WidgetCx<'_>, c: char) {
        if !self.add_char_inner(cx, c) {
            return;
        }
        let p = self.cursor.pos + 1;
        self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(CURSOR_LAST));
        self.value_changed(cx);
    }

    /// LVGL's static `add_text`: character by character (constraints apply to each).
    fn add_text_chars(&mut self, cx: &mut WidgetCx<'_>, s: &str) -> bool {
        let mut changed = false;
        for c in s.chars() {
            if !self.add_char_inner(cx, c) {
                continue;
            }
            self.move_cursor(cx.engine(), self.cursor.pos + 1);
            changed = true;
        }
        if changed {
            self.scroll_to_cursor(cx);
        }
        changed
    }

    /// Inserts `s` at the cursor and moves the cursor after it (LVGL
    /// `lv_textarea_add_text`). With a length limit or an accepted list the characters are
    /// checked one by one.
    pub fn add_text(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        if s.is_empty() {
            return;
        }
        if self.accepted.is_some() || self.max_length > 0 || self.one_line {
            if self.add_text_chars(cx, s) {
                self.value_changed(cx);
            }
            return;
        }
        if !self.insert_handler(cx, s) {
            return;
        }
        self.delete_selection(cx);
        self.placeholder_refresh(cx);
        self.insert_at_cursor(cx, s);
        self.clear_selection(cx);
        let n = s.chars().count();
        if self.pwd.on {
            self.auto_hide(cx, self.cursor.pos + n - 1);
        }
        self.log_edit(cx, "add text");
        let p = self.cursor.pos + n;
        self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(CURSOR_LAST));
        self.value_changed(cx);
    }

    /// Deletes the character before the cursor (backspace; LVGL
    /// `lv_textarea_delete_char`), or the selected text.
    pub fn delete_char(&mut self, cx: &mut WidgetCx<'_>) {
        if self.sel_range(cx.engine()).is_some() {
            if self.insert_handler(cx, DELETE_TEXT) && self.delete_selection(cx) {
                self.placeholder_refresh(cx);
                self.value_changed(cx);
            }
            return;
        }
        let pos = self.cursor.pos;
        if pos == 0 {
            return;
        }
        if !self.insert_handler(cx, DELETE_TEXT) {
            return;
        }
        let text = self.label_text(cx.engine());
        let b = self.cursor.byte.min(text.len());
        let Some((pb, _)) = text[..b].char_indices().next_back() else {
            return;
        };
        let label = self.label;
        cx.engine_mut().with_widget_mut(label, |l: &mut Label, lcx| {
            l.edit_text(lcx, |t| {
                t.replace_range(pb..b, "");
            });
        });
        self.clear_selection(cx);
        self.placeholder_refresh(cx);
        // The cursor's byte moved with the deleted character.
        self.cursor.byte = pb;
        self.log_edit(cx, "delete");
        self.set_cursor_pos(cx, i32::try_from(pos - 1).unwrap_or(0));
        self.value_changed(cx);
    }

    /// Deletes the character at the cursor (the `Del` key; LVGL
    /// `lv_textarea_delete_char_forward`), or the selected text.
    pub fn delete_char_forward(&mut self, cx: &mut WidgetCx<'_>) {
        if self.sel_range(cx.engine()).is_some() {
            self.delete_char(cx);
            return;
        }
        let cp = self.cursor.pos;
        self.set_cursor_pos(cx, i32::try_from(cp + 1).unwrap_or(CURSOR_LAST));
        if self.cursor.pos != cp {
            self.delete_char(cx);
        }
    }

    /// Replaces the text and puts the cursor at the end (LVGL `lv_textarea_set_text`; with a
    /// length limit or an accepted list the characters are checked one by one). Idempotent:
    /// the same text changes nothing (the cursor stays).
    pub fn set_text(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        if self.label_text(cx.engine()) == s {
            return;
        }
        self.set_text_with(cx, s);
    }

    /// [`set_text`](Self::set_text) without the equality check; returns whether the text
    /// changed.
    pub(crate) fn set_text_with(&mut self, cx: &mut WidgetCx<'_>, s: &str) -> bool {
        self.clear_selection(cx);
        let label = self.label;
        let changed = if self.accepted.is_some() || self.max_length > 0 || self.one_line {
            cx.engine_mut()
                .with_widget_mut(label, |l: &mut Label, lcx| l.edit_text(lcx, String::clear));
            self.cursor.pos = usize::MAX;
            self.set_cursor_pos(cx, CURSOR_LAST);
            self.add_text_chars(cx, s)
        } else {
            cx.engine_mut()
                .with_widget_mut(label, |l: &mut Label, lcx| l.set_text(lcx, s));
            self.cursor.pos = usize::MAX;
            self.set_cursor_pos(cx, CURSOR_LAST);
            true
        };
        self.placeholder_refresh(cx);
        if self.pwd.on {
            self.pwd_char_hider(cx);
        }
        self.log_edit(cx, "set text");
        if changed {
            self.value_changed(cx);
        }
        changed
    }

    /// Replaces the text without events, filters or constraints and puts the cursor at the
    /// end (a spinbox formatting its value; the label's buffer is reused).
    pub(crate) fn replace_text_raw(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        let label = self.label;
        cx.engine_mut()
            .with_widget_mut(label, |l: &mut Label, lcx| l.set_text(lcx, s));
        self.cursor.pos = usize::MAX;
        self.set_cursor_pos(cx, CURSOR_LAST);
    }

    /// Moves the cursor fields to character `pos` (clamped), without scrolling.
    fn move_cursor(&mut self, e: &Engine, pos: usize) {
        let text = self.label_text(e);
        let len = text.chars().count();
        let pos = pos.min(len);
        self.cursor.pos = pos;
        self.cursor.byte = byte_of(text, pos);
    }

    /// Places the cursor before character `pos` (LVGL `lv_textarea_set_cursor_pos`):
    /// negative positions count from the end, [`CURSOR_LAST`] (or anything past the end) is
    /// the end. Scrolls to the cursor and restarts the blink. Idempotent.
    pub fn set_cursor_pos(&mut self, cx: &mut WidgetCx<'_>, pos: i32) {
        let len = self.char_count(cx.engine());
        let p = if pos < 0 {
            usize::try_from(i64::try_from(len).unwrap_or(i64::MAX) + i64::from(pos)).unwrap_or(0)
        } else {
            usize::try_from(pos).unwrap_or(usize::MAX).min(len)
        };
        if p == self.cursor.pos {
            return;
        }
        self.move_cursor(cx.engine(), p);
        self.scroll_to_cursor(cx);
    }

    /// Moves the cursor one character right (LVGL `lv_textarea_cursor_right`).
    pub fn cursor_right(&mut self, cx: &mut WidgetCx<'_>) {
        let p = self.cursor.pos + 1;
        self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(CURSOR_LAST));
    }

    /// Moves the cursor one character left (LVGL `lv_textarea_cursor_left`).
    pub fn cursor_left(&mut self, cx: &mut WidgetCx<'_>) {
        if self.cursor.pos > 0 {
            let p = self.cursor.pos - 1;
            self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(0));
        }
    }

    /// The label's letter position of the cursor and the line step.
    fn cursor_letter(&self, cx: &WidgetCx<'_>) -> Option<(Point, i32)> {
        let e = cx.engine();
        let l = e.widget::<Label>(self.label)?;
        let lm = MeasureCx::new(e, self.label);
        let b = byte_of(l.shown_text(), self.cursor.pos);
        let pos = l.letter_pos(&lm, b);
        let m = cx.measure();
        let step = i32::from(m.font(Part::Main).line_height) + m.style_i32(Part::Main, PropId::TextLineSpace);
        Some((pos, step))
    }

    /// Moves the cursor one line down, keeping its x (LVGL `lv_textarea_cursor_down`).
    pub fn cursor_down(&mut self, cx: &mut WidgetCx<'_>) {
        let Some((mut pos, step)) = self.cursor_letter(cx) else {
            return;
        };
        pos.y += step + 1;
        pos.x = self.cursor.valid_x;
        let e = cx.engine();
        let label_h = e.coords(self.label).height();
        if pos.y >= label_h {
            return;
        }
        let Some(l) = e.widget::<Label>(self.label) else {
            return;
        };
        let b = l.letter_on(&MeasureCx::new(e, self.label), pos);
        let p = super::char_of(l.shown_text(), b);
        let x = self.cursor.valid_x;
        self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(CURSOR_LAST));
        self.cursor.valid_x = x;
    }

    /// Moves the cursor one line up, keeping its x (LVGL `lv_textarea_cursor_up`).
    pub fn cursor_up(&mut self, cx: &mut WidgetCx<'_>) {
        let Some((mut pos, step)) = self.cursor_letter(cx) else {
            return;
        };
        pos.y -= step - 1;
        pos.x = self.cursor.valid_x;
        let e = cx.engine();
        let Some(l) = e.widget::<Label>(self.label) else {
            return;
        };
        let b = l.letter_on(&MeasureCx::new(e, self.label), pos);
        let p = super::char_of(l.shown_text(), b);
        let x = self.cursor.valid_x;
        self.set_cursor_pos(cx, i32::try_from(p).unwrap_or(CURSOR_LAST));
        self.cursor.valid_x = x;
    }
}
