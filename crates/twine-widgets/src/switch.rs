//! [`Switch`]: an animated on/off switch (LVGL `lv_switch`).

use alloc::boxed::Box;

use twine_core::Duration;
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, GroupDef,
    MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, State, Widget, WidgetClass, WidgetCx,
};
use twine_style::{Part, PropId};

use crate::util::{self, Area};
use crate::{Orientation, log_set};

/// LVGL `LV_SWITCH_ANIM_STATE_END`: the knob animation runs its state from 0 (off) to this
/// (on).
const ANIM_STATE_END: i32 = 256;
/// LVGL `LV_SWITCH_KNOB_EXT_AREA_CORRECTION`.
const KNOB_EXT_AREA_CORRECTION: i32 = 2;
/// `AnimProp::Custom` id of the knob animation.
pub const ANIM_KNOB: u16 = 0;

/// Default flags of [`SWITCH_CLASS`] (LVGL `lv_switch_constructor`:
/// `lv_obj_set_scrollable(obj, false)`, `lv_obj_set_checkable(obj, true)`,
/// `lv_obj_set_scroll_on_focus(obj, true)`).
const SWITCH_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .union(ObjFlags::CHECKABLE)
    .union(ObjFlags::SCROLL_ON_FOCUS);

/// The class of [`Switch`]: `"switch"`, parts `Main`, `Indicator` and `Knob`, always in the
/// default focus group (LVGL `lv_switch_class`).
pub static SWITCH_CLASS: WidgetClass = WidgetClass::new("switch")
    .parts(&[Part::Main, Part::Indicator, Part::Knob])
    .default_flags(SWITCH_FLAGS)
    .group_def(GroupDef::True);

/// LVGL `lv_switch_class.width_def`: `4 * LV_DPI_DEF / 10`.
pub const SWITCH_DEFAULT_WIDTH: i32 = 4 * util::DPI_DEF / 10;
/// LVGL `lv_switch_class.height_def`: `4 * LV_DPI_DEF / 17`.
pub const SWITCH_DEFAULT_HEIGHT: i32 = 4 * util::DPI_DEF / 17;

/// An on/off switch (LVGL `lv_switch`): a background (`Main`), an indicator (`Indicator`,
/// colored when checked) and a knob (`Knob`) that slides to the "on" end.
///
/// The on state is the engine's `State::CHECKED` (the node is `CHECKABLE`): a click or
/// `Enter` toggles it and sends `ValueChanged` (the engine), and the arrow keys set it. Each
/// such change animates the knob over the `AnimDuration` style of `Main` (the default theme:
/// 120 ms, LVGL `anim_fast`), starting from where the knob is. Only the switch (with its
/// extra draw size) is redrawn per frame.
///
/// [`set_checked`](Self::set_checked) changes the state from code; it animates only when asked
/// to. Changing `State::CHECKED` directly with `Engine::add_state` moves the knob at once
/// (the engine has no state-change notification; LVGL animates any change once the switch was
/// drawn).
///
/// ```
/// use twine_engine::State;
/// use twine_testing::EngineHarness;
/// use twine_widgets::switch::{self, Switch};
///
/// let mut h = EngineHarness::new(100, 60);
/// let screen = h.screen();
/// let s = switch::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(s, |w: &mut Switch, cx| w.set_checked(cx, true, false));
/// assert!(h.engine().tree().node(s).unwrap().state().contains(State::CHECKED));
/// ```
#[derive(Debug, Default)]
pub struct Switch {
    /// Knob position `0..=256` while animating (LVGL `anim_state`).
    anim_state: Option<i32>,
    anim_end: i32,
    anim_id: Option<AnimId>,
    orientation: Orientation,
}

