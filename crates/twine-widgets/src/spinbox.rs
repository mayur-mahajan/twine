//! [`Spinbox`]: a numeric editor with digit-wise stepping (LVGL `lv_spinbox`).

use alloc::boxed::Box;

use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, GroupDef,
    InputKind, Key, MeasureCx, NodeId, Widget, WidgetClass, WidgetCx,
};
use twine_style::{Dir, Length};

use crate::textarea::{self, Textarea};
use crate::{log_set, util};

/// The most digits a spinbox shows (the value is an `i32`).
pub const SPINBOX_MAX_DIGITS: u8 = 10;

/// The class of [`Spinbox`]: `"spinbox"`, the textarea's parts, flags and focus behaviour
/// (LVGL `lv_spinbox_class`, a subclass of `lv_textarea_class`).
pub static SPINBOX_CLASS: WidgetClass = WidgetClass::new("spinbox")
    .parts(textarea::TEXTAREA_CLASS.parts)
    .default_flags(textarea::TEXTAREA_CLASS.default_flags)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// LVGL `lv_spinbox_class.width_def`: `LV_DPI_DEF`.
pub const SPINBOX_DEFAULT_WIDTH: i32 = util::DPI_DEF;

/// `10^n` saturated to `i64`.
fn pow10(n: u32) -> i64 {
    10i64.checked_pow(n).unwrap_or(i64::MAX)
}

