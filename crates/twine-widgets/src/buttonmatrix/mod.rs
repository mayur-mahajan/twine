//! [`ButtonMatrix`]: a grid of text buttons drawn by one widget (LVGL `lv_buttonmatrix`).

mod map;
mod popover;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::num::{NonZeroU16, NonZeroU64};
use core::sync::atomic::{AtomicBool, Ordering};

use bitflags::bitflags;
use twine_core::{Point, Rect, Size};
use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, GroupDef,
    InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, State, Widget, WidgetClass, WidgetCx,
};
use twine_render::BorderSide;
use twine_style::{BaseDir, Part, PropId, TextAlign};
use twine_text::TextLayout;

use crate::log_set;
use crate::util::{self, Area};

pub use map::{BtnMap, BtnTexts, MapBtn, OwnedMap};
pub use popover::POPOVER_CLASS;

bitflags! {
    /// The control bits of one button (LVGL `lv_buttonmatrix_ctrl_t`, same values).
    ///
    /// The low four bits ([`WIDTH_MASK`](Self::WIDTH_MASK)) hold the relative width (1…15;
    /// 0 counts as 1).
    ///
    /// ```
    /// use twine_widgets::buttonmatrix::BtnCtrl;
    /// let c = BtnCtrl::width(3) | BtnCtrl::CHECKABLE;
    /// assert_eq!(c.width_units(), 3);
    /// assert!(c.contains(BtnCtrl::CHECKABLE));
    /// ```
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct BtnCtrl: u16 {
        /// The relative width (`LV_BUTTONMATRIX_WIDTH_MASK`).
        const WIDTH_MASK = 0x000F;
        /// The button is not drawn and cannot be pressed; it still takes its width.
        const HIDDEN = 0x0010;
        /// No repeated `ValueChanged` on long press.
        const NO_REPEAT = 0x0020;
        /// Drawn with the `DISABLED` styles and cannot be pressed.
        const DISABLED = 0x0040;
        /// Toggles `CHECKED` when clicked.
        const CHECKABLE = 0x0080;
        /// Checked (drawn with the `CHECKED` styles).
        const CHECKED = 0x0100;
        /// `ValueChanged` on release instead of on press.
        const CLICK_TRIG = 0x0200;
        /// Shows an enlarged key above the button while pressed (on the top layer);
        /// `ValueChanged` on release.
        const POPOVER = 0x0400;
        /// LVGL's text recoloring; kept for parity, ignored (logs a warning once).
        const RECOLOR = 0x0800;
        /// Free for applications (the keyboard theme uses it for control keys).
        const CUSTOM_1 = 0x4000;
        /// Free for applications.
        const CUSTOM_2 = 0x8000;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for BtnCtrl {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "BtnCtrl({=u16:#x})", self.bits());
    }
}

impl BtnCtrl {
    /// A relative width of `units` (clamped to 1…15).
    #[must_use]
    pub const fn width(units: u8) -> BtnCtrl {
        let u = if units == 0 {
            1
        } else if units > 15 {
            15
        } else {
            units
        };
        BtnCtrl::from_bits_retain(u as u16)
    }

    /// The relative width (LVGL `get_button_width`: 0 counts as 1).
    #[must_use]
    pub const fn width_units(self) -> u32 {
        let w = (self.bits() & BtnCtrl::WIDTH_MASK.bits()) as u32;
        if w == 0 { 1 } else { w }
    }

    /// Hidden or disabled: the button cannot be selected (LVGL `button_is_hidden ||
    /// button_is_inactive`).
    #[must_use]
    pub const fn is_inactive(self) -> bool {
        self.bits() & (BtnCtrl::HIDDEN.bits() | BtnCtrl::DISABLED.bits()) != 0
    }
}

/// LVGL `lv_buttonmatrix_def_map` (`LV_WIDGETS_HAS_DEFAULT_VALUE`).
pub static BUTTONMATRIX_DEFAULT_MAP: [&[MapBtn]; 2] = [
    &[MapBtn::text("Btn1"), MapBtn::text("Btn2"), MapBtn::text("Btn3")],
    &[MapBtn::text("Btn4"), MapBtn::text("Btn5")],
];

/// The class of [`ButtonMatrix`]: `"buttonmatrix"`, parts `Main` and `Items` (the buttons),
/// editable with an encoder and always in the default focus group (LVGL
/// `lv_buttonmatrix_class`).
pub static BUTTONMATRIX_CLASS: WidgetClass = WidgetClass::new("buttonmatrix")
    .parts(&[Part::Main, Part::Items])
    .item_parts(&[Part::Items])
    .default_flags(OBJ_FLAGS)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// LVGL `lv_buttonmatrix_class.width_def`: `LV_DPI_DEF * 2`.
pub const BUTTONMATRIX_DEFAULT_WIDTH: i32 = util::DPI_DEF * 2;
/// LVGL `lv_buttonmatrix_class.height_def`: `LV_DPI_DEF`.
pub const BUTTONMATRIX_DEFAULT_HEIGHT: i32 = util::DPI_DEF;

/// The node states the selected button shows (LVGL `draw_main`).
const SELECTED_STATES: State = State::PRESSED
    .union(State::FOCUSED)
    .union(State::FOCUS_KEY)
    .union(State::EDITED);

/// LVGL `BTN_EXTRA_CLICK_AREA_MAX`: `LV_DPI_DEF / 10`.
const BTN_EXTRA_CLICK_AREA_MAX: i32 = util::DPI_DEF / 10;

static WARN_RECOLOR: AtomicBool = AtomicBool::new(false);

