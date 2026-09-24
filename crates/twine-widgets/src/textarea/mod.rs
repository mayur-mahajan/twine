//! [`Textarea`]: a scrollable text input with a cursor (LVGL `lv_textarea`).

mod edit;

use alloc::boxed::Box;
use alloc::string::String;

use twine_core::{Duration, Point, Rect};
use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventFilter, EventParam, EventResult,
    GroupDef, InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, State, TimerId, Widget, WidgetClass,
    WidgetCx,
};
use twine_style::{Align, Length, Part, PropId, Selector, StyleProp, TextAlign};
use twine_text::{TextDrawFlags, symbols};

use crate::label::{self, Label, LabelText};
use crate::spinbox::Spinbox;
use crate::{log_set, util};

pub use edit::{InsertCx, InsertFilter};

/// The placeholder part (LVGL `LV_PART_TEXTAREA_PLACEHOLDER` = `LV_PART_CUSTOM_FIRST`).
pub const PLACEHOLDER: Part = Part::CustomFirst;

/// [`Textarea::set_cursor_pos`] to the end of the text (LVGL `LV_TEXTAREA_CURSOR_LAST`).
pub const CURSOR_LAST: i32 = i32::MAX;

/// LVGL `LV_TEXTAREA_DEF_PWD_SHOW_TIME`: how long the last typed password character stays
/// visible.
pub const DEFAULT_PWD_SHOW_TIME: Duration = Duration::ms(1500);

/// The password bullet when the font has it (LVGL `LV_SYMBOL_BULLET`, U+2022).
pub const PWD_BULLET: &str = symbols::BULLET;

/// The text an [`InsertFilter`] sees for a deletion (LVGL sends `LV_KEY_DEL`).
pub const DELETE_TEXT: &str = "\u{7f}";

/// Default flags of [`TEXTAREA_CLASS`] (LVGL `lv_textarea_constructor`:
/// `lv_obj_set_scroll_on_focus(obj, true)`, `lv_obj_set_scroll_with_arrow(obj, false)`).
const TEXTAREA_FLAGS: ObjFlags = OBJ_FLAGS
    .union(ObjFlags::SCROLL_ON_FOCUS)
    .difference(ObjFlags::SCROLL_WITH_ARROW);

/// The class of [`Textarea`]: `"textarea"`, parts `Main`, `Scrollbar`, `Selected`, `Cursor`
/// and [`PLACEHOLDER`], editable with an encoder and always in the default focus group (LVGL
/// `lv_textarea_class`).
pub static TEXTAREA_CLASS: WidgetClass = WidgetClass::new("textarea")
    .parts(&[
        Part::Main,
        Part::Scrollbar,
        Part::Selected,
        Part::Cursor,
        PLACEHOLDER,
    ])
    .default_flags(TEXTAREA_FLAGS)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// LVGL `lv_textarea_class.width_def`: `LV_DPI_DEF * 2`.
pub const TEXTAREA_DEFAULT_WIDTH: i32 = util::DPI_DEF * 2;
/// LVGL `lv_textarea_class.height_def`: `LV_DPI_DEF`.
pub const TEXTAREA_DEFAULT_HEIGHT: i32 = util::DPI_DEF;

/// The characters a textarea accepts (LVGL `lv_textarea_set_accepted_chars`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AcceptedChars {
    /// A list in flash (LVGL `lv_textarea_set_accepted_chars_static`).
    Static(&'static str),
    /// An owned list.
    Owned(String),
}

impl AcceptedChars {
    /// The list.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            AcceptedChars::Static(s) => s,
            AcceptedChars::Owned(s) => s,
        }
    }

    /// Whether `c` is in the list (an empty list accepts everything, LVGL).
    #[must_use]
    pub fn accepts(&self, c: char) -> bool {
        let s = self.as_str();
        s.is_empty() || s.contains(c)
    }
}

/// The cursor of a [`Textarea`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorState {
    /// Position in characters (LVGL `cursor.pos`: a letter index).
    pub pos: usize,
    /// The byte offset of `pos` in the text.
    pub byte: usize,
    /// The drawn area relative to the label's content area (LVGL `cursor.area`).
    pub area: Rect,
    /// Whether the cursor is shown (the blink phase).
    pub show: bool,
    /// The x position kept when moving up and down (LVGL `cursor.valid_x`).
    pub valid_x: i32,
    /// The byte offset in the drawn (possibly masked) text of the letter under the cursor.
    shown_byte: usize,
}

/// Password mode state.
#[derive(Debug)]
struct PwdState {
    on: bool,
    bullet: Option<String>,
    show_time: Duration,
    /// The one-shot timer hiding the last typed character.
    timer: Option<TimerId>,
}

/// Text selection state (LVGL `sel_start`, `sel_end`, `text_sel_in_prog`), in characters.
#[derive(Clone, Copy, Debug, Default)]
struct SelState {
    start: usize,
    end: Option<usize>,
    in_progress: bool,
}