/// Formats `value` like LVGL `lv_spinbox_updatevalue` into `buf`: a sign when negative
/// values are possible, leading zeros up to `digit_count`, a decimal point after
/// `dec_point_pos` integer digits (0: none). Returns the text and the cursor position for
/// `step`.
///
/// ```
/// use twine_widgets::spinbox::format_value;
/// let mut buf = [0u8; 16];
/// assert_eq!(format_value(&mut buf, 42, 5, 0, true, 1).0, "+00042");
/// assert_eq!(format_value(&mut buf, -1234, 5, 3, true, 10).0, "-012.34");
/// assert_eq!(format_value(&mut buf, 7, 3, 0, false, 1), ("007", 2));
/// ```
#[must_use]
pub fn format_value(
    buf: &mut [u8; 16],
    value: i32,
    digit_count: u8,
    dec_point_pos: u8,
    signed: bool,
    step: u32,
) -> (&str, usize) {
    let dc = usize::from(digit_count.clamp(1, SPINBOX_MAX_DIGITS));
    let mut n = 0;
    let mut cur_shift_left = 0;
    if signed {
        buf[n] = if value >= 0 { b'+' } else { b'-' };
        n += 1;
    } else {
        cur_shift_left = 1;
    }
    // The digits of |value|, least significant first.
    let mut digits = [b'0'; 10];
    let mut v = value.unsigned_abs();
    let mut len = 0;
    while v > 0 || len == 0 {
        digits[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    let total = dc.max(len);
    let int_digits = if dec_point_pos == 0 {
        total
    } else {
        usize::from(dec_point_pos)
    };
    for i in 0..total {
        if dec_point_pos != 0 && i == int_digits {
            buf[n] = b'.';
            n += 1;
        }
        let from_right = total - 1 - i;
        buf[n] = if from_right < len {
            digits[from_right]
        } else {
            b'0'
        };
        n += 1;
    }
    // The cursor on the digit of `step` (LVGL: `cur_pos = digit_count`, one left per decade).
    let mut cur = dc;
    let mut s = step;
    while s >= 10 {
        s /= 10;
        cur = cur.saturating_sub(1);
    }
    if dec_point_pos != 0 && cur > int_digits {
        cur += 1;
    }
    cur = cur.saturating_sub(cur_shift_left);
    (core::str::from_utf8(&buf[..n]).unwrap_or("0"), cur)
}

/// A numeric editor (LVGL `lv_spinbox`): a one-line [`Textarea`] showing an integer with a
/// fixed number of digits, an optional sign and decimal point, and a cursor highlighting
/// the digit that [`increment`](Self::increment) / [`decrement`](Self::decrement) change.
///
/// - **Keys**: `Left` / `Right` move the step digit, `Up` / `Down` increment and decrement.
/// - **Encoder** (edit mode): turning increments or decrements, a click moves to the next
///   digit (LVGL).
/// - **Pointer**: pressing a digit selects its step.
/// - **Events**: `ValueChanged` with the new value as [`EventParam::Value`] when a key, the
///   encoder, [`increment`](Self::increment) or [`decrement`](Self::decrement) changes it.
/// - Formatting writes into the label's buffer in place: no allocation after the first.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::spinbox::{self, Spinbox};
///
/// let mut h = EngineHarness::new(200, 80);
/// let screen = h.screen();
/// let s = spinbox::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(s, |w: &mut Spinbox, cx| {
///     w.set_range(cx, 0, 999);
///     w.set_digit_format(cx, 3, 0);
///     w.set_value(cx, 41);
///     w.increment(cx);
/// });
/// assert_eq!(h.engine().widget::<Spinbox>(s).unwrap().value(), 42);
/// assert_eq!(twine_widgets::textarea::text_of(h.engine(), s), Some("042"));
/// ```
#[derive(Debug)]
pub struct Spinbox {
    ta: Textarea,
    value: i32,
    range: (i32, i32),
    digit_count: u8,
    dec_point_pos: u8,
    step: u32,
    digit_step_dir: Dir,
    rollover: bool,
}

impl Default for Spinbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinbox {
    /// A spinbox with LVGL's defaults: value 0, range −99 999…99 999, 5 digits, step 1, no
    /// rollover, digits stepping to the right.
    #[must_use]
    pub fn new() -> Self {
        let mut ta = Textarea::new();
        ta.quiet = true;
        Self {
            ta,
            value: 0,
            range: (-99_999, 99_999),
            digit_count: 5,
            dec_point_pos: 0,
            step: 1,
            digit_step_dir: Dir::RIGHT,
            rollover: false,
        }
    }

    /// The textarea part.
    #[must_use]
    pub fn textarea(&self) -> &Textarea {
        &self.ta
    }

    /// The textarea part, mutably (text constraints, cursor…).
    pub fn textarea_mut(&mut self) -> &mut Textarea {
        &mut self.ta
    }

    /// The value.
    #[must_use]
    pub fn value(&self) -> i32 {
        self.value
    }

    /// The range `(min, max)`.
    #[must_use]
    pub fn range(&self) -> (i32, i32) {
        self.range
    }

    /// The step (a power of ten: the digit the cursor is on).
    #[must_use]
    pub fn step(&self) -> u32 {
        self.step
    }

    /// The number of digits.
    #[must_use]
    pub fn digit_count(&self) -> u8 {
        self.digit_count
    }

    /// The number of integer digits before the decimal point (0: none).
    #[must_use]
    pub fn dec_point_pos(&self) -> u8 {
        self.dec_point_pos
    }

    /// Whether stepping past an end wraps to the other end.
    #[must_use]
    pub fn rollover(&self) -> bool {
        self.rollover
    }

    /// The direction the step digit moves on an encoder click.
    #[must_use]
    pub fn digit_step_direction(&self) -> Dir {
        self.digit_step_dir
    }

    /// Sets the value, clamped to the range (LVGL `lv_spinbox_set_value`; no
    /// `ValueChanged`). Idempotent.
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        let v = v.clamp(self.range.0, self.range.1.max(self.range.0));
        if v == self.value {
            return;
        }
        log_set(SPINBOX_CLASS.name, cx.node(), "value");
        self.value = v;
        self.update_value(cx);
    }

    /// Sets the range; the value is clamped (LVGL `lv_spinbox_set_range`). Idempotent.
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, min: i32, max: i32) {
        if self.range == (min, max) {
            return;
        }
        log_set(SPINBOX_CLASS.name, cx.node(), "range");
        self.range = (min, max);
        self.value = self.value.min(max).max(min);
        self.update_value(cx);
    }

    /// Sets the number of digits (1…10) and the integer digits before the decimal point
    /// (0: none; ≥ `count` also none). The range shrinks to what the digits can show (LVGL
    /// `lv_spinbox_set_digit_format`). Idempotent.
    pub fn set_digit_format(&mut self, cx: &mut WidgetCx<'_>, count: u8, sep_pos: u8) {
        let count = count.clamp(1, SPINBOX_MAX_DIGITS);
        let sep = if sep_pos >= count { 0 } else { sep_pos };
        if (count, sep) == (self.digit_count, self.dec_point_pos) {
            return;
        }
        log_set(SPINBOX_CLASS.name, cx.node(), "digit_format");
        if count < SPINBOX_MAX_DIGITS {
            let max = pow10(u32::from(count)) - 1;
            let (lo, hi) = self.range;
            self.range = (
                i32::try_from(i64::from(lo).max(-max)).unwrap_or(lo),
                i32::try_from(i64::from(hi).min(max)).unwrap_or(hi),
            );
            self.value = self.value.clamp(self.range.0, self.range.1.max(self.range.0));
        }
        self.digit_count = count;
        self.dec_point_pos = sep;
        self.update_value(cx);
    }

    /// Sets the step (LVGL `lv_spinbox_set_step`; use powers of ten). Idempotent.
    pub fn set_step(&mut self, cx: &mut WidgetCx<'_>, step: u32) {
        if self.step == step {
            return;
        }
        log_set(SPINBOX_CLASS.name, cx.node(), "step");
        self.step = step.max(1);
        self.update_value(cx);
    }

    /// Wrap around at the ends (LVGL `lv_spinbox_set_rollover`). Idempotent.
    pub fn set_rollover(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.rollover != on {
            log_set(SPINBOX_CLASS.name, cx.node(), "rollover");
            self.rollover = on;
        }
    }

    /// The direction an encoder click moves the step digit (`Dir::RIGHT` or `Dir::LEFT`;
    /// LVGL `lv_spinbox_set_digit_step_direction`). Idempotent.
    pub fn set_digit_step_direction(&mut self, cx: &mut WidgetCx<'_>, dir: Dir) {
        if self.digit_step_dir != dir {
            log_set(SPINBOX_CLASS.name, cx.node(), "digit_step_direction");
            self.digit_step_dir = dir;
            self.update_value(cx);
        }
    }

    /// Puts the step on digit `pos` counted from the right (0 = ones; LVGL
    /// `lv_spinbox_set_cursor_pos`). Ignored when that digit exceeds the range.
    pub fn set_cursor_pos(&mut self, cx: &mut WidgetCx<'_>, pos: u32) {
        let limit = i64::from(self.range.1.max(self.range.0.saturating_abs()));
        let new = pow10(pos);
        let step = if pos == 0 {
            1
        } else if new <= limit {
            u32::try_from(new).unwrap_or(self.step)
        } else {
            self.step
        };
        self.set_step(cx, step);
    }

    /// Moves the step one digit right (÷ 10, at least 1; LVGL `lv_spinbox_step_next`).
    pub fn step_next(&mut self, cx: &mut WidgetCx<'_>) {
        let s = (self.step / 10).max(1);
        self.set_step(cx, s);
    }

    /// Moves the step one digit left (× 10 while within the range; LVGL
    /// `lv_spinbox_step_prev`).
    pub fn step_prev(&mut self, cx: &mut WidgetCx<'_>) {
        let limit = i64::from(self.range.1.max(self.range.0.saturating_abs()));
        let new = i64::from(self.step) * 10;
        if new <= limit {
            self.set_step(cx, u32::try_from(new).unwrap_or(self.step));
        }
    }

    /// Adds the step (LVGL `lv_spinbox_increment`: crossing zero keeps the digits, e.g.
    /// −3 + 10 = 3; at the maximum it wraps with rollover, else clamps). Sends
    /// `ValueChanged` when the value changes.
    pub fn increment(&mut self, cx: &mut WidgetCx<'_>) {
        let (step, value) = (i64::from(self.step), i64::from(self.value));
        let mut v = value;
        if value < 0 && value + step > 0 {
            v = -(step + value);
        }
        if v + step <= i64::from(self.range.1) {
            v += step;
        } else if self.rollover && self.value == self.range.1 {
            v = i64::from(self.range.0);
        } else {
            v = i64::from(self.range.1);
        }
        self.change_to(cx, v);
    }

    /// Subtracts the step (LVGL `lv_spinbox_decrement`, mirrored). Sends `ValueChanged` when
    /// the value changes.
    pub fn decrement(&mut self, cx: &mut WidgetCx<'_>) {
        let (step, value) = (i64::from(self.step), i64::from(self.value));
        let mut v = value;
        if value > 0 && value - step < 0 {
            v = step - value;
        }
        if v - step >= i64::from(self.range.0) {
            v -= step;
        } else if self.rollover && self.value == self.range.0 {
            v = i64::from(self.range.1);
        } else {
            v = i64::from(self.range.0);
        }
        self.change_to(cx, v);
    }

    fn change_to(&mut self, cx: &mut WidgetCx<'_>, v: i64) {
        let v = i32::try_from(v).unwrap_or(self.value);
        if v == self.value {
            return;
        }
        self.value = v;
        util::log_value(SPINBOX_CLASS.name, v);
        self.update_value(cx);
        let n = cx.node();
        cx.engine_mut()
            .send_event(n, EventCode::ValueChanged, EventParam::Value(v));
    }

    /// LVGL `lv_spinbox_updatevalue`: the text and the cursor on the step digit.
    fn update_value(&mut self, cx: &mut WidgetCx<'_>) {
        let mut buf = [0u8; 16];
        let (txt, cur) = format_value(
            &mut buf,
            self.value,
            self.digit_count,
            self.dec_point_pos,
            self.range.0 < 0,
            self.step,
        );
        if textarea::text_of(cx.engine(), cx.node()) != Some(txt) {
            self.ta.replace_text_raw(cx, txt);
        }
        self.ta
            .set_cursor_pos(cx, i32::try_from(cur).unwrap_or(textarea::CURSOR_LAST));
    }

    /// LVGL `LV_EVENT_RELEASED` of the spinbox.
    fn released(&mut self, cx: &mut EventCx<'_>) {
        let node = cx.node();
        let e = cx.engine();
        let editing = e.group_of(node).is_some_and(|g| e.group_editing(g));
        let encoder = util::active_input_kind(e) == Some(InputKind::Encoder);
        let mut wcx = cx.widget_cx();
        if encoder && editing {
            if self.digit_count > 1 {
                let top = pow10(u32::from(self.digit_count) - 1);
                if self.digit_step_dir == Dir::RIGHT {
                    if self.step > 1 {
                        self.step_next(&mut wcx);
                    } else {
                        // Restart from the most significant digit.
                        self.step = u32::try_from(pow10(u32::from(self.digit_count) - 2)).unwrap_or(1);
                        self.step_prev_forced(&mut wcx);
                    }
                } else if i64::from(self.step) < top {
                    self.step_prev(&mut wcx);
                } else {
                    // Restart from the least significant digit.
                    self.step = 10;
                    self.step_next_forced(&mut wcx);
                }
            }
            return;
        }
        // The cursor was put on a digit by the press: take its step.
        let text = textarea::text_of(wcx.engine(), node).unwrap_or("");
        let len = text.chars().count();
        let pos = self.ta.cursor_pos();
        if text.as_bytes().get(pos) == Some(&b'.') {
            self.ta.cursor_left(&mut wcx);
        } else if pos == len {
            self.ta
                .set_cursor_pos(&mut wcx, i32::try_from(len.saturating_sub(1)).unwrap_or(0));
        } else if pos == 0 && self.range.0 < 0 {
            self.ta.set_cursor_pos(&mut wcx, 1);
        }
        let mut cp = self.ta.cursor_pos();
        if cp > usize::from(self.dec_point_pos) && self.dec_point_pos != 0 {
            cp -= 1;
        }
        let mut p = usize::from(self.digit_count) - 1;
        p = p.saturating_sub(cp);
        if self.range.0 < 0 {
            p += 1;
        }
        let step = u32::try_from(pow10(u32::try_from(p).unwrap_or(0))).unwrap_or(1);
        self.step = step;
        self.update_value(&mut wcx);
    }

    fn step_prev_forced(&mut self, cx: &mut WidgetCx<'_>) {
        let limit = i64::from(self.range.1.max(self.range.0.saturating_abs()));
        let new = i64::from(self.step) * 10;
        if new <= limit {
            self.step = u32::try_from(new).unwrap_or(self.step);
        }
        self.update_value(cx);
    }

    fn step_next_forced(&mut self, cx: &mut WidgetCx<'_>) {
        self.step = (self.step / 10).max(1);
        self.update_value(cx);
    }
}

