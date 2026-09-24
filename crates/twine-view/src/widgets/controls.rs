//! The basic controls: [`bar`], [`slider`], [`switch`], [`checkbox`], [`arc`], [`led`],
//! [`line()`] / [`line_static`] and [`spinner`].

use core::cell::Cell;
use core::ops::RangeInclusive;

use alloc::vec::Vec;

use twine_core::{Angle, Color, Duration, Point};
use twine_engine::{Engine, NodeId, ObjFlags, State};
use twine_style::{Part, Selector, StyleProp};
use twine_widgets::Orientation;
use twine_widgets::arc::{Arc, ArcMode};
use twine_widgets::bar::{Bar, BarMode};
use twine_widgets::checkbox::Checkbox;
use twine_widgets::led::{LED_BRIGHT_MAX, Led};
use twine_widgets::line::Line;
use twine_widgets::slider::{Slider, SliderMode};
use twine_widgets::spinner::Spinner;
use twine_widgets::switch::Switch;

use crate::bind::bind_prop;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, bind_model_synced, event_value, on_value_changed};
use crate::modifiers::ViewExt;
use crate::prop::IntoProp;
use crate::text::{IntoText, bind_str};

/// Whether the `CHECKED` state of `n` is set.
pub(crate) fn is_checked(e: &Engine, n: NodeId) -> bool {
    e.tree()
        .node(n)
        .is_some_and(|x| x.state().contains(State::CHECKED))
}

/// `(min, max)` of a range.
fn bounds(r: &RangeInclusive<i32>) -> (i32, i32) {
    (*r.start(), *r.end())
}

// ---- Bar ------------------------------------------------------------------------------------

/// Settings of a [`bar`] view read when it is built.
#[derive(Default)]
struct BarCfg {
    animated: Cell<bool>,
}

/// A progress bar showing `value` (a constant, a signal, a memo or a closure).
///
/// The value is applied after the other settings, so `.range` is in place before the first
/// value is clamped into it. Value changes jump unless [`animated`](WidgetView::animated) is
/// set.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let progress = cx.signal(30);
/// let _v = bar(progress).range(0..=200).animated(Duration::ms(300)).width(160);
/// cx.dispose();
/// ```
pub fn bar(value: impl IntoProp<i32>) -> WidgetView<Bar> {
    let value = value.into_prop();
    let mut v = widget_view(Bar::new);
    let cfg = v.shared::<BarCfg>();
    v.after_children(move |cx, node| {
        let anim = cfg.animated.get();
        bind_prop(cx, node, value, move |b: &mut Bar, wcx, v| {
            b.set_value(wcx, v, anim);
        });
    })
}

impl WidgetView<Bar> {
    /// The value range (`min..=max`; a reversed range draws from the other end, as LVGL).
    #[must_use]
    pub fn range(self, r: impl IntoProp<RangeInclusive<i32>>) -> Self {
        self.bind(r, |b: &mut Bar, cx, r| {
            let (lo, hi) = bounds(&r);
            b.set_range(cx, lo, hi);
        })
    }

    /// Normal, symmetrical (from zero) or range (from [`start_value`](Self::start_value)).
    #[must_use]
    pub fn mode(self, m: impl IntoProp<BarMode>) -> Self {
        self.bind(m, |b: &mut Bar, cx, m| b.set_mode(cx, m))
    }

    /// The start value in [`BarMode::Range`] (applied with the value, after the range).
    #[must_use]
    pub fn start_value(mut self, v: impl IntoProp<i32>) -> Self {
        let v = v.into_prop();
        let cfg = self.shared::<BarCfg>();
        self.after_children(move |cx, node| {
            let anim = cfg.animated.get();
            bind_prop(cx, node, v, move |b: &mut Bar, wcx, v| {
                b.set_start_value(wcx, v, anim);
            });
        })
    }

    /// Horizontal, vertical or automatic (vertical when taller than wide).
    #[must_use]
    pub fn orientation(self, o: impl IntoProp<Orientation>) -> Self {
        self.bind(o, |b: &mut Bar, cx, o| b.set_orientation(cx, o))
    }

    /// Animates value changes over `d` (sets the local `anim_duration` style, and the value
    /// bindings call `set_value(v, anim = true)`).
    #[must_use]
    pub fn animated(mut self, d: Duration) -> Self {
        self.shared::<BarCfg>().animated.set(true);
        self.style_prop(d, |d: Duration| StyleProp::AnimDuration(d.as_millis_u32()))
    }
}

// ---- Slider ---------------------------------------------------------------------------------

