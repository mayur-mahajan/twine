//! [`Spinner`]: the loading spinner, an endlessly animated arc (LVGL `lv_spinner`).

use alloc::boxed::Box;

use twine_core::{Angle, Duration, Point};
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Easing, Engine, EngineError, Event, EventCode, EventCx, EventResult,
    MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Repeat, Widget, WidgetClass, WidgetCx,
};
use twine_style::Part;

use crate::arc::Arc;
use crate::log_set;
use crate::util;

/// LVGL `DEF_TIME`: one turn in 1000 ms.
pub const SPINNER_DEFAULT_PERIOD: Duration = Duration::ms(1000);
/// LVGL `DEF_ARC_ANGLE`: the arc is 200° long.
pub const SPINNER_DEFAULT_SWEEP: Angle = Angle::deg(200);
/// `AnimProp::Custom` id of the spin animation.
pub const ANIM_SPIN: u16 = 0;
/// LVGL `LV_BEZIER_VAL_MAX`: the animation runs 0 → 1024 per period.
const BEZIER_MAX: i32 = 1024;
/// LVGL `lv_cubic_bezier(v, LV_BEZIER_VAL_FLOAT(0.42f), LV_BEZIER_VAL_FLOAT(0.58f),
/// LV_BEZIER_VAL_FLOAT(0.0f), LV_BEZIER_VAL_FLOAT(1.0f))` (x1, y1, x2, y2 ×1024).
const START_EASING: Easing = Easing::CubicBezier(430, 593, 0, 1024);

/// The class of [`Spinner`]: `"spinner"`, the arc's parts, not clickable (LVGL
/// `lv_spinner_constructor`: `lv_obj_set_clickable(obj, false)`).
pub static SPINNER_CLASS: WidgetClass = WidgetClass::new("spinner")
    .parts(&[Part::Main, Part::Indicator, Part::Knob])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::SCROLLABLE)
            .difference(ObjFlags::SCROLL_CHAIN)
            .difference(ObjFlags::CLICKABLE),
    );

/// The loading spinner (LVGL `lv_spinner`): an [`Arc`] whose background is a full ring and
/// whose indicator chases around it forever.
///
/// One infinite animation runs per spinner, as in LVGL 9 (`lv_spinner_set_anim_params`):
/// over each period the indicator's end turns 360° at constant speed while its start follows
/// on an ease-in-out curve, so the arc stretches and shrinks as it spins. The drawing is
/// rotated by 270° (starting at 12 o'clock). Each frame redraws only the ring segments the two
/// ends swept (the arc's invalidation), and nothing is allocated per frame.
///
/// ```
/// use twine_core::{Angle, Duration};
/// use twine_testing::EngineHarness;
/// use twine_widgets::spinner::{self, Spinner};
///
/// let mut h = EngineHarness::new(80, 80);
/// let screen = h.screen();
/// let s = spinner::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(s, |w: &mut Spinner, cx| {
///     w.set_anim_params(cx, Duration::ms(1500), Angle::deg(120));
/// });
/// assert_eq!(h.engine().widget::<Spinner>(s).unwrap().period(), Duration::ms(1500));
/// ```
#[derive(Debug)]
pub struct Spinner {
    arc: Arc,
    period: Duration,
    sweep: Angle,
    anim: Option<AnimId>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    /// A spinner with LVGL's defaults (1000 ms, 200°); the animation starts when it is
    /// created in an engine.
    #[must_use]
    pub fn new() -> Self {
        let mut arc = Arc::new();
        arc.name = "spinner";
        Self {
            arc,
            period: SPINNER_DEFAULT_PERIOD,
            sweep: SPINNER_DEFAULT_SWEEP,
            anim: None,
        }
    }

    /// The underlying arc (angles, rotation).
    #[must_use]
    pub fn arc(&self) -> &Arc {
        &self.arc
    }

    /// The duration of one turn.
    #[must_use]
    pub fn period(&self) -> Duration {
        self.period
    }

    /// The arc length at the start of a turn.
    #[must_use]
    pub fn sweep(&self) -> Angle {
        self.sweep
    }

    /// Restarts the animation with a turn duration and an arc length (LVGL
    /// `lv_spinner_set_anim_params`). Idempotent when both are unchanged and it is running.
    pub fn set_anim_params(&mut self, cx: &mut WidgetCx<'_>, period: Duration, sweep: Angle) {
        let running = self.anim.is_some_and(|id| cx.engine().anim_exists(id));
        if running && self.period == period && self.sweep == sweep {
            return;
        }
        log_set(SPINNER_CLASS.name, cx.node(), "anim_params");
        self.period = period;
        self.sweep = sweep;
        if let Some(id) = self.anim.take() {
            cx.engine_mut().anim_stop(id);
        }
        let node = cx.node();
        let a = Anim::new(0, BEZIER_MAX).duration(period).repeat(Repeat::Infinite);
        self.anim = Some(cx.engine_mut().anim_start(node, AnimProp::Custom(ANIM_SPIN), a));
        self.arc.set_bg_angles(cx, Angle::deg(0), Angle::deg(360));
        self.arc.set_rotation(cx, Angle::deg(270));
    }

    /// Sets the turn duration (LVGL `lv_spinner_set_anim_duration`). Idempotent.
    pub fn set_period(&mut self, cx: &mut WidgetCx<'_>, period: Duration) {
        let sweep = self.sweep;
        self.set_anim_params(cx, period, sweep);
    }

    /// Sets the arc length (LVGL `lv_spinner_set_arc_sweep`). Idempotent.
    pub fn set_arc_sweep(&mut self, cx: &mut WidgetCx<'_>, sweep: Angle) {
        let period = self.period;
        self.set_anim_params(cx, period, sweep);
    }

    /// The indicator angles at animation value `v` (`0..=1024`, LVGL `arc_anim_angles`).
    #[must_use]
    pub fn angles_at(&self, v: i32) -> (Angle, Angle) {
        let t = u16::try_from(v.clamp(0, BEZIER_MAX)).unwrap_or(0);
        let step = START_EASING.value(t, 0, BEZIER_MAX);
        let start = (step * 3600) >> 10;
        let end = self.sweep.0 + ((v * 3600) >> 10);
        (Angle(start), Angle(end))
    }
}

/// Creates a spinning spinner as the last child of `parent` (LVGL `lv_spinner_create`),
/// 130 × 130 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Spinner::new()))
}

impl Widget for Spinner {
    fn class(&self) -> &'static WidgetClass {
        &SPINNER_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut()
            .set_size(id, crate::arc::ARC_DEFAULT_SIZE, crate::arc::ARC_DEFAULT_SIZE);
        let ext = util::ext_u16(Arc::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
        let (period, sweep) = (self.period, self.sweep);
        self.anim = None;
        self.set_anim_params(cx, period, sweep);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        self.arc.draw_arcs(cx, true);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(Arc::ext_for(cx))
    }

    fn hit_test(&self, cx: &MeasureCx<'_>, p: Point) -> bool {
        cx.coords().contains(p)
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && matches!(ev.code, EventCode::StyleChanged | EventCode::SizeChanged) {
            let mut wcx = cx.widget_cx();
            let ext = util::ext_u16(Arc::ext_for(&wcx.measure()));
            if wcx.refresh_ext_draw_with(ext) {
                wcx.invalidate();
            }
        }
        EventResult::Continue
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id == ANIM_SPIN {
            let (s, e) = self.angles_at(v);
            self.arc.set_angles(cx, s, e);
        }
    }
}