/// A text input (LVGL `lv_textarea`): a scrollable container holding a [`Label`] child with
/// the text, a blinking cursor, an optional placeholder, a password mode and input
/// constraints.
///
/// - **Editing** ([`add_char`](Self::add_char), [`add_text`](Self::add_text),
///   [`delete_char`](Self::delete_char), [`delete_char_forward`](Self::delete_char_forward))
///   works at the cursor, a **character** index; the label's buffer is edited in place, so
///   typing reuses its capacity. Before inserting, the `Insert` event is sent and the
///   [insert filter](Self::set_insert_filter) may replace or cancel the text; after every
///   change `ValueChanged` is sent, once the textarea is done: its handlers can read the
///   widget (or use [`text_of`]). `Insert` carries the text as
///   [`EventParam::Text`] (read it with
///   [`EventCx::text`](twine_engine::EventCx::text); a deletion sends [`DELETE_TEXT`]).
/// - **Constraints**: [`max_length`](Self::set_max_length) in characters, an
///   [accepted characters](Self::set_accepted_chars) list, and
///   [one-line mode](Self::set_one_line) (line breaks are dropped, `Enter` sends `Ready`, the
///   height follows the text and the text scrolls horizontally).
/// - **Cursor**: drawn with the `Cursor` part's styles (the default theme: a 2 px left
///   border, only while focused), blinking every `AnimDuration` of the cursor part while
///   the textarea is `FOCUSED`; an unfocused textarea runs no timer (P1). Every edit or
///   move shows the cursor and restarts the blink. The textarea scrolls to keep the cursor
///   line visible.
/// - **Pointer**: a press places the cursor at the pointer (unless
///   [`set_cursor_click_pos`](Self::set_cursor_click_pos)`(false)`); with
///   [text selection](Self::set_text_selection) a drag selects (drawn with the label's
///   `Selected` part). Typing replaces the selection.
/// - **Keys**: arrows, `Home`, `End`, `Backspace`, `Del`, characters, `Enter` (a line break,
///   or `Ready` in one-line mode).
/// - **Password mode**: the label shows bullets ([`PWD_BULLET`], or `*` if the font lacks
///   it), the last typed character stays visible for the
///   [show time](Self::set_password_show_time).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::textarea::{self, Textarea};
///
/// let mut h = EngineHarness::new(200, 100);
/// let screen = h.screen();
/// let ta = textarea::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(ta, |t: &mut Textarea, cx| {
///     t.add_text(cx, "Hello");
///     t.set_cursor_pos(cx, 0);
///     t.add_char(cx, '>');
/// });
/// assert_eq!(textarea::text_of(h.engine(), ta), Some(">Hello"));
/// assert_eq!(h.engine().widget::<Textarea>(ta).unwrap().cursor_pos(), 1);
/// ```
pub struct Textarea {
    label: NodeId,
    cursor: CursorState,
    placeholder: Option<LabelText>,
    pwd: PwdState,
    one_line: bool,
    max_length: u32,
    accepted: Option<AcceptedChars>,
    sel: SelState,
    text_sel_en: bool,
    cursor_click_pos: bool,
    blink: Option<TimerId>,
    blink_running: bool,
    insert_filter: Option<InsertFilter>,
    replace_buf: String,
    in_replace: bool,
    /// No `ValueChanged` from the text edits (a spinbox sends its own).
    pub(crate) quiet: bool,
}

impl core::fmt::Debug for Textarea {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Textarea")
            .field("label", &self.label)
            .field("cursor", &self.cursor)
            .field("one_line", &self.one_line)
            .field("max_length", &self.max_length)
            .field("password", &self.pwd.on)
            .finish_non_exhaustive()
    }
}

impl Default for Textarea {
    fn default() -> Self {
        Self::new()
    }
}

/// The byte offset of character `pos` in `s` (the end when `pos` is past it).
pub(crate) fn byte_of(s: &str, pos: usize) -> usize {
    s.char_indices().nth(pos).map_or(s.len(), |(b, _)| b)
}

/// The character index of byte `b` in `s`.
pub(crate) fn char_of(s: &str, b: usize) -> usize {
    s.char_indices().take_while(|&(i, _)| i < b).count()
}

impl Textarea {
    /// An empty textarea (its label is created when it is inserted into a tree).
    #[must_use]
    pub fn new() -> Self {
        Self {
            label: NodeId::from_raw(u32::MAX),
            cursor: CursorState {
                show: true,
                ..CursorState::default()
            },
            placeholder: None,
            pwd: PwdState {
                on: false,
                bullet: None,
                show_time: DEFAULT_PWD_SHOW_TIME,
                timer: None,
            },
            one_line: false,
            max_length: 0,
            accepted: None,
            sel: SelState::default(),
            text_sel_en: false,
            cursor_click_pos: true,
            blink: None,
            blink_running: false,
            insert_filter: None,
            replace_buf: String::new(),
            in_replace: false,
            quiet: false,
        }
    }

    /// The label node holding the text (LVGL `lv_textarea_get_label`).
    #[must_use]
    pub fn label(&self) -> NodeId {
        self.label
    }