/// A slider editing `value`: a plain value (the slider owns it; see
/// [`on_change`](WidgetView::on_change)) or a signal kept in sync both ways.
///
/// Dragging, keys and the encoder write the new value to the signal in the same update
/// (after the event, when the slider can be read); setting the signal moves the knob.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let level = cx.signal(40);
/// let _v = column((slider(level).range(0..=100), bar(level)));
/// cx.dispose();
/// ```
pub fn slider(value: impl IntoModel<i32>) -> WidgetView<Slider> {
    let model = value.into_model();
    widget_view(Slider::new).after_children(move |cx, node| {
        bind_model_synced(
            cx,
            node,
            model,
            |e, n, v| {
                e.with_widget_mut(n, |s: &mut Slider, wcx| s.set_value(wcx, v, false));
            },
            |e, n| e.widget::<Slider>(n).map(Slider::value),
        );
    })
}

impl WidgetView<Slider> {
    /// The value range.
    #[must_use]
    pub fn range(self, r: impl IntoProp<RangeInclusive<i32>>) -> Self {
        self.bind(r, |s: &mut Slider, cx, r| {
            let (lo, hi) = bounds(&r);
            s.set_range(cx, lo, hi);
        })
    }

    /// Normal, symmetrical or range (two knobs; see [`left_value`](Self::left_value)).
    #[must_use]
    pub fn mode(self, m: impl IntoProp<SliderMode>) -> Self {
        self.bind(m, |s: &mut Slider, cx, m| s.set_mode(cx, m))
    }

    /// The left knob's value in range mode, a plain value or a signal kept in sync both ways.
    #[must_use]
    pub fn left_value(self, v: impl IntoModel<i32>) -> Self {
        let model = v.into_model();
        self.after_children(move |cx, node| {
            bind_model_synced(
                cx,
                node,
                model,
                |e, n, v| {
                    e.with_widget_mut(n, |s: &mut Slider, wcx| s.set_left_value(wcx, v, false));
                },
                |e, n| e.widget::<Slider>(n).map(Slider::left_value),
            );
        })
    }

    /// Horizontal, vertical or automatic.
    #[must_use]
    pub fn orientation(self, o: impl IntoProp<Orientation>) -> Self {
        self.bind(o, |s: &mut Slider, cx, o| s.set_orientation(cx, o))
    }

    /// Called with the new value of the knob the user moved.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(i32) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |_, _, ev| event_value(ev), f))
    }
}

// ---- Switch ---------------------------------------------------------------------------------

/// An on/off switch editing `on` (a plain value or a signal kept in sync both ways).
///
/// A click animates the knob; changes of the signal jump (like LVGL's programmatic state
/// changes).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let wifi = cx.signal(true);
/// let _v = row((label("Wi-Fi"), switch(wifi)));
/// cx.dispose();
/// ```
pub fn switch(on: impl IntoModel<bool>) -> WidgetView<Switch> {
    let model = on.into_model();
    widget_view(Switch::new).after_children(move |cx, node| {
        bind_model(
            cx,
            node,
            model,
            |e, n, on| {
                e.with_widget_mut(n, |s: &mut Switch, wcx| s.set_checked(wcx, on, false));
            },
            |e, n, _| is_checked(e, n),
        );
    })
}

impl WidgetView<Switch> {
    /// Horizontal, vertical or automatic.
    #[must_use]
    pub fn orientation(self, o: impl IntoProp<Orientation>) -> Self {
        self.bind(o, |s: &mut Switch, cx, o| s.set_orientation(cx, o))
    }

    /// Called with the new state whenever the user toggles it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(bool) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |e, n, _| Some(is_checked(e, n)), f))
    }
}

// ---- Checkbox -------------------------------------------------------------------------------

/// A checkbox with a `text` (any [`IntoText`]) editing `checked` (a plain value or a signal
/// kept in sync both ways).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let agree = cx.signal(false);
/// let _v = checkbox("I agree", agree);
/// cx.dispose();
/// ```
pub fn checkbox(text: impl IntoText, checked: impl IntoModel<bool>) -> WidgetView<Checkbox> {
    let text = text.into_text();
    let model = checked.into_model();
    widget_view(|| Checkbox::new(""))
        .op(move |cx, node| {
            bind_str(
                cx,
                node,
                text,
                |e, n, s| {
                    e.with_widget_mut(n, |c: &mut Checkbox, wcx| c.set_text_static(wcx, s));
                },
                |e, n, s| {
                    e.with_widget_mut(n, |c: &mut Checkbox, wcx| c.set_text(wcx, s));
                },
            );
        })
        .after_children(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, on| {
                    e.with_widget_mut(n, |c: &mut Checkbox, wcx| c.set_checked(wcx, on));
                },
                |e, n, _| is_checked(e, n),
            );
        })
}

