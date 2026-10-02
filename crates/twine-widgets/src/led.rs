//! [`Led`]: a light-emitting diode (LVGL `lv_led`).

use alloc::boxed::Box;

use twine_core::{Color, Fraction, Opa};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, NodeId, OBJ_FLAGS, Widget,
    WidgetClass, WidgetCx,
};
use twine_render::{GradStop, Gradient, MAX_STOPS};
use twine_style::Part;
use twine_style::design::{self, ColorValue};

use crate::log_set;
use crate::util::{self, log_value};

/// LVGL `LV_LED_BRIGHT_MIN` (80 of 255): the brightness of an LED that is off.
pub const LED_BRIGHT_MIN: Fraction = Fraction::from_raw(80);
/// LVGL `LV_LED_BRIGHT_MAX` (255 of 255): the brightness of an LED that is on.
pub const LED_BRIGHT_MAX: Fraction = Fraction::ONE;
/// LVGL `lv_led_class.width_def` / `height_def`: `LV_DPI_DEF / 5`.
pub const LED_DEFAULT_SIZE: i32 = util::DPI_DEF / 5;
/// The color an LED shows when its color is a design element its display's theme does not
/// define (e.g. no theme): LVGL's default primary color (blue 500, `#2196F3`).
pub const LED_DEFAULT_COLOR: Color = Color::hex(0x0021_96F3);

/// The class of [`Led`]: `"led"`, part `Main`, the base object's flags (LVGL `lv_led_class`).
pub static LED_CLASS: WidgetClass = WidgetClass::new("led").default_flags(OBJ_FLAGS);

/// An LED (LVGL `lv_led`): a circle (with the default theme) whose background, border,
/// outline and shadow colors take the LED's color, darkened by its brightness; the shadow
/// ("glow") shrinks with the brightness.
///
/// The color is a [`ColorValue`]: by default the theme's [`design::PRIMARY`] (LVGL's
/// `lv_theme_get_color_primary`), so an LED follows the theme mode (light, dark, night, high
/// contrast) like every styled color; set a fixed color or another element (e.g.
/// `design::DANGER`) with [`set_color`](Self::set_color). The element is resolved when the LED
/// is created and again on every `StyleChanged` (a theme or mode switch), never while drawing.
///
/// Brightness runs from [`LED_BRIGHT_MIN`] (off) to [`LED_BRIGHT_MAX`] (on). Each style color
/// is first replaced by the LED color scaled by that color's own brightness (so a white
/// style gives the pure LED color), then mixed with black by the LED's brightness — exactly
/// LVGL's `lv_led_event` `DRAW_MAIN`.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::led::{self, Led, LED_BRIGHT_MIN};
///
/// let mut h = EngineHarness::new(60, 60);
/// let screen = h.screen();
/// let l = led::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(l, |w: &mut Led, cx| w.off(cx));
/// assert_eq!(h.engine().widget::<Led>(l).unwrap().brightness(), LED_BRIGHT_MIN);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Led {
    /// The color as set (a fixed color or a design element).
    color: ColorValue,
    /// `color` resolved for the LED's display (what is drawn).
    resolved: Color,
    bright: u8,
}

impl Default for Led {
    fn default() -> Self {
        Self::new()
    }
}

/// LVGL `lv_color_brightness`: `(3 r + g + 4 b) / 8`.
#[must_use]
pub const fn color_brightness(c: Color) -> u8 {
    ((3 * c.r as u16 + c.g as u16 + 4 * c.b as u16) >> 3) as u8
}