/// A matrix of text buttons drawn by one widget (LVGL `lv_buttonmatrix`): no node per
/// button, so a keyboard of 40 keys costs one node.
///
/// - **Map**: rows of buttons, each with its text and initial [`BtnCtrl`] bits ([`BtnMap`]:
///   a [`MapBtn`] table in flash or an [`OwnedMap`]). Buttons have relative widths (the
///   [`BtnCtrl`] width bits); a row's content width minus the column gaps is shared by units,
///   like LVGL (`(w − (n − 1)·pad_column) · units / total`), rows are equally tall with
///   `pad_row` between them. A button's text can be changed alone
///   ([`set_btn_text`](Self::set_btn_text), in place).
/// - **Drawing**: `Main` is the background; each button is an `Items` rectangle with its
///   text centered, drawn with the `Items` styles of the button's own state: `CHECKED`,
///   `DISABLED`, and for the selected button the widget's `PRESSED`, `FOCUSED`,
///   `FOCUS_KEY` and `EDITED` states (without style transitions). `Items` is an item part
///   of the class: pressing, focusing or editing redraws only the selected button, not the
///   whole matrix.
/// - **Events**: pressing a button sends `ValueChanged` with the button index as
///   [`EventParam::Value`] (on release for [`BtnCtrl::CLICK_TRIG`] and
///   [`BtnCtrl::POPOVER`] buttons), long press repeats it (unless
///   [`BtnCtrl::NO_REPEAT`]). Checkable buttons toggle on release; with
///   [`set_one_checked`](Self::set_one_checked) only one stays checked.
/// - **Keys**: the arrows move the selection (skipping hidden and disabled buttons),
///   `Enter` presses it. With an encoder, turning moves the selection in edit mode.
/// - **Popovers**: a pressed [`BtnCtrl::POPOVER`] button shows an enlarged copy one row
///   higher on the display's top layer (so nothing covers or clips it).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::buttonmatrix::{self, BtnCtrl, ButtonMatrix, MapBtn};
///
/// static MAP: [&[MapBtn]; 2] = [&[MapBtn::text("1"), MapBtn::text("2")], &[MapBtn::text("OK")]];
/// let mut h = EngineHarness::new(200, 100);
/// let screen = h.screen();
/// let m = buttonmatrix::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(m, |w: &mut ButtonMatrix, cx| {
///     w.set_map(cx, (&MAP).into());
///     w.set_btn_ctrl(cx, 2, BtnCtrl::CHECKABLE);
/// });
/// let w = h.engine().widget::<ButtonMatrix>(m).unwrap();
/// assert_eq!(w.btn_count(), 3);
/// assert_eq!(w.btn_text(2), Some("OK"));
/// ```
#[derive(Debug)]
pub struct ButtonMatrix {
    map: BtnMap,
    /// The current control bits of every button (from the map's, then changed by the
    /// setters and by checking).
    ctrl: Vec<BtnCtrl>,
    /// Button areas relative to the widget's top-left corner (inclusive, like LVGL).
    areas: Vec<Area>,
    row_cnt: u16,
    /// LVGL `btn_id_sel`: the pressed or keyboard-selected button.
    selected: Option<u16>,
    one_checked: bool,
    /// The popover node on the top layer (created on first use).
    popover: Option<NodeId>,
}

impl Default for ButtonMatrix {
    fn default() -> Self {
        Self::new(BtnMap::from(&BUTTONMATRIX_DEFAULT_MAP))
    }
}

impl ButtonMatrix {
    /// A button matrix with `map` (no areas until it is laid out).
    #[must_use]
    pub fn new(map: BtnMap) -> Self {
        let mut m = Self {
            map: BtnMap::default(),
            ctrl: Vec::new(),
            areas: Vec::new(),
            row_cnt: 1,
            selected: None,
            one_checked: false,
            popover: None,
        };
        m.map = map;
        // Part of the widget's construction (noted as its creation when the engine adds it).
        m.fill_from_map();
        m
    }

    /// The map (the buttons' texts and initial control bits; the current bits are
    /// [`btn_ctrl`](Self::btn_ctrl)).
    #[must_use]
    pub fn map(&self) -> &BtnMap {
        &self.map
    }

    /// The number of buttons.
    #[must_use]
    pub fn btn_count(&self) -> u16 {
        u16::try_from(self.ctrl.len()).unwrap_or(u16::MAX)
    }

    /// The number of rows.
    #[must_use]
    pub fn row_count(&self) -> u16 {
        self.row_cnt
    }

    /// The text of button `idx` (LVGL `lv_buttonmatrix_get_button_text`).
    #[must_use]
    pub fn btn_text(&self, idx: u16) -> Option<&str> {
        self.map.text(usize::from(idx))
    }

    /// The control bits of button `idx`.
    #[must_use]
    pub fn btn_ctrl(&self, idx: u16) -> Option<BtnCtrl> {
        self.ctrl.get(usize::from(idx)).copied()
    }

    /// Whether button `idx` has every bit of `ctrl` (LVGL
    /// `lv_buttonmatrix_has_button_ctrl`).
    #[must_use]
    pub fn has_btn_ctrl(&self, idx: u16, ctrl: BtnCtrl) -> bool {
        self.btn_ctrl(idx).is_some_and(|c| c.contains(ctrl))
    }

    /// The selected (pressed or keyboard-selected) button (LVGL
    /// `lv_buttonmatrix_get_selected_button`).
    #[must_use]
    pub fn selected_btn(&self) -> Option<u16> {
        self.selected
    }

    /// Whether at most one button can be checked.
    #[must_use]
    pub fn one_checked(&self) -> bool {
        self.one_checked
    }

    /// The absolute area of button `idx` as of the last layout.
    #[must_use]
    pub fn btn_area(&self, cx: &MeasureCx<'_>, idx: u16) -> Option<Rect> {
        let a = self.areas.get(usize::from(idx))?;
        let c = cx.coords();
        Some(a.to_rect().translate(c.x0, c.y0))
    }

    /// The popover node on the top layer, once one was shown.
    #[must_use]
    pub fn popover_node(&self) -> Option<NodeId> {
        self.popover
    }