/// Creates a spinbox, 130 px wide and one line high, as the last child of `parent` (LVGL
/// `lv_spinbox_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Spinbox::new()))
}

impl Widget for Spinbox {
    fn class(&self) -> &'static WidgetClass {
        &SPINBOX_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        self.ta.construct(cx);
        let id = cx.node();
        cx.engine_mut().set_width(id, Length::Px(SPINBOX_DEFAULT_WIDTH));
        self.ta.set_one_line(cx, true);
        self.ta.set_cursor_click_pos(cx, true);
        self.update_value(cx);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        Widget::draw(&self.ta, cx);
    }

    fn draw_post(&self, cx: &mut DrawCx<'_, '_>) {
        Widget::draw_post(&self.ta, cx);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match ev.code {
            EventCode::Key => {
                let encoder = util::active_input_kind(cx.engine()) == Some(InputKind::Encoder);
                let mut wcx = cx.widget_cx();
                match ev.key() {
                    Some(Key::Right) if encoder => self.increment(&mut wcx),
                    Some(Key::Left) if encoder => self.decrement(&mut wcx),
                    Some(Key::Right) => self.step_next(&mut wcx),
                    Some(Key::Left) => self.step_prev(&mut wcx),
                    Some(Key::Up) => self.increment(&mut wcx),
                    Some(Key::Down) => self.decrement(&mut wcx),
                    // LVGL inserts other keys into the text (breaking the format until the
                    // next update); Twine ignores them.
                    _ => {}
                }
            }
            EventCode::Released => {
                self.ta.handle(cx, ev);
                self.released(cx);
            }
            _ => {
                self.ta.handle(cx, ev);
            }
        }
        EventResult::Continue
    }
}

impl Spinbox {
    /// The text as shown (read from the label).
    #[must_use]
    pub fn text<'e>(&self, cx: &MeasureCx<'e>) -> &'e str {
        self.ta.text(cx)
    }
}
