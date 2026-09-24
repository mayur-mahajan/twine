//! # twine-widgets
//!
//! The basic widgets of the Twine GUI library, each a port of the LVGL widget of the same
//! name:
//!
//! | Widget | LVGL | Parts |
//! |--------|------|-------|
//! | [`Container`](container::Container) | `lv_obj` | `Main`, `Scrollbar` |
//! | [`Label`](label::Label) | `lv_label` | `Main`, `Scrollbar`, `Selected` |
//! | [`Button`](button::Button) | `lv_button` | `Main`, `Scrollbar` |
//! | [`Image`](image::Image) | `lv_image` | `Main` |
//! | [`Bar`](bar::Bar) | `lv_bar` | `Main`, `Indicator` |
//! | [`Slider`](slider::Slider) | `lv_slider` | `Main`, `Indicator`, `Knob` |
//! | [`Switch`](switch::Switch) | `lv_switch` | `Main`, `Indicator`, `Knob` |
//! | [`Checkbox`](checkbox::Checkbox) | `lv_checkbox` | `Main`, `Indicator` |
//! | [`Arc`](arc::Arc) | `lv_arc` | `Main`, `Indicator`, `Knob` |
//! | [`Spinner`](spinner::Spinner) | `lv_spinner` | `Main`, `Indicator`, `Knob` |
//! | [`Led`](led::Led) | `lv_led` | `Main` |
//! | [`Line`](line::Line) | `lv_line` | `Main` |
//! | [`ImageButton`](image_button::ImageButton) | `lv_imagebutton` | `Main` |
//! | [`AnimImg`](animimg::AnimImg) | `lv_animimg` | `Main` |
//! | [`ButtonMatrix`](buttonmatrix::ButtonMatrix) | `lv_buttonmatrix` | `Main`, `Items` |
//! | [`Keyboard`](keyboard::Keyboard) | `lv_keyboard` | `Main`, `Items` |
//! | [`Textarea`](textarea::Textarea) | `lv_textarea` | `Main`, `Scrollbar`, `Selected`, `Cursor`, [`PLACEHOLDER`](textarea::PLACEHOLDER) |
//! | [`Spinbox`](spinbox::Spinbox) | `lv_spinbox` | as the textarea |
//! | [`SpanGroup`](spangroup::SpanGroup) | `lv_spangroup` | `Main` |
//!
//! They are
//! [`Widget`](twine_engine::Widget)s of `twine-engine`, usable imperatively (LVGL-style) and
//! wrapped by the declarative view layer.
//!
//! ## Conventions (every widget follows them)
//!
//! - A widget module `x` has `pub struct X` (the widget state), `pub static X_CLASS:
//!   WidgetClass` (name, parts, default flags, focus group default, as LVGL's
//!   `lv_x_class`) and `pub fn create(engine, parent) -> Result<NodeId, EngineError>`, the
//!   imperative constructor (LVGL `lv_x_create`): it creates the node, the display's theme
//!   styles it, then `Widget::init` runs.
//! - Setters are `pub fn set_*(&mut self, cx: &mut WidgetCx<'_>, v)` and are **idempotent**:
//!   setting the value a widget already has does nothing — no invalidation, no layout (P3).
//!   A change invalidates only the widget's own area (P2) and marks the layout only when the
//!   widget's content size can change. Call them through
//!   [`Engine::with_widget_mut`](twine_engine::Engine::with_widget_mut):
//!   `engine.with_widget_mut(id, |l: &mut Label, cx| l.set_text(cx, "Hi"))`.
//! - Getters take `&self` (and a [`MeasureCx`](twine_engine::MeasureCx) when they need
//!   styles or geometry).
//! - Every setter that changes something logs
//!   `trace!(target: "twine::engine", "<class>#<node> set_<name>")`.
//! - Look comes from styles only: widgets read their parts' resolved styles (themes set
//!   the LVGL look, applications override with styles or local properties).
//! - Invalid input never panics in release builds: it logs `warn!` and is ignored (P7).
//!
//! ## Example
//!
//! ```
//! use twine_engine::Engine;
//! use twine_testing::EngineHarness;
//! use twine_widgets::prelude::*;
//!
//! let mut h = EngineHarness::new(120, 60);
//! let screen = h.screen();
//! let e = h.engine_mut();
//! let btn = button::create(e, screen).unwrap();
//! let lbl = label::create(e, btn).unwrap();
//! e.with_widget_mut(lbl, |l: &mut Label, cx| l.set_text(cx, "OK"));
//! e.align(lbl, twine_style::Align::Center, 0, 0);
//! h.run_until_idle();
//! assert_eq!(h.engine().widget::<Label>(lbl).unwrap().text(), "OK");
//! ```
//!
//! Value widgets (slider, arc, spinbox, button matrix) send `ValueChanged` with the new value
//! (the button index for a button matrix) as
//! [`EventParam::Value`](twine_engine::EventParam::Value): the widget itself is busy handling
//! the input while its handlers run. A textarea's handlers read its text with
//! [`textarea::text_of`] (from the label child that holds it). Checkable widgets (switch, checkbox) use the engine's
//! `State::CHECKED`; set it from code with their `set_checked` so they redraw.
//!
//! ## Features
//!
//! - `gif`: [`AnimImg::from_gif`](animimg::AnimImg) plays animated GIFs.
//! - `std`, `log`, `defmt`: as in every Twine crate.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;

pub mod animimg;
pub mod arc;
pub mod bar;
pub mod button;
pub mod buttonmatrix;
pub mod checkbox;
pub mod container;
pub mod image;
pub mod image_button;
pub mod keyboard;
pub mod label;
pub mod led;
pub mod line;
pub mod prelude;
pub mod slider;
pub mod spangroup;
pub mod spinbox;
pub mod spinner;
mod state_dsc;
pub mod switch;
pub mod textarea;
mod util;

pub use twine_text::LongMode;

use twine_engine::{NodeId, fmt_node_id};

/// The direction of a bar, slider or switch (LVGL `lv_bar_orientation_t`,
/// `lv_slider_orientation_t`, `lv_switch_orientation_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Orientation {
    /// Vertical when the widget is taller than wide, else horizontal.
    #[default]
    Auto,
    /// Left to right (right to left with an RTL base direction).
    Horizontal,
    /// Bottom to top.
    Vertical,
}

/// Logs a setter change (`trace!` at `twine::engine`).
#[inline]
pub(crate) fn log_set(class: &str, id: NodeId, name: &str) {
    twine_core::trace!(target: "twine::engine", "{}#{} set_{}", class, fmt_node_id(id), name);
    let _ = (class, id, name);
}