    /// Sets the map (LVGL `lv_buttonmatrix_set_map` and `lv_buttonmatrix_set_ctrl_map`
    /// in one): the buttons, their texts and their control bits, which replace the current
    /// ones. Idempotent: the same map (the same static table, or an equal one) changes
    /// nothing, also not the control bits changed since. The selection is kept on the button
    /// that now occupies its position. The texts of an [`OwnedMap`] are cut to
    /// [`Limits::max_text_len`](twine_core::Limits::max_text_len) like a label's text (a
    /// `Capacity` fault, code `TextLen`, per cut text); a static map's texts are used as they
    /// are (in flash, never copied). The button tables are kept between maps: a map with more
    /// buttons than any before grows them. Never panics.
    pub fn set_map(&mut self, cx: &mut WidgetCx<'_>, mut map: BtnMap) {
        // The texts of an owned map are widget texts: cut in place (no allocation).
        for t in map.owned_texts_mut() {
            let cap = t.capacity();
            cx.limit_text(t, cap);
        }
        if self.map.same(&map) {
            if let (BtnMap::Static(_), BtnMap::Static(_)) = (&self.map, &map) {
                self.map = map;
            }
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "map");
        let m = cx.measure();
        let sel_center = self
            .selected
            .and_then(|s| self.areas.get(usize::from(s)))
            .map(|a| {
                let c = m.coords();
                let at = |o: i32, a1: i32, len: i32| o.saturating_add(a1).saturating_add(len >> 1);
                Point::new(at(c.x0, a.x1, a.width()), at(c.y0, a.y1, a.height()))
            });
        self.map = map;
        self.parse_map(cx.partition());
        self.update_map(cx);
        if let Some(p) = sel_center {
            let new_sel = self
                .button_from_point(&cx.measure(), p)
                .filter(|&b| self.btn_ctrl(b).is_some_and(|c| !c.is_inactive()));
            self.selected = new_sel;
            self.invalidate_btn(cx, new_sel);
        }
    }

    /// Replaces the text of button `idx` in place (a static map is copied into an owned one
    /// first, once; after that the button's string is reused, so a text that fits its
    /// capacity allocates nothing — with [`Limits::preallocate`](twine_core::Limits::preallocate)
    /// a growing string is reserved for the full `max_text_len` at once). Cut to
    /// [`Limits::max_text_len`](twine_core::Limits::max_text_len) like a label's text (a
    /// `Capacity` fault, code `TextLen`). Redraws only that button; the layout is unchanged.
    /// Idempotent; an index out of range is ignored with a warning. Never panics.
    ///
    /// ```
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::buttonmatrix::{self, ButtonMatrix};
    ///
    /// let mut h = EngineHarness::new(200, 100);
    /// let screen = h.screen();
    /// let m = buttonmatrix::create(h.engine_mut(), screen).unwrap(); // "Btn1" … "Btn5"
    /// h.engine_mut().with_widget_mut(m, |w: &mut ButtonMatrix, cx| w.set_btn_text(cx, 1, "Two"));
    /// assert_eq!(h.engine().widget::<ButtonMatrix>(m).unwrap().btn_text(1), Some("Two"));
    /// ```
    pub fn set_btn_text(&mut self, cx: &mut WidgetCx<'_>, idx: u16, text: &str) {
        if idx >= self.btn_count() {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: button {} out of range", idx);
            return;
        }
        let text = cx.fit_text(text);
        if let Some(buf) = self.map.text_buf(usize::from(idx), text, cx.partition()) {
            cx.store_text(buf, text);
            log_set(BUTTONMATRIX_CLASS.name, cx.node(), "btn_text");
            self.invalidate_btn(cx, Some(idx));
        }
    }

    /// Sets the control bits of every button to `f(index)`. Idempotent.
    pub(crate) fn set_ctrl_map_with(&mut self, cx: &mut WidgetCx<'_>, f: impl Fn(usize) -> BtnCtrl) {
        if self.ctrl.iter().enumerate().all(|(i, c)| *c == f(i)) {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "ctrl_map");
        for (i, c) in self.ctrl.iter_mut().enumerate() {
            *c = f(i);
        }
        self.warn_recolor();
        self.update_map(cx);
    }

    /// Adds `ctrl` bits to button `idx` (LVGL `lv_buttonmatrix_set_button_ctrl`); with
    /// [`one_checked`](Self::one_checked), checking one unchecks the others. Idempotent.
    pub fn set_btn_ctrl(&mut self, cx: &mut WidgetCx<'_>, idx: u16, ctrl: BtnCtrl) {
        let Some(cur) = self.btn_ctrl(idx) else {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: button {} out of range", idx);
            return;
        };
        if cur.contains(ctrl) {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "btn_ctrl");
        if self.one_checked && ctrl.contains(BtnCtrl::CHECKED) {
            self.clear_btn_ctrl_all(cx, BtnCtrl::CHECKED);
        }
        let width_changed = (ctrl & BtnCtrl::WIDTH_MASK) != BtnCtrl::empty();
        if let Some(c) = self.ctrl.get_mut(usize::from(idx)) {
            *c |= ctrl;
        }
        self.warn_recolor();
        if width_changed {
            self.update_map(cx);
        } else {
            self.invalidate_btn(cx, Some(idx));
        }
    }

    /// Removes `ctrl` bits from button `idx` (LVGL `lv_buttonmatrix_clear_button_ctrl`).
    /// Idempotent.
    pub fn clear_btn_ctrl(&mut self, cx: &mut WidgetCx<'_>, idx: u16, ctrl: BtnCtrl) {
        let Some(cur) = self.btn_ctrl(idx) else {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: button {} out of range", idx);
            return;
        };
        if !cur.intersects(ctrl) {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "btn_ctrl");
        let width_changed = cur.intersects(ctrl & BtnCtrl::WIDTH_MASK);
        if let Some(c) = self.ctrl.get_mut(usize::from(idx)) {
            *c &= !ctrl;
        }
        if width_changed {
            self.update_map(cx);
        } else {
            self.invalidate_btn(cx, Some(idx));
        }
    }

    /// Adds `ctrl` to every button (LVGL `lv_buttonmatrix_set_button_ctrl_all`).
    pub fn set_btn_ctrl_all(&mut self, cx: &mut WidgetCx<'_>, ctrl: BtnCtrl) {
        for i in 0..self.btn_count() {
            self.set_btn_ctrl(cx, i, ctrl);
        }
    }

