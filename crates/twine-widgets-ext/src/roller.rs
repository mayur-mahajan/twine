//! [`Roller`]: a drum-style picker (LVGL `lv_roller`).

use alloc::boxed::Box;
use alloc::string::String;
use core::cell::Cell;

use twine_anim::{Anim, AnimId};
use twine_core::{Duration, Point, Rect, Size};
use twine_engine::{
    AnimProp, DrawCx, Easing, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam,
    EventResult, GroupDef, InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Widget, WidgetClass,
    WidgetCx, fmt_node_id,
};
use twine_style::{COORD_MAX, Length, Part, PropId, TextAlign};
use twine_text::{TextDrawFlags, TextLayout};

use crate::options::{self, Options};
use crate::util::{self, log_set};

/// The class of [`Roller`]: `"roller"`, parts `Main` and `Selected` (the band in the middle
/// and the text on it), the base object's flags without `SCROLLABLE` and `SCROLL_CHAIN_VER`
/// (dragging moves the options, not a parent), always in the default focus group, editable
/// (LVGL `lv_roller_class` and constructor).
pub static ROLLER_CLASS: WidgetClass = WidgetClass::new("roller")
    .parts(&[Part::Main, Part::Selected])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::SCROLLABLE)
            .difference(ObjFlags::SCROLL_CHAIN_VER),
    )
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// LVGL `lv_roller_class.height_def`: `LV_DPI_DEF`.
pub const ROLLER_DEFAULT_HEIGHT: i32 = util::DPI_DEF;

/// The options of a new roller (LVGL with `LV_WIDGETS_HAS_DEFAULT_VALUE`).
pub const ROLLER_DEFAULT_OPTIONS: &str = "Option 1\nOption 2\nOption 3\nOption 4\nOption 5";

/// LVGL `EXTRA_INF_SIZE`: in infinite mode the options repeat until they are this high.
const EXTRA_INF_SIZE: i32 = 1000;

/// The custom animation id of the options' vertical offset.
const ANIM_Y: u16 = 0x0707;

/// How a roller ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RollerMode {
    /// The first and the last option are the ends.
    #[default]
    Normal,
    /// The options repeat endlessly (LVGL `LV_ROLLER_MODE_INFINITE`).
    Infinite,
}

/// A roller (LVGL `lv_roller`): the options in a column that the user drags (or steps with
/// keys / an encoder) under a selection band in the middle; releasing snaps the nearest option
/// into the band with an animation (the `Main` `anim_duration`, 200 ms in the default theme).
///
/// - **Infinite mode** repeats the options (as many times as fit in 1000 px, 3…15, odd;
///   LVGL's `inf_page_cnt`) **virtually**: the option string is stored once and drawn modulo
///   the option count. After each move the offset is normalized to the middle copy, so it
///   never grows.
/// - **Drawing** visits only the rows inside the clip area (P2), and draws the band's rows a
///   second time in the `Selected` text style, clipped to the band (LVGL draws its label twice).
/// - **Events**: `ValueChanged` with the selected index as [`EventParam::Value`] when a
///   release (pointer click or drag, keypad `Enter`, encoder click) changes the selection.
/// - **Keys**: `Up`/`Down` (`Left`/`Right`) move with the animation, `Enter` confirms,
///   `Esc` returns to the confirmed option; an encoder moves in edit mode and a click
///   confirms; defocusing without confirming returns to the confirmed option (LVGL).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::roller::{self, Roller, RollerMode};
///
/// let mut h = EngineHarness::new(200, 200);
/// let screen = h.screen();
/// let r = roller::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(r, |w: &mut Roller, cx| {
///     w.set_options(cx, "Mon\nTue\nWed\nThu\nFri", RollerMode::Infinite);
///     w.set_selected(cx, 3, false);
/// });
/// let w = h.engine().widget::<Roller>(r).unwrap();
/// assert_eq!((w.selected(), w.option_count()), (3, 5));
/// ```
#[derive(Debug)]
pub struct Roller {
    options: Options,
    /// Options per page.
    real_cnt: u16,
    mode: RollerMode,
    /// Virtual pages: 1 in normal mode.
    pages: u16,
    /// The selected row of the virtual column (`0 .. real_cnt * pages`).
    sel: u32,
    /// The confirmed row (restored by `Esc`, defocus, encoder navigation).
    sel_orig: u32,
    /// Dragged since the press.
    moved: bool,
    /// The last pointer point of a drag and its last vertical step.
    last_y: Option<i32>,
    last_dy: i32,
    /// The selection at the press (a release reports a change against it).
    sel_at_press: u16,
    visible_rows: u8,
    /// The top of the option column relative to the content area's top (LVGL: the label's
    /// `y`).
    offset_y: i32,
    anim: Option<AnimId>,
    /// Rows drawn by the last `draw` (instrumentation for tests).
    rows_drawn: Cell<u32>,
}

