//! [`Bar`]: a progress bar (LVGL `lv_bar`).

use alloc::boxed::Box;

use twine_core::{Duration, Opa, Rect};
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx,
    NodeId, OBJ_FLAGS, ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_image::ImageSource;
use twine_render::{Mask, ShadowDsc};
use twine_style::{GradDir, Part, PropId};

use crate::util::{self, Area, log_value};
use crate::{Orientation, log_set};

/// `AnimProp::Custom` id of the value animation (LVGL `cur_value_anim`).
pub const ANIM_VALUE: u16 = 0;
/// `AnimProp::Custom` id of the start value animation (LVGL `start_value_anim`).
pub const ANIM_START_VALUE: u16 = 1;

/// LVGL `LV_BAR_ANIM_STATE_END`: an animation runs its state from 0 to this.
const ANIM_STATE_END: i32 = 256;
/// LVGL `LV_BAR_SIZE_MIN`: paddings cannot make the indicator thinner than this.
const BAR_SIZE_MIN: i32 = 4;

/// How a [`Bar`] (and a [`Slider`](crate::slider::Slider)) draws its indicator (LVGL
/// `lv_bar_mode_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BarMode {
    /// From the minimum to the value.
    #[default]
    Normal,
    /// From zero to the value when the range includes zero (e.g. −50…50).
    Symmetrical,
    /// From the start value to the value (both settable).
    Range,
}

/// Default flags of [`BAR_CLASS`]: the base object's minus `SCROLLABLE` (LVGL
/// `lv_bar_constructor`: `lv_obj_set_checkable(obj, false)`, `lv_obj_set_scrollable(obj,
/// false)`).
const BAR_FLAGS: ObjFlags = OBJ_FLAGS.difference(ObjFlags::SCROLLABLE);

/// The class of [`Bar`]: `"bar"`, parts `Main` (background) and `Indicator`.
pub static BAR_CLASS: WidgetClass = WidgetClass::new("bar")
    .parts(&[Part::Main, Part::Indicator])
    .default_flags(BAR_FLAGS);

/// LVGL `lv_bar_class.width_def`: `LV_DPI_DEF * 2`.
pub const BAR_DEFAULT_WIDTH: i32 = util::DPI_DEF * 2;
/// LVGL `lv_bar_class.height_def`: `LV_DPI_DEF / 10`.
pub const BAR_DEFAULT_HEIGHT: i32 = util::DPI_DEF / 10;

/// The animation of one of the two values (LVGL `lv_bar_anim_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ValueAnim {
    /// Value the animation starts from.
    start: i32,
    /// Value it ends at.
    end: i32,
    /// Progress `0..=256`, or `None` when no animation runs.
    state: Option<i32>,
    id: Option<AnimId>,
}

impl ValueAnim {
    const IDLE: Self = Self {
        start: 0,
        end: 0,
        state: None,
        id: None,
    };

    /// The value shown now.
    fn current(&self, value: i32) -> i32 {
        match self.state {
            Some(s) => self.start + ((i64::from(self.end - self.start) * i64::from(s)) / 256) as i32,
            None => value,
        }
    }

    /// Stops the engine animation.
    fn stop(&mut self, e: &mut Engine) {
        if let Some(id) = self.id.take() {
            e.anim_stop(id);
        }
        self.state = None;
    }
}

/// Where the indicator is drawn (computed from the value, styles and coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IndicGeom {
    /// The indicator area (LVGL `indic_area`; may be inverted or tiny).
    pub area: Area,
    /// The bar's coordinates grown by its transform size.
    pub bar: Rect,
    /// Horizontal (else vertical) bar.
    pub hor: bool,
    /// Whether the indicator is drawn (LVGL skips indicators ≤ 1 px unless symmetrical).
    pub drawn: bool,
    /// Whether the indicator runs right-to-left / top-to-bottom.
    pub reversed: bool,
}

impl IndicGeom {
    /// The drawn area as a half-open rectangle (empty when not drawn).
    pub fn rect(&self) -> Rect {
        if self.drawn {
            self.area.to_rect()
        } else {
            Rect::new(self.area.x1, self.area.y1, self.area.x1, self.area.y1)
        }
    }
}

