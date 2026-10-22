//! [`Arc`]: an arc slider with a knob (LVGL `lv_arc`).
//!
//! Angles are [`Angle`]s (0.1°), clockwise from 3 o'clock like LVGL's. The widget keeps them in
//! tenths of a degree internally, so values map to angles like LVGL built with
//! `LV_USE_FLOAT` (the integer build rounds every angle to a whole degree).

use alloc::boxed::Box;

use twine_core::math::{atan2, cos, sin};
use twine_core::{Angle, AngularSpeed, Duration, Point, Rect};
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam,
    EventResult, InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Repeat, Widget, WidgetClass,
    WidgetCx,
};
use twine_style::{Part, PropId};

use crate::log_set;
use crate::util::{self, log_value};

/// A full turn in tenths of a degree.
const TURN: i32 = 3600;
/// `AnimProp::Custom` id of the drag clock (elapsed milliseconds while dragging).
const ANIM_CLOCK: u16 = 0x7F00;
/// Period of the drag clock animation (its value wraps every 1024 ms).
const CLOCK_WRAP: i32 = 1024;

/// How the indicator of an [`Arc`] grows with the value (LVGL `lv_arc_mode_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ArcMode {
    /// Clockwise from the background's start angle.
    #[default]
    Normal,
    /// From the middle of the background arc towards either end.
    Symmetrical,
    /// Counter-clockwise from the background's end angle.
    Reverse,
}

/// Default flags of [`ARC_CLASS`] (LVGL `lv_arc_constructor`: `lv_obj_set_clickable(obj,
/// true)`, `lv_obj_set_scroll_chain(obj, false)`, `lv_obj_set_scrollable(obj, false)`).
const ARC_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .difference(ObjFlags::SCROLL_CHAIN)
    .union(ObjFlags::CLICKABLE);

/// The class of [`Arc`]: `"arc"`, parts `Main` (background arc), `Indicator` and `Knob`,
/// editable with an encoder (LVGL `lv_arc_class`; not in the default group, like LVGL).
pub static ARC_CLASS: WidgetClass = WidgetClass::new("arc")
    .parts(&[Part::Main, Part::Indicator, Part::Knob])
    .default_flags(ARC_FLAGS)
    .editable(Editable::True);

/// LVGL's default object size (`lv_obj_class.width_def = LV_DPI_DEF`), which arcs inherit.
pub const ARC_DEFAULT_SIZE: i32 = util::DPI_DEF;

/// Elapsed milliseconds while dragging, from an engine animation (the engine does not expose
/// its clock to widgets).
#[derive(Clone, Copy, Debug, Default)]
struct DragClock {
    id: Option<AnimId>,
    wraps: i64,
    last: i32,
}

impl DragClock {
    fn now_ms(&self) -> i64 {
        self.wraps
            .saturating_mul(i64::from(CLOCK_WRAP))
            .saturating_add(i64::from(self.last))
    }

    fn start(&mut self, cx: &mut WidgetCx<'_>) {
        if self.id.is_some() {
            return;
        }
        let node = cx.node();
        let a = Anim::new(0, CLOCK_WRAP)
            .duration(Duration::ms(CLOCK_WRAP.unsigned_abs().into()))
            .repeat(Repeat::Forever)
            // A clock, not motion: the motion preference must not stop it.
            .essential();
        self.id = Some(cx.engine_mut().anim_start(node, AnimProp::Custom(ANIM_CLOCK), a));
    }

    fn stop(&mut self, e: &mut Engine) {
        if let Some(id) = self.id.take() {
            e.anim_stop(id);
        }
    }

    fn tick(&mut self, v: i32) {
        if v < self.last {
            self.wraps = self.wraps.saturating_add(1);
        }
        self.last = v;
    }
}

