// The gauge of the custom widget guide (`twine::guide::custom_widgets`): a widget with its
// own class and parts, drawn with the renderer, themed through a theme builder registration
// with design elements, and wrapped in a typed view. Shared by the `gauge` example
// (`main.rs`), its tests (`tests/gauge.rs`) and the guide's doctests, which `include!` this
// file (so it has no inner `//!` docs).

use core::ops::RangeInclusive;

use twine::core::math::{TRIG_SHIFT, cos, sin};
use twine::engine::{AnimId, AnimProp, Editable, EventParam, GroupDef, OBJ_FLAGS};
use twine::prelude::*;
use twine::theme::ClassCx;
use twine::view::{BuildOp, event_value, on_value_changed};
use twine::widgets::container::CARD_CLASS;

// ---- Parts and class ----------------------------------------------------------------------

/// The ring behind the indicator (`arc_width`, `arc_color`, `arc_rounded`, …).
pub const TRACK: Part = Part::custom::<0>();
/// The arc from the minimum to the value (`arc_*`): the built-in indicator part.
pub const INDICATOR: Part = Part::Indicator;
/// The needle pointing at the value (`line_width`, `line_color`, `line_rounded`; its
/// `padding_top` is the gap between its tip and the ring).
pub const NEEDLE: Part = Part::custom::<1>();
/// The value readout in the opening of the ring (`text_color`, `font`, `text_align`).
pub const LABEL: Part = Part::custom::<2>();

/// The class of [`Gauge`]: its parts, focusable and adjustable with a keypad or an encoder,
/// and themed like a card where a theme does not know it (`base`).
pub static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge")
    .parts(&[Part::Main, TRACK, INDICATOR, NEEDLE, LABEL])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::SCROLLABLE)
            .union(ObjFlags::CLICKABLE),
    )
    .group_def(GroupDef::True)
    .editable(Editable::True)
    .base(&CARD_CLASS);

/// Width and height of a new gauge, in pixels.
pub const GAUGE_DEFAULT_SIZE: i32 = 120;
/// Where the scale starts: 135°, clockwise from 3 o'clock (bottom left), in 0.1°.
const START: i32 = 1350;
/// How far the scale sweeps: 270°, so it ends at the bottom right, in 0.1°.
const SWEEP: i32 = 2700;

// ---- The widget ---------------------------------------------------------------------------

/// A round gauge: a [`TRACK`] ring, an [`INDICATOR`] arc and a [`NEEDLE`] showing the value
/// in its range, and the value as text ([`LABEL`]).
///
/// - Value changes move the needle over the `anim_duration` style of `Main` (none: they
///   jump), honouring the motion preference ([`Motion`]) like every engine animation.
/// - Keys `Right`/`Up` and `Left`/`Down` (and an encoder in edit mode) step the value by 1
///   and send `ValueChanged` with the new value as [`EventParam::Value`]. Setters never send
///   events.
/// - Setters are idempotent. The requested value is kept and clamped into the range whenever
///   either changes, so narrowing the range and widening it again restores the value.
/// - Drawing allocates nothing: the readout text is formatted when the value changes.
#[derive(Debug)]
pub struct Gauge {
    /// The value as requested (clamped into the range when shown or read).
    requested: i32,
    /// `requested` clamped into `min..=max`.
    value: i32,
    min: i32,
    max: i32,
    /// The value the needle shows (moves to `value` while an animation runs).
    shown: i32,
    /// The running needle animation.
    anim: Option<AnimId>,
    /// `value` as text.
    readout: Readout,
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

impl Gauge {
    /// A gauge at 0 in `0..=100`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            requested: 0,
            value: 0,
            min: 0,
            max: 100,
            shown: 0,
            anim: None,
            readout: Readout::new(0),
        }
    }

    /// The value (in the range; the target while the needle moves).
    #[must_use]
    pub fn value(&self) -> i32 {
        self.value
    }

    /// The value the needle shows now.
    #[must_use]
    pub fn shown(&self) -> i32 {
        self.shown
    }

    /// The range.
    #[must_use]
    pub fn range(&self) -> RangeInclusive<i32> {
        self.min..=self.max
    }

    /// Sets the value, clamped into the range; the needle moves there (see [`Gauge`]).
    /// Idempotent: an unchanged value does nothing.
    pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        if self.requested == v {
            return;
        }
        self.requested = v;
        self.update(cx, true);
    }

    /// Sets the range (a reversed range is turned around); the value is clamped into it and
    /// shown at once. Idempotent.
    pub fn set_range(&mut self, cx: &mut WidgetCx<'_>, r: RangeInclusive<i32>) {
        let (a, b) = r.into_inner();
        let (min, max) = (a.min(b), a.max(b));
        if (self.min, self.max) == (min, max) {
            return;
        }
        (self.min, self.max) = (min, max);
        self.update(cx, false);
        // The scale changed under the needle: redraw even if the value did not change.
        cx.invalidate_for("gauge.range");
    }

    /// Clamps the requested value and moves the needle (animated or not).
    fn update(&mut self, cx: &mut WidgetCx<'_>, animate: bool) {
        let v = self.requested.clamp(self.min, self.max);
        if v == self.value {
            return;
        }
        self.value = v;
        self.readout = Readout::new(v);
        if let Some(id) = self.anim.take() {
            cx.engine_mut().anim_stop(id);
        }
        let ms = cx.style_i32(Part::Main, PropId::AnimDuration);
        if animate && ms > 0 && self.shown != v {
            // From where the needle is now, so a new value during a move continues smoothly.
            let a = Anim::new(self.shown, v)
                .duration(Duration::ms(ms.unsigned_abs().into()))
                .easing(Easing::EaseOut);
            let node = cx.node();
            self.anim = Some(cx.engine_mut().anim_start(node, AnimProp::Value, a));
        } else {
            self.shown = v;
        }
        cx.invalidate_for("gauge.value");
    }

    /// The angle of value `v` on the scale, in 0.1°.
    fn angle(&self, v: i32) -> i32 {
        let span = i64::from(self.max - self.min).max(1);
        let pos = i64::from(v.clamp(self.min, self.max) - self.min);
        START + (pos * i64::from(SWEEP) / span) as i32
    }

    /// Steps the value by `d` for a key or an encoder and reports the change.
    fn step(&mut self, cx: &mut EventCx<'_>, d: i32) {
        let before = self.value;
        self.set_value(&mut cx.widget_cx(), self.value.saturating_add(d));
        if self.value != before {
            let node = cx.node();
            cx.post(node, EventCode::ValueChanged, EventParam::Value(self.value));
        }
    }
}