    /// The text (the real text in password mode; LVGL `lv_textarea_get_text`).
    #[must_use]
    pub fn text<'e>(&self, cx: &MeasureCx<'e>) -> &'e str {
        cx.engine().widget::<Label>(self.label).map_or("", Label::text)
    }

    /// The cursor state.
    #[must_use]
    pub fn cursor(&self) -> CursorState {
        self.cursor
    }

    /// The cursor position in characters (LVGL `lv_textarea_get_cursor_pos`).
    #[must_use]
    pub fn cursor_pos(&self) -> usize {
        self.cursor.pos
    }

    /// The cursor's absolute area as drawn.
    #[must_use]
    pub fn cursor_area(&self, cx: &MeasureCx<'_>) -> Rect {
        let o = cx.engine().content_area(self.label);
        self.cursor.area.translate(o.x0, o.y0)
    }

    /// The placeholder text (`""` when none).
    #[must_use]
    pub fn placeholder_text(&self) -> &str {
        self.placeholder.as_ref().map_or("", LabelText::as_str)
    }

    /// Whether password mode is on.
    #[must_use]
    pub fn password_mode(&self) -> bool {
        self.pwd.on
    }

    /// The password bullet: the one set, else [`PWD_BULLET`] if the font has it, else `"*"`
    /// (LVGL `lv_textarea_get_password_bullet`).
    #[must_use]
    pub fn password_bullet<'s>(&'s self, cx: &MeasureCx<'_>) -> &'s str {
        if let Some(b) = &self.pwd.bullet {
            return b;
        }
        let font = cx.font(Part::Main);
        match font.glyph('\u{2022}', None) {
            Some((_, g)) if !g.is_placeholder => PWD_BULLET,
            _ => "*",
        }
    }

    /// How long the last typed password character stays visible.
    #[must_use]
    pub fn password_show_time(&self) -> Duration {
        self.pwd.show_time
    }

    /// Whether one-line mode is on.
    #[must_use]
    pub fn one_line(&self) -> bool {
        self.one_line
    }

    /// The accepted characters.
    #[must_use]
    pub fn accepted_chars(&self) -> Option<&AcceptedChars> {
        self.accepted.as_ref()
    }

    /// The maximum length in characters (0 = unlimited).
    #[must_use]
    pub fn max_length(&self) -> u32 {
        self.max_length
    }

    /// Whether a press places the cursor.
    #[must_use]
    pub fn cursor_click_pos(&self) -> bool {
        self.cursor_click_pos
    }

    /// Whether text selection with the pointer is enabled.
    #[must_use]
    pub fn text_selection(&self) -> bool {
        self.text_sel_en
    }

    /// Whether some text is selected (LVGL `lv_textarea_text_is_selected`).
    #[must_use]
    pub fn text_is_selected(&self, cx: &MeasureCx<'_>) -> bool {
        cx.engine()
            .widget::<Label>(self.label)
            .is_some_and(|l| l.selection().is_some())
    }

    /// The selected character range (start < end), if any.
    #[must_use]
    pub fn selection(&self, cx: &MeasureCx<'_>) -> Option<(usize, usize)> {
        let l = cx.engine().widget::<Label>(self.label)?;
        let (a, b) = l.selection()?;
        let s = l.shown_text();
        Some((char_of(s, a), char_of(s, b)))
    }

    /// The character before the cursor (LVGL `lv_textarea_get_current_char`).
    #[must_use]
    pub fn current_char(&self, cx: &MeasureCx<'_>) -> Option<char> {
        let t = self.text(cx);
        t[..self.cursor.byte.min(t.len())].chars().next_back()
    }

    /// Sets (or removes) the placeholder shown while the text is empty (LVGL
    /// `lv_textarea_set_placeholder_text`). Idempotent.
    pub fn set_placeholder_text(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        if self.placeholder_text() == s {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "placeholder_text");
        if s.is_empty() {
            self.placeholder = None;
        } else {
            match &mut self.placeholder {
                Some(LabelText::Owned(b)) => {
                    b.clear();
                    b.push_str(s);
                }
                p => *p = Some(LabelText::Owned(String::from(s))),
            }
        }
        cx.invalidate_for("textarea.placeholder");
    }

    /// Sets a `'static` placeholder without copying. Idempotent.
    pub fn set_placeholder_text_static(&mut self, cx: &mut WidgetCx<'_>, s: &'static str) {
        if self.placeholder_text() == s {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "placeholder_text");
        self.placeholder = (!s.is_empty()).then_some(LabelText::Static(s));
        cx.invalidate_for("textarea.placeholder");
    }

    /// Enables placing the cursor by pressing (LVGL `lv_textarea_set_cursor_click_pos`).
    /// Idempotent.
    pub fn set_cursor_click_pos(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.cursor_click_pos != on {
            log_set(TEXTAREA_CLASS.name, cx.node(), "cursor_click_pos");
            self.cursor_click_pos = on;
        }
    }

    /// Limits the text to `n` characters (0 = unlimited; LVGL
    /// `lv_textarea_set_max_length`). Idempotent; an existing longer text is kept.
    pub fn set_max_length(&mut self, cx: &mut WidgetCx<'_>, n: u32) {
        if self.max_length != n {
            log_set(TEXTAREA_CLASS.name, cx.node(), "max_length");
            self.max_length = n;
        }
    }

    /// Accepts only the listed characters (`None` accepts all; LVGL
    /// `lv_textarea_set_accepted_chars`). Idempotent.
    pub fn set_accepted_chars(&mut self, cx: &mut WidgetCx<'_>, list: Option<AcceptedChars>) {
        if self.accepted.as_ref().map(AcceptedChars::as_str) == list.as_ref().map(AcceptedChars::as_str) {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "accepted_chars");
        self.accepted = list;
    }

    /// Enables selecting text with the pointer (LVGL `lv_textarea_set_text_selection`;
    /// disabling clears the selection). Idempotent.
    pub fn set_text_selection(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.text_sel_en == on {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "text_selection");
        self.text_sel_en = on;
        if !on {
            self.clear_selection(cx);
        }
    }

    /// Installs a filter called with every text about to be inserted (and with
    /// [`DELETE_TEXT`] before a deletion); it may [replace](InsertCx::replace) or
    /// [cancel](InsertCx::cancel) it. This is LVGL's `LV_EVENT_INSERT` +
    /// `lv_textarea_set_insert_replace`.
    pub fn set_insert_filter(&mut self, f: Option<InsertFilter>) {
        self.insert_filter = f;
    }

    /// Clears the selection (LVGL `lv_textarea_clear_selection`). Idempotent.
    pub fn clear_selection(&mut self, cx: &mut WidgetCx<'_>) {
        self.sel.end = None;
        let label = self.label;
        cx.engine_mut()
            .with_widget_mut(label, |l: &mut Label, lcx| l.clear_selection(lcx));
    }

    /// Selects the characters `start..end`. Idempotent.
    pub fn set_selection(&mut self, cx: &mut WidgetCx<'_>, start: usize, end: usize) {
        let label = self.label;
        cx.engine_mut().with_widget_mut(label, |l: &mut Label, lcx| {
            let s = l.shown_text();
            let (a, b) = (byte_of(s, start), byte_of(s, end));
            l.set_selection(lcx, a, b);
        });
    }

    /// Password mode on or off (LVGL `lv_textarea_set_password_mode`). Idempotent.
    pub fn set_password_mode(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.pwd.on == on {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "password_mode");
        self.pwd.on = on;
        self.clear_selection(cx);
        if on {
            self.pwd_char_hider(cx);
        } else {
            self.pwd_timer_pause(cx.engine_mut());
            let label = self.label;
            cx.engine_mut()
                .with_widget_mut(label, |l: &mut Label, lcx| l.set_mask(lcx, None, None));
            self.refr_cursor_area(cx);
        }
    }

    /// Sets the password bullet (`None`: [`PWD_BULLET`] or `*`; LVGL
    /// `lv_textarea_set_password_bullet`). Idempotent.
    pub fn set_password_bullet(&mut self, cx: &mut WidgetCx<'_>, bullet: Option<&str>) {
        if self.pwd.bullet.as_deref() == bullet {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "password_bullet");
        self.pwd.bullet = bullet.map(String::from);
        self.pwd_char_hider(cx);
    }

    /// How long the last typed password character stays visible (0: hidden at once; LVGL
    /// `lv_textarea_set_password_show_time`). Idempotent.
    pub fn set_password_show_time(&mut self, cx: &mut WidgetCx<'_>, t: Duration) {
        if self.pwd.show_time == t {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "password_show_time");
        self.pwd.show_time = t;
        self.pwd_char_hider(cx);
    }

    /// One-line mode on or off (LVGL `lv_textarea_set_one_line`): the label gets the width of
    /// its text (at least the textarea's), the textarea the height of one line, line breaks
    /// are dropped and `Enter` sends `Ready`. Idempotent.
    pub fn set_one_line(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.one_line == on {
            return;
        }
        log_set(TEXTAREA_CLASS.name, cx.node(), "one_line");
        self.one_line = on;
        let (id, label) = (cx.node(), self.label);
        let e = cx.engine_mut();
        if on {
            e.set_width(label, Length::Content);
            e.set_local_prop(label, Selector::MAIN, StyleProp::MinWidth(Length::Pct(100)));
            e.set_height(id, Length::Content);
        } else {
            e.set_width(label, Length::Pct(100));
            e.set_local_prop(label, Selector::MAIN, StyleProp::MinWidth(Length::Px(0)));
            e.remove_local_prop(id, PropId::Height, Selector::MAIN);
        }
        e.scroll_to(id, 0, 0, false);
        cx.invalidate_for("textarea.one_line");
    }

    // ---- Blink ---------------------------------------------------------------------------

    /// LVGL `start_cursor_blink`: shows the cursor and restarts the blink period, or stops the
    /// blink when the textarea is not focused or the cursor part has no `AnimDuration`.
    pub(crate) fn start_blink(&mut self, cx: &mut WidgetCx<'_>) {
        let t = u32::try_from(cx.style_i32(Part::Cursor, PropId::AnimDuration)).unwrap_or(0);
        let focused = cx.state().contains(State::FOCUSED);
        if !self.cursor.show {
            self.cursor.show = true;
            self.invalidate_cursor(cx);
        }
        let e = cx.engine_mut();
        if t == 0 || !focused {
            if let Some(tid) = self.blink {
                e.timer_pause(tid);
            }
            self.blink_running = false;
            return;
        }
        self.blink_running = true;
        let period = Duration::ms(u64::from(t));
        if let Some(tid) = self.blink.filter(|tid| e.timer_exists(*tid)) {
            e.timer_set_period(tid, period);
            e.timer_reset(tid);
            e.timer_resume(tid);
        } else {
            let id = cx.node();
            self.blink = Some(cx.engine_mut().timer_add(period, move |e, tid| {
                let alive = with_textarea(e, id, Textarea::blink_tick);
                if alive.is_none() && !e.tree().contains(id) {
                    e.timer_remove(tid);
                }
            }));
            twine_core::debug!(target: "twine::engine", "textarea#{} blink every {} ms", twine_engine::fmt_node_id(id), t);
        }
    }

    /// Stops blinking (the cursor stays "shown"; its styles hide it when unfocused).
    pub(crate) fn stop_blink(&mut self, cx: &mut WidgetCx<'_>) {
        if let Some(tid) = self.blink {
            cx.engine_mut().timer_pause(tid);
        }
        self.blink_running = false;
        if !self.cursor.show {
            self.cursor.show = true;
            self.invalidate_cursor(cx);
        }
    }

    /// LVGL `cursor_blink_anim_cb`: toggles the cursor and redraws only its area.
    fn blink_tick(&mut self, cx: &mut WidgetCx<'_>) {
        if !cx.state().contains(State::FOCUSED) {
            self.stop_blink(cx);
            return;
        }
        self.cursor.show = !self.cursor.show;
        self.invalidate_cursor(cx);
    }

    fn invalidate_cursor(&self, cx: &mut WidgetCx<'_>) {
        let o = cx.engine().content_area(self.label);
        let r = self.cursor.area.translate(o.x0, o.y0);
        if !r.is_empty() {
            cx.invalidate_area(r);
        }
    }

    /// Whether a blink timer is running (for tests and diagnostics).
    #[must_use]
    pub fn is_blinking(&self, cx: &MeasureCx<'_>) -> bool {
        self.blink_running && self.blink.is_some_and(|t| cx.engine().timer_exists(t))
    }

    // ---- Password --------------------------------------------------------------------------

    /// LVGL `pwd_char_hider`: every character shown as a bullet.
    fn pwd_char_hider(&mut self, cx: &mut WidgetCx<'_>) {
        if !self.pwd.on {
            return;
        }
        self.pwd_timer_pause(cx.engine_mut());
        let bullet = self.password_bullet(&cx.measure());
        let mut buf = [0u8; 16];
        let bullet = copy_small(bullet, &mut buf);
        let label = self.label;
        cx.engine_mut()
            .with_widget_mut(label, |l: &mut Label, lcx| l.set_mask(lcx, Some(bullet), None));
        self.refr_cursor_area(cx);
    }

    fn pwd_timer_pause(&self, e: &mut Engine) {
        if let Some(t) = self.pwd.timer {
            e.timer_pause(t);
        }
    }

    /// LVGL `auto_hide_characters`: the character at `reveal` stays visible for the show time.
    fn auto_hide(&mut self, cx: &mut WidgetCx<'_>, reveal: usize) {
        if self.pwd.show_time == Duration::ZERO {
            self.pwd_char_hider(cx);
            return;
        }
        let bullet = self.password_bullet(&cx.measure());
        let mut buf = [0u8; 16];
        let bullet = copy_small(bullet, &mut buf);
        let label = self.label;
        cx.engine_mut().with_widget_mut(label, |l: &mut Label, lcx| {
            l.set_mask(lcx, Some(bullet), Some(reveal));
        });
        let t = self.pwd.show_time;
        let e = cx.engine_mut();
        if let Some(tid) = self.pwd.timer.filter(|tid| e.timer_exists(*tid)) {
            e.timer_set_period(tid, t);
            e.timer_reset(tid);
            e.timer_resume(tid);
        } else {
            let id = cx.node();
            self.pwd.timer = Some(cx.engine_mut().timer_add(t, move |e, tid| {
                let alive = with_textarea(e, id, Textarea::pwd_char_hider);
                if alive.is_none() && !e.tree().contains(id) {
                    e.timer_remove(tid);
                }
            }));
        }
    }

    // ---- Cursor geometry --------------------------------------------------------------------

    /// LVGL `refr_cursor_area`: the cursor's area from the letter under it and the cursor
    /// part's border and padding; redraws the old and the new area.
    pub(crate) fn refr_cursor_area(&mut self, cx: &mut WidgetCx<'_>) {
        let e = cx.engine();
        let Some(l) = e.widget::<Label>(self.label) else {
            return;
        };
        let lm = MeasureCx::new(e, self.label);
        let m = cx.measure();
        let font = m.font(Part::Main);
        let line_space = m.style_i32(Part::Main, PropId::TextLineSpace);
        let shown = l.shown_text();
        let mut byte = byte_of(shown, self.cursor.pos);
        let letter = shown[byte..].chars().next();
        let printable = |c: Option<char>| match c {
            None | Some('\n' | '\r') => ' ',
            Some(c) => c,
        };
        let letter_h = i32::from(font.line_height);
        let mut letter_w = font.advance_px(printable(letter), None);
        let mut pos = l.letter_pos(&lm, byte);
        let label_c = e.content_area(self.label);
        let align = lm.text_dsc(Part::Main).align;
        // The cursor out of the text on the right is drawn at the start of the next line.
        if label_c.x0 + pos.x + letter_w > label_c.x1 - 1 && !self.one_line && align != TextAlign::Right {
            pos.x = 0;
            pos.y += letter_h + line_space;
            if let Some(c) = letter {
                byte += c.len_utf8();
            }
            letter_w = font.advance_px(printable(shown[byte..].chars().next()), None);
        }
        let bw = m.style_i32(Part::Cursor, PropId::BorderWidth);
        let pad = m.padding(Part::Cursor);
        let (top, bottom) = (pad.top + bw, pad.bottom + bw);
        let (left, right) = (pad.left + bw, pad.right + bw);
        let ls = lm.style_i32(Part::Main, PropId::TextLetterSpace);
        let area = Rect::new(
            pos.x - left - ls / 2,
            pos.y - top,
            pos.x + right + letter_w + (ls + 1) / 2,
            pos.y + bottom + letter_h,
        );
        self.cursor.shown_byte = byte;
        if area != self.cursor.area {
            self.invalidate_cursor(cx);
            self.cursor.area = area;
        }
        self.invalidate_cursor(cx);
    }

    /// LVGL `lv_textarea_scroll_to_cursor_pos`: scrolls so the cursor line is visible, keeps
    /// the x for up/down moves, restarts the blink and refreshes the cursor area.
    fn scroll_to_cursor(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine();
        if let Some(l) = e.widget::<Label>(self.label) {
            let lm = MeasureCx::new(e, self.label);
            let byte = byte_of(l.shown_text(), self.cursor.pos);
            let cur = l.letter_pos(&lm, byte);
            let m = cx.measure();
            let font_h = i32::from(m.font(Part::Main).line_height);
            let content = m.content_area();
            let (h, w) = (content.height(), content.width());
            let anim = m.style_i32(Part::Main, PropId::AnimDuration) > 0;
            let scroll = e.scroll_offset(id);
            let (top, left) = (scroll.y, e.scroll_left(id));
            let e = cx.engine_mut();
            if cur.y < top {
                e.scroll_to_y(id, cur.y, anim);
            } else if cur.y + font_h - top > h {
                e.scroll_to_y(id, cur.y - h + font_h, anim);
            }
            if cur.x < left {
                e.scroll_to_x(id, cur.x, anim);
            } else if cur.x + font_h > left + w {
                e.scroll_to_x(id, cur.x - w + font_h, anim);
            }
            self.cursor.valid_x = cur.x;
        }
        self.start_blink(cx);
        self.refr_cursor_area(cx);
    }

    /// The cursor position under the absolute point `p` (LVGL
    /// `update_cursor_position_on_click`): 0 left of the label, the end right of it. Also
    /// whether the point is outside the text.
    fn char_at_point(&self, e: &Engine, p: Point) -> (usize, bool) {
        let Some(l) = e.widget::<Label>(self.label) else {
            return (0, true);
        };
        let lc = e.content_area(self.label);
        let rel = Point::new(p.x - lc.x0, p.y - lc.y0);
        let shown = l.shown_text();
        if rel.x < 0 {
            (0, true)
        } else if rel.x >= lc.width() {
            (shown.chars().count(), true)
        } else {
            let b = l.letter_on(&MeasureCx::new(e, self.label), rel);
            let outside = rel.y < 0 || rel.y >= lc.height();
            (char_of(shown, b), outside)
        }
    }

    /// LVGL `update_cursor_position_on_click`.
    fn click_pos(&mut self, cx: &mut EventCx<'_>, code: EventCode) {
        if !self.cursor_click_pos {
            return;
        }
        if matches!(
            util::active_input_kind(cx.engine()),
            Some(InputKind::Keypad | InputKind::Encoder) | None
        ) {
            return;
        }
        let Some(p) = cx.point() else {
            return;
        };
        if p.x < 0 || p.y < 0 {
            return;
        }
        let node = cx.node();
        let (id, outside) = self.char_at_point(cx.engine(), p);
        if self.text_sel_en {
            if !self.sel.in_progress && !outside && code == EventCode::Pressed {
                self.sel = SelState {
                    start: id,
                    end: None,
                    in_progress: true,
                };
                cx.engine_mut().set_flag(node, ObjFlags::SCROLL_CHAIN, false);
            } else if self.sel.in_progress && code == EventCode::Pressing {
                self.sel.end = Some(id);
            } else if self.sel.in_progress && matches!(code, EventCode::PressLost | EventCode::Released) {
                cx.engine_mut().set_flag(node, ObjFlags::SCROLL_CHAIN, true);
            }
        }
        let mut wcx = cx.widget_cx();
        if self.sel.in_progress || code == EventCode::Pressed {
            self.set_cursor_pos(&mut wcx, i32::try_from(id).unwrap_or(CURSOR_LAST));
        }
        if self.sel.in_progress {
            match self.sel.end {
                Some(end) if end != self.sel.start => {
                    let (a, b) = (self.sel.start.min(end), self.sel.start.max(end));
                    self.set_selection(&mut wcx, a, b);
                }
                _ => {
                    let label = self.label;
                    wcx.engine_mut()
                        .with_widget_mut(label, |l: &mut Label, lcx| l.clear_selection(lcx));
                }
            }
            if matches!(code, EventCode::PressLost | EventCode::Released) {
                self.sel.in_progress = false;
            }
        }
    }

    /// Called when the label was resized or restyled (LVGL `label_event_cb`).
    fn label_changed(&mut self, cx: &mut WidgetCx<'_>) {
        self.scroll_to_cursor(cx);
    }
}