    /// Removes `ctrl` from every button (LVGL `lv_buttonmatrix_clear_button_ctrl_all`).
    pub fn clear_btn_ctrl_all(&mut self, cx: &mut WidgetCx<'_>, ctrl: BtnCtrl) {
        for i in 0..self.btn_count() {
            self.clear_btn_ctrl(cx, i, ctrl);
        }
    }

    /// Sets the relative width of button `idx` (1…15; LVGL
    /// `lv_buttonmatrix_set_button_width`). Idempotent.
    pub fn set_btn_width(&mut self, cx: &mut WidgetCx<'_>, idx: u16, units: u8) {
        let Some(cur) = self.btn_ctrl(idx) else {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: button {} out of range", idx);
            return;
        };
        if !(1..=15).contains(&units) {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: width {} not in 1..=15", units);
        }
        let new = (cur & !BtnCtrl::WIDTH_MASK) | BtnCtrl::width(units);
        if new.width_units() == cur.width_units() {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "btn_width");
        if let Some(c) = self.ctrl.get_mut(usize::from(idx)) {
            *c = new;
        }
        self.update_map(cx);
    }

    /// Allows at most one checked button (LVGL `lv_buttonmatrix_set_one_checked`: when
    /// enabled, only the first checked button stays checked). Idempotent.
    pub fn set_one_checked(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.one_checked == on {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "one_checked");
        self.one_checked = on;
        if on {
            let first = self.ctrl.iter().position(|c| c.contains(BtnCtrl::CHECKED));
            for i in 0..self.ctrl.len() {
                if Some(i) != first && self.uncheck(i) {
                    self.invalidate_btn(cx, u16::try_from(i).ok());
                }
            }
        }
    }

    /// Selects button `idx` (or none; LVGL `lv_buttonmatrix_set_selected_button`).
    /// Idempotent; an index out of range is ignored with a warning.
    pub fn set_selected_btn(&mut self, cx: &mut WidgetCx<'_>, idx: Option<u16>) {
        if idx.is_some_and(|i| i >= self.btn_count()) {
            twine_core::warn!(target: "twine::engine", "buttonmatrix: button {:?} out of range", idx);
            return;
        }
        if self.selected == idx {
            return;
        }
        log_set(BUTTONMATRIX_CLASS.name, cx.node(), "selected_btn");
        let old = self.selected;
        self.invalidate_btn(cx, old);
        self.selected = idx;
        self.invalidate_btn(cx, idx);
    }

    /// Removes `CHECKED` from button `i`; whether it was checked.
    fn uncheck(&mut self, i: usize) -> bool {
        match self.ctrl.get_mut(i) {
            Some(c) if c.contains(BtnCtrl::CHECKED) => {
                c.remove(BtnCtrl::CHECKED);
                true
            }
            _ => false,
        }
    }

    fn warn_recolor(&self) {
        if self.ctrl.iter().any(|c| c.contains(BtnCtrl::RECOLOR)) && !WARN_RECOLOR.load(Ordering::Relaxed) {
            WARN_RECOLOR.store(true, Ordering::Relaxed);
            twine_core::warn!(target: "twine::engine", "buttonmatrix: BtnCtrl::RECOLOR is not supported (ignored)");
        }
    }

    /// Takes the buttons, rows and control bits of a new map (LVGL
    /// `allocate_button_areas_and_controls` + `set_ctrl_map`); growth of the tables is noted in
    /// partition `part`'s seal domain (the widget's engine).
    fn parse_map(&mut self, part: twine_core::Partition) {
        let n = self.map.btn_count().min(usize::from(u16::MAX));
        // Kept between maps: a map with more buttons than any before grows them.
        if self.ctrl.capacity() < n || self.areas.capacity() < n {
            twine_core::alloc_guard::grew(part, twine_core::alloc_guard::AllocSite::WidgetBuffer);
        }
        self.fill_from_map();
    }

    /// Fills the rows, control bits and areas from the map.
    fn fill_from_map(&mut self) {
        self.row_cnt = u16::try_from(self.map.row_count()).unwrap_or(u16::MAX);
        let n = self.map.btn_count().min(usize::from(u16::MAX));
        self.ctrl.clear();
        self.ctrl
            .extend((0..n).map(|i| self.map.initial_ctrl(i).unwrap_or_default()));
        self.warn_recolor();
        self.areas.resize(n, Area::default());
    }

    /// LVGL `update_map`: the button areas from the content area, then a full redraw.
    fn update_map(&mut self, cx: &mut WidgetCx<'_>) {
        self.compute_areas(&cx.measure());
        cx.invalidate_for("buttonmatrix.map");
    }