impl Led {
    /// An LED, on, in the theme's [`design::PRIMARY`] (LVGL `lv_led_constructor`); it shows
    /// [`LED_DEFAULT_COLOR`] until it is created (and where the theme does not define the
    /// element).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color: ColorValue::Element(design::PRIMARY),
            resolved: LED_DEFAULT_COLOR,
            bright: LED_BRIGHT_MAX.raw(),
        }
    }

    /// The color as drawn: [`color_value`](Self::color_value) resolved for the LED's display.
    #[must_use]
    pub fn color(&self) -> Color {
        self.resolved
    }

    /// The color as set: a fixed color or a design element (default [`design::PRIMARY`]).
    #[must_use]
    pub fn color_value(&self) -> ColorValue {
        self.color
    }

    /// Resolves the color for the display of `node` ([`LED_DEFAULT_COLOR`] for an element
    /// the display's theme does not define). Returns whether the drawn color changed.
    fn resolve(&mut self, engine: &Engine, node: NodeId) -> bool {
        let c = engine
            .resolve_design_value(node, self.color)
            .unwrap_or(LED_DEFAULT_COLOR);
        let changed = c != self.resolved;
        self.resolved = c;
        changed
    }

    /// The brightness (`LED_BRIGHT_MIN ..= LED_BRIGHT_MAX`).
    #[must_use]
    pub fn brightness(&self) -> Fraction {
        Fraction::from_raw(self.bright)
    }

    /// Whether the LED is brighter than halfway (LVGL `lv_led_toggle`'s test).
    #[must_use]
    pub fn is_on(&self) -> bool {
        u16::from(self.bright)
            > u16::midpoint(u16::from(LED_BRIGHT_MIN.raw()), u16::from(LED_BRIGHT_MAX.raw()))
    }

    /// Sets the color: a [`Color`] or a design element such as `design::DANGER` (followed
    /// through theme mode switches). Idempotent; redraws only when the drawn color changes.
    ///
    /// ```
    /// use twine_core::Color;
    /// use twine_style::design;
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::led::{self, Led};
    ///
    /// let mut h = EngineHarness::new(60, 60);
    /// let screen = h.screen();
    /// let l = led::create(h.engine_mut(), screen).unwrap();
    /// h.engine_mut().with_widget_mut(l, |w: &mut Led, cx| w.set_color(cx, Color::RED));
    /// assert_eq!(h.engine().widget::<Led>(l).unwrap().color(), Color::RED);
    /// h.engine_mut().with_widget_mut(l, |w: &mut Led, cx| w.set_color(cx, design::DANGER));
    /// assert_eq!(h.engine().widget::<Led>(l).unwrap().color_value(), design::DANGER.into());
    /// ```
    pub fn set_color(&mut self, cx: &mut WidgetCx<'_>, c: impl Into<ColorValue>) {
        let c = c.into();
        if self.color == c {
            return;
        }
        log_set(LED_CLASS.name, cx.node(), "color");
        self.color = c;
        let node = cx.node();
        if self.resolve(cx.engine(), node) {
            cx.invalidate_for("led.color");
        }
    }

    /// Sets the brightness, clamped to `LED_BRIGHT_MIN ..= LED_BRIGHT_MAX` (LVGL's 80…255 of
    /// 255: an LED is never fully black). Idempotent.
    pub fn set_brightness(&mut self, cx: &mut WidgetCx<'_>, b: Fraction) {
        let b = b.raw().clamp(LED_BRIGHT_MIN.raw(), LED_BRIGHT_MAX.raw());
        if self.bright == b {
            return;
        }
        log_set(LED_CLASS.name, cx.node(), "brightness");
        self.bright = b;
        log_value(LED_CLASS.name, i32::from(b));
        cx.invalidate_for("led.brightness");
    }

    /// Full brightness (LVGL `lv_led_on`).
    pub fn on(&mut self, cx: &mut WidgetCx<'_>) {
        self.set_brightness(cx, LED_BRIGHT_MAX);
    }

    /// Minimal brightness (LVGL `lv_led_off`).
    pub fn off(&mut self, cx: &mut WidgetCx<'_>) {
        self.set_brightness(cx, LED_BRIGHT_MIN);
    }

    /// Off when brighter than halfway, else on (LVGL `lv_led_toggle`).
    pub fn toggle(&mut self, cx: &mut WidgetCx<'_>) {
        if self.is_on() {
            self.off(cx);
        } else {
            self.on(cx);
        }
    }

    /// A style color turned into the LED's color at this brightness.
    fn tint(self, c: Color) -> Color {
        let c = Color::mix(self.resolved, Color::BLACK, Opa::from_raw(color_brightness(c)));
        Color::mix(c, Color::BLACK, Opa::from_raw(self.bright))
    }
}

/// Creates an LED as the last child of `parent` (LVGL `lv_led_create`), 26 × 26 px.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Led::new()))
}

impl Widget for Led {
    fn class(&self) -> &'static WidgetClass {
        &LED_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        // LVGL `lv_led_constructor`: the theme's primary color (an element, resolved here).
        self.resolve(cx.engine(), id);
        cx.engine_mut().set_size(id, LED_DEFAULT_SIZE, LED_DEFAULT_SIZE);
    }

    /// A theme or theme mode switch (`StyleChanged`) resolves the color again.
    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.code == EventCode::StyleChanged && ev.target == cx.node() {
            let node = cx.node();
            if self.resolve(cx.engine(), node) {
                cx.widget_cx().invalidate_for("led.color");
            }
        }
        EventResult::Continue
    }

    /// LVGL `lv_led_event` `DRAW_MAIN` (it replaces the base object's drawing).
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let rs = cx.rect_dsc(Part::Main);
        let mut d = rs.dsc();
        d.bg_color = self.tint(d.bg_color);
        let grad = d.bg_grad.map(|g| {
            let src = g.stops();
            let mut stops = [GradStop::new(Color::BLACK, Fraction::ZERO); MAX_STOPS];
            stops[..src.len()].copy_from_slice(src);
            for s in stops.iter_mut().take(2.min(src.len())) {
                s.color = self.tint(s.color);
            }
            let mut n = Gradient::new(g.kind, &stops[..src.len()]);
            n.extend = g.extend;
            n.dither = g.dither;
            n
        });
        d.bg_grad = grad.as_ref();
        d.shadow.color = self.tint(d.shadow.color);
        d.border_color = self.tint(d.border_color);
        d.outline_color = self.tint(d.outline_color);
        let span = i32::from(LED_BRIGHT_MAX.raw() - LED_BRIGHT_MIN.raw());
        let b = i32::from(self.bright - LED_BRIGHT_MIN.raw());
        d.shadow.width = b * d.shadow.width / span;
        d.shadow.spread = b * d.shadow.spread / span;
        let area = cx.engine().draw_area(cx.node());
        cx.painter().rect(area, &d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_matches_lvgl() {
        assert_eq!(color_brightness(Color::WHITE), 255);
        assert_eq!(color_brightness(Color::BLACK), 0);
        assert_eq!(color_brightness(Color::new(255, 0, 0)), 95);
        let l = Led::new();
        assert_eq!(l.tint(Color::WHITE), LED_DEFAULT_COLOR);
    }
}
