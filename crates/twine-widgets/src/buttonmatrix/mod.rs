//! [`ButtonMatrix`]: a grid of text buttons drawn by one widget (LVGL `lv_buttonmatrix`).

mod popover;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
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

/// The texts of a button matrix: rows of button labels separated by `"\n"` entries (LVGL's
/// map format). The map ends at the end of the slice; an empty string also ends it (LVGL's
/// `""` terminator, accepted for compatibility).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MapSrc {
    /// A map in flash (no allocation).
    Static(&'static [&'static str]),
    /// An owned map.
    Owned(Vec<String>),
}

impl MapSrc {
    /// The number of entries (row breaks included, the terminator and anything after it
    /// excluded).
    #[must_use]
    pub fn len(&self) -> usize {
        (0..self.raw_len())
            .find(|&i| self.get(i).is_empty())
            .unwrap_or(self.raw_len())
    }

    /// Whether the map has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn raw_len(&self) -> usize {
        match self {
            MapSrc::Static(m) => m.len(),
            MapSrc::Owned(m) => m.len(),
        }
    }

    /// Entry `i` (`""` past the end).
    #[must_use]
    pub fn get(&self, i: usize) -> &str {
        match self {
            MapSrc::Static(m) => m.get(i).copied().unwrap_or(""),
            MapSrc::Owned(m) => m.get(i).map_or("", String::as_str),
        }
    }

    fn same(&self, other: &MapSrc) -> bool {
        match (self, other) {
            (MapSrc::Static(a), MapSrc::Static(b)) => core::ptr::eq(*a, *b) || a == b,
            _ => (0..self.len().max(other.len())).all(|i| self.get(i) == other.get(i)),
        }
    }
}

impl From<&'static [&'static str]> for MapSrc {
    fn from(m: &'static [&'static str]) -> Self {
        MapSrc::Static(m)
    }
}

impl<const N: usize> From<&'static [&'static str; N]> for MapSrc {
    fn from(m: &'static [&'static str; N]) -> Self {
        MapSrc::Static(m)
    }
}

/// LVGL `lv_buttonmatrix_def_map` (`LV_WIDGETS_HAS_DEFAULT_VALUE`).
pub const BUTTONMATRIX_DEFAULT_MAP: &[&str] = &["Btn1", "Btn2", "Btn3", "\n", "Btn4", "Btn5"];

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
/// - **Map**: rows of labels separated by `"\n"` ([`MapSrc`]). Buttons have relative widths
///   (the [`BtnCtrl`] width bits); a row's content width minus the column gaps is shared
///   by units, like LVGL (`(w − (n − 1)·pad_column) · units / total`), rows are equally tall
///   with `pad_row` between them.
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
/// use twine_widgets::buttonmatrix::{self, BtnCtrl, ButtonMatrix};
///
/// static MAP: [&str; 5] = ["1", "2", "\n", "OK", ""];
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
    map: MapSrc,
    /// The map index of every button's text.
    text_idx: Vec<u16>,
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
        Self::new(MapSrc::Static(BUTTONMATRIX_DEFAULT_MAP))
    }
}

impl ButtonMatrix {
    /// A button matrix with `map` (no areas until it is laid out).
    #[must_use]
    pub fn new(map: MapSrc) -> Self {
        let mut m = Self {
            map: MapSrc::Static(&[]),
            text_idx: Vec::new(),
            ctrl: Vec::new(),
            areas: Vec::new(),
            row_cnt: 1,
            selected: None,
            one_checked: false,
            popover: None,
        };
        m.map = map;
        m.parse_map();
        m
    }