    fn compute_areas(&mut self, m: &MeasureCx<'_>) {
        let rtl = m.style(Part::Main, PropId::BaseDir).get::<BaseDir>() == Some(BaseDir::Rtl);
        let border = m.style_i32(Part::Main, PropId::BorderWidth);
        let pad = m.padding(Part::Main);
        let sleft = pad.left.saturating_add(border);
        let stop = pad.top.saturating_add(border);
        let prow = m.style_i32(Part::Main, PropId::RowGap);
        let pcol = m.style_i32(Part::Main, PropId::ColumnGap);
        let content = m.content_area();
        let max_w = content.width();
        let max_h = content.height();
        let Some(row_cnt) = NonZeroU16::new(self.row_cnt) else {
            return;
        };
        let rows = i64::from(row_cnt.get());
        // Style gaps are arbitrary `i32`s: the row geometry is computed in `i64` (products of
        // an `i32` and a row index below 2^16) and saturated back.
        // SAFE-ARITH: `|prow| ≤ 2^31` and `rows − 1 < 2^16`, so the product is below 2^47.
        let max_h_no_gap =
            i64::from(max_h).saturating_sub(i64::from(prow).wrapping_mul(rows.wrapping_sub(1)));
        // The share of rows `0 .. row` of the gap-less height (`row ≤ rows < 2^16`).
        let row_edge = |row: i64| {
            // SAFE-ARITH: `|max_h_no_gap| < 2^33` and `row < 2^17`, so the product is below 2^50.
            max_h_no_gap.wrapping_mul(row).checked_div(rows).unwrap_or(0)
        };
        // The control bits and areas not yet assigned to a row: both tables hold one entry
        // per button (`fill_from_map`), each row takes its buttons from the front.
        let mut ctrl_rest = self.ctrl.as_slice();
        let mut areas_rest = self.areas.as_mut_slice();
        for (row, btn_cnt) in (0..rows).zip(self.map.row_lens()) {
            let btn_cnt = btn_cnt.min(ctrl_rest.len()).min(areas_rest.len());
            let (ctrl_row, c_rest) = ctrl_rest.split_at(btn_cnt);
            let (areas_row, a_rest) = core::mem::take(&mut areas_rest).split_at_mut(btn_cnt);
            (ctrl_rest, areas_rest) = (c_rest, a_rest);
            // The units of this row (each button `1 ..= 15`, at most `u16::MAX` buttons).
            let units: u32 = ctrl_row.iter().fold(0, |s, c| s.saturating_add(c.width_units()));
            let Some(units) = NonZeroU64::new(u64::from(units)) else {
                // An empty row.
                continue;
            };
            // SAFE-ARITH: `|prow| ≤ 2^31` and `row < 2^16`: the product is below 2^47.
            let gaps = i64::from(prow).wrapping_mul(row);
            let y = |edge: i64| i64::from(stop).saturating_add(edge).saturating_add(gaps);
            let row_y1 = util::sat_i32(y(row_edge(row)));
            // SAFE-ARITH: `row < 2^16`.
            let row_y2 = util::sat_i32(y(row_edge(row.wrapping_add(1))).saturating_sub(1));
            let n = i64::try_from(btn_cnt).unwrap_or(i64::MAX);
            // SAFE-ARITH: `|pcol| ≤ 2^31` and `n − 1 < 2^16` (buttons per row of a map).
            let col_gaps = i64::from(pcol).wrapping_mul(n.saturating_sub(1));
            let max_w_no_gap = i64::from(max_w).saturating_sub(col_gaps).max(0).unsigned_abs();
            let mut row_unit = 0u64;
            for (b, (c, area)) in (0i64..).zip(ctrl_row.iter().zip(areas_row.iter_mut())) {
                let u = u64::from(c.width_units());
                // The share of the units before the button's ends: `max_w_no_gap < 2^34` and
                // `row_unit + u ≤ units < 2^20`, so the products stay below 2^54.
                // SAFE-ARITH: see above.
                let share =
                    |unit: u64| i64::try_from(max_w_no_gap.wrapping_mul(unit) / units).unwrap_or(i64::MAX);
                // SAFE-ARITH: `|pcol| ≤ 2^31` and `b < 2^16`.
                let ofs = i64::from(pcol).wrapping_mul(b);
                let mut x1 = util::sat_i32(share(row_unit).saturating_add(ofs));
                // SAFE-ARITH: `row_unit + u ≤ units < 2^20`.
                let next = row_unit.wrapping_add(u);
                let mut x2 = util::sat_i32(share(next).saturating_add(ofs).saturating_sub(1));
                if rtl {
                    core::mem::swap(&mut x1, &mut x2);
                    x1 = max_w.saturating_sub(x1);
                    x2 = max_w.saturating_sub(x2);
                }
                *area = Area {
                    x1: x1.saturating_add(sleft),
                    y1: row_y1,
                    x2: x2.saturating_add(sleft),
                    y2: row_y2,
                };
                row_unit = next;
            }
        }
    }

    /// LVGL `get_button_from_point`: the button under the absolute point `p`, with the gaps
    /// between buttons shared by their neighbours.
    fn button_from_point(&self, m: &MeasureCx<'_>, p: Point) -> Option<u16> {
        let c = m.coords();
        let (w, h) = (c.width(), c.height());
        let pad = m.padding(Part::Main);
        let pleft = pad.left;
        let prow = m.style_i32(Part::Main, PropId::RowGap);
        let pcol = m.style_i32(Part::Main, PropId::ColumnGap);
        // Half a gap, rounded up, plus one (at most `BTN_EXTRA_CLICK_AREA_MAX`).
        let half_gap = |g: i32| {
            (g / 2)
                .saturating_add(1)
                .saturating_add(g & 1)
                .min(BTN_EXTRA_CLICK_AREA_MAX)
        };
        let (prow, pcol) = (half_gap(prow), half_gap(pcol));
        let pright = pad.right.min(BTN_EXTRA_CLICK_AREA_MAX);
        let ptop = pad.top.min(BTN_EXTRA_CLICK_AREA_MAX);
        let pbottom = pad.bottom.min(BTN_EXTRA_CLICK_AREA_MAX);
        // The last pixels next to the right and bottom paddings.
        let (x_end, y_end) = (
            w.saturating_sub(pright).saturating_sub(2),
            h.saturating_sub(pbottom).saturating_sub(2),
        );
        let lo = |v: i32, o: i32, d: i32| v.saturating_add(o).saturating_sub(d);
        let hi = |v: i32, o: i32, d: i32| v.saturating_add(o).saturating_add(d);
        for (i, a) in self.areas.iter().enumerate() {
            let mut b = *a;
            b.x1 = lo(
                b.x1,
                c.x0,
                if b.x1 <= pleft {
                    pleft.min(BTN_EXTRA_CLICK_AREA_MAX)
                } else {
                    pcol
                },
            );
            b.y1 = lo(
                b.y1,
                c.y0,
                if b.y1 <= ptop {
                    ptop.min(BTN_EXTRA_CLICK_AREA_MAX)
                } else {
                    prow
                },
            );
            b.x2 = hi(
                b.x2,
                c.x0,
                if b.x2 >= x_end {
                    pright.min(BTN_EXTRA_CLICK_AREA_MAX)
                } else {
                    pcol
                },
            );
            b.y2 = hi(
                b.y2,
                c.y0,
                if b.y2 >= y_end {
                    pbottom.min(BTN_EXTRA_CLICK_AREA_MAX)
                } else {
                    prow
                },
            );
            if p.x >= b.x1 && p.x <= b.x2 && p.y >= b.y1 && p.y <= b.y2 {
                return u16::try_from(i).ok();
            }
        }
        None
    }