/// An arc slider (LVGL `lv_arc`): a background arc (`Main`: `ArcWidth`, `ArcColor`,
/// `ArcRounded`), an indicator arc (`Indicator`) showing the value and a knob (`Knob`) at its
/// end.
///
/// - **Angles**: the background spans [`bg_angle_start`](Self::bg_angle_start) …
///   [`bg_angle_end`](Self::bg_angle_end) (default 135° … 45°, clockwise through 270°), the
///   whole drawing turned by [`rotation`](Self::rotation). The value (0…100 by default) sets
///   the indicator angles as LVGL's `value_update` does; [`ArcMode::Reverse`] grows
///   counter-clockwise from the end, [`ArcMode::Symmetrical`] from the middle.
/// - **Dragging**: pressing the ring and moving sets the value from the pointer's angle. The
///   angle moves at most [`change_rate`](Self::change_rate) (default 720 °/s)
///   and never jumps across the gap between the ends. `ValueChanged` (with the value as
///   [`EventParam::Value`]) is sent on each change.
/// - **Keys** / encoder: ±1 per step.
/// - **Invalidation**: a value change redraws only the bounding boxes of the swept ring
///   segments and the knob's old and new areas (LVGL `inv_arc_area`).
/// - **Hit area**: the bounding box plus `LV_DPI_DEF / 10` (LVGL `ext_click_area`); with
///   `ObjFlags::ADV_HITTEST` only the ring.
///
/// Default size: 130 × 130 px.
///
/// ```
/// use twine_core::Angle;
/// use twine_testing::EngineHarness;
/// use twine_widgets::arc::{self, Arc};
///
/// let mut h = EngineHarness::new(160, 160);
/// let screen = h.screen();
/// let a = arc::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(a, |w: &mut Arc, cx| w.set_value(cx, 50));
/// let w = h.engine().widget::<Arc>(a).unwrap();
/// assert_eq!(w.angle_end(), Angle::deg(270)); // halfway through 135° … 405°
/// ```
#[derive(Debug)]
pub struct Arc {
    rotation: i32,
    indic_start: i32,
    indic_end: i32,
    bg_start: i32,
    bg_end: i32,
    min: i32,
    max: i32,
    /// `None` until a value is set (LVGL `VALUE_UNSET`).
    value: Option<i32>,
    mode: ArcMode,
    dragging: bool,
    chg_rate: u16,
    knob_offset: i32,
    /// Last angle set from the value (LVGL `last_angle`).
    last_angle: i32,
    /// Time (drag clock ms) of the last applied drag step (LVGL `last_tick`).
    last_tick: i64,
    /// The last press inside the background angles was nearer to the minimum end.
    min_close: bool,
    /// The last press was inside the background angles (LVGL `in_out`).
    inside: bool,
    clock: DragClock,
    pub(crate) name: &'static str,
}

impl Default for Arc {
    fn default() -> Self {
        Self::new()
    }
}

/// LVGL `lv_map`: maps `x` from `min_in..=max_in` to `min_out..=max_out` (clamped).
///
/// Exact for every `i32` range whose other span is an angle (below 2^14): the product of the
/// spans stays below 2^46 in [`twine_core::math::map`].
fn map(x: i32, min_in: i32, max_in: i32, min_out: i32, max_out: i32) -> i32 {
    if max_in >= min_in && x >= max_in {
        return max_out;
    }
    if max_in >= min_in && x <= min_in {
        return min_out;
    }
    if max_in <= min_in && x <= max_in {
        return max_out;
    }
    if max_in <= min_in && x >= min_in {
        return min_out;
    }
    // `max_in != min_in` here (equal bounds return above), where the core `map` computes the
    // same truncated `(x − min_in)·Δout / Δin + min_out` without overflow.
    twine_core::math::map(x, min_in, max_in, min_out, max_out)
}

/// `0 ..= 360°`: "start = 0, end = 360 means a full circle" (LVGL `lv_arc_set_angles`):
/// angles above 360° step down by whole turns into `(0, 360°]`, negative ones up into
/// `[0, 360°)`.
fn norm_incl(a: i32) -> i32 {
    if a > TURN {
        // SAFE-ARITH: `a > 3600`, so `a − 1` does not overflow and the remainder is in
        // `0..3600`.
        (a.wrapping_sub(1) % TURN).wrapping_add(1)
    } else if a < 0 {
        a.rem_euclid(TURN)
    } else {
        a
    }
}

/// `0 .. 360°`.
fn norm(a: i32) -> i32 {
    a.rem_euclid(TURN)
}

/// Bounding box of the ring segment from `start` clockwise to `end` (0.1°) of outer radius
/// `r` and width `w` around `c` (LVGL `lv_draw_arc_get_area`), one pixel larger for
/// anti-aliasing.
pub(crate) fn arc_segment_area(c: Point, r: i32, w: i32, start: i32, end: i32, rounded: bool) -> Rect {
    let outer = Rect::new(
        c.x.saturating_sub(r),
        c.y.saturating_sub(r),
        c.x.saturating_add(r),
        c.y.saturating_add(r),
    )
    .expand(1);
    let s = norm(start);
    let e = norm(end);
    // SAFE-ARITH: `s, e ∈ 0..3600` (normalized): the unwrapped end is below 7200 and the
    // sweep `e − s` is in `0..7200`.
    let e = if e <= s { e.wrapping_add(TURN) } else { e };
    let full = e.wrapping_sub(s) >= TURN
        || end.checked_add(TURN) == Some(start)
        || start.checked_add(TURN) == Some(end);
    if r <= w || full {
        return outer;
    }
    let ri = r.saturating_sub(w);
    let mut b: Option<Rect> = None;
    let mut add = |a: i32, rad: i32| {
        let x = c.x.saturating_add(util::trig_scale(rad, cos(Angle::deci_deg(a))));
        let y = c.y.saturating_add(util::trig_scale(rad, sin(Angle::deci_deg(a))));
        let p = Rect::new(
            x.saturating_sub(1),
            y.saturating_sub(1),
            x.saturating_add(2),
            y.saturating_add(2),
        );
        b = Some(b.map_or(p, |b| b.union(&p)));
    };
    for a in [s, e] {
        add(a, r);
        add(a, ri);
    }
    // The quarter points (multiples of 90°) strictly inside the sweep `s .. e` (`e < 7200`).
    for q in (900..e).step_by(900).skip_while(|&q| q <= s) {
        add(q, r);
    }
    let extra = if rounded { (w / 2).saturating_add(1) } else { 0 };
    b.map_or(outer, |b| b.expand(extra).intersection(&outer).unwrap_or(b))
}

