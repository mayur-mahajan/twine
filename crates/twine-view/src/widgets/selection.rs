//! Pickers: [`dropdown`] and [`roller`].

use core::cell::RefCell;

use twine_style::Side;
use twine_widgets_ext::dropdown::Dropdown;
use twine_widgets_ext::roller::{Roller, RollerMode};

use crate::bind::bind_prop;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, event_value, on_value_changed};
use crate::prop::{IntoIcon, IntoProp, Prop};
use crate::text::{IntoOptions, IntoText, bind_str};

/// Wires the selected index of a dropdown or roller: shown after the other settings, read
/// back from `ValueChanged`'s `EventParam::Value`.
fn bind_selected<W: twine_engine::Widget>(
    v: WidgetView<W>,
    selected: impl IntoModel<usize>,
    set: fn(&mut W, &mut twine_engine::WidgetCx<'_>, usize),
    get: fn(&W) -> usize,
) -> WidgetView<W> {
    let model = selected.into_model();
    v.after_children(move |cx, node| {
        bind_model(
            cx,
            node,
            model,
            move |e, n, idx| {
                e.with_widget_mut(n, |w: &mut W, wcx| set(w, wcx, idx));
            },
            move |e, n, ev| {
                event_value(ev)
                    .and_then(|v| usize::try_from(v).ok())
                    .or_else(|| e.widget::<W>(n).map(get))
                    .unwrap_or_default()
            },
        );
    })
}

// ---- Dropdown -------------------------------------------------------------------------------

/// A dropdown choosing one of `options` (any [`IntoOptions`]: an array, a `Vec` or an
/// iterator of texts, or a signal, memo or closure of a `Vec` of strings), with the index of
/// the chosen option in `selected` (a plain value or a signal kept in sync both ways).
///
/// When the options change, the selected index is kept (clamped to the new options).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn picker(cx: Scope) -> impl View {
///     let city = cx.signal(0usize);
///     column((
///         dropdown(["Berlin", "Paris", "Rome"], city).dir(Side::Bottom),
///         label(text!("Selected: {}", city.get())),
///     ))
/// }
/// # let _ = picker;
/// ```
pub fn dropdown(options: impl IntoOptions, selected: impl IntoModel<usize>) -> WidgetView<Dropdown> {
    let options = options.into_options();
    let v = widget_view(Dropdown::new).op(move |cx, node| {
        bind_str(
            cx,
            node,
            options,
            |e, n, s| {
                e.with_widget_mut(n, |d: &mut Dropdown, wcx| {
                    let keep = d.selected();
                    d.set_options_static(wcx, s);
                    d.set_selected(wcx, keep);
                });
            },
            |e, n, s| {
                e.with_widget_mut(n, |d: &mut Dropdown, wcx| {
                    let keep = d.selected();
                    d.set_options(wcx, s);
                    d.set_selected(wcx, keep);
                });
            },
        );
    });
    bind_selected(v, selected, set_dropdown_selected, |d| usize::from(d.selected()))
}

fn set_dropdown_selected(d: &mut Dropdown, cx: &mut twine_engine::WidgetCx<'_>, idx: usize) {
    d.set_selected(cx, u16::try_from(idx).unwrap_or(u16::MAX));
}

impl WidgetView<Dropdown> {
    /// The side the list opens to ([`Side::Bottom`] by default). It flips to the opposite
    /// side when that has more room.
    #[must_use]
    pub fn dir(self, dir: impl IntoProp<Side>) -> Self {
        self.bind(dir, |d: &mut Dropdown, cx, dir| d.set_dir(cx, dir))
    }

    /// The symbol at the side ([`Symbol::Down`](twine_text::Symbol::Down) by default): a
    /// [`Symbol`](twine_text::Symbol), an image, or `()` for none (any [`IntoIcon`]).
    #[must_use]
    pub fn symbol(self, icon: impl IntoIcon) -> Self {
        self.bind(icon.into_icon(), |d: &mut Dropdown, cx, s| d.set_symbol(cx, s))
    }

    /// A fixed text on the button instead of the selected option (e.g. a menu title; any
    /// [`IntoText`]).
    #[must_use]
    pub fn text(self, text: impl IntoText) -> Self {
        let text = text.into_text();
        self.op(move |cx, node| {
            bind_str(
                cx,
                node,
                text,
                |e, n, s| {
                    e.with_widget_mut(n, |d: &mut Dropdown, wcx| d.set_text_static(wcx, Some(s)));
                },
                |e, n, s| {
                    e.with_widget_mut(n, |d: &mut Dropdown, wcx| d.set_text(wcx, Some(s)));
                },
            );
        })
    }

    /// Highlights the selected option in the open list (on by default).
    #[must_use]
    pub fn highlight(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |d: &mut Dropdown, cx, on| d.set_selected_highlight(cx, on))
    }

    /// Called with the index of the option the user chose.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(usize) + 'static) -> Self {
        self.op(move |cx, node| {
            on_value_changed(
                cx,
                node,
                |_, _, ev| event_value(ev).and_then(|v| usize::try_from(v).ok()),
                f,
            );
        })
    }
}