    /// LVGL `invalidate_button_area`: the button grown by the gaps (at least `dpi / 10`),
    /// for outlines and shadows.
    fn invalidate_btn(&self, cx: &mut WidgetCx<'_>, idx: Option<u16>) {
        let Some(r) = idx.and_then(|i| self.btn_invalidation_area(&cx.measure(), i)) else {
            return;
        };
        cx.invalidate_area(r);
    }

    /// The absolute area redrawn when button `idx` changes (pressed, selected, checked):
    /// the button grown by the row and column gaps, at least `dpi / 10` (LVGL
    /// `invalidate_button_area`: outlines and shadows fit in the gaps).
    #[must_use]
    pub fn btn_invalidation_area(&self, m: &MeasureCx<'_>, idx: u16) -> Option<Rect> {
        let a = *self.areas.get(usize::from(idx))?;
        let c = m.coords();
        let dpi = i32::from(util::display_dpi(m.engine(), m.node()));
        let row_gap = m.style_i32(Part::Main, PropId::RowGap).max(dpi / 10);
        let col_gap = m.style_i32(Part::Main, PropId::ColumnGap).max(dpi / 10);
        // LVGL grows x by the row gap and y by the column gap.
        let lo = |v: i32, o: i32, d: i32| v.saturating_add(o).saturating_sub(d);
        let hi = |v: i32, o: i32, d: i32| v.saturating_add(o).saturating_add(d);
        let b = Area {
            x1: lo(a.x1, c.x0, row_gap),
            y1: lo(a.y1, c.y0, col_gap),
            x2: hi(a.x2, c.x0, row_gap),
            y2: hi(a.y2, c.y0, col_gap),
        };
        Some(b.to_rect())
    }

    /// LVGL `make_one_button_checked`.
    fn make_one_checked(&mut self, cx: &mut WidgetCx<'_>, idx: u16) {
        let was = self.has_btn_ctrl(idx, BtnCtrl::CHECKED);
        for i in 0..self.ctrl.len() {
            if self.uncheck(i) {
                self.invalidate_btn(cx, u16::try_from(i).ok());
            }
        }
        if was {
            if let Some(c) = self.ctrl.get_mut(usize::from(idx)) {
                c.insert(BtnCtrl::CHECKED);
            }
            self.invalidate_btn(cx, Some(idx));
        }
    }

    /// The first selectable button (LVGL `LV_EVENT_FOCUSED`; with one-checked the checked
    /// one).
    fn first_selectable(&self) -> Option<u16> {
        let b = self
            .ctrl
            .iter()
            .position(|c| !c.is_inactive() && (!self.one_checked || c.contains(BtnCtrl::CHECKED)));
        // LVGL sets `btn_cnt` (an invalid index) when nothing matches; Twine selects none.
        b.and_then(|b| u16::try_from(b).ok())
    }