impl Default for Roller {
    fn default() -> Self {
        Self::new()
    }
}

impl Roller {
    /// A roller with LVGL's defaults: five options ("Option 1" … "Option 5"), normal mode,
    /// the first selected.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(ROLLER_DEFAULT_OPTIONS)
    }

    /// A roller of `'static` options in normal mode (stored without copying).
    #[must_use]
    pub fn with_options(options: &'static str) -> Self {
        Self {
            options: Options::from_static(options),
            real_cnt: options::count(options),
            mode: RollerMode::Normal,
            pages: 1,
            sel: 0,
            sel_orig: 0,
            moved: false,
            last_y: None,
            last_dy: 0,
            sel_at_press: 0,
            visible_rows: 0,
            offset_y: 0,
            anim: None,
            rows_drawn: Cell::new(0),
        }
    }

    // ---- Getters --------------------------------------------------------------------------

    /// The options as one `'\n'`-separated string (once, also in infinite mode).
    #[must_use]
    pub fn options(&self) -> &str {
        self.options.as_str()
    }

    /// The option storage (its size is the size of the options given, in both modes).
    #[must_use]
    pub fn options_storage(&self) -> &Options {
        &self.options
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> RollerMode {
        self.mode
    }

    /// The number of options (LVGL `lv_roller_get_option_count`: the real ones in infinite
    /// mode).
    #[must_use]
    pub fn option_count(&self) -> u16 {
        self.real_cnt
    }

    /// The index of the selected option (LVGL `lv_roller_get_selected`).
    #[must_use]
    pub fn selected(&self) -> u16 {
        if self.real_cnt == 0 {
            return 0;
        }
        (self.sel % u32::from(self.real_cnt)) as u16
    }

    /// Writes the selected option into `buf` (cleared first; LVGL
    /// `lv_roller_get_selected_str`).
    pub fn selected_str(&self, buf: &mut String) {
        buf.clear();
        if let Some(o) = self.options.get(self.selected()) {
            buf.push_str(o);
        }
    }

    /// The number of rows set with [`set_visible_row_count`](Self::set_visible_row_count)
    /// (0: the default height).
    #[must_use]
    pub fn visible_row_count(&self) -> u8 {
        self.visible_rows
    }

    /// The vertical offset of the option column from the top of the content area.
    #[must_use]
    pub fn offset_y(&self) -> i32 {
        self.offset_y
    }

    /// The number of virtual pages (1 in normal mode).
    #[must_use]
    pub fn page_count(&self) -> u16 {
        self.pages
    }

    /// The rows the last draw visited (both text passes).
    #[must_use]
    pub fn rows_drawn(&self) -> u32 {
        self.rows_drawn.get()
    }

    fn total_rows(&self) -> u32 {
        u32::from(self.real_cnt) * u32::from(self.pages)
    }

    // ---- Setters --------------------------------------------------------------------------

    /// Sets the options (copied into the owned buffer, its capacity reused) and the mode;
    /// selects the first option (LVGL `lv_roller_set_options`). Idempotent: the same options
    /// and mode change nothing.
    pub fn set_options(&mut self, cx: &mut WidgetCx<'_>, options: &str, mode: RollerMode) {
        if self.options.same_as(options) && self.mode == mode {
            return;
        }
        log_set(ROLLER_CLASS.name, cx.node(), "options");
        self.options.set_owned(options);
        self.update_options(cx, mode);
    }

    /// Sets `'static` options (stored without copying) and the mode. Idempotent.
    pub fn set_options_static(&mut self, cx: &mut WidgetCx<'_>, options: &'static str, mode: RollerMode) {
        if self.options.is_static(options) && self.mode == mode {
            return;
        }
        log_set(ROLLER_CLASS.name, cx.node(), "options_static");
        self.options = Options::from_static(options);
        self.update_options(cx, mode);
    }

    /// Sets the mode, keeping the options; selects the first option. Idempotent.
    /// Never panics.
    ///
    /// ```
    /// use twine_testing::EngineHarness;
    /// use twine_widgets_ext::roller::{self, Roller, RollerMode};
    ///
    /// let mut h = EngineHarness::new(200, 200);
    /// let screen = h.screen();
    /// let r = roller::create(h.engine_mut(), screen).unwrap();
    /// h.engine_mut().with_widget_mut(r, |w: &mut Roller, cx| {
    ///     w.set_options(cx, "Mon\nTue\nWed", RollerMode::Normal);
    ///     w.set_selected(cx, 2, false);
    ///     w.set_mode(cx, RollerMode::Infinite); // endless wheel, back to the first option
    /// });
    /// let w = h.engine().widget::<Roller>(r).unwrap();
    /// assert_eq!((w.mode(), w.selected(), w.option_count()), (RollerMode::Infinite, 0, 3));
    /// ```
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: RollerMode) {
        if self.mode == mode {
            return;
        }
        log_set(ROLLER_CLASS.name, cx.node(), "mode");
        self.update_options(cx, mode);
    }

    /// LVGL `update_options`.
    fn update_options(&mut self, cx: &mut WidgetCx<'_>, mode: RollerMode) {
        self.real_cnt = self.options.count();
        self.mode = mode;
        self.sel = 0;
        self.pages = 1;
        if mode == RollerMode::Infinite && self.real_cnt > 0 {
            let m = cx.measure();
            let font_h = i32::from(m.font(Part::Main).line_height);
            let ls = m.style_i32(Part::Main, PropId::LetterSpacing);
            let normal_h = (i32::from(self.real_cnt) * (font_h + ls)).max(1);
            let mut pages = (EXTRA_INF_SIZE / normal_h).clamp(3, 15);
            if pages & 1 == 0 {
                pages += 1;
            }
            self.pages = pages as u16;
            self.sel = u32::from(self.pages / 2) * u32::from(self.real_cnt);
            twine_core::debug!(
                target: "twine::engine",
                "roller#{}: {} virtual pages",
                fmt_node_id(cx.node()),
                self.pages
            );
        }
        self.sel_orig = self.sel;
        Self::refresh_size(cx);
        self.refr_position(cx, false);
    }

    /// Selects option `idx` (clamped), animated when `anim` (LVGL `lv_roller_set_selected`;
    /// no `ValueChanged`). In infinite mode the nearest copy of the option is chosen (a jump
    /// from the last to the first option rolls forward). Idempotent.
    pub fn set_selected(&mut self, cx: &mut WidgetCx<'_>, idx: u16, anim: bool) {
        if self.real_cnt == 0 {
            return;
        }
        let target = self.target_row(idx);
        if target == self.sel && self.sel_orig == self.sel && self.anim.is_none() {
            let m = cx.measure();
            if self.offset_y == Self::row_offset(&m, self.sel) {
                return;
            }
        }
        log_set(ROLLER_CLASS.name, cx.node(), "selected");
        self.sel = target;
        self.sel_orig = target;
        self.refr_position(cx, anim);
    }

    /// The virtual row of option `idx` (LVGL `lv_roller_set_selected`'s page logic).
    fn target_row(&self, idx: u16) -> u32 {
        let total = self.total_rows();
        let mut sel = u32::from(idx);
        if self.mode == RollerMode::Infinite {
            let real = u32::from(self.real_cnt);
            let page = self.sel / real;
            if sel < real {
                let act = self.sel - page * real;
                let mut s = i64::from(sel);
                if (i64::from(act) - s).unsigned_abs() > u64::from(real / 2) {
                    if act > sel {
                        s += i64::from(real);
                    } else {
                        s -= i64::from(real);
                    }
                }
                sel = u32::try_from(s + i64::from(real * page)).unwrap_or(0);
            }
        }
        sel.min(total.saturating_sub(1))
    }

    /// Sets the height to show `rows` rows: `(line height + line space) × rows + 2 × border`
    /// (LVGL `lv_roller_set_visible_row_count`). Idempotent.
    pub fn set_visible_row_count(&mut self, cx: &mut WidgetCx<'_>, rows: u8) {
        let h = Self::height_for_rows(&cx.measure(), rows);
        let id = cx.node();
        if self.visible_rows == rows
            && cx.style(Part::Main, PropId::Height).as_length() == Some(Length::Px(h))
        {
            return;
        }
        log_set(ROLLER_CLASS.name, id, "visible_row_count");
        self.visible_rows = rows;
        cx.engine_mut().set_height(id, Length::Px(h));
    }

    /// The height of `rows` rows (LVGL `lv_roller_set_visible_row_count`).
    #[must_use]
    pub fn height_for_rows(m: &MeasureCx<'_>, rows: u8) -> i32 {
        let font_h = i32::from(m.font(Part::Main).line_height);
        let ls = m.style_i32(Part::Main, PropId::LineSpacing);
        let border = m.style_i32(Part::Main, PropId::BorderWidth);
        (font_h + ls) * i32::from(rows) + 2 * border
    }

    /// Redraw, and relayout a content-sized roller.
    fn refresh_size(cx: &mut WidgetCx<'_>) {
        cx.invalidate_for("roller");
        let content_sized = [PropId::Width, PropId::Height]
            .iter()
            .any(|&p| cx.style(Part::Main, p).as_length() == Some(Length::Content));
        if content_sized {
            cx.mark_layout();
        }
    }

    // ---- Geometry -------------------------------------------------------------------------

    /// The height of one row: line height + line space of `Main`.
    fn unit(m: &MeasureCx<'_>) -> (i32, i32) {
        let font_h = i32::from(m.font(Part::Main).line_height);
        let ls = m.style_i32(Part::Main, PropId::LineSpacing);
        (font_h, font_h + ls)
    }

    /// The column offset that puts row `sel` in the middle (LVGL `refr_position`).
    fn row_offset(m: &MeasureCx<'_>, sel: u32) -> i32 {
        let (font_h, unit) = Self::unit(m);
        let h = m.content_area().height();
        let mid_y1 = h / 2 - font_h / 2;
        mid_y1 - i32::try_from(sel).unwrap_or(i32::MAX / unit.max(1)) * unit
    }

    /// The selection band (LVGL `get_sel_area`), absolute.
    fn sel_area(m: &MeasureCx<'_>) -> Rect {
        let c = m.coords();
        let font_main_h = i32::from(m.font(Part::Main).line_height);
        let font_sel_h = i32::from(m.font(Part::Selected).line_height);
        let ls = m.style_i32(Part::Main, PropId::LineSpacing);
        let d = (font_sel_h + font_main_h) / 2 + ls;
        let y1 = c.y0 + c.height() / 2 - d / 2;
        // LVGL's inclusive `y2 = y1 + d`.
        Rect::new(c.x0, y1, c.x1, y1 + d + 1)
    }

    /// The area a move redraws: the roller inside its border (not its outline or shadow).
    fn moving_area(m: &MeasureCx<'_>) -> Rect {
        let b = m.style_i32(Part::Main, PropId::BorderWidth).max(0);
        m.coords().inset(twine_core::Insets::new(b, b, b, b))
    }

    /// The width of the widest option in `part`'s text style.
    fn text_width(&self, m: &MeasureCx<'_>, part: Part) -> i32 {
        let d = m.text_dsc(part);
        let mut l = TextLayout::new(self.options.as_str(), d.font);
        l.letter_space = d.letter_space;
        l.max_width = COORD_MAX;
        l.measure().w
    }

    /// Moves the column to `y` (redrawing the moving area).
    fn set_offset(&mut self, cx: &mut WidgetCx<'_>, y: i32) {
        if self.offset_y == y {
            return;
        }
        self.offset_y = y;
        let a = Self::moving_area(&cx.measure());
        cx.invalidate_area(a);
    }

    fn stop_anim(&mut self, e: &mut Engine) {
        if let Some(a) = self.anim.take() {
            e.anim_stop(a);
        }
    }

    /// LVGL `refr_position`: moves the selected row into the band, animated with the `Main`
    /// `anim_duration` when `anim`.
    fn refr_position(&mut self, cx: &mut WidgetCx<'_>, anim: bool) {
        let time = cx.style_i32(Part::Main, PropId::AnimDuration);
        if !anim || time <= 0 {
            self.inf_normalize(cx);
        }
        let new_y = Self::row_offset(&cx.measure(), self.sel);
        if !anim || time <= 0 {
            self.stop_anim(cx.engine_mut());
            self.set_offset(cx, new_y);
            return;
        }
        let node = cx.node();
        let a = Anim::new(self.offset_y, new_y)
            .duration(Duration::ms(u64::from(time.unsigned_abs())))
            .easing(Easing::EaseOut)
            .on_complete(move |acx| {
                twine_engine::defer(acx, move |e| {
                    e.with_widget_mut(node, |r: &mut Roller, cx| {
                        r.anim = None;
                        r.inf_normalize(cx);
                    });
                });
            });
        self.stop_anim(cx.engine_mut());
        self.anim = Some(cx.engine_mut().anim_start(node, AnimProp::Custom(ANIM_Y), a));
    }

    /// LVGL `inf_normalize`: in infinite mode moves the selection (and the column) to the
    /// middle page, so the offset stays bounded.
    fn inf_normalize(&mut self, cx: &mut WidgetCx<'_>) {
        if self.mode != RollerMode::Infinite || self.real_cnt == 0 {
            return;
        }
        let real = u32::from(self.real_cnt);
        let mid = u32::from(self.pages / 2) * real;
        self.sel = self.sel % real + mid;
        self.sel_orig = self.sel_orig % real + mid;
        let y = Self::row_offset(&cx.measure(), self.sel);
        self.set_offset(cx, y);
    }

    /// Moves the selection by `delta` rows (keys, rotary), keeping the confirmed row.
    fn step(&mut self, cx: &mut WidgetCx<'_>, delta: i32) {
        let last = i64::from(self.total_rows()) - 1;
        let new = (i64::from(self.sel) + i64::from(delta)).clamp(0, last.max(0));
        if new != i64::from(self.sel) {
            self.sel = new as u32;
            self.refr_position(cx, true);
        }
    }

    /// Sends `ValueChanged` when the selection differs from `before`.
    fn report(&self, cx: &mut WidgetCx<'_>, before: u16) {
        let now = self.selected();
        if now != before {
            util::log_value(ROLLER_CLASS.name, cx.node(), i32::from(now));
            cx.post_event(EventCode::ValueChanged, EventParam::Value(i32::from(now)));
        }
    }

    /// LVGL `release_handler`.
    fn release(&mut self, cx: &mut WidgetCx<'_>, p: Option<Point>) {
        let kind = util::active_input_kind(cx.engine());
        let before = if matches!(kind, Some(InputKind::Pointer | InputKind::Button)) {
            self.sel_at_press
        } else {
            (self.sel_orig % u32::from(self.real_cnt.max(1))) as u16
        };
        if matches!(kind, Some(InputKind::Encoder | InputKind::Keypad)) {
            self.sel_orig = self.sel;
            if kind == Some(InputKind::Encoder) {
                let id = cx.node();
                if let Some(g) = cx.engine().group_of(id) {
                    if cx.engine().group_editing(g) {
                        cx.engine_mut().set_editing(g, false);
                    }
                }
            }
        }
        if matches!(kind, Some(InputKind::Pointer | InputKind::Button)) {
            let m = cx.measure();
            let (font_h, unit) = Self::unit(&m);
            let c = m.content_area();
            let top = c.y0 + self.offset_y;
            let total = i64::from(self.total_rows());
            let row = if self.moved {
                // Snap to the row at the middle after the throw (LVGL: the geometric sum of
                // the last vector with the scroll throw decay).
                let throw = i64::from(cx.engine().config().scroll_throw.min(99));
                let mut sum = 0i64;
                let mut v = i64::from(self.last_dy);
                while v != 0 {
                    sum += v;
                    v = v * (100 - throw) / 100;
                }
                let coords = m.coords();
                let mid = coords.y0 + (coords.height() - 1) / 2;
                (i64::from(mid) - (i64::from(top) + sum)) / i64::from(unit.max(1))
            } else if let Some(p) = p {
                // The clicked row (LVGL `lv_label_get_letter_on`: a row reaches down to its
                // line height; the line space belongs to the next one).
                let y = i64::from(p.y - top);
                if y <= i64::from(font_h) {
                    0
                } else {
                    (y - i64::from(font_h) + i64::from(unit) - 1) / i64::from(unit.max(1))
                }
            } else {
                i64::from(self.sel)
            };
            let row = row.clamp(0, (total - 1).max(0)) as u32;
            self.sel = row;
            self.sel_orig = row;
            self.refr_position(cx, true);
        }
        self.report(cx, before);
    }

    /// The label's `x` inside the content area (LVGL `refr_position`: by the text alignment)
    /// and its width.
    fn label_x(&self, m: &MeasureCx<'_>) -> (i32, i32, TextAlign) {
        let w = self.text_width(m, Part::Main);
        let cw = m.content_area().width();
        let align = match m.text_dsc(Part::Main).align {
            TextAlign::Auto if util::is_rtl(m) => TextAlign::Right,
            TextAlign::Auto => TextAlign::Left,
            a => a,
        };
        let x = match align {
            TextAlign::Center => (cw - w) / 2,
            TextAlign::Right => cw - w,
            _ => 0,
        };
        (x, w, align)
    }

    /// Draws the rows of the virtual column that intersect `clip`: row `r` at
    /// `top + r × unit`, `font_h` high, in `dsc` within the x range `x`.
    #[allow(clippy::too_many_arguments)]
    fn draw_rows(
        &self,
        cx: &mut DrawCx<'_, '_>,
        clip: Rect,
        top: i32,
        font_h: i32,
        unit: i32,
        x: (i32, i32),
        dsc: &twine_text::TextDsc,
    ) {
        let total = i64::from(self.total_rows());
        if total == 0 || unit <= 0 {
            return;
        }
        let first = (i64::from(clip.y0 - top - font_h) / i64::from(unit)).max(0);
        let last = (i64::from(clip.y1 - 1 - top) / i64::from(unit)).min(total - 1);
        if first > last {
            return;
        }
        let real = i64::from(self.real_cnt);
        // Walk the options from the first visible row (one scan of the string per pass).
        let text = self.options.as_str();
        // Rows past the last option start over (the virtual pages).
        let mut it = text.split('\n').cycle().skip((first % real) as usize);
        for r in first..=last {
            let opt = it.next().unwrap_or("");
            let y = top + (r as i32) * unit;
            let area = Rect::new(x.0, y, x.1, y + font_h);
            cx.draw_text(area, opt, dsc);
            self.rows_drawn.set(self.rows_drawn.get() + 1);
        }
    }
}