impl WidgetView<Checkbox> {
    /// Called with the new state whenever the user toggles it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(bool) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |e, n, _| Some(is_checked(e, n)), f))
    }
}

// ---- Arc ------------------------------------------------------------------------------------

/// An arc slider editing `value` (a plain value or a signal kept in sync both ways).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let volume = cx.signal(25);
/// let _v = arc(volume).range(0..=50).rotation(Angle::deg(135)).bg_angles(Angle::deg(0), Angle::deg(270));
/// cx.dispose();
/// ```
pub fn arc(value: impl IntoModel<i32>) -> WidgetView<Arc> {
    let model = value.into_model();
    widget_view(Arc::new).after_children(move |cx, node| {
        bind_model(
            cx,
            node,
            model,
            |e, n, v| {
                e.with_widget_mut(n, |a: &mut Arc, wcx| a.set_value(wcx, v));
            },
            |e, n, ev| {
                event_value(ev)
                    .or_else(|| e.widget::<Arc>(n).map(Arc::value))
                    .unwrap_or_default()
            },
        );
    })
}

impl WidgetView<Arc> {
    /// The value range.
    #[must_use]
    pub fn range(self, r: impl IntoProp<RangeInclusive<i32>>) -> Self {
        self.bind(r, |a: &mut Arc, cx, r| {
            let (lo, hi) = bounds(&r);
            a.set_range(cx, lo, hi);
        })
    }

    /// The indicator's start and end angles (normally set from the value; for an arc used as
    /// a plain gauge).
    #[must_use]
    pub fn angles(self, start: impl IntoProp<Angle>, end: impl IntoProp<Angle>) -> Self {
        self.bind(start, |a: &mut Arc, cx, s| {
            let e = a.angle_end();
            a.set_angles(cx, s, e);
        })
        .bind(end, |a: &mut Arc, cx, e| {
            let s = a.angle_start();
            a.set_angles(cx, s, e);
        })
    }

    /// The background arc's start and end angles.
    #[must_use]
    pub fn bg_angles(self, start: impl IntoProp<Angle>, end: impl IntoProp<Angle>) -> Self {
        self.bind(start, |a: &mut Arc, cx, s| {
            let e = a.bg_angle_end();
            a.set_bg_angles(cx, s, e);
        })
        .bind(end, |a: &mut Arc, cx, e| {
            let s = a.bg_angle_start();
            a.set_bg_angles(cx, s, e);
        })
    }

    /// Normal, symmetrical or reverse (the value grows counter-clockwise).
    #[must_use]
    pub fn mode(self, m: impl IntoProp<ArcMode>) -> Self {
        self.bind(m, |a: &mut Arc, cx, m| a.set_mode(cx, m))
    }

    /// The rotation of the whole arc (0° = 3 o'clock, clockwise).
    #[must_use]
    pub fn rotation(self, r: impl IntoProp<Angle>) -> Self {
        self.bind(r, |a: &mut Arc, cx, r| a.set_rotation(cx, r))
    }

    /// `false` removes the knob's styles and makes the arc not clickable (a display-only
    /// arc, LVGL `lv_obj_remove_style(arc, NULL, LV_PART_KNOB)`).
    #[must_use]
    pub fn knob(self, on: bool) -> Self {
        if on {
            return self;
        }
        self.op(|cx, node| {
            let e = cx.engine();
            e.remove_style(node, None, Some(Selector::part(Part::Knob)));
            e.set_flag(node, ObjFlags::CLICKABLE, false);
        })
    }

    /// The maximum drag speed in degrees per second (LVGL default 720).
    #[must_use]
    pub fn change_rate(self, deg_per_s: impl IntoProp<u16>) -> Self {
        self.bind(deg_per_s, |a: &mut Arc, cx, r| a.set_change_rate(cx, r))
    }

    /// Called with the new value whenever the user changes it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(i32) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |_, _, ev| event_value(ev), f))
    }
}

// ---- LED ------------------------------------------------------------------------------------

/// Settings of a [`led`] view shared by its bindings.
struct LedCfg {
    on: Cell<bool>,
    bright: Cell<u8>,
}

impl Default for LedCfg {
    fn default() -> Self {
        LedCfg {
            on: Cell::new(true),
            bright: Cell::new(LED_BRIGHT_MAX),
        }
    }
}

