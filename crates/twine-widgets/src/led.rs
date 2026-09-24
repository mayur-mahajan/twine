//! [`Led`]: a light-emitting diode (LVGL `lv_led`).

use alloc::boxed::Box;

use twine_core::{Color, Opa};
use twine_engine::{DrawCx, Engine, EngineError, NodeId, OBJ_FLAGS, Widget, WidgetClass, WidgetCx};
use twine_render::{GradStop, Gradient, MAX_STOPS};
use twine_style::Part;

use crate::log_set;
use crate::util::{self, log_value};

/// LVGL `LV_LED_BRIGHT_MIN`: the brightness of an LED that is off.
pub const LED_BRIGHT_MIN: u8 = 80;
/// LVGL `LV_LED_BRIGHT_MAX`: the brightness of an LED that is on.
pub const LED_BRIGHT_MAX: u8 = 255;
/// LVGL `lv_led_class.width_def` / `height_def`: `LV_DPI_DEF / 5`.
pub const LED_DEFAULT_SIZE: i32 = util::DPI_DEF / 5;
/// The LED color without a theme: LVGL's default primary color (blue 500). With a theme a new
/// LED takes the theme's primary color (LVGL `lv_theme_get_color_primary`).
pub const LED_DEFAULT_COLOR: Color = twine_engine::DEFAULT_COLOR_PRIMARY;

/// The class of [`Led`]: `"led"`, part `Main`, the base object's flags (LVGL `lv_led_class`).
pub static LED_CLASS: WidgetClass = WidgetClass::new("led").default_flags(OBJ_FLAGS);

/// An LED (LVGL `lv_led`): a circle (with the default theme) whose background, border,
/// outline and shadow colors take the LED's color, darkened by its brightness; the shadow
/// ("glow") shrinks with the brightness.
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
    color: Color,
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
    /// An LED, on, in [`LED_DEFAULT_COLOR`] until it is created: then it takes the primary
    /// color of its display's theme (LVGL `lv_led_constructor`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color: LED_DEFAULT_COLOR,
            bright: LED_BRIGHT_MAX,
        }
    }

    /// The color.
    #[must_use]
    pub fn color(&self) -> Color {
        self.color
    }

    /// The brightness (`LED_BRIGHT_MIN ..= LED_BRIGHT_MAX`).
    #[must_use]
    pub fn brightness(&self) -> u8 {
        self.bright
    }

    /// Whether the LED is brighter than halfway (LVGL `lv_led_toggle`'s test).
    #[must_use]
    pub fn is_on(&self) -> bool {
        u16::from(self.bright) > u16::midpoint(u16::from(LED_BRIGHT_MIN), u16::from(LED_BRIGHT_MAX))
    }

    /// Sets the color. Idempotent.
    pub fn set_color(&mut self, cx: &mut WidgetCx<'_>, c: Color) {
        if self.color == c {
            return;
        }
        log_set(LED_CLASS.name, cx.node(), "color");
        self.color = c;
        cx.invalidate_for("led.color");
    }

    /// Sets the brightness, clamped to `LED_BRIGHT_MIN ..= LED_BRIGHT_MAX`. Idempotent.
    pub fn set_brightness(&mut self, cx: &mut WidgetCx<'_>, b: u8) {
        let b = b.clamp(LED_BRIGHT_MIN, LED_BRIGHT_MAX);
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
        let c = Color::mix(self.color, Color::BLACK, Opa(color_brightness(c)));
        Color::mix(c, Color::BLACK, Opa(self.bright))
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
        // LVGL `lv_led_constructor`: the theme's primary color.
        self.color = cx.engine().color_primary(id);
        cx.engine_mut().set_size(id, LED_DEFAULT_SIZE, LED_DEFAULT_SIZE);
    }

    /// LVGL `lv_led_event` `DRAW_MAIN` (it replaces the base object's drawing).
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let rs = cx.rect_dsc(Part::Main);
        let mut d = rs.dsc();
        d.bg_color = self.tint(d.bg_color);
        let grad = d.bg_grad.map(|g| {
            let src = g.stops();
            let mut stops = [GradStop::new(Color::BLACK, 0); MAX_STOPS];
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
        let span = i32::from(LED_BRIGHT_MAX - LED_BRIGHT_MIN);
        let b = i32::from(self.bright - LED_BRIGHT_MIN);
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
