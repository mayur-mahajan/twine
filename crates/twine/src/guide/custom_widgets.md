Writing a custom widget: a gauge, step by step.

This guide builds a round **gauge** outside Twine, the way an application or a widget
library would: a [`Widget`](crate::engine::Widget) with its own
[`WidgetClass`](crate::engine::WidgetClass) and parts, drawn with the renderer, themed through
a theme builder registration with [design elements](crate::style::design) (so it follows all
four [theme modes](crate::style::design::ThemeMode)), wrapped in a view with typed
modifiers, animated within the [motion preference](crate::anim::Motion), adjustable with a
keypad or an encoder, and tested with `twine-testing`.

The complete code is the `gauge` example of this crate (`examples/gauge/`): run it with
`cargo run -p twine --example gauge` (or `cargo xtask sim gauge`; F12 cycles the theme
modes). Its tests are `tests/gauge.rs`. Every code block below is compiled and run as a
doctest; blocks starting with `// examples/gauge/gauge.rs` are excerpts of the example, and a
test (`guide_excerpts_match_the_example`) checks that they still are.

**Contents:** [1. Parts and the class](#1-parts-and-the-class) ·
[2. State and setters](#2-state-and-setters) · [3. Drawing and input](#3-drawing-and-input) ·
[4. Theming](#4-theming) · [5. The view](#5-the-view) · [6. Using it](#6-using-it) ·
[7. Testing](#7-testing) · [Checklist](#checklist)

# 1. Parts and the class

A widget draws **parts**, each styled on its own (`.part(NEEDLE, |s| ..)` in a view,
`Selector::part(NEEDLE)` in a theme). Use a built-in [`Part`](crate::style::Part) where one
fits — the gauge's value arc is an `Indicator`, like a bar's or an arc's — and **custom
parts** for the rest: [`Part::custom::<N>()`](crate::style::Part::custom) with `N` below
[`Part::CUSTOM_COUNT`](crate::style::Part::CUSTOM_COUNT) (7; an `N` out of range does not
compile). A custom part means what its class says (the text area's placeholder is custom part
0 as well), so give each a named constant and a doc comment listing the style properties it
reads.

The [`WidgetClass`](crate::engine::WidgetClass) is a `static` describing the widget type:
its name (for logs and dumps; classes are compared by address, never by name), its parts,
default flags, and whether the keypad/encoder group takes it
([`GroupDef`](crate::engine::GroupDef), [`Editable`](crate::engine::Editable): an editable
widget enters edit mode on an encoder click and then receives the turns as arrow keys).
[`base`](crate::engine::WidgetClass::base) names the class themes style it like when they do
not know it: with the card as base, every built-in theme gives the gauge's `Main` part the
card look of the current theme.

```rust
// examples/gauge/gauge.rs
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
# use twine::core::math::{TRIG_SHIFT, cos, sin};
# use twine::engine::{AnimId, AnimProp, Editable, EventParam, GroupDef, OBJ_FLAGS};
# use twine::prelude::*;
# use twine::theme::ClassCx;
# use twine::view::{BuildOp, event_value, on_value_changed};
# use twine::widgets::container::CARD_CLASS;
#
# assert!(GAUGE_CLASS.is_a(&CARD_CLASS));
# assert_eq!(NEEDLE.custom_index(), Some(1));
```

# 2. State and setters

The widget struct holds only the gauge's own data; the node (coordinates, flags, state,
styles) lives in the engine's tree. Its setters follow the rules every Twine widget follows,
because view bindings call them whenever a signal changes:

- **Idempotent:** an unchanged value returns at once, without invalidating anything (a
  binding that re-runs with the same value must cost nothing and redraw nothing).
- **Never panic on input:** clamp, swap a reversed range, saturate.
- **Keep what was asked for:** the gauge keeps the requested value and clamps it whenever
  the value or the range changes, so narrowing the range and widening it again restores the
  value. (The view applies the value after the range when it is built; see section 5.)
- **Prepare for drawing:** the readout text is formatted here, into a fixed buffer, so
  drawing never allocates or formats.
- **Animate through the engine:** [`Engine::anim_start`](crate::engine::Engine::anim_start)
  with [`AnimProp::Value`](crate::engine::AnimProp::Value) delivers each step to
  `Widget::anim_value` (section 3). The engine applies the
  [motion preference](crate::anim::Motion) to every animation it starts (reduced: capped,
  none: the final value at the next update), so the widget does nothing for it. The duration
  is the `anim_duration` style of `Main`, so a theme or an application sets it like any
  property (`.anim_duration(Duration::ms(500))`).

```rust
// examples/gauge/gauge.rs
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
}
# use core::ops::RangeInclusive;
# use twine::core::math::{TRIG_SHIFT, cos, sin};
# use twine::engine::{AnimId, AnimProp, Editable, EventParam, GroupDef, OBJ_FLAGS};
# use twine::prelude::*;
# use twine::theme::ClassCx;
# use twine::view::{BuildOp, event_value, on_value_changed};
# use twine::widgets::container::CARD_CLASS;
#
# /// A value as decimal text in a fixed buffer (no allocation; `i32::MIN` fits).
# #[derive(Clone, Copy, Debug)]
# struct Readout {
#     buf: [u8; 11],
#     len: u8,
# }
#
# impl Readout {
#     const fn new(v: i32) -> Self {
#         let mut tmp = [0u8; 11];
#         let mut i = tmp.len();
#         let mut n = v.unsigned_abs();
#         loop {
#             i -= 1;
#             tmp[i] = b'0' + (n % 10) as u8;
#             n /= 10;
#             if n == 0 {
#                 break;
#             }
#         }
#         if v < 0 {
#             i -= 1;
#             tmp[i] = b'-';
#         }
#         let mut buf = [0u8; 11];
#         let mut j = 0;
#         while i + j < tmp.len() {
#             buf[j] = tmp[i + j];
#             j += 1;
#         }
#         Self { buf, len: j as u8 }
#     }
#
#     fn as_str(&self) -> &str {
#         core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
#     }
# }
#
# let g = Gauge::new();
# assert_eq!((g.value(), g.shown(), g.range()), (0, 0, 0..=100));
```

# 3. Drawing and input

[`Widget`](crate::engine::Widget) has a default for everything but `class`.
The gauge implements:

- `init`: the default size (the theme has styled the node already).
- `draw`: through the [`DrawCx`](crate::engine::DrawCx). `draw_base(Part::Main)` draws the
  background, border, shadow and outline of `Main` (here: the card and the focus ring); the
  `*_dsc(part)` helpers turn a part's **resolved** style into a renderer descriptor
  (`arc_dsc`, `line_dsc`, `text_dsc`, `rect_dsc`), so state styles, transitions, design
  elements and the node's opacity all apply; `cx.painter()` draws arcs, lines, rectangles,
  and `draw_text` text. Use integer math ([`sin`](crate::core::math::sin) /
  [`cos`](crate::core::math::cos) return `sin · 32767`, angles are tenths of a degree): no
  floating point in drawing, and no allocation.
- `event`: arrow keys (and an encoder's turns, which arrive as keys in edit mode, or as
  `Rotary`) step the value. While `event` runs the widget is out of its node, so it notifies
  with [`EventCx::post`](crate::engine::EventCx::post): the `ValueChanged` reaches the
  handlers (the view's `on_change`) right after `event` returns.
- `anim_value`: one step of the needle animation.
- `text`: the readout, so tests can find the gauge `by_text("42")` and tree dumps show it.

This gauge invalidates its whole area on a change, which is right for a small widget; a large
one would invalidate only what changed ([`WidgetCx::invalidate_area`](crate::engine::WidgetCx::invalidate_area),
as the built-in arc does for the swept segment).

```rust
// examples/gauge/gauge.rs
impl Gauge {
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
# use core::ops::RangeInclusive;
# use twine::core::math::{TRIG_SHIFT, cos, sin};
# use twine::engine::{AnimId, AnimProp, Editable, EventParam, GroupDef, OBJ_FLAGS};
# use twine::prelude::*;
# use twine::theme::ClassCx;
# use twine::view::{BuildOp, event_value, on_value_changed};
# use twine::widgets::container::CARD_CLASS;
#
# /// The ring behind the indicator (`arc_width`, `arc_color`, `arc_rounded`, …).
# pub const TRACK: Part = Part::custom::<0>();
# /// The arc from the minimum to the value (`arc_*`): the built-in indicator part.
# pub const INDICATOR: Part = Part::Indicator;
# /// The needle pointing at the value (`line_width`, `line_color`, `line_rounded`; its
# /// `padding_top` is the gap between its tip and the ring).
# pub const NEEDLE: Part = Part::custom::<1>();
# /// The value readout in the opening of the ring (`text_color`, `font`, `text_align`).
# pub const LABEL: Part = Part::custom::<2>();
#
# /// The class of [`Gauge`]: its parts, focusable and adjustable with a keypad or an encoder,
# /// and themed like a card where a theme does not know it (`base`).
# pub static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge")
#     .parts(&[Part::Main, TRACK, INDICATOR, NEEDLE, LABEL])
#     .default_flags(
#         OBJ_FLAGS
#             .difference(ObjFlags::SCROLLABLE)
#             .union(ObjFlags::CLICKABLE),
#     )
#     .group_def(GroupDef::True)
#     .editable(Editable::True)
#     .base(&CARD_CLASS);
#
# /// Width and height of a new gauge, in pixels.
# pub const GAUGE_DEFAULT_SIZE: i32 = 120;
# /// Where the scale starts: 135°, clockwise from 3 o'clock (bottom left), in 0.1°.
# const START: i32 = 1350;
# /// How far the scale sweeps: 270°, so it ends at the bottom right, in 0.1°.
# const SWEEP: i32 = 2700;
#
# /// A round gauge: a [`TRACK`] ring, an [`INDICATOR`] arc and a [`NEEDLE`] showing the value
# /// in its range, and the value as text ([`LABEL`]).
# ///
# /// - Value changes move the needle over the `anim_duration` style of `Main` (none: they
# ///   jump), honouring the motion preference ([`Motion`]) like every engine animation.
# /// - Keys `Right`/`Up` and `Left`/`Down` (and an encoder in edit mode) step the value by 1
# ///   and send `ValueChanged` with the new value as [`EventParam::Value`]. Setters never send
# ///   events.
# /// - Setters are idempotent. The requested value is kept and clamped into the range whenever
# ///   either changes, so narrowing the range and widening it again restores the value.
# /// - Drawing allocates nothing: the readout text is formatted when the value changes.
# #[derive(Debug)]
# pub struct Gauge {
#     /// The value as requested (clamped into the range when shown or read).
#     requested: i32,
#     /// `requested` clamped into `min..=max`.
#     value: i32,
#     min: i32,
#     max: i32,
#     /// The value the needle shows (moves to `value` while an animation runs).
#     shown: i32,
#     /// The running needle animation.
#     anim: Option<AnimId>,
#     /// `value` as text.
#     readout: Readout,
# }
#
# impl Gauge {
#     /// Sets the value, clamped into the range; the needle moves there (see [`Gauge`]).
#     /// Idempotent: an unchanged value does nothing.
#     pub fn set_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
#         if self.requested == v {
#             return;
#         }
#         self.requested = v;
#         self.update(cx, true);
#     }
#     fn update(&mut self, _cx: &mut WidgetCx<'_>, _animate: bool) {}
# }
#
# /// A value as decimal text in a fixed buffer (no allocation; `i32::MIN` fits).
# #[derive(Clone, Copy, Debug)]
# struct Readout {
#     buf: [u8; 11],
#     len: u8,
# }
#
# impl Readout {
#     const fn new(v: i32) -> Self {
#         let mut tmp = [0u8; 11];
#         let mut i = tmp.len();
#         let mut n = v.unsigned_abs();
#         loop {
#             i -= 1;
#             tmp[i] = b'0' + (n % 10) as u8;
#             n /= 10;
#             if n == 0 {
#                 break;
#             }
#         }
#         if v < 0 {
#             i -= 1;
#             tmp[i] = b'-';
#         }
#         let mut buf = [0u8; 11];
#         let mut j = 0;
#         while i + j < tmp.len() {
#             buf[j] = tmp[i + j];
#             j += 1;
#         }
#         Self { buf, len: j as u8 }
#     }
#
#     fn as_str(&self) -> &str {
#         core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
#     }
# }
```

# 4. Theming

Theme styles are plain `static` [`Style`](crate::style::Style)s (in flash), one per part and
state, whose colours, opacities and fonts are **design elements** — `design::PRIMARY`,
`design::ON_SURFACE`, `design::FONT_LARGE`, … — resolved from the display's theme when the
style is resolved. That is what makes the gauge follow the theme's mode (light, dark, night,
high contrast) when it switches at run time: nothing is rebuilt and no binding runs. A fixed
[`Color`](crate::core::Color) would stay the same in every mode.

The registration function adds them to a node of the gauge's class through a
[`ClassCx`](crate::theme::ClassCx). It is generic over the theme's style set `S`, so the same
function works with every built-in theme; `cx.styles()` gives that style set (e.g. the
default theme's `card`, `btn`, `pressed`) when a widget wants to reuse the theme's own looks.
State styles use the [state precedence](crate::style::State#precedence): `DISABLED` beats the
default look; application states (`State::custom`) beat both.

```rust
// examples/gauge/gauge.rs
/// `Main`: how long the needle takes to a new value.
static MAIN_STYLE: Style = style! { anim_duration: Duration::ms(300) };
/// `Main` focused by a keypad or an encoder: the theme's focus ring around the card.
static FOCUS_STYLE: Style = style! {
    outline_width: 2,
    outline_offset: 2,
    outline_color: design::FOCUS_RING,
    outline_opacity: design::FOCUS_RING_OPACITY,
};
# use core::ops::RangeInclusive;
# use twine::core::math::{TRIG_SHIFT, cos, sin};
# use twine::engine::{AnimId, AnimProp, Editable, EventParam, GroupDef, OBJ_FLAGS};
# use twine::prelude::*;
# use twine::theme::ClassCx;
# use twine::view::{BuildOp, event_value, on_value_changed};
# use twine::widgets::container::CARD_CLASS;
#
# /// The ring behind the indicator (`arc_width`, `arc_color`, `arc_rounded`, …).
# pub const TRACK: Part = Part::custom::<0>();
# /// The arc from the minimum to the value (`arc_*`): the built-in indicator part.
# pub const INDICATOR: Part = Part::Indicator;
# /// The needle pointing at the value (`line_width`, `line_color`, `line_rounded`; its
# /// `padding_top` is the gap between its tip and the ring).
# pub const NEEDLE: Part = Part::custom::<1>();
# /// The value readout in the opening of the ring (`text_color`, `font`, `text_align`).
# pub const LABEL: Part = Part::custom::<2>();
#
# /// The class of [`Gauge`]: its parts, focusable and adjustable with a keypad or an encoder,
# /// and themed like a card where a theme does not know it (`base`).
# pub static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge")
#     .parts(&[Part::Main, TRACK, INDICATOR, NEEDLE, LABEL])
#     .default_flags(
#         OBJ_FLAGS
#             .difference(ObjFlags::SCROLLABLE)
#             .union(ObjFlags::CLICKABLE),
#     )
#     .group_def(GroupDef::True)
#     .editable(Editable::True)
#     .base(&CARD_CLASS);
```

Register it with the theme builder of the theme you install: the registration runs after the
theme's own styles for nodes of the class (and of classes derived from it), so it wins over
the base class's look.

```rust
# #[allow(dead_code)]
# mod gauge {
#     include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/gauge/gauge.rs"));
# }
# use twine::prelude::*;
use std::rc::Rc;
use gauge::{GAUGE_CLASS, style_gauge};

let theme = DefaultTheme::builder().class(&GAUGE_CLASS, style_gauge).build();
let mono = MonoTheme::builder().class(&GAUGE_CLASS, style_gauge).build();
# let _ = (Rc::new(theme), Rc::new(mono));
```

# 5. The view

A view wraps [`widget_view`](crate::view::widget_view) — the generic builder every widget view
wraps — in a newtype, so the gauge gets its own **typed modifiers** next to every modifier of
[`ViewExt`](crate::view::ViewExt) and [`StyleExt`](crate::view::StyleExt) (`.size(..)`,
`.part(NEEDLE, |s| ..)`, `.on_state(..)`, `.test_id(..)`, …): implementing `ViewExt` only
needs `push_op`. Each modifier takes [`IntoProp`](crate::view::IntoProp), so a constant, a
signal, a memo or a closure works; [`WidgetView::bind`](crate::view::WidgetView::bind) applies
constants once and turns the dynamic ones into one binding each that calls the (idempotent)
setter, in the order of the builder chain.
[`WidgetView::bind_after_children`](crate::view::WidgetView::bind_after_children) applies a
value after every other build step instead — the mechanism the built-in `bar`, `led` and
`animimg` views use — so the gauge's value meets its final range (`gauge(150).range(0..=200)`
starts at 150, not at 100). `on_change` uses [`on_value_changed`](crate::view::on_value_changed) and
[`event_value`](crate::view::event_value), the building blocks of the built-in widgets'
`on_change`; a widget that edits a signal both ways would use
[`bind_model`](crate::view::bind_model).

```rust
// examples/gauge/gauge.rs
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
# #[allow(dead_code)]
# mod real {
#     include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/gauge/gauge.rs"));
# }
# use core::ops::RangeInclusive;
# use real::Gauge;
# use twine::prelude::*;
# use twine::view::{BuildOp, event_value, on_value_changed};
# let cx = twine::reactive::create_root();
# let _v = gauge(cx.signal(3)).range(0..=10).on_change(|_| {}).padding(4);
# cx.dispose();
```

# 6. Using it

In an application the gauge is a view like any other. Here a setpoint is edited with the keys
and written back to its signal, and a second gauge shows a load with an **application state**:
[`State::custom`](crate::style::State::custom) `ALARM` is bound with `.state(..)` and restyles
the indicator through `.on_state(..)` with a part scope, in every theme mode
(`design::DANGER`).

```rust
# #[allow(dead_code)]
# mod gauge {
#     include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/gauge/gauge.rs"));
# }
use gauge::{INDICATOR, NEEDLE, gauge};
use twine::prelude::*;

const ALARM: State = State::custom::<0>();

fn app(cx: Scope) -> impl View {
    State::set_custom_name(ALARM, "ALARM"); // for tree dumps and logs
    let setpoint = cx.signal(21);
    let load = cx.signal(35);
    row((
        gauge(setpoint)
            .range(10..=30)
            .on_change(move |v| setpoint.set(v)),
        gauge(load)
            .state(ALARM, move || load.get() > 80)
            .on_state(ALARM, |s| s.part(INDICATOR, |p| p.arc_color(design::DANGER)))
            .part(NEEDLE, |s| s.line_color(design::PRIMARY)),
    ))
    .gap(12)
}
# let _ = app;
```

# 7. Testing

[`twine_testing::TestUi`][TestUi] runs a view with a mock display, clock, keypad and encoder.
Check, for every custom widget:

[TestUi]: https://docs.rs/twine-testing/latest/twine_testing/ui/struct.TestUi.html

- **Defaults** and the class: parts, size, the base look, the registration's styles.
- **Idempotent setters:** set a signal to its current value and `assert_idle()` — nothing is
  redrawn, the update returns `Wake::Idle`.
- **Part styling** through `.part(..)` (constant and reactive values).
- **Input:** keys and encoder change the value, report it once per change, and nothing at the
  end of the range.
- **Animation and motion:** the needle moves over time, and jumps with `Motion::None`.
- **Theme modes:** each part's colour is the theme's element in every
  [`ThemeMode`](crate::style::design::ThemeMode).
- **Snapshots** per state and mode (`t.assert_snapshot("gauge_dark")`): references are created
  on the first local run and compared in CI.

```rust
# #[allow(dead_code)]
# mod gauge {
#     include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/gauge/gauge.rs"));
# }
use std::rc::Rc;
use gauge::{GAUGE_CLASS, Gauge, INDICATOR, gauge, style_gauge};
use twine::prelude::*;
use twine_testing::{TestUi, by_id};

let theme = Rc::new(DefaultTheme::builder().class(&GAUGE_CLASS, style_gauge).build());
let mut t = TestUi::new(200, 160)
    .theme(theme)
    .mount(|_| gauge(50).on_change(|_| {}).test_id("g"));
t.run_until_idle();
let g = t.find(by_id("g")).id();

// The keypad steps the focused gauge (it is in the default group).
t.key(Key::Right);
t.run_until_idle();
assert_eq!(t.engine().widget::<Gauge>(g).unwrap().value(), 51);
t.assert_idle();

// The indicator follows the theme mode through `design::PRIMARY`.
let display = t.engine().default_display().unwrap();
for mode in ThemeMode::ALL {
    t.engine_mut().set_theme_mode(display, mode);
    t.run_until_idle();
    let e = t.engine();
    let primary = e.design_value(display, design::PRIMARY).unwrap();
    assert_eq!(e.style_color(g, INDICATOR, PropId::ArcColor), primary);
}
```

The example's full test suite is `tests/gauge.rs`.

# Checklist

- A `static` [`WidgetClass`](crate::engine::WidgetClass) with every part it draws, a `base`
  for themes that do not know it, and the group/edit settings its input needs.
- Named constants for custom parts ([`Part::custom`](crate::style::Part::custom)), documented
  with the properties each reads.
- Setters: idempotent, never panicking, invalidating only on change; values that depend on
  other settings bound with `bind_after_children`.
- Drawing: resolved part styles (`*_dsc`), integer math, no allocation, no formatting.
- Animations through the engine, durations from styles: the motion preference applies.
- Input: `post` the `ValueChanged`, never call the handlers from `event`.
- Theme: `static` styles with design elements, registered with
  `.class(&CLASS, style_fn)`; no fixed colours.
- A view newtype implementing [`View`](crate::view::View) and
  [`ViewExt`](crate::view::ViewExt), typed modifiers taking
  [`IntoProp`](crate::view::IntoProp).
- Tests: defaults, idempotent setters, input, motion, theme modes, snapshots.