impl Arc {
    /// An arc with LVGL's defaults: background 135° … 45°, indicator 135° … 270°, range
    /// 0…100, no value yet, change rate 720°/s.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rotation: 0,
            indic_start: 1350,
            indic_end: 2700,
            bg_start: 1350,
            bg_end: 450,
            min: 0,
            max: 100,
            value: None,
            mode: ArcMode::Normal,
            dragging: false,
            chg_rate: 720,
            knob_offset: 0,
            last_angle: 2700,
            last_tick: 0,
            min_close: true,
            inside: false,
            clock: DragClock {
                id: None,
                wraps: 0,
                last: 0,
            },
            name: "arc",
        }
    }

    /// The value (the minimum while none was set).
    #[must_use]
    pub fn value(&self) -> i32 {
        self.value.unwrap_or(self.min)
    }

    /// The minimum.
    #[must_use]
    pub fn min(&self) -> i32 {
        self.min
    }

    /// The maximum.
    #[must_use]
    pub fn max(&self) -> i32 {
        self.max
    }

    /// The indicator's start angle.
    #[must_use]
    pub fn angle_start(&self) -> Angle {
        Angle::deci_deg(self.indic_start)
    }

    /// The indicator's end angle.
    #[must_use]
    pub fn angle_end(&self) -> Angle {
        Angle::deci_deg(self.indic_end)
    }

    /// The background's start angle.
    #[must_use]
    pub fn bg_angle_start(&self) -> Angle {
        Angle::deci_deg(self.bg_start)
    }

    /// The background's end angle.
    #[must_use]
    pub fn bg_angle_end(&self) -> Angle {
        Angle::deci_deg(self.bg_end)
    }

    /// The rotation of the whole arc.
    #[must_use]
    pub fn rotation(&self) -> Angle {
        Angle::deci_deg(self.rotation)
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> ArcMode {
        self.mode
    }

    /// The maximal drag speed.
    #[must_use]
    pub fn change_rate(&self) -> AngularSpeed {
        AngularSpeed::deg_per_s(self.chg_rate)
    }

    /// The knob's angular offset from the indicator's end.
    #[must_use]
    pub fn knob_offset(&self) -> Angle {
        Angle::deci_deg(self.knob_offset)
    }

    /// Whether the arc is being dragged.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// The end of the background arc unwrapped past its start (`bg_end + 360°` when it is
    /// smaller).
    fn bg_end_unwrapped(&self) -> i32 {
        if self.bg_end < self.bg_start {
            // SAFE-ARITH: `bg_end ∈ 0..=3600` (normalized by `norm_incl`).
            self.bg_end.wrapping_add(TURN)
        } else {
            self.bg_end
        }
    }

    /// The middle of the background arc (normalized angles: no overflow).
    fn bg_mid(&self) -> i32 {
        util::mid(self.bg_start, self.bg_end_unwrapped())
    }

    /// Sets the value, clamped to the range (LVGL `lv_arc_set_value`). Idempotent.
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        if self.value == Some(v) {
            return;
        }
        let nv = v.clamp(self.min.min(self.max), self.max.max(self.min));
        if self.value == Some(nv) {
            return;
        }
        log_set(self.name, cx.node(), "value");
        self.value = Some(nv);
        log_value(self.name, nv);
        self.value_update(cx);
    }

    /// Sets the range (LVGL `lv_arc_set_range`); the value is clamped into it. Idempotent.
    ///
    /// Any pair of `i32`s is a valid range, up to `i32::MIN ..= i32::MAX`: values map to
    /// angles (and dragged angles back to values) in `i64`, exactly; the symmetrical mode's
    /// middle is `(min + max) / 2` rounded toward zero. Never panics.
    ///
    /// ```
    /// use twine_core::Angle;
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::arc::{self, Arc};
    ///
    /// let mut h = EngineHarness::new(160, 160);
    /// let screen = h.screen();
    /// let a = arc::create(h.engine_mut(), screen).unwrap();
    /// h.engine_mut().with_widget_mut(a, |w: &mut Arc, cx| {
    ///     w.set_range(cx, i32::MIN, i32::MAX);
    ///     w.set_value(cx, 0); // the middle: 135° + 270° / 2
    /// });
    /// assert_eq!(h.engine().widget::<Arc>(a).unwrap().angle_end(), Angle::deg(270));
    /// ```
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, min: i32, max: i32) {
        if self.min == min && self.max == max {
            return;
        }
        log_set(self.name, cx.node(), "range");
        self.min = min;
        self.max = max;
        if let Some(v) = self.value {
            self.value = Some(v.clamp(min.min(max), max.max(min)));
        }
        self.value_update(cx);
    }

    /// Sets the indicator's angles directly (LVGL `lv_arc_set_angles`; values are normalized
    /// to 0…360°). Idempotent.
    pub fn set_angles(&mut self, cx: &mut WidgetCx<'_>, start: Angle, end: Angle) {
        self.set_indic(cx, start.as_deci_deg(), end.as_deci_deg());
    }

    /// Sets the background's angles (LVGL `lv_arc_set_bg_angles`); the indicator follows the
    /// value. Idempotent.
    pub fn set_bg_angles(&mut self, cx: &mut WidgetCx<'_>, start: Angle, end: Angle) {
        let (start, end) = (norm_incl(start.as_deci_deg()), norm_incl(end.as_deci_deg()));
        if (start, end) == (self.bg_start, self.bg_end) {
            return;
        }
        log_set(self.name, cx.node(), "bg_angles");
        let (old_start, old_end) = (self.bg_start, self.bg_end);
        self.bg_start = start;
        self.bg_end = end;
        self.value_update(cx);
        self.invalidate_sweep(cx, (old_start, old_end), (start, end), Part::Main);
    }

    /// Turns the whole arc (LVGL `lv_arc_set_rotation`; normalized to 0…360°). Idempotent.
    pub fn set_rotation(&mut self, cx: &mut WidgetCx<'_>, rotation: Angle) {
        let r = norm(rotation.as_deci_deg());
        if r == self.rotation {
            return;
        }
        log_set(self.name, cx.node(), "rotation");
        self.rotation = r;
        cx.invalidate_for("arc.rotation");
    }

    /// Sets the mode (LVGL `lv_arc_set_mode`): the indicator is re-anchored and the value
    /// re-applied. Idempotent.
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: ArcMode) {
        if self.mode == mode {
            return;
        }
        log_set(self.name, cx.node(), "mode");
        self.mode = mode;
        let bg_end = self.bg_end_unwrapped();
        match mode {
            ArcMode::Symmetrical => {
                let mid = util::mid(self.bg_start, bg_end);
                self.set_indic(cx, mid, mid);
            }
            ArcMode::Reverse => {
                let s = self.indic_start;
                self.set_indic(cx, s, self.bg_end);
            }
            ArcMode::Normal => {
                let e = self.indic_end;
                self.set_indic(cx, self.bg_start, e);
            }
        }
        self.value_update(cx);
    }

    /// Limits how fast dragging may turn the value (LVGL `lv_arc_set_change_rate`).
    /// Idempotent.
    pub fn set_change_rate(&mut self, cx: &mut WidgetCx<'_>, rate: AngularSpeed) {
        let rate = rate.as_deg_per_s();
        if self.chg_rate == rate {
            return;
        }
        log_set(self.name, cx.node(), "change_rate");
        self.chg_rate = rate;
    }

    /// Moves the knob by `offset` along the arc (LVGL `lv_arc_set_knob_offset`). Idempotent.
    pub fn set_knob_offset(&mut self, cx: &mut WidgetCx<'_>, offset: Angle) {
        if self.knob_offset == offset.as_deci_deg() {
            return;
        }
        log_set(self.name, cx.node(), "knob_offset");
        let old = self.knob_inv_area(&cx.measure());
        self.knob_offset = offset.as_deci_deg();
        let new = self.knob_inv_area(&cx.measure());
        for a in [old, new].into_iter().flatten() {
            cx.invalidate_area(a);
        }
    }

    /// LVGL `value_update`: the indicator angles for the value and mode.
    fn value_update(&mut self, cx: &mut WidgetCx<'_>) {
        let Some(value) = self.value else {
            return;
        };
        let bg_end = self.bg_end_unwrapped();
        let angle = match self.mode {
            ArcMode::Symmetrical => {
                let mid = util::mid(self.bg_start, bg_end);
                let range_mid = util::mid(self.min, self.max);
                if value < range_mid {
                    let a = map(value, self.min, range_mid, self.bg_start, mid);
                    self.set_indic(cx, a, mid);
                    a
                } else {
                    let a = map(value, range_mid, self.max, mid, bg_end);
                    self.set_indic(cx, mid, a);
                    a
                }
            }
            ArcMode::Reverse => {
                let a = map(value, self.min, self.max, bg_end, self.bg_start);
                self.set_indic(cx, a, self.bg_end);
                a
            }
            ArcMode::Normal => {
                let a = map(value, self.min, self.max, self.bg_start, bg_end);
                self.set_indic(cx, self.bg_start, a);
                a
            }
        };
        self.last_angle = angle;
    }

    /// LVGL `lv_arc_set_angles` with its invalidation (the swept segments and the knob).
    pub(crate) fn set_indic(&mut self, cx: &mut WidgetCx<'_>, start: i32, end: i32) {
        let (start, end) = (norm_incl(start), norm_incl(end));
        if (start, end) == (self.indic_start, self.indic_end) {
            return;
        }
        let m = cx.measure();
        let old_knob = self.knob_inv_area(&m);
        let (old_start, old_end) = (self.indic_start, self.indic_end);
        self.indic_start = start;
        self.indic_end = end;
        self.invalidate_sweep(cx, (old_start, old_end), (start, end), Part::Indicator);
        let m = cx.measure();
        let new_knob = self.knob_inv_area(&m);
        for a in [old_knob, new_knob].into_iter().flatten() {
            cx.invalidate_area(a);
        }
    }

    /// The invalidation of `lv_arc_set_angles` / `lv_arc_set_bg_angles`: the segments swept by
    /// the start and the end angle (the whole arc when a sweep exceeds 180°).
    fn invalidate_sweep(&self, cx: &mut WidgetCx<'_>, old: (i32, i32), new: (i32, i32), part: Part) {
        if cx.coords().is_empty() {
            cx.invalidate_for("arc.angles");
            return;
        }
        let (old_start, old_end) = old;
        let (start, end) = new;
        // Angles are normalized (`0..=3600`): every difference below is within ±7200.
        let wrap = |d: i32| if d < 0 { TURN.saturating_add(d) } else { d };
        let so = wrap(end.saturating_sub(old_start));
        let sn = wrap(end.saturating_sub(start));
        if sn.saturating_sub(so).saturating_abs() > TURN / 2 {
            cx.invalidate_for("arc.angles");
        } else if sn < so {
            self.inv_arc_area(cx, old_start, start, part);
        } else if so < sn {
            self.inv_arc_area(cx, start, old_start, part);
        }
        let eo = wrap(old_end.saturating_sub(old_start));
        let en = wrap(end.saturating_sub(old_start));
        if en.saturating_sub(eo).saturating_abs() > TURN / 2 {
            cx.invalidate_for("arc.angles");
        } else if en < eo {
            self.inv_arc_area(cx, end, old_end, part);
        } else if eo < en {
            self.inv_arc_area(cx, old_end, end, part);
        }
    }

    /// LVGL `inv_arc_area`: invalidates the box of the ring segment `start` … `end` of `part`.
    fn inv_arc_area(&self, cx: &mut WidgetCx<'_>, start: i32, end: i32, part: Part) {
        if start == end {
            return;
        }
        let m = cx.measure();
        let (c, mut r) = self.center(&m);
        if part == Part::Indicator {
            r = r.saturating_sub(Self::indicator_max_pad(&m));
        }
        if r <= 0 {
            cx.invalidate_for("arc.angles");
            return;
        }
        let w = m.style_i32(part, PropId::ArcWidth);
        let rounded = m.style(part, PropId::ArcRounded).as_bool().unwrap_or(false);
        let (start, end) = (
            start.saturating_add(self.rotation),
            end.saturating_add(self.rotation),
        );
        let area = arc_segment_area(c, r, w, start, end, rounded);
        cx.invalidate_area(area);
    }

    /// The center and radius of the background arc (LVGL `get_center`).
    #[must_use]
    pub fn center(&self, cx: &MeasureCx<'_>) -> (Point, i32) {
        let c = cx.coords();
        let pad = cx.padding(Part::Main);
        let w = c.width().saturating_sub(pad.horizontal());
        let h = c.height().saturating_sub(pad.vertical());
        let r = w.min(h) / 2;
        let at = |o: i32, p: i32| o.saturating_add(r).saturating_add(p);
        (Point::new(at(c.x0, pad.left), at(c.y0, pad.top)), r)
    }

    fn indicator_max_pad(m: &MeasureCx<'_>) -> i32 {
        let p = m.padding(Part::Indicator);
        p.left.max(p.right).max(p.top.max(p.bottom))
    }

    /// The knob's angle (LVGL `get_angle`). Every term is a normalized angle (`rotation <
    /// 3600`, the others `≤ 7200`), so the sum stays below 2^14.
    fn knob_angle(&self) -> i32 {
        let along = match self.mode {
            ArcMode::Normal => self.indic_end,
            ArcMode::Reverse => self.indic_start,
            ArcMode::Symmetrical => {
                let indic_end = if self.indic_end < self.indic_start {
                    // SAFE-ARITH: `indic_end ∈ 0..=3600` (normalized).
                    self.indic_end.wrapping_add(TURN)
                } else {
                    self.indic_end
                };
                let mid = self.bg_mid();
                if self.indic_start < mid {
                    self.indic_start
                } else if indic_end > mid {
                    self.indic_end
                } else {
                    mid
                }
            }
        };
        // SAFE-ARITH: see above.
        self.rotation.wrapping_add(along)
    }

    /// The knob's area (LVGL `get_knob_area`), centered on the indicator ring.
    #[must_use]
    pub fn knob_area(&self, cx: &MeasureCx<'_>) -> Rect {
        let (c, r) = self.center(cx);
        let w = cx.style_i32(Part::Indicator, PropId::ArcWidth);
        let half = w / 2;
        let r = r.saturating_sub(half).saturating_sub(Self::indicator_max_pad(cx));
        // SAFE-ARITH: `knob_angle() < 2^14` and the normalized offset is below 3600.
        let a = Angle::deci_deg(self.knob_angle().wrapping_add(norm(self.knob_offset)));
        let kx = util::trig_scale(r, cos(a));
        let ky = util::trig_scale(r, sin(a));
        let (kx, ky) = (c.x.saturating_add(kx), c.y.saturating_add(ky));
        let p = cx.padding(Part::Knob);
        // The ring's center line runs through pixel corners: an even-sized knob around
        // `center + (kx, ky)` sits exactly on it.
        let lo = |k: i32, pad: i32| k.saturating_sub(pad).saturating_sub(half);
        let hi = |k: i32, pad: i32| k.saturating_add(pad).saturating_add(half);
        Rect::new(lo(kx, p.left), lo(ky, p.top), hi(kx, p.right), hi(ky, p.bottom))
    }

    /// LVGL `knob_get_extra_size`: the knob's shadow or outline reach.
    fn knob_extra(m: &MeasureCx<'_>) -> i32 {
        let k = Part::Knob;
        let shadow = m
            .style_i32(k, PropId::ShadowWidth)
            .saturating_add(m.style_i32(k, PropId::ShadowSpread))
            .saturating_add(m.style_i32(k, PropId::ShadowOffsetX).saturating_abs())
            .saturating_add(m.style_i32(k, PropId::ShadowOffsetY).saturating_abs());
        let outline = m
            .style_i32(k, PropId::OutlineWidth)
            .saturating_add(m.style_i32(k, PropId::OutlineOffset));
        shadow.max(outline)
    }

    /// LVGL `get_knob_inv_area`: the knob's area with its extra size, `None` when the knob is
    /// invisible.
    fn knob_inv_area(&self, m: &MeasureCx<'_>) -> Option<Rect> {
        let d = m.rect_dsc(Part::Knob).base;
        let img = m
            .style(Part::Knob, PropId::BgImage)
            .get::<&'static twine_image::ImageSource>()
            .is_some();
        let visible = d.bg_opa.raw() > twine_core::Opa::MIN.raw()
            || img
            || (d.border_opa.raw() > twine_core::Opa::MIN.raw() && d.border_width > 0)
            || (d.outline_opa.raw() > twine_core::Opa::MIN.raw() && d.outline_width > 0)
            || (d.shadow.opa.raw() > twine_core::Opa::MIN.raw() && d.shadow.width > 0);
        if !visible || m.coords().is_empty() {
            return None;
        }
        Some(
            self.knob_area(m)
                .expand(Self::knob_extra(m).max(0).saturating_add(1)),
        )
    }

    /// LVGL `lv_arc_angle_within_bg_bounds`: whether `angle` (relative to the background
    /// start) is on the background arc or within `tolerance` of an end; if so, whether it is
    /// nearer to the minimum end and whether it is inside the angles.
    fn bg_bounds(&self, angle: i32, tolerance: i32) -> Option<(bool, bool)> {
        if self.bg_end == self.bg_start {
            return None;
        }
        // SAFE-ARITH: both ends are normalized (`0..=3600`).
        let mut bounds = norm(self.bg_end.wrapping_sub(self.bg_start));
        if bounds == 0 {
            bounds = TURN;
        }
        if angle <= bounds {
            return Some((angle < bounds / 2, true));
        }
        // SAFE-ARITH: `bounds ∈ 1..=3600`.
        if TURN.wrapping_sub(bounds) <= tolerance {
            return Some((true, true));
        }
        if TURN.saturating_sub(angle) <= tolerance {
            return Some((true, false));
        }
        if angle <= bounds.saturating_add(tolerance) {
            return Some((false, false));
        }
        None
    }

    /// [`bg_bounds`](Self::bg_bounds) remembering the result for the next drag step.
    fn within_bg_bounds(&mut self, angle: i32, tolerance: i32) -> bool {
        match self.bg_bounds(angle, tolerance) {
            Some((min_close, inside)) => {
                self.min_close = min_close;
                self.inside = inside;
                true
            }
            None => false,
        }
    }

    /// LVGL `LV_EVENT_PRESSING`: the value from the pointer's angle.
    fn pressing(&mut self, cx: &mut EventCx<'_>) {
        let node = cx.node();
        if util::active_input_kind(cx.engine()) != Some(InputKind::Pointer) {
            return;
        }
        let Some(pt) = cx.point() else {
            return;
        };
        let m = MeasureCx::new(cx.engine(), node);
        let (c, mut r) = self.center(&m);
        // Pixel centers relative to the center (a pixel corner).
        let (px, py) = (util::center_offset2(pt.x, c.x), util::center_offset2(pt.y, c.y));
        if !self.dragging {
            let indic_w = m.style_i32(Part::Indicator, PropId::ArcWidth);
            let ri = r.saturating_sub(indic_w);
            let ri = ri.saturating_sub(if m.flags().contains(ObjFlags::ADV_HITTEST) {
                indic_w
            } else {
                (ri / 4).max(indic_w)
            });
            let ri = ri.max(1);
            if util::hyp2(px, py) > util::diam2(ri) {
                self.dragging = true;
                self.clock.start(&mut cx.widget_cx());
                self.last_tick = self.clock.now_ms();
            }
        }
        if !self.dragging {
            return;
        }
        let bg_end = self.bg_end_unwrapped();
        // Angles below are normalized (`bg_start`, `bg_end ≤ 7200`, `last_angle` within the
        // background arc, `rotation < 3600`): their sums and differences stay within ±2^15.
        let rel = atan2(py, px)
            .as_deci_deg()
            .saturating_sub(self.rotation)
            .saturating_sub(self.bg_start);
        let mut angle = norm(rel);
        r = r.max(1);
        let circumference = util::circumference(r);
        let tolerance = util::per_circumference(TURN, util::dpx(cx.engine(), node, 20), circumference);
        let min_close_prev = self.min_close;
        if !self.within_bg_bounds(angle, tolerance) {
            return;
        }
        let deg_range = bg_end.saturating_sub(self.bg_start);
        let last_rel = self.last_angle.saturating_sub(self.bg_start);
        let delta = angle.saturating_sub(last_rel);
        // No big jumps (more than 280°): prefer the end that was nearer on the last press.
        if delta.saturating_abs() > 2800 {
            angle = if self.min_close { 0 } else { deg_range };
        } else if !self.inside {
            angle = if self.min_close {
                deg_range.saturating_neg()
            } else {
                deg_range
            };
        }
        // No jump from one end to the other through the gap without releasing.
        if min_close_prev && !self.min_close && !self.inside && delta.saturating_abs() > 2800 {
            angle = 0;
            self.min_close = min_close_prev;
        } else if !min_close_prev
            && self.min_close
            && !self.inside
            && TURN.saturating_sub(delta.saturating_abs()) > 2800
        {
            angle = deg_range;
            self.min_close = min_close_prev;
        }
        // The change rate limit (0.1° per ms: chg_rate [°/s] · Δt [ms] / 100).
        let delta = angle.saturating_sub(last_rel);
        let now = self.clock.now_ms();
        let dt = now.saturating_sub(self.last_tick).max(0);
        let max_delta = util::sat_i32(i64::from(self.chg_rate).saturating_mul(dt) / 100);
        // The value range in `i64`: exact for every `min`/`max` (no `i32` overflow).
        let steps = util::span(self.min, self.max);
        // `deg_range / steps` (truncated, `0` for an empty range); `|steps| ≥ 1` when taken.
        let per_step = |num: i64| num.checked_div(steps).unwrap_or(0);
        let max_delta = if steps > 0 {
            max_delta.max(util::sat_i32(per_step(i64::from(deg_range))))
        } else {
            max_delta
        };
        // `max_delta ≥ 0` (a non-negative rate and time), so the bounds are ordered.
        let delta = delta.clamp(max_delta.saturating_neg(), max_delta);
        angle = last_rel.saturating_add(delta);
        // Rounding for symmetry.
        if steps != 0 {
            // SAFE-ARITH: `|deg_range| < 2^15`.
            let round = per_step(i64::from(deg_range).wrapping_mul(8));
            angle = angle.saturating_add(util::sat_i32(round.saturating_add(4) / 16));
        }
        angle = angle.saturating_add(self.bg_start);
        let old = self.value;
        let mut v = map(angle, self.bg_start, bg_end, self.min, self.max);
        if self.mode == ArcMode::Reverse {
            v = util::reflect(v, self.min, self.max);
        }
        if Some(v) != self.value {
            self.last_tick = now;
            self.set_value(&mut cx.widget_cx(), v);
            if self.value != old {
                cx.post(node, EventCode::ValueChanged, EventParam::Value(v));
            }
        }
        if v == self.min || v == self.max {
            self.last_tick = now;
        }
    }

    /// Steps the value for a key (LVGL `LV_EVENT_KEY`).
    fn step(&mut self, cx: &mut EventCx<'_>, d: i32) {
        let old = self.value;
        let v = self.value().saturating_add(d);
        self.set_value(&mut cx.widget_cx(), v);
        if self.value != old {
            let node = cx.node();
            cx.post(node, EventCode::ValueChanged, EventParam::Value(self.value()));
        }
    }

    /// LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE` of the arc.
    pub(crate) fn ext_for(m: &MeasureCx<'_>) -> i32 {
        let bg = m.padding(Part::Main);
        let bg_pad = bg.left.max(bg.right).max(bg.top.max(bg.bottom));
        let k = m.padding(Part::Knob);
        let knob_pad = k.left.max(k.right).max(k.top.max(k.bottom)).saturating_add(2);
        let knob = knob_pad
            .saturating_sub(bg_pad)
            .saturating_add(Self::knob_extra(m));
        knob.max(util::part_ext_draw(m, Part::Indicator)).max(0)
    }

    /// Draws the background arc, the indicator arc and the knob (LVGL `lv_arc_draw`).
    pub(crate) fn draw_arcs(&self, cx: &mut DrawCx<'_, '_>, knob: bool) {
        let m = MeasureCx::new(cx.engine(), cx.node());
        let (c, r) = self.center(&m);
        if r > 0 {
            let d = cx.arc_dsc(Part::Main);
            // SAFE-ARITH: normalized angles (`≤ 3600` each).
            let (s, e) = (
                self.bg_start.wrapping_add(self.rotation),
                self.bg_end.wrapping_add(self.rotation),
            );
            if s != e {
                cx.painter().arc(c, r, Angle::deci_deg(s), Angle::deci_deg(e), &d);
            }
        }
        let ir = r.saturating_sub(Self::indicator_max_pad(&m));
        if ir > 0 {
            let d = cx.arc_dsc(Part::Indicator);
            // SAFE-ARITH: normalized angles (`≤ 3600` each).
            let (s, e) = (
                self.indic_start.wrapping_add(self.rotation),
                self.indic_end.wrapping_add(self.rotation),
            );
            if s != e {
                cx.painter()
                    .arc(c, ir, Angle::deci_deg(s), Angle::deci_deg(e), &d);
            }
        }
        if knob {
            let k = self.knob_area(&m);
            cx.draw_rect_style(k, Part::Knob);
        }
    }
}