// ---- Roller ---------------------------------------------------------------------------------

/// Settings of a roller view read when it is built (the mode applies to the options).
#[derive(Default)]
struct RollerSettings {
    mode: RefCell<Option<Prop<RollerMode>>>,
}

/// A roller choosing one of `options` (any [`IntoOptions`], like [`dropdown`]), with the
/// index of the chosen option in `selected` (a plain value or a signal kept in sync both
/// ways; the roller rolls to it with its animation).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn hours(cx: Scope) -> impl View {
///     let h = cx.signal(7usize);
///     roller((0..24).map(|i| format!("{i:02}")), h).mode(RollerMode::Infinite).visible_rows(3)
/// }
/// # let _ = hours;
/// ```
pub fn roller(options: impl IntoOptions, selected: impl IntoModel<usize>) -> WidgetView<Roller> {
    let options = options.into_options();
    let mut v = widget_view(Roller::new);
    let settings = v.shared::<RollerSettings>();
    let v = v.op(move |cx, node| {
        // The mode first (cheap on a roller without options), so the options are laid out
        // once, in their mode.
        if let Some(mode) = settings.mode.borrow_mut().take() {
            bind_prop(cx, node, mode, |r: &mut Roller, wcx, m| r.set_mode(wcx, m));
        }
        bind_str(
            cx,
            node,
            options,
            |e, n, s| {
                e.with_widget_mut(n, |r: &mut Roller, wcx| {
                    let keep = r.selected();
                    let mode = r.mode();
                    r.set_options_static(wcx, s, mode);
                    r.set_selected(wcx, keep, false);
                });
            },
            |e, n, s| {
                e.with_widget_mut(n, |r: &mut Roller, wcx| {
                    let keep = r.selected();
                    let mode = r.mode();
                    r.set_options(wcx, s, mode);
                    r.set_selected(wcx, keep, false);
                });
            },
        );
    });
    bind_selected(v, selected, set_roller_selected, |r| usize::from(r.selected()))
}

/// Model values roll to the option with the animation once the roller is shown (the first
/// value is applied at once).
fn set_roller_selected(r: &mut Roller, cx: &mut twine_engine::WidgetCx<'_>, idx: usize) {
    let anim = cx.coords().height() > 0;
    r.set_selected(cx, u16::try_from(idx).unwrap_or(u16::MAX), anim);
}

impl WidgetView<Roller> {
    /// Normal (ends at the first and last option) or infinite (the options repeat).
    #[must_use]
    pub fn mode(mut self, mode: impl IntoProp<RollerMode>) -> Self {
        *self.shared::<RollerSettings>().mode.borrow_mut() = Some(mode.into_prop());
        self
    }

    /// The height in rows (LVGL `lv_roller_set_visible_row_count`).
    #[must_use]
    pub fn visible_rows(self, rows: impl IntoProp<u8>) -> Self {
        self.bind(rows, |r: &mut Roller, cx, n| r.set_visible_row_count(cx, n))
    }

    /// Called with the index of the option the user chose.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(usize) + 'static) -> Self {
        self.op(move |cx, node| {
            on_value_changed(
                cx,
                node,
                |_, _, ev| event_value(ev).and_then(|v| usize::try_from(v).ok()),
                f,
            );
        })
    }
}