/// Creates a gauge as the last child of `parent`.
///
/// # Errors
/// The engine's error when `parent` does not exist or the tree is full.
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, twine::engine::EngineError> {
    engine.create(parent, Box::new(Gauge::new()))
}

impl Widget for Gauge {
    fn class(&self) -> &'static WidgetClass {
        &GAUGE_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let node = cx.node();
        cx.engine_mut()
            .set_size(node, GAUGE_DEFAULT_SIZE, GAUGE_DEFAULT_SIZE);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        // The card behind (background, border, shadow, focus outline of `Main`).
        cx.draw_base(Part::Main);
        let area = cx.content_area();
        let r = area.width().min(area.height()) / 2;
        if r <= 0 {
            return;
        }
        let c = area.center();
        let track = cx.arc_dsc(TRACK);
        let indicator = cx.arc_dsc(INDICATOR);
        let end = self.angle(self.shown);
        cx.painter().arc(
            c,
            r,
            Angle::deci_deg(START),
            Angle::deci_deg(START + SWEEP),
            &track,
        );
        if end != START {
            cx.painter()
                .arc(c, r, Angle::deci_deg(START), Angle::deci_deg(end), &indicator);
        }
        // The needle, from the centre to `padding_top` inside the ring.
        let gap = MeasureCx::new(cx.engine(), cx.node()).padding(NEEDLE).top;
        let len = r - track.width.max(indicator.width) - gap;
        if len > 0 {
            let a = Angle::deci_deg(end);
            let tip = Point::new(
                c.x + ((len * cos(a)) >> TRIG_SHIFT),
                c.y + ((len * sin(a)) >> TRIG_SHIFT),
            );
            let needle = cx.line_dsc(NEEDLE);
            cx.painter().line(c, tip, &needle);
        }
        // The readout in the opening of the ring, below the centre.
        let text = cx.text_dsc(LABEL);
        let top = c.y + r / 2;
        let label = Rect::new(area.x0, top, area.x1, top + i32::from(text.font.line_height));
        cx.draw_text(label, self.readout.as_str(), &text);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match (ev.code, ev.param) {
            (EventCode::Key, EventParam::Key(Key::Right | Key::Up)) => self.step(cx, 1),
            (EventCode::Key, EventParam::Key(Key::Left | Key::Down)) => self.step(cx, -1),
            (EventCode::Rotary, EventParam::Rotary(d)) => self.step(cx, d),
            _ => {}
        }
        EventResult::Continue
    }

    /// A step of the needle animation started in `update`.
    fn anim_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        if self.shown != v {
            self.shown = v;
            cx.invalidate_for("gauge.needle");
        }
        if v == self.value {
            self.anim = None;
        }
    }

    /// The readout, for test queries (`by_text("42")`) and tree dumps.
    fn text(&self) -> Option<&str> {
        Some(self.readout.as_str())
    }
}

/// A value as decimal text in a fixed buffer (no allocation; `i32::MIN` fits).
#[derive(Clone, Copy, Debug)]
struct Readout {
    buf: [u8; 11],
    len: u8,
}