/// The text of the textarea or spinbox `id`, read from its label, so it also works while the
/// widget itself is busy (e.g. in handlers of the `Insert` event it sends, or of a
/// `ValueChanged` sent to a spinbox's textarea through its own handlers).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::textarea::{self, Textarea};
/// let mut h = EngineHarness::new(160, 80);
/// let screen = h.screen();
/// let ta = textarea::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(ta, |t: &mut Textarea, cx| t.set_text(cx, "abc"));
/// assert_eq!(textarea::text_of(h.engine(), ta), Some("abc"));
/// ```
#[must_use]
pub fn text_of(engine: &Engine, id: NodeId) -> Option<&str> {
    let label = if let Some(t) = engine.widget::<Textarea>(id) {
        t.label
    } else if let Some(s) = engine.widget::<Spinbox>(id) {
        s.textarea().label
    } else {
        // The widget is busy: its label is its first child.
        engine
            .tree()
            .children(id)
            .find(|c| engine.widget::<Label>(*c).is_some())?
    };
    engine.widget::<Label>(label).map(Label::text)
}

/// Calls `f` with the [`Textarea`] of `id` (a textarea, or a spinbox's textarea). `None` when
/// `id` is neither, is gone, or is busy (its own event or setter is running).
pub fn with_textarea<R>(
    engine: &mut Engine,
    id: NodeId,
    f: impl FnOnce(&mut Textarea, &mut WidgetCx<'_>) -> R,
) -> Option<R> {
    if engine.widget::<Textarea>(id).is_some() {
        return engine.with_widget_mut(id, f);
    }
    if engine.widget::<Spinbox>(id).is_some() {
        return engine.with_widget_mut(id, |s: &mut Spinbox, cx| f(s.textarea_mut(), cx));
    }
    None
}

/// Copies a short string into `buf` (the bullet, while the widget is borrowed).
fn copy_small<'b>(s: &str, buf: &'b mut [u8; 16]) -> &'b str {
    let mut n = s.len().min(16);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    core::str::from_utf8(&buf[..n]).unwrap_or("*")
}

