//! [`canvas`]: one small custom widget behind all of the player's hand-drawn graphics — the
//! album artwork, the visualizer, the equalizer's response curve and the level meters.
//!
//! **The pattern.** A canvas holds a small `Copy` value `T` and a paint function
//! `fn(T, &mut DrawCx)`.
//! The value comes from a signal or memo through an ordinary view binding; the setter
//! compares it with the current one and invalidates the node only when it differs (Twine's
//! idempotent-setter rule), so a visualizer frame that did not change redraws nothing, and a
//! paused player costs nothing at all. Drawing is a pure function of the value: no state, no
//! allocation — every figure is computed into fixed-size arrays and handed to the renderer.
//!
//! Why not one widget class per graphic: they would differ only in their data and paint
//! function, and none needs input handling or theming of its own (clicks go to the enclosing
//! view, the palette is the app's). A generic canvas keeps each graphic to its drawing code.

use std::fmt;

use twine::prelude::*;

/// The canvas class: just `Main`; not clickable or scrollable (input goes to the parent), no
/// base class (themes leave it alone: the paint function draws everything).
pub static CANVAS_CLASS: WidgetClass = WidgetClass::new("canvas");

/// A widget drawing `data` with `paint`.
pub struct Canvas<T> {
    data: T,
    paint: fn(T, &mut DrawCx<'_, '_>),
}

impl<T> fmt::Debug for Canvas<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Canvas")
    }
}

impl<T: Copy + PartialEq + 'static> Canvas<T> {
    /// Sets the value; redraws only when it changed.
    fn set(&mut self, cx: &mut WidgetCx<'_>, data: T) {
        if self.data != data {
            self.data = data;
            cx.invalidate_for("canvas.data");
        }
    }
}

impl<T: Copy + 'static> Widget for Canvas<T> {
    fn class(&self) -> &'static WidgetClass {
        &CANVAS_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        (self.paint)(self.data, cx);
    }
}

/// A canvas showing `data` (a constant, a signal, a memo or a closure), drawn by `paint`
/// over the node's area. Size it like any view.
pub fn canvas<T, M>(data: impl IntoProp<T, M>, paint: fn(T, &mut DrawCx<'_, '_>)) -> WidgetView<Canvas<T>>
where
    T: Copy + Default + PartialEq + 'static,
{
    widget_view(move || Canvas {
        data: T::default(),
        paint,
    })
    .bind(data, Canvas::set)
}
