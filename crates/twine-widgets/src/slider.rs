//! [`Slider`]: a bar with a draggable knob (LVGL `lv_slider`).

use alloc::boxed::Box;

use twine_core::{Point, Rect};
use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, GroupDef,
    InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_style::Part;

use crate::Orientation;
use crate::bar::{Bar, BarMode, IndicGeom, Slot};
use crate::util::{self, Area};

/// Default flags of [`SLIDER_CLASS`] (LVGL `lv_slider_constructor`:
/// `lv_obj_set_scroll_chain_hor(obj, false)`, `lv_obj_set_scrollable(obj, false)`,
/// `lv_obj_set_scroll_on_focus(obj, true)`).
const SLIDER_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .difference(ObjFlags::SCROLL_CHAIN_HOR)
    .union(ObjFlags::SCROLL_ON_FOCUS);

/// The class of [`Slider`]: `"slider"`, parts `Main`, `Indicator` and `Knob`, editable with an
/// encoder and always in the default focus group (LVGL `lv_slider_class`).
pub static SLIDER_CLASS: WidgetClass = WidgetClass::new("slider")
    .parts(&[Part::Main, Part::Indicator, Part::Knob])
    .default_flags(SLIDER_FLAGS)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// The mode of a [`Slider`] (LVGL `lv_slider_mode_t`): the [`BarMode`]s.
pub type SliderMode = BarMode;

/// A slider (LVGL `lv_slider`): a [`Bar`] whose value is set by dragging a knob
/// (`Part::Knob`), with the arrow keys, or by turning an encoder in edit mode.
///
/// - **Pointer**: the value follows the pointer once it moved `scroll_limit` pixels (or
///   jumps to the point of a tap on release), exactly like LVGL; `ValueChanged` is sent only
///   when the value changes, with the new value as [`EventParam::Value`] (handlers may also
///   read the slider: the event is dispatched once the slider is done). In
///   [`SliderMode::Range`] the knob nearer to the pointer is
///   dragged. A right-to-left base direction reverses a horizontal slider.
/// - **Keys**: `Right`/`Up` add 1, `Left`/`Down` subtract 1 (on the left knob when it has the
///   focus in range mode).
/// - **Encoder**: a click enters edit mode (the engine), turning changes the value, the next
///   click leaves edit mode — in range mode it first switches to the left knob.
/// - **Hit area**: LVGL's `ext_click_area` of `dpx(8)` around the slider (with
///   `ObjFlags::ADV_HITTEST` only the knobs react).
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::slider::{self, Slider};
///
/// let mut h = EngineHarness::new(300, 60);
/// let screen = h.screen();
/// let s = slider::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(s, |w: &mut Slider, cx| w.set_value(cx, 30, false));
/// assert_eq!(h.engine().widget::<Slider>(s).unwrap().value(), 30);
/// ```
#[derive(Debug)]
pub struct Slider {
    bar: Bar,
    /// The value being dragged (LVGL `value_to_set`).
    value_to_set: Option<Slot>,
    dragging: bool,
    left_knob_focus: bool,
    pressed_point: Point,
}

impl Default for Slider {
    fn default() -> Self {
        Self::new()
    }
}

impl Slider {
    /// A slider with range 0…100 and value 0.
    #[must_use]
    pub fn new() -> Self {
        let mut bar = Bar::new();
        bar.name = "slider";
        bar.has_knob = true;
        Self {
            bar,
            value_to_set: None,
            dragging: false,
            left_knob_focus: false,
            pressed_point: Point::ZERO,
        }
    }

    /// The underlying bar state (range, mode, orientation, values).
    #[must_use]
    pub fn bar(&self) -> &Bar {
        &self.bar
    }

    /// The value (of the right knob in range mode).
    #[must_use]
    pub fn value(&self) -> i32 {
        self.bar.value()
    }

    /// The left knob's value in range mode, else the minimum (LVGL
    /// `lv_slider_get_left_value`).
    #[must_use]
    pub fn left_value(&self) -> i32 {
        self.bar.start_value()
    }

    /// The minimum.
    #[must_use]
    pub fn min(&self) -> i32 {
        self.bar.min()
    }

    /// The maximum.
    #[must_use]
    pub fn max(&self) -> i32 {
        self.bar.max()
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> SliderMode {
        self.bar.mode()
    }

    /// Whether a knob is being dragged (LVGL `lv_slider_is_dragged`).
    #[must_use]
    pub fn is_dragged(&self) -> bool {
        self.dragging
    }

    /// Whether the left knob has the keypad / encoder focus (range mode).
    #[must_use]
    pub fn left_knob_focused(&self) -> bool {
        self.left_knob_focus
    }

    /// Sets the value (see [`Bar::set_value`]). Idempotent.
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32, anim: bool) {
        self.bar.set_value(cx, v, anim);
    }