/// A progress bar (LVGL `lv_bar`): a background (`Part::Main`) and an indicator
/// (`Part::Indicator`) showing a value within a range.
///
/// - **Modes** ([`BarMode`]): `Normal` fills from the minimum, `Symmetrical` from zero when
///   the range spans zero, `Range` from a settable start value.
/// - **Orientation** ([`Orientation`]): `Auto` is vertical when the bar is taller than wide.
/// - **Animation**: [`set_value`](Self::set_value) with `anim = true` moves the indicator
///   over the `AnimDuration` style of `Main` (the default theme sets none for bars; set it
///   locally, e.g. 200 ms). Each animation frame redraws only the strip around the moving
///   end of the indicator. Retargeting a running animation continues from the value shown.
/// - A range with `min > max` draws the indicator from the other end (LVGL "reversed").
///
/// The default size is LVGL's: 260 × 13 px.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::bar::{self, Bar};
///
/// let mut h = EngineHarness::new(300, 40);
/// let screen = h.screen();
/// let b = bar::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(b, |w: &mut Bar, cx| w.set_value(cx, 70, false));
/// assert_eq!(h.engine().widget::<Bar>(b).unwrap().value(), 70);
/// ```
#[derive(Debug)]
pub struct Bar {
    pub(crate) name: &'static str,
    pub(crate) min: i32,
    pub(crate) max: i32,
    pub(crate) value: i32,
    pub(crate) start_value: i32,
    pub(crate) reversed: bool,
    pub(crate) mode: BarMode,
    pub(crate) orientation: Orientation,
    /// A slider: the knob moves with the indicator's end, so invalidations cover it too.
    pub(crate) has_knob: bool,
    cur_anim: ValueAnim,
    start_anim: ValueAnim,
}

impl Default for Bar {
    fn default() -> Self {
        Self::new()
    }
}

/// Which value a change applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Value,
    Start,
}