impl Switch {
    /// A switch (off, automatic orientation).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            anim_state: None,
            anim_end: 0,
            anim_id: None,
            orientation: Orientation::Auto,
        }
    }

    /// The orientation.
    #[must_use]
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Whether the knob is animating.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.anim_state.is_some()
    }

    /// The knob position `0..=256` (0 = off end) while animating.
    #[must_use]
    pub fn anim_state(&self) -> Option<i32> {
        self.anim_state
    }

    /// Whether the switch is on (`State::CHECKED`).
    #[must_use]
    pub fn is_checked(&self, cx: &MeasureCx<'_>) -> bool {
        cx.state().contains(State::CHECKED)
    }

    /// Sets the orientation (LVGL `lv_switch_set_orientation`). `Auto` is vertical when the
    /// switch is taller than wide. Idempotent.
    pub fn set_orientation(&mut self, cx: &mut WidgetCx<'_>, o: Orientation) {
        if self.orientation == o {
            return;
        }
        log_set(SWITCH_CLASS.name, cx.node(), "orientation");
        self.orientation = o;
        cx.invalidate_for("switch.orientation");
    }

    /// Turns the switch on or off from code (no `ValueChanged`). With `anim` the knob slides
    /// like after a click. Idempotent.
    pub fn set_checked(&mut self, cx: &mut WidgetCx<'_>, on: bool, anim: bool) {
        if cx.state().contains(State::CHECKED) == on {
            return;
        }
        log_set(SWITCH_CLASS.name, cx.node(), "checked");
        if on {
            cx.add_state(State::CHECKED);
        } else {
            cx.clear_state(State::CHECKED);
        }
        util::log_value(SWITCH_CLASS.name, i32::from(on));
        if anim {
            self.trigger_anim(cx);
        } else {
            self.stop_anim(cx.engine_mut());
        }
        cx.invalidate_for("switch.checked");
    }

    fn stop_anim(&mut self, e: &mut Engine) {
        if let Some(id) = self.anim_id.take() {
            e.anim_stop(id);
        }
        self.anim_state = None;
    }

    /// LVGL `lv_switch_trigger_anim`: slides the knob to the current state's end over the
    /// `AnimDuration` style (a partial way takes proportionally less time).
    fn trigger_anim(&mut self, cx: &mut WidgetCx<'_>) {
        let full = cx
            .style(Part::Main, PropId::AnimDuration)
            .as_i32()
            .unwrap_or(0)
            .max(0);
        if full == 0 {
            self.stop_anim(cx.engine_mut());
            return;
        }
        let chk = cx.state().contains(State::CHECKED);
        let end = if chk { ANIM_STATE_END } else { 0 };
        let start = self.anim_state.unwrap_or(ANIM_STATE_END - end);
        let dur = full * (start - end).abs() / ANIM_STATE_END;
        self.stop_anim(cx.engine_mut());
        if dur == 0 {
            return;
        }
        self.anim_state = Some(start);
        self.anim_end = end;
        let node = cx.node();
        let id = cx.engine_mut().anim_start(
            node,
            AnimProp::Custom(ANIM_KNOB),
            Anim::new(start, end).duration(Duration::ms(dur.unsigned_abs().into())),
        );
        self.anim_id = Some(id);
    }

    /// Whether the switch is drawn horizontally.
    fn is_hor(&self, c: twine_core::Rect) -> bool {
        match self.orientation {
            Orientation::Horizontal => true,
            Orientation::Vertical => false,
            Orientation::Auto => c.width() >= c.height(),
        }
    }

    /// The knob's area (LVGL `draw_main`).
    #[must_use]
    pub fn knob_area(&self, cx: &MeasureCx<'_>) -> twine_core::Rect {
        let c = cx.coords();
        let mut k = Area::from_rect(c);
        let chk = cx.state().contains(State::CHECKED);
        let rtl = util::is_rtl(cx);
        if self.is_hor(c) {
            let size = c.height();
            let len = c.width() - size;
            let mut v = match self.anim_state {
                Some(s) => len * s / ANIM_STATE_END,
                None if chk => len,
                None => 0,
            };
            if rtl {
                v = len - v;
            }
            k.x1 += v;
            k.x2 = k.x1 + (size - 1).max(0);
        } else {
            let size = c.width();
            let len = c.height() - size;
            let mut v = match self.anim_state {
                Some(s) => len * s / ANIM_STATE_END,
                None if chk => len,
                None => 0,
            };
            if rtl {
                v = len - v;
            }
            k.y2 -= v;
            k.y1 = k.y2 - (size - 1).max(0);
        }
        let pad = cx.padding(Part::Knob);
        k.x1 -= pad.left;
        k.x2 += pad.right;
        k.y1 -= pad.top;
        k.y2 += pad.bottom;
        k.to_rect()
    }

    /// LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE` of the switch.
    fn ext_for(m: &MeasureCx<'_>) -> i32 {
        let pad = m.padding(Part::Knob);
        let knob = pad.left.max(pad.right).max(pad.top.max(pad.bottom))
            + KNOB_EXT_AREA_CORRECTION
            + util::part_ext_draw(m, Part::Knob);
        knob.max(util::part_ext_draw(m, Part::Indicator))
    }
}

/// Creates a switch as the last child of `parent` (LVGL `lv_switch_create`), 52 × 30 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Switch::new()))
}

impl Widget for Switch {
    fn class(&self) -> &'static WidgetClass {
        &SWITCH_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut()
            .set_size(id, SWITCH_DEFAULT_WIDTH, SWITCH_DEFAULT_HEIGHT);
        let ext = util::ext_u16(Self::ext_for(&cx.measure()));
        cx.refresh_ext_draw_with(ext);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        // The indicator fills the content area (LVGL: coordinates minus the paddings).
        let indic = cx.content_area();
        cx.draw_rect_style(indic, Part::Indicator);
        let m = MeasureCx::new(cx.engine(), cx.node());
        let knob = self.knob_area(&m);
        cx.draw_rect_style(knob, Part::Knob);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(Self::ext_for(cx))
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match ev.code {
            // The engine toggled `CHECKED` (click, Enter, arrow keys).
            EventCode::ValueChanged => {
                let mut wcx = cx.widget_cx();
                util::log_value(SWITCH_CLASS.name, i32::from(wcx.state().contains(State::CHECKED)));
                self.trigger_anim(&mut wcx);
                wcx.invalidate_for("switch.checked");
            }
            EventCode::StyleChanged => {
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

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id != ANIM_KNOB || self.anim_state.is_none() {
            return;
        }
        if v == self.anim_end {
            // LVGL `lv_switch_anim_completed`.
            self.anim_state = None;
            self.anim_id = None;
        } else {
            self.anim_state = Some(v);
        }
        cx.invalidate_for("switch.anim");
    }
}