/// Creates a textarea, 260 × 130 px, as the last child of `parent` (LVGL
/// `lv_textarea_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Textarea::new()))
}

impl Textarea {
    /// LVGL `lv_textarea_constructor`: the label child and the default size.
    pub(crate) fn construct(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, TEXTAREA_DEFAULT_WIDTH, TEXTAREA_DEFAULT_HEIGHT);
        let Ok(label) = label::create_with(e, id, "") else {
            return;
        };
        e.set_width(label, Length::Pct(100));
        e.align(label, Align::TopLeft, 0, 0);
        for code in [EventCode::SizeChanged, EventCode::StyleChanged] {
            e.add_event_handler(label, EventFilter::Code(code), move |cx, ev| {
                if ev.target == cx.node() {
                    with_textarea(cx.engine_mut(), id, Textarea::label_changed);
                }
                EventResult::Continue
            });
        }
        self.label = label;
        self.refr_cursor_area(cx);
    }

    /// The textarea part of the event handling (LVGL `lv_textarea_event`); `false` when the
    /// event is not for this node.
    pub(crate) fn handle(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> bool {
        if ev.target != cx.node() {
            return false;
        }
        match ev.code {
            // Gaining or losing `FOCUSED` by any means (focus groups, a keyboard attaching,
            // `add_state` from application code) starts or stops the blink.
            EventCode::StateChanged => {
                let prev = ev.prev_state().unwrap_or_default();
                let mut wcx = cx.widget_cx();
                let now = wcx.state();
                if now.contains(State::FOCUSED) && !prev.contains(State::FOCUSED) {
                    self.start_blink(&mut wcx);
                } else if !now.contains(State::FOCUSED) && prev.contains(State::FOCUSED) {
                    self.stop_blink(&mut wcx);
                }
            }
            EventCode::Leave => self.stop_blink(&mut cx.widget_cx()),
            EventCode::Key => {
                if let Some(k) = ev.key() {
                    self.key(cx, k);
                }
            }
            EventCode::Pressed | EventCode::Pressing | EventCode::PressLost | EventCode::Released => {
                self.click_pos(cx, ev.code);
            }
            EventCode::SizeChanged => self.scroll_to_cursor(&mut cx.widget_cx()),
            EventCode::StyleChanged => {
                let mut wcx = cx.widget_cx();
                self.refr_cursor_area(&mut wcx);
                if wcx.state().contains(State::FOCUSED) {
                    self.start_blink(&mut wcx);
                }
            }
            EventCode::Delete => {
                let e = cx.engine_mut();
                for t in [self.blink.take(), self.pwd.timer.take()].into_iter().flatten() {
                    e.timer_remove(t);
                }
            }
            _ => {}
        }
        true
    }

    /// LVGL `LV_EVENT_KEY` of the textarea.
    fn key(&mut self, cx: &mut EventCx<'_>, k: Key) {
        let node = cx.node();
        let mut wcx = cx.widget_cx();
        match k {
            Key::Right => self.cursor_right(&mut wcx),
            Key::Left => self.cursor_left(&mut wcx),
            Key::Up => self.cursor_up(&mut wcx),
            Key::Down => self.cursor_down(&mut wcx),
            Key::Backspace => self.delete_char(&mut wcx),
            Key::Del => self.delete_char_forward(&mut wcx),
            Key::Home => self.set_cursor_pos(&mut wcx, 0),
            Key::End => self.set_cursor_pos(&mut wcx, CURSOR_LAST),
            Key::Enter if self.one_line => {
                cx.send(node, EventCode::Ready, EventParam::None);
            }
            Key::Enter => self.add_char(&mut wcx, '\n'),
            Key::Char(c) => self.add_char(&mut wcx, c),
            // LVGL would insert the key code as a character; Twine ignores navigation keys.
            Key::Esc | Key::Next | Key::Prev => {}
        }
    }

    /// Draws the placeholder (LVGL `draw_placeholder`).
    fn draw_placeholder(&self, cx: &mut DrawCx<'_, '_>) {
        let Some(ph) = self.placeholder.as_ref().map(LabelText::as_str) else {
            return;
        };
        let e = cx.engine();
        if ph.is_empty() || e.widget::<Label>(self.label).is_none_or(|l| !l.text().is_empty()) {
            return;
        }
        let mut dsc = cx.text_dsc(PLACEHOLDER);
        if self.one_line {
            dsc.flags |= TextDrawFlags::EXPAND;
        }
        let pad = MeasureCx::new(e, cx.node()).padding(PLACEHOLDER);
        let lc = e.content_area(self.label);
        let area = if self.one_line {
            // The label is as wide as its (empty) text: use the textarea's content width.
            let c = cx.content_area();
            Rect::new(lc.x0, lc.y0, lc.x0.max(c.x1), lc.y1.max(c.y1))
        } else {
            lc
        };
        let area = area.translate(pad.left, pad.top);
        cx.draw_text(
            Rect::new(area.x0, area.y0, area.x1, area.y1.max(area.y0 + 1)),
            ph,
            &dsc,
        );
    }

    /// Draws the cursor (LVGL `draw_cursor`).
    fn draw_cursor(&self, cx: &mut DrawCx<'_, '_>) {
        if !self.cursor.show {
            return;
        }
        let e = cx.engine();
        let Some(l) = e.widget::<Label>(self.label) else {
            return;
        };
        let o = e.content_area(self.label);
        let area = self.cursor.area.translate(o.x0, o.y0);
        let rs = cx.rect_dsc(Part::Cursor);
        cx.painter().rect(area, &rs.dsc());
        let bw = cx.style_i32(Part::Cursor, PropId::BorderWidth);
        let left = cx.style_i32(Part::Cursor, PropId::PadLeft) + bw;
        let top = cx.style_i32(Part::Cursor, PropId::PadTop) + bw;
        let label_color = e.style_color(self.label, Part::Main, PropId::TextColor);
        let mut t = cx.text_dsc(Part::Cursor);
        // Draw the letter again only over a cursor background or in another color, else the
        // letter would look bold (LVGL).
        if rs.base.bg_opa.0 > 2 || t.color != label_color {
            let shown = l.shown_text();
            let b = self.cursor.shown_byte.min(shown.len());
            let letter = shown[b..].chars().next().map_or(0, char::len_utf8);
            let txt = &shown[b..b + letter];
            if !txt.is_empty() && txt != "\n" && txt != "\r" {
                t.align = TextAlign::Left;
                let a = Rect::new(
                    area.x0 + left,
                    area.y0 + top,
                    area.x1,
                    area.y1.max(area.y0 + top + 1),
                );
                cx.draw_text(a, txt, &t);
            }
        }
    }
}

impl Widget for Textarea {
    fn class(&self) -> &'static WidgetClass {
        &TEXTAREA_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        self.construct(cx);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.draw_placeholder(cx);
    }

    fn draw_post(&self, cx: &mut DrawCx<'_, '_>) {
        self.draw_cursor(cx);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        self.handle(cx, ev);
        EventResult::Continue
    }
}