/// Creates an arc as the last child of `parent` (LVGL `lv_arc_create`), 130 × 130 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Arc::new()))
}

impl Widget for Arc {
    fn class(&self) -> &'static WidgetClass {
        &ARC_CLASS
    }

    /// The range and the value (little-endian `i32`s; an unset value as `i32::MIN`), for
    /// display-integrity cross-checks.
    #[cfg(feature = "display-integrity")]
    fn value_digest(&self, crc: &mut twine_core::crc::Crc32) {
        for v in [self.min, self.max, self.value.unwrap_or(i32::MIN)] {
            crc.update(&v.to_le_bytes());
        }
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut().set_size(id, ARC_DEFAULT_SIZE, ARC_DEFAULT_SIZE);
        let ext = util::ext_u16(Self::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.draw_arcs(cx, true);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(Self::ext_for(cx))
    }

    /// The bounding box plus LVGL's `ext_click_area` (`LV_DPI_DEF / 10`); with
    /// `ADV_HITTEST` only the ring (LVGL `LV_EVENT_HIT_TEST`).
    fn hit_test(&self, cx: &MeasureCx<'_>, p: Point) -> bool {
        let ext = util::DPI_DEF / 10;
        if !cx.flags().contains(ObjFlags::ADV_HITTEST) {
            return cx.coords().expand(ext).contains(p);
        }
        let (c, r) = self.center(cx);
        let w = cx.style_i32(Part::Main, PropId::ArcWidth);
        let (px, py) = (util::center_offset2(p.x, c.x), util::center_offset2(p.y, c.y));
        let d2 = util::hyp2(px, py);
        let inner = r.saturating_sub(w).saturating_sub(ext).max(0);
        if d2 < util::diam2(inner) {
            return false;
        }
        let rel = atan2(py, px)
            .as_deci_deg()
            .saturating_sub(self.rotation)
            .saturating_sub(self.bg_start);
        let angle = norm(rel);
        let circ = util::circumference(r);
        let tolerance = util::per_circumference(TURN, util::dpx(cx.engine(), cx.node(), 20), circ);
        if self.bg_bounds(angle, tolerance).is_none() {
            return false;
        }
        d2 <= util::diam2(r.saturating_add(ext))
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        let node = cx.node();
        match ev.code {
            EventCode::Pressing => self.pressing(cx),
            EventCode::Released | EventCode::PressLost => {
                self.dragging = false;
                self.clock.stop(cx.engine_mut());
                let e = cx.engine_mut();
                if util::active_input_kind(e) == Some(InputKind::Encoder) {
                    if let Some(g) = e.group_of(node).filter(|g| e.group_editing(*g)) {
                        e.set_editing(g, false);
                    }
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
            EventCode::StyleChanged | EventCode::SizeChanged => {
                let mut wcx = cx.widget_cx();
                let ext = util::ext_u16(Self::ext_for(&wcx.measure()));
                if wcx.refresh_ext_draw_with(ext) {
                    wcx.invalidate();
                }
            }
            _ => {}
        }
        EventResult::Continue
    }

    fn anim_custom(&mut self, _cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id == ANIM_CLOCK {
            self.clock.tick(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_matches_lvgl() {
        assert_eq!(map(50, 0, 100, 1350, 4050), 2700);
        assert_eq!(map(150, 0, 100, 1350, 4050), 4050);
        assert_eq!(map(-5, 0, 100, 1350, 4050), 1350);
        assert_eq!(map(25, 0, 100, 4050, 1350), 3375);
    }

    #[test]
    fn segment_area_is_small_for_small_sweeps() {
        let c = Point::new(100, 100);
        let a = arc_segment_area(c, 60, 10, 0, 100, false);
        assert!(a.width() < 20 && a.height() < 20, "{a}");
        let full = arc_segment_area(c, 60, 10, 0, 3600, false);
        assert_eq!(full, Rect::new(39, 39, 161, 161));
        // A sweep through 90° includes the bottom point.
        let b = arc_segment_area(c, 60, 10, 800, 1000, false);
        assert!(b.y1 >= 160, "{b}");
    }
}