/// A LED that is on while `on` is `true` (at [`brightness`](WidgetView::brightness), full
/// by default) and dimmed to the minimum brightness otherwise.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let alarm = cx.signal(false);
/// let _v = led(alarm).color(Color::RED).brightness(200);
/// cx.dispose();
/// ```
pub fn led(on: impl IntoProp<bool>) -> WidgetView<Led> {
    let on = on.into_prop();
    let mut v = widget_view(Led::new);
    let cfg = v.shared::<LedCfg>();
    v.after_children(move |cx, node| {
        bind_prop(cx, node, on, move |l: &mut Led, wcx, on| {
            cfg.on.set(on);
            if on {
                l.set_brightness(wcx, cfg.bright.get());
            } else {
                l.off(wcx);
            }
        });
    })
}

impl WidgetView<Led> {
    /// The LED's color.
    #[must_use]
    pub fn color(self, c: impl IntoProp<Color>) -> Self {
        self.bind(c, |l: &mut Led, cx, c| l.set_color(cx, c))
    }

    /// The brightness while on (80…255, LVGL's `LV_LED_BRIGHT_MIN`…`MAX`).
    #[must_use]
    pub fn brightness(mut self, b: impl IntoProp<u8>) -> Self {
        let cfg = self.shared::<LedCfg>();
        self.bind(b, move |l: &mut Led, cx, b| {
            cfg.bright.set(b);
            if cfg.on.get() {
                l.set_brightness(cx, b);
            }
        })
    }
}

// ---- Line -----------------------------------------------------------------------------------

/// A polyline through `points` (a constant, a signal, a memo or a closure of `Vec<Point>`).
/// For points in flash use [`line_static`].
///
/// Note: on a line view [`width`](WidgetView::width) is the stroke width; size the widget
/// with [`size`](ViewExt::size) (or let it take its content size).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let pts = cx.signal(vec![Point::new(0, 20), Point::new(30, 0), Point::new(60, 20)]);
/// let _v = line(pts).width(3).rounded(true);
/// cx.dispose();
/// ```
pub fn line(points: impl IntoProp<Vec<Point>>) -> WidgetView<Line> {
    widget_view(Line::new).bind(points, |l: &mut Line, cx, p: Vec<Point>| l.set_points(cx, &p))
}

/// A polyline through `'static` points (no copy).
///
/// ```
/// use twine_view::prelude::*;
///
/// static ZIGZAG: [Point; 3] = [Point::new(0, 0), Point::new(20, 20), Point::new(40, 0)];
/// let _v = line_static(&ZIGZAG).dash(4, 2);
/// ```
pub fn line_static(points: &'static [Point]) -> WidgetView<Line> {
    widget_view(Line::new).op(move |cx, node| {
        cx.engine()
            .with_widget_mut(node, |l: &mut Line, wcx| l.set_points_static(wcx, points));
    })
}

impl WidgetView<Line> {
    /// Mirrors the y coordinates (y grows upwards).
    #[must_use]
    pub fn y_invert(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |l: &mut Line, cx, on| l.set_y_invert(cx, on))
    }

    /// The stroke width (`line_width`). This shadows [`ViewExt::width`] on line views.
    #[must_use]
    pub fn width(self, w: impl IntoProp<i32>) -> Self {
        self.style_prop(w, StyleProp::LineWidth)
    }

    /// Rounded line ends and joints (`line_rounded`).
    #[must_use]
    pub fn rounded(self, on: impl IntoProp<bool>) -> Self {
        self.style_prop(on, StyleProp::LineRounded)
    }

    /// A dashed line: `w` px dashes with `gap` px gaps.
    #[must_use]
    pub fn dash(self, w: impl IntoProp<i32>, gap: impl IntoProp<i32>) -> Self {
        self.style_prop(w, StyleProp::LineDashWidth)
            .style_prop(gap, StyleProp::LineDashGap)
    }
}

// ---- Spinner --------------------------------------------------------------------------------

/// The loading spinner (an arc chasing around a ring; not clickable).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = spinner().period(Duration::ms(1500)).arc_angle(Angle::deg(120)).size(48, 48);
/// ```
#[must_use]
pub fn spinner() -> WidgetView<Spinner> {
    widget_view(Spinner::new)
}

impl WidgetView<Spinner> {
    /// The time of one turn (default 1 s).
    #[must_use]
    pub fn period(self, d: impl IntoProp<Duration>) -> Self {
        self.bind(d, |s: &mut Spinner, cx, d| s.set_period(cx, d))
    }

    /// The length of the chasing arc (default 200°).
    #[must_use]
    pub fn arc_angle(self, a: impl IntoProp<Angle>) -> Self {
        self.bind(a, |s: &mut Spinner, cx, a| s.set_arc_sweep(cx, a))
    }
}