    /// Sets the left knob's value in range mode (see [`Bar::set_start_value`]). Idempotent.
    pub fn set_left_value(&mut self, cx: &mut WidgetCx<'_>, v: i32, anim: bool) {
        self.bar.set_start_value(cx, v, anim);
    }

    /// Sets the range (see [`Bar::set_range`]). Idempotent.
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, min: i32, max: i32) {
        self.bar.set_range(cx, min, max);
    }

    /// Sets the mode. Idempotent.
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: SliderMode) {
        self.bar.set_mode(cx, mode);
    }

    /// Sets the orientation. Idempotent.
    pub fn set_orientation(&mut self, cx: &mut WidgetCx<'_>, o: Orientation) {
        if self.bar.orientation() == o {
            return;
        }
        self.bar.set_orientation(cx, o);
        let ext = util::ext_u16(Self::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
        cx.invalidate();
    }

    /// The knob areas: `(right, left)`; the left one only in range mode (LVGL `draw_knob`,
    /// `position_knob`).
    #[must_use]
    pub fn knob_areas(&self, cx: &MeasureCx<'_>) -> (Rect, Option<Rect>) {
        let g = self.bar.geom(cx);
        self.knobs(cx, &g)
    }

    fn knobs(&self, m: &MeasureCx<'_>, g: &IndicGeom) -> (Rect, Option<Rect>) {
        let c = Area::from_rect(m.coords());
        let rev = g.reversed;
        let ia = g.area;
        let neg_sym = self.bar.is_symmetrical() && self.bar.value < 0;
        let pad = m.padding(Part::Knob);
        let (tw, th) = util::transform_wh(m, Part::Knob);
        let place = |pos: i32| {
            let mut k = Area::default();
            if g.hor {
                let size = c.height();
                k.x1 = pos - (size >> 1);
                k.x2 = k.x1 + size - 1;
                k.y1 = c.y1;
                k.y2 = c.y2;
            } else {
                let size = c.width();
                k.y1 = pos - (size >> 1);
                k.y2 = k.y1 + size - 1;
                k.x1 = c.x1;
                k.x2 = c.x2;
            }
            k.x1 -= pad.left + tw;
            k.x2 += pad.right + tw;
            k.y1 -= pad.top + th;
            k.y2 += pad.bottom + th;
            k.to_rect()
        };
        // LV_SLIDER_KNOB_COORD(is_reversed, area): reversed ? x1 : x2;
        // LV_SLIDER_KNOB_COORD_VERTICAL(is_reversed, area): reversed ? y2 : y1.
        let coord = |r: bool| {
            if g.hor {
                if r { ia.x1 } else { ia.x2 }
            } else if r {
                ia.y2
            } else {
                ia.y1
            }
        };
        let right = place(coord(if neg_sym { !rev } else { rev }));
        let left = (self.bar.mode == BarMode::Range).then(|| place(coord(!rev)));
        (right, left)
    }

    /// LVGL `drag_start`: picks the value to drag (the nearer knob in range mode).
    fn drag_start(&mut self, m: &MeasureCx<'_>, p: Point) {
        self.dragging = true;
        if self.bar.mode != BarMode::Range {
            self.value_to_set = Some(Slot::Value);
            return;
        }
        let g = self.bar.geom(m);
        let (r, l) = self.knobs(m, &g);
        let Some(l) = l else {
            self.value_to_set = Some(Slot::Value);
            return;
        };
        let (r, l) = (Area::from_rect(r), Area::from_rect(l));
        let rev = g.reversed;
        let (pr, pl, pc, rc, lc) = if g.hor {
            (
                (!rev && p.x > r.x2) || (rev && p.x < r.x1),
                (!rev && p.x < l.x1) || (rev && p.x > l.x2),
                p.x,
                r.x1 + (r.x2 - r.x1) / 2,
                l.x1 + (l.x2 - l.x1) / 2,
            )
        } else {
            (
                (!rev && p.y < r.y1) || (rev && p.y > r.y2),
                (!rev && p.y > l.y2) || (rev && p.y < l.y1),
                p.y,
                r.y1 + (r.y2 - r.y1) / 2,
                l.y1 + (l.y2 - l.y1) / 2,
            )
        };
        if pr {
            self.value_to_set = Some(Slot::Value);
        } else if pl {
            self.value_to_set = Some(Slot::Start);
        } else if (rc - pc).abs() < (lc - pc).abs() {
            self.value_to_set = Some(Slot::Value);
            self.left_knob_focus = false;
        } else {
            self.value_to_set = Some(Slot::Start);
            self.left_knob_focus = true;
        }
    }

    /// LVGL `update_knob_pos`: sets the dragged value from the pointer.
    fn update_knob_pos(&mut self, cx: &mut EventCx<'_>, check_drag: bool) {
        let e = cx.engine();
        let node = cx.node();
        if util::active_input_kind(e) != Some(InputKind::Pointer) {
            return;
        }
        if !self.dragging && util::ancestor_scrolling(e, node) {
            return;
        }
        let Some(p) = cx.point() else {
            return;
        };
        let m = MeasureCx::new(e, node);
        let hor = self.bar.is_horizontal(&m);
        if check_drag && !self.dragging {
            let ofs = if hor {
                p.x - self.pressed_point.x
            } else {
                p.y - self.pressed_point.y
            };
            if ofs.abs() < e.config().scroll_limit {
                return;
            }
        }
        if self.value_to_set.is_none() {
            self.drag_start(&m, p);
        }
        let g = self.bar.geom(&m);
        let c = Area::from_rect(m.coords());
        let pad = m.padding(Part::Main);
        let (min, max) = (self.bar.min, self.bar.max);
        let range = i64::from(max - min);
        let mut v = if hor {
            let indic_w = c.width() - pad.left - pad.right;
            let rel = if g.reversed {
                (c.x2 - pad.right) - p.x
            } else {
                p.x - (c.x1 + pad.left)
            };
            if indic_w == 0 {
                rel
            } else {
                ((i64::from(rel) * range + i64::from(indic_w / 2)) / i64::from(indic_w)) as i32 + min
            }
        } else {
            let indic_h = c.height() - pad.bottom - pad.top;
            let rel = if g.reversed {
                p.y - (c.y1 + pad.top)
            } else {
                -(p.y - (c.y2 + pad.bottom))
            };
            if indic_h == 0 {
                rel
            } else {
                ((i64::from(rel) * range + i64::from(indic_h / 2)) / i64::from(indic_h)) as i32 + min
            }
        };
        let slot = self.value_to_set.unwrap_or(Slot::Value);
        let (lo, hi) = match slot {
            Slot::Start => (min, self.bar.value),
            Slot::Value => (self.bar.start_value, max),
        };
        v = v.clamp(lo, hi.max(lo));
        let cur = match slot {
            Slot::Value => self.bar.value,
            Slot::Start => self.bar.start_value,
        };
        if cur == v {
            return;
        }
        let mut wcx = cx.widget_cx();
        self.bar.set_slot(&mut wcx, slot, v, false);
        let flag = if hor {
            ObjFlags::SCROLL_CHAIN_VER
        } else {
            ObjFlags::SCROLL_CHAIN_HOR
        };
        wcx.engine_mut().set_flag(node, flag, false);
        cx.post(node, EventCode::ValueChanged, EventParam::Value(v));
    }

    /// Changes the focused knob's value by `d` for a key (LVGL `LV_EVENT_KEY`); sends
    /// `ValueChanged` when it changed.
    fn step(&mut self, cx: &mut EventCx<'_>, d: i32) {
        let before = (self.bar.value, self.bar.start_value);
        let mut wcx = cx.widget_cx();
        if self.left_knob_focus && self.bar.mode == BarMode::Range {
            let v = self.bar.start_value + d;
            self.bar.set_start_value(&mut wcx, v, true);
        } else {
            let v = self.bar.value + d;
            self.bar.set_value(&mut wcx, v, true);
        }
        if (self.bar.value, self.bar.start_value) != before {
            let node = cx.node();
            let v = if self.left_knob_focus && self.bar.mode == BarMode::Range {
                self.bar.start_value
            } else {
                self.bar.value
            };
            cx.post(node, EventCode::ValueChanged, EventParam::Value(v));
        }
    }

    /// Updates the scroll chain flags for the orientation (LVGL `LV_EVENT_SIZE_CHANGED`).
    fn update_chain(&self, e: &mut Engine, node: NodeId) {
        let hor = self.bar.is_horizontal(&MeasureCx::new(e, node));
        e.set_flag(node, ObjFlags::SCROLL_CHAIN_VER, hor);
        e.set_flag(node, ObjFlags::SCROLL_CHAIN_HOR, !hor);
    }

    /// LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE` of the slider: the knob's reach (half the
    /// slider's thickness plus the knob paddings and its own extra draw size).
    fn ext_for(m: &MeasureCx<'_>) -> i32 {
        let pad = m.padding(Part::Knob);
        let (tw, th) = util::transform_wh(m, Part::Knob);
        let c = m.coords();
        let mut knob = (c.width() + 2 * tw).min(c.height() + 2 * th) >> 1;
        knob += pad.left.max(pad.right).max(pad.top.max(pad.bottom));
        knob += 2; // for rounding errors
        knob += util::part_ext_draw(m, Part::Knob);
        Bar::ext_for(m).max(knob)
    }
}