impl Readout {
    const fn new(v: i32) -> Self {
        let mut tmp = [0u8; 11];
        let mut i = tmp.len();
        let mut n = v.unsigned_abs();
        loop {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if v < 0 {
            i -= 1;
            tmp[i] = b'-';
        }
        let mut buf = [0u8; 11];
        let mut j = 0;
        while i + j < tmp.len() {
            buf[j] = tmp[i + j];
            j += 1;
        }
        Self { buf, len: j as u8 }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }
}

// ---- Theme --------------------------------------------------------------------------------

/// `Main`: how long the needle takes to a new value.
static MAIN_STYLE: Style = style! { anim_duration: Duration::ms(300) };
/// `Main` focused by a keypad or an encoder: the theme's focus ring around the card.
static FOCUS_STYLE: Style = style! {
    outline_width: 2,
    outline_offset: 2,
    outline_color: design::FOCUS_RING,
    outline_opacity: design::FOCUS_RING_OPACITY,
};
/// [`TRACK`]: a muted ring.
static TRACK_STYLE: Style = style! {
    arc_width: 10,
    arc_rounded: true,
    arc_color: design::SURFACE_VARIANT,
};
/// [`INDICATOR`]: the accent over the track.
static INDICATOR_STYLE: Style = style! {
    arc_width: 10,
    arc_rounded: true,
    arc_color: design::PRIMARY,
};
/// [`INDICATOR`] while disabled.
static INDICATOR_DISABLED_STYLE: Style = style! { arc_color: design::DISABLED };
/// [`NEEDLE`]: a rounded line in the text colour, 6 px short of the ring.
static NEEDLE_STYLE: Style = style! {
    line_width: 4,
    line_rounded: true,
    line_color: design::ON_SURFACE,
    padding_top: 6,
};
/// [`LABEL`]: large centred text.
static LABEL_STYLE: Style = style! {
    text_color: design::ON_SURFACE,
    font: design::FONT_LARGE,
    text_align: TextAlign::Center,
};

/// Styles a gauge node in any theme: register it with the theme builder,
/// `DefaultTheme::builder().class(&GAUGE_CLASS, style_gauge)` (also `MonoTheme` and
/// `SimpleTheme`). Every colour and the font are design elements, so the gauge follows the
/// theme's modes (light, dark, night, high contrast) without being rebuilt. `Main` keeps the
/// card look the theme gives the base class.
pub fn style_gauge<S>(cx: &mut ClassCx<'_, '_, S>) {
    cx.add_style(Selector::MAIN, &MAIN_STYLE);
    cx.add_style(Selector::state(State::FOCUS_KEY), &FOCUS_STYLE);
    cx.add_style(Selector::part(TRACK), &TRACK_STYLE);
    cx.add_style(Selector::part(INDICATOR), &INDICATOR_STYLE);
    cx.add_style(
        Selector::part(INDICATOR).with_state(State::DISABLED),
        &INDICATOR_DISABLED_STYLE,
    );
    cx.add_style(Selector::part(NEEDLE), &NEEDLE_STYLE);
    cx.add_style(Selector::part(LABEL), &LABEL_STYLE);
}

// ---- View ---------------------------------------------------------------------------------

/// A gauge showing `value`: a constant, a signal, a memo or a closure. Style it like any view
/// (`.size(..)`, `.part(NEEDLE, |s| s.line_color(..))`, `.on_state(..)`); see [`GaugeView`]
/// for its own modifiers. The value is applied after every modifier (`bind_after_children`),
/// so `gauge(150).range(0..=200)` starts at 150, not clamped into the default range.
pub fn gauge<M>(value: impl IntoProp<i32, M>) -> GaugeView {
    GaugeView(widget_view(Gauge::new).bind_after_children(value, Gauge::set_value))
}

/// The view of a [`Gauge`] ([`gauge`]): a [`WidgetView`] with the gauge's typed modifiers;
/// every modifier of [`ViewExt`] and [`StyleExt`] works too.
#[derive(Debug)]
pub struct GaugeView(WidgetView<Gauge>);

impl GaugeView {
    /// The range (default `0..=100`): a constant, a signal, a memo or a closure.
    #[must_use]
    pub fn range<M>(self, r: impl IntoProp<RangeInclusive<i32>, M>) -> Self {
        Self(self.0.bind(r, Gauge::set_range))
    }

    /// Called with the new value when the user changes it (keys, encoder).
    #[must_use]
    pub fn on_change(self, f: impl FnMut(i32) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |_, _, ev| event_value(ev), f))
    }
}

impl View for GaugeView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for GaugeView {
    type Widget = Gauge;

    fn push_op(self, op: BuildOp) -> Self {
        Self(self.0.push_op(op))
    }
}
