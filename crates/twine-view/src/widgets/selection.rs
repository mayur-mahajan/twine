//! Pickers: [`dropdown`], [`dropdown_static`], [`roller`] and [`roller_static`].

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_image::ImageSource;
use twine_style::Dir;
use twine_widgets_ext::dropdown::Dropdown;
use twine_widgets_ext::roller::{Roller, RollerMode};

use crate::build::BuildCx;

use crate::bind::bind_node;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, event_value, on_value_changed};
use crate::prop::IntoProp;

/// Joins `items` with `'\n'` into `buf` (cleared first; its capacity is reused).
fn join_into(buf: &mut String, items: &[String]) {
    buf.clear();
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            buf.push('\n');
        }
        buf.push_str(s);
    }
}

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

/// A dropdown choosing one of `options` (a `Vec<String>`, a signal or a closure returning
/// one), with the index of the chosen option in `selected` (a plain value or a signal kept in
/// sync both ways).
///
/// The options are joined with `'\n'` into the widget's own buffer (capacity reused); when
/// they change, the selected index is kept (clamped to the new options). For options known at
/// compile time, [`dropdown_static`] stores them without any allocation.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn picker(cx: Scope) -> impl View {
///     let city = cx.signal(0usize);
///     let names = vec!["Berlin".to_string(), "Paris".to_string(), "Rome".to_string()];
///     column((
///         dropdown(names, city).dir(Dir::BOTTOM),
///         label(text!("Selected: {}", city.get())),
///     ))
/// }
/// # let _ = picker;
/// ```
pub fn dropdown(
    options: impl IntoProp<Vec<String>>,
    selected: impl IntoModel<usize>,
) -> WidgetView<Dropdown> {
    let options = options.into_prop();
    let v = widget_view(Dropdown::new).op(move |cx, node| {
        let buf = RefCell::new(String::new());
        bind_node(cx, node, options, move |e, n, items: Vec<String>| {
            let mut b = buf.borrow_mut();
            join_into(&mut b, &items);
            e.with_widget_mut(n, |d: &mut Dropdown, wcx| {
                let keep = d.selected();
                d.set_options(wcx, &b);
                d.set_selected(wcx, keep);
            });
        });
    });
    bind_selected(v, selected, set_dropdown_selected, |d| usize::from(d.selected()))
}

/// A dropdown of `'static` options (`'\n'`-separated, stored without copying: no allocation).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = dropdown_static("Low\nMedium\nHigh", 1usize).on_change(|i| { let _ = i; });
/// ```
pub fn dropdown_static(options: &'static str, selected: impl IntoModel<usize>) -> WidgetView<Dropdown> {
    let v = widget_view(move || Dropdown::with_options(options));
    bind_selected(v, selected, set_dropdown_selected, |d| usize::from(d.selected()))
}

fn set_dropdown_selected(d: &mut Dropdown, cx: &mut twine_engine::WidgetCx<'_>, idx: usize) {
    d.set_selected(cx, u16::try_from(idx).unwrap_or(u16::MAX));
}

impl WidgetView<Dropdown> {
    /// The side the list opens to (`Dir::BOTTOM` by default; `TOP`, `LEFT`, `RIGHT`). It
    /// flips to the opposite side when that has more room.
    #[must_use]
    pub fn dir(self, dir: impl IntoProp<Dir>) -> Self {
        self.bind(dir, |d: &mut Dropdown, cx, dir| d.set_dir(cx, dir))
    }

    /// The symbol at the side (`SYMBOL_DOWN` by default; a symbol or an image).
    #[must_use]
    pub fn symbol(self, src: impl IntoProp<ImageSource>) -> Self {
        self.bind(src, |d: &mut Dropdown, cx, s| d.set_symbol(cx, Some(s)))
    }

    /// Hides the symbol.
    #[must_use]
    pub fn no_symbol(self) -> Self {
        self.op(|cx, node| {
            cx.engine()
                .with_widget_mut(node, |d: &mut Dropdown, wcx| d.set_symbol(wcx, None));
        })
    }

    /// A fixed text on the button instead of the selected option (e.g. a menu title).
    #[must_use]
    pub fn text(self, text: &'static str) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .with_widget_mut(node, |d: &mut Dropdown, wcx| d.set_text_static(wcx, Some(text)));
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
    mode: core::cell::Cell<RollerMode>,
}

/// A roller choosing one of `options` (a `Vec<String>`, a signal or a closure returning one),
/// with the index of the chosen option in `selected` (a plain value or a signal kept in sync
/// both ways; the roller rolls to it with its animation).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn hours(cx: Scope) -> impl View {
///     let h = cx.signal(7usize);
///     let names: Vec<String> = (0..24).map(|i| format!("{i:02}")).collect();
///     roller(names, h).mode(RollerMode::Infinite).visible_rows(3)
/// }
/// # let _ = hours;
/// ```
pub fn roller(options: impl IntoProp<Vec<String>>, selected: impl IntoModel<usize>) -> WidgetView<Roller> {
    let options = options.into_prop();
    let mut v = widget_view(Roller::new);
    let settings = v.shared::<RollerSettings>();
    let v = v.op(move |cx, node| {
        let buf = RefCell::new(String::new());
        bind_node(cx, node, options, move |e, n, items: Vec<String>| {
            let mut b = buf.borrow_mut();
            join_into(&mut b, &items);
            let mode = settings.mode.get();
            e.with_widget_mut(n, |r: &mut Roller, wcx| {
                let keep = r.selected();
                r.set_options(wcx, &b, mode);
                r.set_selected(wcx, keep, false);
            });
        });
    });
    bind_selected(v, selected, set_roller_selected, |r| usize::from(r.selected()))
}

/// A roller of `'static` options (`'\n'`-separated, stored without copying, also in
/// infinite mode).
///
/// ```
/// use twine_view::prelude::*;
/// let _v = roller_static("Mon\nTue\nWed", 0usize).mode(RollerMode::Infinite);
/// ```
pub fn roller_static(options: &'static str, selected: impl IntoModel<usize>) -> WidgetView<Roller> {
    let mut v = widget_view(move || Roller::with_options(options));
    let settings = v.shared::<RollerSettings>();
    let v = v.op(move |cx: &mut BuildCx<'_>, node| {
        let mode = settings.mode.get();
        cx.engine().with_widget_mut(node, |r: &mut Roller, wcx| {
            r.set_options_static(wcx, options, mode);
        });
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
    pub fn mode(mut self, mode: RollerMode) -> Self {
        self.shared::<RollerSettings>().mode.set(mode);
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