    /// Handles `ev` like LVGL's `lv_buttonmatrix_event` and returns the button whose
    /// `ValueChanged` is due (the caller sends it, a keyboard acts on it first).
    pub(crate) fn handle(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> Option<u16> {
        if ev.target != cx.node() {
            return None;
        }
        let node = cx.node();
        match ev.code {
            // `Items` are item parts: a state change of the node redraws only the selected
            // button, the one that shows the node's pressed / focused / edited state.
            EventCode::StateChanged => {
                let prev = ev.prev_state().unwrap_or_default();
                let mut wcx = cx.widget_cx();
                if (prev ^ wcx.state()).intersects(SELECTED_STATES) {
                    self.invalidate_btn(&mut wcx, self.selected);
                }
                None
            }
            EventCode::StyleChanged | EventCode::SizeChanged => {
                let mut wcx = cx.widget_cx();
                self.update_map(&mut wcx);
                None
            }
            EventCode::Pressed => {
                let kind = util::active_input_kind(cx.engine());
                let mut wcx = cx.widget_cx();
                self.invalidate_btn(&mut wcx, self.selected);
                if matches!(kind, Some(InputKind::Pointer | InputKind::Button)) {
                    let p = cx.point().unwrap_or(Point::new(-1, -1));
                    let hit = self.button_from_point(&MeasureCx::new(cx.engine(), node), p);
                    self.selected = hit.filter(|&b| self.btn_ctrl(b).is_some_and(|c| !c.is_inactive()));
                    let mut wcx = cx.widget_cx();
                    self.invalidate_btn(&mut wcx, self.selected);
                }
                let sel = self.selected?;
                let c = self.btn_ctrl(sel)?;
                if c.contains(BtnCtrl::POPOVER) {
                    self.show_popover(cx.engine_mut(), node, sel);
                }
                if c.intersects(BtnCtrl::CLICK_TRIG | BtnCtrl::POPOVER) || c.is_inactive() {
                    return None;
                }
                Some(sel)
            }
            EventCode::Pressing => {
                let sel = self.selected?;
                if matches!(
                    util::active_input_kind(cx.engine()),
                    Some(InputKind::Pointer | InputKind::Button)
                ) {
                    let p = cx.point()?;
                    let hit = self.button_from_point(&MeasureCx::new(cx.engine(), node), p);
                    if hit != Some(sel) {
                        // Slid to another button: nothing is pressed any more (LVGL).
                        let mut wcx = cx.widget_cx();
                        self.invalidate_btn(&mut wcx, Some(sel));
                        self.selected = None;
                        self.hide_popover(cx.engine_mut());
                    }
                }
                None
            }
            EventCode::Released => {
                self.hide_popover(cx.engine_mut());
                let sel = self.selected?;
                let mut wcx = cx.widget_cx();
                let c = self.btn_ctrl(sel)?;
                if c.contains(BtnCtrl::CHECKABLE) && !c.contains(BtnCtrl::DISABLED) {
                    let one_checked = self.one_checked;
                    if let Some(bits) = self.ctrl.get_mut(usize::from(sel)) {
                        // Unchecked when checked, unless one stays checked.
                        bits.set(BtnCtrl::CHECKED, !c.contains(BtnCtrl::CHECKED) || one_checked);
                    }
                    if self.one_checked {
                        self.make_one_checked(&mut wcx, sel);
                    }
                }
                self.invalidate_btn(&mut wcx, Some(sel));
                (c.intersects(BtnCtrl::CLICK_TRIG | BtnCtrl::POPOVER) && !c.is_inactive()).then_some(sel)
            }
            EventCode::LongPressedRepeat => {
                let sel = self.selected?;
                let c = self.btn_ctrl(sel)?;
                (!c.contains(BtnCtrl::NO_REPEAT) && !c.is_inactive()).then_some(sel)
            }
            EventCode::PressLost => {
                self.hide_popover(cx.engine_mut());
                let mut wcx = cx.widget_cx();
                self.invalidate_btn(&mut wcx, self.selected);
                self.selected = None;
                None
            }
            EventCode::Focused => {
                if self.ctrl.is_empty() || self.selected.is_some() {
                    return None;
                }
                let e = cx.engine();
                let editing = e.group_of(node).is_some_and(|g| e.group_editing(g));
                let kind = util::active_input_kind(e);
                if kind == Some(InputKind::Keypad) || (kind == Some(InputKind::Encoder) && editing) {
                    self.selected = self.first_selectable();
                    let mut wcx = cx.widget_cx();
                    self.invalidate_btn(&mut wcx, self.selected);
                }
                None
            }
            EventCode::Key => {
                let key = ev.key()?;
                let mut wcx = cx.widget_cx();
                self.invalidate_btn(&mut wcx, self.selected);
                self.key_nav(&wcx.measure(), key);
                self.invalidate_btn(&mut wcx, self.selected);
                None
            }
            EventCode::Delete => {
                if let Some(p) = self.popover.take() {
                    if cx.engine().tree().contains(p) {
                        let _ = cx.engine_mut().delete(p);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// LVGL `LV_EVENT_KEY`: moves the selection with the arrow keys.
    fn key_nav(&mut self, m: &MeasureCx<'_>, key: Key) {
        let n = self.ctrl.len();
        if n == 0 {
            return;
        }
        // A button index out of the table counts as inactive (never selected).
        let inactive = |i: usize| self.ctrl.get(i).is_none_or(|c| c.is_inactive());
        // The next / previous index, cycling through `0 .. n` (`n ≥ 1`).
        let next = |s: usize| {
            let t = s.saturating_add(1);
            if t >= n { 0 } else { t }
        };
        let prev = |s: usize| {
            if s == 0 {
                n.saturating_sub(1)
            } else {
                s.saturating_sub(1)
            }
        };
        match key {
            Key::Right => {
                let mut s = self.selected.map_or(0, |s| usize::from(s).saturating_add(1));
                if s >= n {
                    s = 0;
                }
                let start = s;
                let mut found = Some(s);
                while inactive(s) {
                    s = next(s);
                    if s == start {
                        found = None;
                        break;
                    }
                    found = Some(s);
                }
                self.selected = found.and_then(|s| u16::try_from(s).ok());
            }
            Key::Left => {
                let mut s = prev(self.selected.map_or(0, usize::from));
                let start = s;
                let mut found = Some(s);
                while inactive(s) {
                    s = prev(s);
                    if s == start {
                        found = None;
                        break;
                    }
                    found = Some(s);
                }
                self.selected = found.and_then(|s| u16::try_from(s).ok());
            }
            Key::Down | Key::Up => {
                let col_gap = m.style_i32(Part::Main, PropId::ColumnGap);
                let Some(cur) = self.selected.map(usize::from) else {
                    self.selected = (0..n).find(|&i| !inactive(i)).and_then(|i| u16::try_from(i).ok());
                    return;
                };
                let Some(&a) = self.areas.get(cur) else {
                    return;
                };
                let center = a.x1.saturating_add(a.width() >> 1);
                let below =
                    |b: &Area| b.y1 > a.y1 && center >= b.x1 && center <= b.x2.saturating_add(col_gap);
                let above =
                    |b: &Area| b.y1 < a.y1 && center >= b.x1.saturating_sub(col_gap) && center <= b.x2;
                let target = if key == Key::Down {
                    (cur..n).find(|&i| self.areas.get(i).is_some_and(below) && !inactive(i))
                } else {
                    (0..=cur)
                        .rev()
                        .find(|&i| self.areas.get(i).is_some_and(above) && !inactive(i))
                };
                if let Some(t) = target {
                    self.selected = u16::try_from(t).ok();
                }
            }
            _ => {}
        }
    }

    /// The `Items` state of button `i` with control bits `c` (LVGL `draw_main`).
    fn btn_state(&self, i: usize, c: BtnCtrl, node_state: State) -> State {
        let mut s = State::DEFAULT;
        if c.contains(BtnCtrl::CHECKED) {
            s |= State::CHECKED;
        }
        if c.contains(BtnCtrl::DISABLED) {
            s |= State::DISABLED;
        } else if self.selected == u16::try_from(i).ok() {
            s |= node_state & SELECTED_STATES;
        }
        s
    }

    /// Draws the buttons (LVGL `draw_main`).
    fn draw_buttons(&self, cx: &mut DrawCx<'_, '_>) {
        if self.ctrl.is_empty() {
            return;
        }
        let e = cx.engine();
        let node = cx.node();
        let obj = cx.coords();
        let clip = cx.clip();
        let opa = cx.opa();
        let node_state = cx.state();
        let m = MeasureCx::new(e, node);
        let pad = m.padding(Part::Main);
        let dpi_margin = i32::from(util::display_dpi(e, node)) / 10;
        let clip_margin = m
            .style_i32(Part::Main, PropId::RowGap)
            .max(dpi_margin)
            .max(m.style_i32(Part::Main, PropId::ColumnGap).max(dpi_margin));
        let def_rect = e.rect_dsc_for_state(node, Part::Items, State::DEFAULT, opa);
        let def_text = e.text_dsc_for_state(node, Part::Items, State::DEFAULT, opa);
        // The edges of the content box: a button touching one draws no internal border there.
        let (left, top) = (obj.x0.saturating_add(pad.left), obj.y0.saturating_add(pad.top));
        let right = obj.x1.saturating_sub(1).saturating_sub(pad.right);
        let bottom = obj.y1.saturating_sub(1).saturating_sub(pad.bottom);
        let buttons = self.areas.iter().zip(&self.ctrl).zip(self.map.texts());
        for (i, ((a, &c), txt)) in buttons.enumerate() {
            if c.contains(BtnCtrl::HIDDEN) {
                continue;
            }
            let btn = Area {
                x1: a.x1.saturating_add(obj.x0),
                y1: a.y1.saturating_add(obj.y0),
                x2: a.x2.saturating_add(obj.x0),
                y2: a.y2.saturating_add(obj.y0),
            };
            let test = btn.to_rect().expand(clip_margin);
            if test.intersection(&clip).is_none() {
                continue;
            }
            let st = self.btn_state(i, c, node_state);
            let (rect, mut text) = if st == State::DEFAULT {
                (def_rect, def_text)
            } else {
                (
                    e.rect_dsc_for_state(node, Part::Items, st, opa),
                    e.text_dsc_for_state(node, Part::Items, st, opa),
                )
            };
            let mut dsc = rect.dsc();
            // Each button is drawn in one go (LVGL `lv_draw_rect`).
            dsc.border_post = false;
            if dsc.border_side.0 & BorderSide::INTERNAL.0 != 0 {
                let mut side = BorderSide::FULL.0;
                if btn.x1 == left {
                    side &= !BorderSide::LEFT.0;
                }
                if btn.x2 == right {
                    side &= !BorderSide::RIGHT.0;
                }
                if btn.y1 == top {
                    side &= !BorderSide::TOP.0;
                }
                if btn.y2 == bottom {
                    side &= !BorderSide::BOTTOM.0;
                }
                dsc.border_side = BorderSide(side);
            }
            cx.painter().rect(btn.to_rect(), &dsc);
            let mut l = TextLayout::new(txt, text.font);
            l.letter_space = text.letter_space;
            l.line_space = text.line_space;
            l.max_width = obj.width();
            let size: Size = l.measure();
            let x1 = btn.x1.saturating_add(btn.width().saturating_sub(size.w) / 2);
            let y1 = btn.y1.saturating_add(btn.height().saturating_sub(size.h) / 2);
            // The text is centered by position; its lines are centered within it.
            text.align = TextAlign::Center;
            let end = |o: i32, len: i32| o.saturating_add(len).saturating_add(1);
            cx.draw_text(Rect::new(x1, y1, end(x1, size.w), end(y1, size.h)), txt, &text);
        }
    }

    fn show_popover(&mut self, e: &mut Engine, node: NodeId, btn: u16) {
        let Some(area) = self.btn_area(&MeasureCx::new(e, node), btn) else {
            return;
        };
        let node_state = e
            .tree()
            .node(node)
            .map_or(State::DEFAULT, twine_engine::Node::state);
        let state = self.btn_state(
            usize::from(btn),
            self.btn_ctrl(btn).unwrap_or_default(),
            node_state,
        );
        let text = self.btn_text(btn).unwrap_or("");
        let p = popover::show(e, node, self.popover, area, text, state);
        self.popover = p;
    }

    fn hide_popover(&self, e: &mut Engine) {
        if let Some(p) = self.popover {
            popover::hide(e, p);
        }
    }
}

/// Creates a button matrix with LVGL's default map ("Btn1" … "Btn5"), 260 × 130 px, as the
/// last child of `parent` (LVGL `lv_buttonmatrix_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(ButtonMatrix::default()))
}

/// Creates a button matrix showing `map`.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create_with(engine: &mut Engine, parent: NodeId, map: BtnMap) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(ButtonMatrix::new(map)))
}

impl Widget for ButtonMatrix {
    fn class(&self) -> &'static WidgetClass {
        &BUTTONMATRIX_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut()
            .set_size(id, BUTTONMATRIX_DEFAULT_WIDTH, BUTTONMATRIX_DEFAULT_HEIGHT);
        self.update_map(cx);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.draw_buttons(cx);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if let Some(b) = self.handle(cx, ev) {
            let node = cx.node();
            cx.post(node, EventCode::ValueChanged, EventParam::Value(i32::from(b)));
        }
        EventResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_parsing_counts_rows_and_buttons() {
        static MAP: [&[MapBtn]; 2] = [
            &[MapBtn::text("a"), MapBtn::text("")],
            &[MapBtn::new("c", BtnCtrl::CHECKED)],
        ];
        let m = ButtonMatrix::new(BtnMap::from(&MAP));
        assert_eq!(m.btn_count(), 3);
        assert_eq!(m.row_count(), 2);
        assert_eq!(m.btn_text(1), Some(""), "an empty text is a button");
        assert_eq!(m.btn_text(2), Some("c"));
        assert_eq!(m.btn_text(3), None);
        assert_eq!(m.btn_ctrl(2), Some(BtnCtrl::CHECKED), "the map's control bits");
    }

    #[test]
    fn ctrl_width_helpers() {
        assert_eq!(BtnCtrl::width(0).width_units(), 1);
        assert_eq!(BtnCtrl::width(20).width_units(), 15);
        assert_eq!(BtnCtrl::empty().width_units(), 1);
        assert!(BtnCtrl::HIDDEN.is_inactive());
        assert!(!BtnCtrl::CHECKED.is_inactive());
    }

    #[test]
    fn class_matches_lvgl() {
        assert_eq!(BUTTONMATRIX_CLASS.editable, Editable::True);
        assert_eq!(BUTTONMATRIX_CLASS.group_def, GroupDef::True);
        assert_eq!(
            (BUTTONMATRIX_DEFAULT_WIDTH, BUTTONMATRIX_DEFAULT_HEIGHT),
            (260, 130)
        );
    }
}