impl Bar {
    /// A bar with range 0…100 and value 0 (LVGL `lv_bar_constructor`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            name: "bar",
            min: 0,
            max: 100,
            value: 0,
            start_value: 0,
            reversed: false,
            mode: BarMode::Normal,
            orientation: Orientation::Auto,
            has_knob: false,
            cur_anim: ValueAnim::IDLE,
            start_anim: ValueAnim::IDLE,
        }
    }

    /// The value (the target value while an animation runs, LVGL `lv_bar_get_value`).
    #[must_use]
    pub fn value(&self) -> i32 {
        self.value
    }

    /// The start value (the minimum unless the mode is [`BarMode::Range`]).
    #[must_use]
    pub fn start_value(&self) -> i32 {
        if self.mode == BarMode::Range {
            self.start_value
        } else {
            self.min_real()
        }
    }

    /// The minimum as set (LVGL returns the swapped value of a reversed range).
    #[must_use]
    pub fn min(&self) -> i32 {
        if self.reversed { self.max } else { self.min }
    }

    /// The maximum as set.
    #[must_use]
    pub fn max(&self) -> i32 {
        if self.reversed { self.min } else { self.max }
    }

    fn min_real(&self) -> i32 {
        self.min
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> BarMode {
        self.mode
    }

    /// The orientation.
    #[must_use]
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Whether an animation of the value or the start value is running.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.cur_anim.state.is_some() || self.start_anim.state.is_some()
    }

    /// Whether the indicator is drawn from zero (LVGL `lv_bar_is_symmetrical`): symmetrical
    /// mode with a range spanning zero.
    #[must_use]
    pub fn is_symmetrical(&self) -> bool {
        self.mode == BarMode::Symmetrical && self.min < 0 && self.max > 0 && self.start_value == self.min
    }

    /// Sets the value, clamped to the range (and not below the start value). With `anim` the
    /// indicator moves there over the `AnimDuration` style of `Main` (LVGL
    /// `lv_bar_set_value`). Idempotent.
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32, anim: bool) {
        if self.value == v {
            return;
        }
        let v = v.clamp(self.min, self.max).max(self.start_value);
        if self.value == v {
            return;
        }
        log_set(self.name, cx.node(), "value");
        self.set_slot(cx, Slot::Value, v, anim);
    }

    /// Sets the start value in [`BarMode::Range`] (clamped to the range and not above the
    /// value; ignored in the other modes, like LVGL). Idempotent.
    pub fn set_start_value(&mut self, cx: &mut WidgetCx<'_>, v: i32, anim: bool) {
        if self.mode != BarMode::Range {
            twine_core::trace!(target: "twine::engine", "{} set_start_value ignored: not in range mode", self.name);
            return;
        }
        let v = v.clamp(self.min, self.max).min(self.value);
        if self.start_value == v {
            return;
        }
        log_set(self.name, cx.node(), "start_value");
        self.set_slot(cx, Slot::Start, v, anim);
    }

    /// Sets the range (LVGL `lv_bar_set_range`). `min > max` reverses the bar: the
    /// indicator grows from the other end. The value is clamped into the new range.
    /// Idempotent.
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, min: i32, max: i32) {
        let reversed = min > max;
        let (lo, hi) = if reversed { (max, min) } else { (min, max) };
        if self.min == lo && self.max == hi && self.reversed == reversed {
            return;
        }
        log_set(self.name, cx.node(), "range");
        if reversed {
            twine_core::debug!(target: "twine::engine", "{} range {}..{} reversed", self.name, min, max);
        }
        self.reversed = reversed;
        self.min = lo;
        self.max = hi;
        if self.mode != BarMode::Range {
            self.start_value = lo;
        }
        self.start_value = self.start_value.clamp(lo, hi);
        if !(lo..=hi).contains(&self.value) {
            let v = self.value.clamp(lo, hi);
            self.set_slot(cx, Slot::Value, v, false);
        }
        cx.invalidate_for("bar.range");
    }

    /// Sets the mode. Leaving [`BarMode::Range`] resets the start value to the minimum.
    /// Idempotent.
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: BarMode) {
        if self.mode == mode {
            return;
        }
        log_set(self.name, cx.node(), "mode");
        self.mode = mode;
        if mode != BarMode::Range {
            self.start_anim.stop(cx.engine_mut());
            self.start_value = self.min;
        }
        cx.invalidate_for("bar.mode");
    }

    /// Sets the orientation. Idempotent.
    pub fn set_orientation(&mut self, cx: &mut WidgetCx<'_>, o: Orientation) {
        if self.orientation == o {
            return;
        }
        log_set(self.name, cx.node(), "orientation");
        self.orientation = o;
        cx.invalidate_for("bar.orientation");
    }

    /// Writes a value (animated or not) and invalidates what changed (LVGL
    /// `lv_bar_set_value_with_anim`).
    pub(crate) fn set_slot(&mut self, cx: &mut WidgetCx<'_>, slot: Slot, v: i32, anim: bool) {
        let before = self.geom(&cx.measure());
        let dur = cx
            .style(Part::Main, PropId::AnimDuration)
            .as_i32()
            .unwrap_or(0)
            .max(0);
        let (val, a, custom) = match slot {
            Slot::Value => (&mut self.value, &mut self.cur_anim, ANIM_VALUE),
            Slot::Start => (&mut self.start_value, &mut self.start_anim, ANIM_START_VALUE),
        };
        if anim && dur > 0 {
            let from = a.current(*val);
            a.stop(cx.engine_mut());
            *a = ValueAnim {
                start: from,
                end: v,
                state: Some(0),
                id: None,
            };
            *val = v;
            let node = cx.node();
            let id = cx.engine_mut().anim_start(
                node,
                AnimProp::Custom(custom),
                Anim::new(0, ANIM_STATE_END).duration(Duration::ms(dur.unsigned_abs().into())),
            );
            a.id = Some(id);
        } else {
            a.stop(cx.engine_mut());
            *val = v;
        }
        log_value(self.name, v);
        self.invalidate_change(cx, before);
    }

    /// Applies an animation step (LVGL `lv_bar_anim` / `lv_bar_anim_completed`).
    pub(crate) fn apply_anim(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        let before = self.geom(&cx.measure());
        let a = match id {
            ANIM_VALUE => &mut self.cur_anim,
            ANIM_START_VALUE => &mut self.start_anim,
            _ => return,
        };
        if a.state.is_none() {
            return;
        }
        if v >= ANIM_STATE_END {
            a.state = None;
            a.id = None;
        } else {
            a.state = Some(v.max(0));
        }
        self.invalidate_change(cx, before);
    }

    /// Invalidates the strips around the ends of the indicator that moved (plus the
    /// indicator's radius and extra draw size; for a slider the knob), never the whole bar.
    pub(crate) fn invalidate_change(&self, cx: &mut WidgetCx<'_>, before: IndicGeom) {
        let m = cx.measure();
        let after = self.geom(&m);
        if after == before {
            return;
        }
        let c = m.coords();
        if c.is_empty() || before.hor != after.hor {
            cx.invalidate_for("bar.value");
            return;
        }
        let hor = after.hor;
        let ind_ext = util::part_ext_draw(&m, Part::Indicator);
        // Cross axis: the indicator (a slider's knob spans the whole widget plus its extra
        // draw size).
        let (cross0, cross1, half) = if self.has_knob {
            let node_ext = m
                .engine()
                .tree()
                .node(m.node())
                .map_or(0, |n| i32::from(n.ext_draw()));
            let e = node_ext.max(ind_ext);
            let (c0, c1, len) = if hor {
                (c.y0, c.y1, c.height())
            } else {
                (c.x0, c.x1, c.width())
            };
            (c0 - e, c1 + e, len / 2 + e)
        } else {
            let (a, b) = (before.area, after.area);
            let (c0, c1) = if hor {
                (a.y1.min(b.y1), a.y2.max(b.y2) + 1)
            } else {
                (a.x1.min(b.x1), a.x2.max(b.x2) + 1)
            };
            let r = m
                .style_i32(Part::Indicator, PropId::Radius)
                .clamp(0, (c1 - c0) / 2);
            (c0 - ind_ext, c1 + ind_ext, r + ind_ext)
        };
        let r = half + 1;
        let (a, b) = (before.area, after.area);
        let edges = if hor {
            [(a.x1, b.x1), (a.x2 + 1, b.x2 + 1)]
        } else {
            [(a.y1, b.y1), (a.y2 + 1, b.y2 + 1)]
        };
        // A shown / hidden indicator changes between its (unchanged) ends as well.
        let span = |g: &IndicGeom| {
            if hor {
                (g.area.x1, g.area.x2 + 1)
            } else {
                (g.area.y1, g.area.y2 + 1)
            }
        };
        if before.drawn != after.drawn {
            let g = if after.drawn { &after } else { &before };
            let (lo, hi) = span(g);
            let area = if hor {
                Rect::new(lo - r, cross0, hi + r, cross1)
            } else {
                Rect::new(cross0, lo - r, cross1, hi + r)
            };
            cx.invalidate_area(area);
        }
        for (a, b) in edges {
            if a == b {
                continue;
            }
            let (lo, hi) = (a.min(b) - r, a.max(b) + r);
            let strip = if hor {
                Rect::new(lo, cross0, hi, cross1)
            } else {
                Rect::new(cross0, lo, cross1, hi)
            };
            cx.invalidate_area(strip);
        }
    }

    /// Whether the bar is drawn horizontally.
    #[must_use]
    pub fn is_horizontal(&self, cx: &MeasureCx<'_>) -> bool {
        let c = cx.engine().draw_area(cx.node());
        match self.orientation {
            Orientation::Horizontal => true,
            Orientation::Vertical => false,
            Orientation::Auto => c.width() >= c.height(),
        }
    }

    /// The area the indicator is drawn in (empty when it is not drawn), as computed by
    /// LVGL's `draw_indic`.
    #[must_use]
    pub fn indicator_area(&self, cx: &MeasureCx<'_>) -> Rect {
        self.geom(cx).rect()
    }

    /// LVGL `draw_indic`: the indicator area.
    pub(crate) fn geom(&self, m: &MeasureCx<'_>) -> IndicGeom {
        let bar = m.engine().draw_area(m.node());
        let coords = m.coords();
        let (barw, barh) = (bar.width(), bar.height());
        let range = if self.max == self.min {
            1
        } else {
            self.max - self.min
        };
        let hor = match self.orientation {
            Orientation::Horizontal => true,
            Orientation::Vertical => false,
            Orientation::Auto => barw >= barh,
        };
        let sym = self.is_symmetrical();
        let pad = m.padding(Part::Main);
        let b = Area::from_rect(bar);
        let mut ia = Area {
            x1: b.x1 + pad.left,
            y1: b.y1 + pad.top,
            x2: b.x2 - pad.right,
            y2: b.y2 - pad.bottom,
        };
        if hor && ia.height() < BAR_SIZE_MIN {
            ia.y1 = coords.y0 + barh / 2 - BAR_SIZE_MIN / 2;
            ia.y2 = ia.y1 + BAR_SIZE_MIN;
        } else if !hor && ia.width() < BAR_SIZE_MIN {
            ia.x1 = coords.x0 + barw / 2 - BAR_SIZE_MIN / 2;
            ia.x2 = ia.x1 + BAR_SIZE_MIN;
        }
        let anim_length = i64::from(if hor { ia.width() } else { ia.height() });
        let pos = |v: i32| ((anim_length * i64::from(v - self.min)) / i64::from(range)) as i32;
        let anim_pos = |a: &ValueAnim, v: i32| match a.state {
            Some(s) => {
                let (from, to) = (pos(a.start), pos(a.end));
                from + (((i64::from(to - from)) * i64::from(s)) / i64::from(ANIM_STATE_END)) as i32
            }
            None => pos(v),
        };
        let mut start_x = anim_pos(&self.start_anim, self.start_value);
        let mut cur_x = anim_pos(&self.cur_anim, self.value);
        let hor_rtl = hor && util::is_rtl(m);
        let reversed = self.reversed ^ hor_rtl;
        // An area of width 0 is {x1 = 0, x2 = -1}.
        cur_x -= 1;
        let mut a = ia.to_array();
        let (mut ax1, mut ax2) = if hor { (0, 2) } else { (1, 3) };
        if reversed {
            core::mem::swap(&mut ax1, &mut ax2);
            cur_x = -cur_x;
            start_x = -start_x;
        }
        if hor {
            a[ax2] = a[ax1] + cur_x;
            a[ax1] += start_x;
        } else {
            a[ax1] = a[ax2] - cur_x;
            a[ax2] -= start_x;
        }
        if sym {
            let shift = ((i64::from(-self.min) * anim_length) / i64::from(range)) as i32;
            if hor {
                let (left, right) = if reversed { (ax2, ax1) } else { (ax1, ax2) };
                let zero = if reversed {
                    a[ax1] - shift + 1
                } else {
                    a[ax1] + shift
                };
                if a[ax2] > zero {
                    a[right] = a[ax2];
                    a[left] = zero;
                } else {
                    a[left] = a[ax2];
                    a[right] = zero;
                }
            } else {
                let (top, bottom) = if reversed { (ax2, ax1) } else { (ax1, ax2) };
                let zero = if reversed {
                    a[ax2] + shift
                } else {
                    a[ax2] - shift + 1
                };
                if a[ax1] > zero {
                    a[bottom] = a[ax1];
                    a[top] = zero;
                } else {
                    a[top] = a[ax1];
                    a[bottom] = zero;
                }
            }
        }
        let area = Area::from_array(a);
        let len = if hor { area.width() } else { area.height() };
        IndicGeom {
            area,
            bar,
            hor,
            drawn: sym || len > 1,
            reversed,
        }
    }

    /// LVGL `draw_indic`: draws the indicator, clipped to the background's rounded corners
    /// when the indicator's own radius is smaller, and over the whole length with a mask when
    /// its gradient or image runs along the bar.
    pub(crate) fn draw_indicator(&self, cx: &mut DrawCx<'_, '_>) {
        let m = MeasureCx::new(cx.engine(), cx.node());
        let g = self.geom(&m);
        if !g.drawn {
            return;
        }
        let indic = g.area.to_rect();
        if indic.is_empty() {
            return;
        }
        let pad = m.padding(Part::Main);
        let short = g.bar.width().min(g.bar.height());
        let bg_radius = m.style_i32(Part::Main, PropId::Radius).min(short >> 1).max(0);
        let rs = cx.rect_dsc(Part::Indicator);
        let d = rs.dsc();
        let indic_short = indic.width().min(indic.height());
        let indic_radius = d.radius.min(indic_short >> 1).max(0);
        let grad_dir = m
            .style(Part::Indicator, PropId::BgGradDir)
            .get::<GradDir>()
            .unwrap_or_default();
        let has_image = m
            .style(Part::Indicator, PropId::BgImageSrc)
            .get::<&'static ImageSource>()
            .is_some();
        let mask_needed =
            (g.hor && grad_dir == GradDir::Hor) || (!g.hor && grad_dir == GradDir::Ver) || has_image;
        let radius_issue = !(pad.left < 0 || pad.right < 0 || pad.top < 0 || pad.bottom < 0)
            && indic_radius < bg_radius
            && !util::rect_in_rounded(indic, g.bar, bg_radius);
        if !(radius_issue || mask_needed) {
            cx.draw_rect_style(indic, Part::Indicator);
            return;
        }
        let mut d = d;
        if radius_issue {
            d.border_opa = Opa::TRANSP;
            d.outline_opa = Opa::TRANSP;
        } else {
            // Only the shadow, unclipped.
            let mut sh = d;
            sh.border_opa = Opa::TRANSP;
            sh.outline_opa = Opa::TRANSP;
            sh.bg_opa = Opa::TRANSP;
            cx.painter().rect(indic, &sh);
        }
        d.shadow = ShadowDsc {
            opa: Opa::TRANSP,
            ..d.shadow
        };
        let mut body = d;
        body.border_opa = Opa::TRANSP;
        body.outline_opa = Opa::TRANSP;
        let mut body_area = indic;
        if mask_needed {
            if g.hor {
                body_area.x0 = g.bar.x0 + pad.left;
                body_area.x1 = g.bar.x1 - pad.right;
            } else {
                body_area.y0 = g.bar.y0 + pad.top;
                body_area.y1 = g.bar.y1 - pad.bottom;
            }
            body.radius = 0;
        }
        let p = cx.painter();
        let m1 = radius_issue.then(|| {
            p.push_mask(Mask::Radius {
                area: g.bar,
                radius: bg_radius,
                outer: false,
            })
        });
        let m2 = mask_needed.then(|| {
            p.push_mask(Mask::Radius {
                area: indic,
                radius: indic_radius,
                outer: false,
            })
        });
        p.rect(body_area, &body);
        if let Some(id) = m2 {
            p.pop_mask(id);
        }
        if let Some(id) = m1 {
            p.pop_mask(id);
        }
        let mut frame = d;
        frame.bg_opa = Opa::TRANSP;
        cx.painter().rect(indic, &frame);
    }

    /// LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE` of the bar: the indicator's extra size, plus a
    /// negative padding (an indicator larger than the background).
    pub(crate) fn ext_for(m: &MeasureCx<'_>) -> i32 {
        let pad = m.padding(Part::Main);
        let p = pad.left.min(pad.right).min(pad.top).min(pad.bottom);
        util::part_ext_draw(m, Part::Indicator) - p.min(0)
    }
}

/// Creates a bar as the last child of `parent` (LVGL `lv_bar_create`), 260 × 13 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Bar::new()))
}

impl Widget for Bar {
    fn class(&self) -> &'static WidgetClass {
        &BAR_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut()
            .set_size(id, BAR_DEFAULT_WIDTH, BAR_DEFAULT_HEIGHT);
        let ext = util::ext_u16(Self::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.draw_indicator(cx);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(Self::ext_for(cx))
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && ev.code == EventCode::StyleChanged {
            let mut wcx = cx.widget_cx();
            let ext = util::ext_u16(Self::ext_for(&wcx.measure()));
            if wcx.refresh_ext_draw_with(ext) {
                wcx.invalidate();
            }
        }
        EventResult::Continue
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        self.apply_anim(cx, id, v);
    }
}