    /// The map.
    #[must_use]
    pub fn map(&self) -> &MapSrc {
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
        self.text_idx
            .get(usize::from(idx))
            .map(|&i| self.map.get(usize::from(i)))
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

    /// Sets the map (LVGL `lv_buttonmatrix_set_map`). Idempotent. The control bits are kept
    /// when the button count is unchanged (LVGL), else cleared. The selection is kept on the
    /// button that now occupies its position.
    pub fn set_map(&mut self, cx: &mut WidgetCx<'_>, map: MapSrc) {
        if self.map.same(&map) {
            if let (MapSrc::Static(_), MapSrc::Static(_)) = (&self.map, &map) {
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
                Point::new(c.x0 + a.x1 + (a.width() >> 1), c.y0 + a.y1 + (a.height() >> 1))
            });
        self.map = map;
        self.parse_map();
        self.update_map(cx);
        if let Some(p) = sel_center {
            let new_sel = self
                .button_from_point(&cx.measure(), p)
                .filter(|&b| !self.ctrl[usize::from(b)].is_inactive());
            self.selected = new_sel;
            self.invalidate_btn(cx, new_sel);
        }
    }

    /// Sets the control bits of every button from `ctrl` (LVGL
    /// `lv_buttonmatrix_set_ctrl_map`; missing entries are cleared). Idempotent.
    pub fn set_ctrl_map(&mut self, cx: &mut WidgetCx<'_>, ctrl: &[BtnCtrl]) {
        self.set_ctrl_map_with(cx, |i| ctrl.get(i).copied().unwrap_or_default());
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
        self.ctrl[usize::from(idx)] |= ctrl;
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
        self.ctrl[usize::from(idx)] &= !ctrl;
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
        self.ctrl[usize::from(idx)] = new;
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
                if Some(i) != first && self.ctrl[i].contains(BtnCtrl::CHECKED) {
                    self.ctrl[i].remove(BtnCtrl::CHECKED);
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

    fn warn_recolor(&self) {
        if self.ctrl.iter().any(|c| c.contains(BtnCtrl::RECOLOR)) && !WARN_RECOLOR.load(Ordering::Relaxed) {
            WARN_RECOLOR.store(true, Ordering::Relaxed);
            twine_core::warn!(target: "twine::engine", "buttonmatrix: BtnCtrl::RECOLOR is not supported (ignored)");
        }
    }

    /// Counts buttons and rows (LVGL `allocate_button_areas_and_controls`): the control
    /// bits are kept when the count is unchanged.
    fn parse_map(&mut self) {
        let len = self.map.len();
        let mut rows = 1u16;
        self.text_idx.clear();
        for i in 0..len {
            if self.map.get(i) == "\n" {
                rows = rows.saturating_add(1);
            } else {
                self.text_idx.push(u16::try_from(i).unwrap_or(u16::MAX));
            }
        }
        self.row_cnt = rows;
        let n = self.text_idx.len();
        if n != self.ctrl.len() {
            self.ctrl.clear();
            self.ctrl.resize(n, BtnCtrl::empty());
        }
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
        let sleft = pad.left + border;
        let stop = pad.top + border;
        let prow = m.style_i32(Part::Main, PropId::PadRow);
        let pcol = m.style_i32(Part::Main, PropId::PadColumn);
        let content = m.content_area();
        let max_w = content.width();
        let max_h = content.height();
        let row_cnt = i32::from(self.row_cnt);
        let max_h_no_gap = max_h - prow * (row_cnt - 1);
        let mut btn_tot = 0usize;
        let mut map_i = 0usize;
        let len = self.map.len();
        for row in 0..row_cnt {
            // Count the buttons and units of this row.
            let mut units = 0u32;
            let mut btn_cnt = 0usize;
            while map_i + btn_cnt < len && self.map.get(map_i + btn_cnt) != "\n" {
                units += self.ctrl[btn_tot + btn_cnt].width_units();
                btn_cnt += 1;
            }
            if btn_cnt == 0 {
                map_i += 1;
                continue;
            }
            let row_y1 = stop + (max_h_no_gap * row) / row_cnt + row * prow;
            let row_y2 = stop + (max_h_no_gap * (row + 1)) / row_cnt + row * prow - 1;
            let n = i32::try_from(btn_cnt).unwrap_or(i32::MAX);
            let max_w_no_gap = (max_w - pcol * (n - 1)).max(0);
            let units = i64::from(units);
            let mut row_unit = 0i64;
            for b in 0..n {
                let u = i64::from(self.ctrl[btn_tot].width_units());
                let mut x1 = (i64::from(max_w_no_gap) * row_unit / units) as i32 + b * pcol;
                let mut x2 = (i64::from(max_w_no_gap) * (row_unit + u) / units) as i32 + b * pcol - 1;
                if rtl {
                    core::mem::swap(&mut x1, &mut x2);
                    x1 = max_w - x1;
                    x2 = max_w - x2;
                }
                self.areas[btn_tot] = Area {
                    x1: x1 + sleft,
                    y1: row_y1,
                    x2: x2 + sleft,
                    y2: row_y2,
                };
                row_unit += u;
                btn_tot += 1;
            }
            map_i += btn_cnt + 1;
        }
    }

    /// LVGL `get_button_from_point`: the button under the absolute point `p`, with the gaps
    /// between buttons shared by their neighbours.
    fn button_from_point(&self, m: &MeasureCx<'_>, p: Point) -> Option<u16> {
        let c = m.coords();
        let (w, h) = (c.width(), c.height());
        let pad = m.padding(Part::Main);
        let pleft = pad.left;
        let prow = m.style_i32(Part::Main, PropId::PadRow);
        let pcol = m.style_i32(Part::Main, PropId::PadColumn);
        let prow = ((prow / 2) + 1 + (prow & 1)).min(BTN_EXTRA_CLICK_AREA_MAX);
        let pcol = ((pcol / 2) + 1 + (pcol & 1)).min(BTN_EXTRA_CLICK_AREA_MAX);
        let pright = pad.right.min(BTN_EXTRA_CLICK_AREA_MAX);
        let ptop = pad.top.min(BTN_EXTRA_CLICK_AREA_MAX);
        let pbottom = pad.bottom.min(BTN_EXTRA_CLICK_AREA_MAX);
        for (i, a) in self.areas.iter().enumerate() {
            let mut b = *a;
            if b.x1 <= pleft {
                b.x1 += c.x0 - pleft.min(BTN_EXTRA_CLICK_AREA_MAX);
            } else {
                b.x1 += c.x0 - pcol;
            }
            if b.y1 <= ptop {
                b.y1 += c.y0 - ptop.min(BTN_EXTRA_CLICK_AREA_MAX);
            } else {
                b.y1 += c.y0 - prow;
            }
            if b.x2 >= w - pright - 2 {
                b.x2 += c.x0 + pright.min(BTN_EXTRA_CLICK_AREA_MAX);
            } else {
                b.x2 += c.x0 + pcol;
            }
            if b.y2 >= h - pbottom - 2 {
                b.y2 += c.y0 + pbottom.min(BTN_EXTRA_CLICK_AREA_MAX);
            } else {
                b.y2 += c.y0 + prow;
            }
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
        let row_gap = m.style_i32(Part::Main, PropId::PadRow).max(dpi / 10);
        let col_gap = m.style_i32(Part::Main, PropId::PadColumn).max(dpi / 10);
        // LVGL grows x by the row gap and y by the column gap.
        let b = Area {
            x1: a.x1 + c.x0 - row_gap,
            y1: a.y1 + c.y0 - col_gap,
            x2: a.x2 + c.x0 + row_gap,
            y2: a.y2 + c.y0 + col_gap,
        };
        Some(b.to_rect())
    }

    /// LVGL `make_one_button_checked`.
    fn make_one_checked(&mut self, cx: &mut WidgetCx<'_>, idx: u16) {
        let was = self.has_btn_ctrl(idx, BtnCtrl::CHECKED);
        for i in 0..self.ctrl.len() {
            if self.ctrl[i].contains(BtnCtrl::CHECKED) {
                self.ctrl[i].remove(BtnCtrl::CHECKED);
                self.invalidate_btn(cx, u16::try_from(i).ok());
            }
        }
        if was {
            self.ctrl[usize::from(idx)].insert(BtnCtrl::CHECKED);
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
                    self.selected = hit.filter(|&b| !self.ctrl[usize::from(b)].is_inactive());
                    let mut wcx = cx.widget_cx();
                    self.invalidate_btn(&mut wcx, self.selected);
                }
                let sel = self.selected?;
                let c = self.ctrl[usize::from(sel)];
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
                let i = usize::from(sel);
                let mut wcx = cx.widget_cx();
                let c = self.ctrl[i];
                if c.contains(BtnCtrl::CHECKABLE) && !c.contains(BtnCtrl::DISABLED) {
                    if c.contains(BtnCtrl::CHECKED) && !self.one_checked {
                        self.ctrl[i].remove(BtnCtrl::CHECKED);
                    } else {
                        self.ctrl[i].insert(BtnCtrl::CHECKED);
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
                let c = self.ctrl[usize::from(sel)];
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
        let inactive = |i: usize| self.ctrl[i].is_inactive();
        match key {
            Key::Right => {
                let mut s = self.selected.map_or(0, |s| usize::from(s) + 1);
                if s >= n {
                    s = 0;
                }
                let start = s;
                let mut found = Some(s);
                while inactive(s) {
                    s = if s + 1 >= n { 0 } else { s + 1 };
                    if s == start {
                        found = None;
                        break;
                    }
                    found = Some(s);
                }
                self.selected = found.and_then(|s| u16::try_from(s).ok());
            }
            Key::Left => {
                let mut s = self.selected.map_or(0, usize::from);
                s = if s == 0 { n - 1 } else { s - 1 };
                let start = s;
                let mut found = Some(s);
                while inactive(s) {
                    s = if s > 0 { s - 1 } else { n - 1 };
                    if s == start {
                        found = None;
                        break;
                    }
                    found = Some(s);
                }
                self.selected = found.and_then(|s| u16::try_from(s).ok());
            }
            Key::Down | Key::Up => {
                let col_gap = m.style_i32(Part::Main, PropId::PadColumn);
                let Some(cur) = self.selected.map(usize::from) else {
                    self.selected = (0..n).find(|&i| !inactive(i)).and_then(|i| u16::try_from(i).ok());
                    return;
                };
                let a = self.areas[cur];
                let center = a.x1 + (a.width() >> 1);
                let target = if key == Key::Down {
                    (cur..n).find(|&i| {
                        let b = self.areas[i];
                        b.y1 > a.y1 && center >= b.x1 && center <= b.x2 + col_gap && !inactive(i)
                    })
                } else {
                    (0..=cur).rev().find(|&i| {
                        let b = self.areas[i];
                        b.y1 < a.y1 && center >= b.x1 - col_gap && center <= b.x2 && !inactive(i)
                    })
                };
                if let Some(t) = target {
                    self.selected = u16::try_from(t).ok();
                }
            }
            _ => {}
        }
    }

    /// The `Items` state of button `i` (LVGL `draw_main`).
    fn btn_state(&self, i: usize, node_state: State) -> State {
        let c = self.ctrl[i];
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
            .style_i32(Part::Main, PropId::PadRow)
            .max(dpi_margin)
            .max(m.style_i32(Part::Main, PropId::PadColumn).max(dpi_margin));
        let def_rect = e.rect_dsc_for_state(node, Part::Items, State::DEFAULT, opa);
        let def_text = e.text_dsc_for_state(node, Part::Items, State::DEFAULT, opa);
        for (i, a) in self.areas.iter().enumerate() {
            let c = self.ctrl[i];
            if c.contains(BtnCtrl::HIDDEN) {
                continue;
            }
            let btn = Area {
                x1: a.x1 + obj.x0,
                y1: a.y1 + obj.y0,
                x2: a.x2 + obj.x0,
                y2: a.y2 + obj.y0,
            };
            let test = btn.to_rect().expand(clip_margin);
            if test.intersection(&clip).is_none() {
                continue;
            }
            let st = self.btn_state(i, node_state);
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
                if btn.x1 == obj.x0 + pad.left {
                    side &= !BorderSide::LEFT.0;
                }
                if btn.x2 == obj.x1 - 1 - pad.right {
                    side &= !BorderSide::RIGHT.0;
                }
                if btn.y1 == obj.y0 + pad.top {
                    side &= !BorderSide::TOP.0;
                }
                if btn.y2 == obj.y1 - 1 - pad.bottom {
                    side &= !BorderSide::BOTTOM.0;
                }
                dsc.border_side = BorderSide(side);
            }
            cx.painter().rect(btn.to_rect(), &dsc);
            let txt = self.btn_text(u16::try_from(i).unwrap_or(u16::MAX)).unwrap_or("");
            let mut l = TextLayout::new(txt, text.font);
            l.letter_space = text.letter_space;
            l.line_space = text.line_space;
            l.max_width = obj.width();
            let size: Size = l.measure();
            let x1 = btn.x1 + (btn.width() - size.w) / 2;
            let y1 = btn.y1 + (btn.height() - size.h) / 2;
            // The text is centered by position; its lines are centered within it.
            text.align = TextAlign::Center;
            cx.draw_text(Rect::new(x1, y1, x1 + size.w + 1, y1 + size.h + 1), txt, &text);
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
        let state = self.btn_state(usize::from(btn), node_state);
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
pub fn create_with(engine: &mut Engine, parent: NodeId, map: MapSrc) -> Result<NodeId, EngineError> {
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
        let m = ButtonMatrix::new(MapSrc::Static(&["a", "b", "\n", "c", ""]));
        assert_eq!(m.btn_count(), 3);
        assert_eq!(m.row_count(), 2);
        assert_eq!(m.btn_text(2), Some("c"));
        assert_eq!(m.btn_text(3), None);
        // The first "" ends the map (LVGL).
        let m = ButtonMatrix::new(MapSrc::Static(&["a", "", "b"]));
        assert_eq!(m.btn_count(), 1);
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