/// Creates a roller, content wide and `LV_DPI_DEF` high, as the last child of `parent` (LVGL
/// `lv_roller_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Roller::new()))
}

impl Widget for Roller {
    fn class(&self) -> &'static WidgetClass {
        &ROLLER_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut().set_width(id, Length::Content);
        cx.engine_mut().set_height(id, Length::Px(ROLLER_DEFAULT_HEIGHT));
        let mode = self.mode;
        self.update_options(cx, mode);
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE`: the options' width (in the wider of the `Main` label
    /// and the `Selected` text).
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        Size::new(
            self.text_width(cx, Part::Main)
                .max(self.text_width(cx, Part::Selected)),
            0,
        )
    }

    /// LVGL `draw_main` + `draw_label`: the band, the rows outside it in `Main`, the rows in
    /// it in `Selected`.
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.rows_drawn.set(0);
        let m = MeasureCx::new(cx.engine(), cx.node());
        let sel = Self::sel_area(&m);
        let rect = cx.rect_dsc(Part::Selected);
        if let Some(clip) = sel.intersection(&cx.clip()) {
            let d = rect.dsc();
            let _ = cx.with_clip(clip, |cx| cx.painter().rect(sel, &d));
        }
        let c = m.coords();
        let content = m.content_area();
        let Some(clip) = c.intersection(&cx.clip()) else {
            return;
        };
        let (font_h, unit) = Self::unit(&m);
        let top = content.y0 + self.offset_y;
        let (lx, lw, align) = self.label_x(&m);
        let x0 = content.x0 + lx;
        let mut dsc = cx.text_dsc(Part::Main);
        dsc.align = align;
        dsc.flags |= TextDrawFlags::EXPAND;
        // Above the band (LVGL's inclusive clip ends on the band's first row) and below it.
        let above = Rect::new(clip.x0, clip.y0, clip.x1, clip.y1.min(sel.y0 + 1));
        let below = Rect::new(clip.x0, clip.y0.max(sel.y1 - 1), clip.x1, clip.y1);
        for part in [above, below] {
            if part.is_empty() {
                continue;
            }
            let _ = cx.with_clip(part, |cx| {
                self.draw_rows(cx, part, top, font_h, unit, (x0, x0 + lw), &dsc);
            });
        }
        // The band's rows in the `Selected` style, moving proportionally with the column
        // (LVGL `draw_main` `DRAW_POST`: equal to the column for equal fonts).
        let Some(band) = sel.intersection(&clip) else {
            return;
        };
        let mut sd = cx.text_dsc(Part::Selected);
        sd.flags |= TextDrawFlags::EXPAND;
        if sd.align == TextAlign::Auto {
            sd.align = align;
        }
        let sel_font_h = i32::from(sd.font.line_height);
        let sel_unit = sel_font_h + sd.line_space;
        let rows = i64::from(self.total_rows());
        let label_h = i64::from(rows as i32 * unit - (unit - font_h));
        let sel_size_y = rows * i64::from(sel_unit) - i64::from(sd.line_space);
        let roller_h = i64::from(c.height());
        let mut prop = (i64::from(top) + i64::from(font_h / 2)) - (roller_h / 2 + i64::from(c.y0));
        let remain_h = label_h - i64::from(font_h);
        if remain_h > 0 {
            prop = (prop << 14) / remain_h;
        }
        let corr = i64::from(sel_font_h);
        let sel_y = roller_h / 2 + i64::from(c.y0) + (((sel_size_y - corr) * prop) >> 14) - corr / 2;
        let sel_top = i32::try_from(sel_y).unwrap_or(0);
        let x = (content.x0, content.x1);
        let _ = cx.with_clip(band, |cx| {
            self.draw_rows(cx, band, sel_top, sel_font_h, sel_unit, x, &sd);
        });
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        let node = cx.node();
        if ev.target != node {
            return EventResult::Continue;
        }
        match ev.code {
            EventCode::StyleChanged => {
                let mut wcx = cx.widget_cx();
                Self::refresh_size(&mut wcx);
                self.refr_position(&mut wcx, false);
            }
            EventCode::SizeChanged => self.refr_position(&mut cx.widget_cx(), false),
            EventCode::Pressed => {
                if self.real_cnt <= 1 {
                    return EventResult::Continue;
                }
                self.moved = false;
                self.last_dy = 0;
                self.last_y = cx.point().map(|p| p.y);
                self.sel_at_press = self.selected();
                self.stop_anim(cx.engine_mut());
            }
            EventCode::Pressing => {
                if self.real_cnt <= 1 {
                    return EventResult::Continue;
                }
                if let (Some(p), Some(last)) = (cx.point(), self.last_y) {
                    let dy = p.y - last;
                    self.last_y = Some(p.y);
                    // The throw uses the last step: a finger held still throws nothing.
                    self.last_dy = dy;
                    if dy != 0 {
                        self.moved = true;
                        let y = self.offset_y + dy;
                        self.set_offset(&mut cx.widget_cx(), y);
                    }
                }
            }
            EventCode::Released | EventCode::PressLost => {
                let p = cx.point();
                self.release(&mut cx.widget_cx(), p);
                self.last_y = None;
            }
            // A drag is not a click (LVGL stops the event).
            EventCode::Clicked if self.moved => return EventResult::Consumed,
            EventCode::Focused => {
                let e = cx.engine();
                let encoder = util::active_input_kind(e) == Some(InputKind::Encoder);
                let editing = util::editing(e, node);
                let mut wcx = cx.widget_cx();
                if encoder && !editing {
                    if self.sel != self.sel_orig {
                        self.sel = self.sel_orig;
                        self.refr_position(&mut wcx, true);
                    }
                } else {
                    self.sel_orig = self.sel;
                }
            }
            EventCode::Defocused => {
                if self.sel != self.sel_orig {
                    self.sel = self.sel_orig;
                    self.refr_position(&mut cx.widget_cx(), true);
                }
            }
            EventCode::Key => {
                if self.real_cnt <= 1 {
                    return EventResult::Continue;
                }
                let mut wcx = cx.widget_cx();
                match ev.key() {
                    Some(Key::Right | Key::Down) => self.step(&mut wcx, 1),
                    Some(Key::Left | Key::Up) => self.step(&mut wcx, -1),
                    Some(Key::Esc) if self.sel != self.sel_orig => {
                        self.sel = self.sel_orig;
                        self.refr_position(&mut wcx, true);
                    }
                    _ => {}
                }
            }
            EventCode::Rotary => {
                if let EventParam::Rotary(r) = ev.param {
                    if self.real_cnt > 1 {
                        self.step(&mut cx.widget_cx(), r);
                    }
                }
            }
            _ => {}
        }
        EventResult::Continue
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id == ANIM_Y {
            self.set_offset(cx, v);
        }
    }
}