/// Creates a slider as the last child of `parent` (LVGL `lv_slider_create`), 260 × 13 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Slider::new()))
}

impl Widget for Slider {
    fn class(&self) -> &'static WidgetClass {
        &SLIDER_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut()
            .set_size(id, crate::bar::BAR_DEFAULT_WIDTH, crate::bar::BAR_DEFAULT_HEIGHT);
        let ext = util::ext_u16(Self::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
        self.update_chain(cx.engine_mut(), id);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.bar.draw_indicator(cx);
        let m = MeasureCx::new(cx.engine(), cx.node());
        let (right, left) = self.knob_areas(&m);
        cx.draw_rect_style(right, Part::Knob);
        if let Some(l) = left {
            cx.draw_rect_style(l, Part::Knob);
        }
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(Self::ext_for(cx))
    }

    /// LVGL `lv_obj_set_ext_click_area(obj, LV_DPX(8))`; with `ADV_HITTEST` only the knobs
    /// (LVGL `LV_EVENT_HIT_TEST`).
    fn hit_test(&self, cx: &MeasureCx<'_>, p: Point) -> bool {
        let ext = util::dpx(cx.engine(), cx.node(), 8);
        if cx.flags().contains(ObjFlags::ADV_HITTEST) {
            let (r, l) = self.knob_areas(cx);
            return r.expand(ext).contains(p) || l.is_some_and(|l| l.expand(ext).contains(p));
        }
        cx.coords().expand(ext).contains(p)
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        let node = cx.node();
        match ev.code {
            EventCode::Pressed => {
                self.pressed_point = cx.point().unwrap_or(Point::ZERO);
            }
            EventCode::Pressing => self.update_knob_pos(cx, true),
            EventCode::Released | EventCode::PressLost => {
                self.update_knob_pos(cx, false);
                self.dragging = false;
                self.value_to_set = None;
                cx.widget_cx().invalidate();
                let e = cx.engine_mut();
                match util::active_input_kind(e) {
                    Some(InputKind::Encoder) => {
                        if let Some(g) = e.group_of(node).filter(|g| e.group_editing(*g)) {
                            if self.bar.mode == BarMode::Range && !self.left_knob_focus {
                                self.left_knob_focus = true;
                            } else {
                                self.left_knob_focus = false;
                                e.set_editing(g, false);
                            }
                        }
                    }
                    Some(InputKind::Pointer) => {
                        let hor = self.bar.is_horizontal(&MeasureCx::new(e, node));
                        let f = if hor {
                            ObjFlags::SCROLL_CHAIN_VER
                        } else {
                            ObjFlags::SCROLL_CHAIN_HOR
                        };
                        e.set_flag(node, f, true);
                    }
                    _ => {}
                }
            }
            EventCode::Focused => {
                if matches!(
                    util::active_input_kind(cx.engine()),
                    Some(InputKind::Encoder | InputKind::Keypad)
                ) {
                    self.left_knob_focus = false;
                }
            }
            EventCode::SizeChanged | EventCode::StyleChanged => {
                self.update_chain(cx.engine_mut(), node);
                let mut wcx = cx.widget_cx();
                let ext = util::ext_u16(Self::ext_for(&wcx.measure()));
                if wcx.refresh_ext_draw_with(ext) {
                    wcx.invalidate();
                }
            }
            EventCode::Key => match ev.key() {
                Some(Key::Right | Key::Up) => self.step(cx, 1),
                Some(Key::Left | Key::Down) => self.step(cx, -1),
                _ => {}
            },
            // Encoders turn in edit mode as arrow keys (above); `Rotary` comes from other
            // sources (e.g. a mouse wheel forwarded by the application).
            EventCode::Rotary => {
                if let EventParam::Rotary(d) = ev.param {
                    self.step(cx, d);
                }
            }
            _ => {}
        }
        EventResult::Continue
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        self.bar.apply_anim(cx, id, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_matches_lvgl() {
        assert_eq!(SLIDER_CLASS.editable, Editable::True);
        assert_eq!(SLIDER_CLASS.group_def, GroupDef::True);
        assert!(!SLIDER_FLAGS.contains(ObjFlags::SCROLL_CHAIN_HOR));
        assert!(SLIDER_FLAGS.contains(ObjFlags::SCROLL_CHAIN_VER));
        assert!(!SLIDER_FLAGS.contains(ObjFlags::SCROLLABLE));
    }
}
