//! Text and number entry: [`buttonmatrix`], [`keyboard`], [`textarea`] and [`spinbox`].

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use core::ops::RangeInclusive;

use twine_engine::{EventCode, NodeId};
use twine_widgets::buttonmatrix::{BtnCtrl, ButtonMatrix, MapSrc};
use twine_widgets::keyboard::{Keyboard, KeyboardMode};
use twine_widgets::spinbox::Spinbox;
use twine_widgets::textarea::{AcceptedChars, InsertCx, Textarea, text_of};

use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, event_value, on_value_changed};
use crate::modifiers::ViewExt;
use crate::node_ref::NodeRef;
use crate::prop::{IntoProp, Prop};
use crate::text::{IntoText, bind_str};

// ---- Button matrix --------------------------------------------------------------------------

/// A matrix of buttons from an LVGL-style map: button texts, rows separated by `"\n"`.
///
/// ```
/// use twine_view::prelude::*;
///
/// static MAP: [&str; 7] = ["1", "2", "3", "\n", "4", "5", "6"];
/// let _v = buttonmatrix(&MAP)
///     .ctrl(0, BtnCtrl::CHECKABLE)
///     .on_select(|idx| { let _ = idx; });
/// ```
pub fn buttonmatrix(map: &'static [&'static str]) -> WidgetView<ButtonMatrix> {
    widget_view(move || ButtonMatrix::new(MapSrc::Static(map)))
}

impl WidgetView<ButtonMatrix> {
    /// Sets control bits of button `idx` (width units, hidden, checkable, …).
    #[must_use]
    pub fn ctrl(self, idx: u16, ctrl: BtnCtrl) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .with_widget_mut(node, |m: &mut ButtonMatrix, wcx| m.set_btn_ctrl(wcx, idx, ctrl));
        })
    }

    /// At most one checkable button is checked at a time.
    #[must_use]
    pub fn one_checked(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |m: &mut ButtonMatrix, cx, on| m.set_one_checked(cx, on))
    }

    /// Called with the index of the button the user pressed (released, for `CLICK_TRIG`
    /// buttons).
    #[must_use]
    pub fn on_select(self, f: impl FnMut(usize) + 'static) -> Self {
        self.op(move |cx, node| {
            on_value_changed(
                cx,
                node,
                |_, _, ev| event_value(ev).and_then(|v| usize::try_from(v).ok()),
                f,
            );
        })
    }

    /// The selected button (the one pressed last, or highlighted by keys): a plain value or
    /// a signal kept in sync both ways.
    #[must_use]
    pub fn selected(self, sel: impl IntoModel<Option<u16>>) -> Self {
        let model = sel.into_model();
        self.after_children(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, idx| {
                    e.with_widget_mut(n, |m: &mut ButtonMatrix, wcx| m.set_selected_btn(wcx, idx));
                },
                |_, _, ev| event_value(ev).and_then(|v| u16::try_from(v).ok()),
            );
        })
    }
}

// ---- Keyboard -------------------------------------------------------------------------------

/// An on-screen keyboard typing into the textarea `target` refers to (it attaches as soon as
/// the reference is filled; the textarea gets the focus look and its cursor blinks while
/// attached).
///
/// A common pattern shows the keyboard only while a field is focused:
///
/// ```
/// use twine_view::prelude::*;
///
/// fn form(cx: Scope) -> impl View {
///     let name = cx.signal(String::new());
///     let editing = cx.signal(false);
///     let ta: NodeRef<Textarea> = cx.node_ref();
///     column((
///         textarea(name).one_line(true).node_ref(ta).on_focus(move || editing.set(true)),
///         when(move || editing.get(), move |_| {
///             keyboard(ta).on_ready(move || editing.set(false)).on_cancel(move || editing.set(false))
///         }),
///     ))
/// }
/// # let _ = form;
/// ```
pub fn keyboard(target: NodeRef<Textarea>) -> WidgetView<Keyboard> {
    widget_view(Keyboard::new).op(move |cx, node| {
        crate::bind::bind_node(
            cx,
            node,
            Prop::Dynamic(Box::new(move || target.get())),
            |e, n, ta: Option<NodeId>| {
                e.with_widget_mut(n, |k: &mut Keyboard, wcx| {
                    if k.textarea() != ta {
                        k.set_textarea(wcx, ta);
                    }
                });
            },
        );
        // A deleted keyboard (e.g. hidden by `when`) releases its textarea, which then stops
        // looking focused and stops blinking — unless the textarea has the input focus of its
        // group (keypad typing goes on there).
        cx.on_delete(node, move || {
            crate::access::EngineAccess::with(|e| {
                let ta = e.widget::<Keyboard>(node).and_then(Keyboard::textarea);
                let group_focused = ta.is_some_and(|t| e.group_of(t).and_then(|g| e.focused(g)) == Some(t));
                if !group_focused {
                    e.with_widget_mut(node, |k: &mut Keyboard, wcx| k.set_textarea(wcx, None));
                }
            });
        });
    })
}

impl WidgetView<Keyboard> {
    /// The key map shown: lower/upper case letters, special characters, a number pad or a
    /// user map.
    #[must_use]
    pub fn mode(self, m: impl IntoProp<KeyboardMode>) -> Self {
        self.bind(m, |k: &mut Keyboard, cx, m| k.set_mode(cx, m))
    }

    /// Replaces the map (and its control bits, one per button) of `mode` for this keyboard.
    #[must_use]
    pub fn custom_map(
        self,
        mode: KeyboardMode,
        map: &'static [&'static str],
        ctrl: &'static [BtnCtrl],
    ) -> Self {
        self.op(move |cx, node| {
            cx.engine()
                .with_widget_mut(node, |k: &mut Keyboard, wcx| k.set_map(wcx, mode, map, ctrl));
        })
    }

    /// Shows the pressed key enlarged above it.
    #[must_use]
    pub fn popovers(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |k: &mut Keyboard, cx, on| k.set_popovers(cx, on))
    }

    /// The OK key was pressed (`Ready`).
    #[must_use]
    pub fn on_ready<R>(self, f: impl FnMut() -> R + 'static) -> Self {
        self.on_code(EventCode::Ready, f)
    }

    /// The close key was pressed (`Cancel`).
    #[must_use]
    pub fn on_cancel<R>(self, f: impl FnMut() -> R + 'static) -> Self {
        self.on_code(EventCode::Cancel, f)
    }
}

// ---- Textarea -------------------------------------------------------------------------------

/// A text field editing `text`: a plain value or a `Signal<String>` kept in sync both ways.
///
/// Every edit (typing, deleting, the keyboard) writes the new text to the signal; setting the
/// signal replaces the text and puts the cursor at the end — unless it equals the text shown
/// (the round trip of the field's own edit), so typing in the middle keeps the cursor. The
/// text is applied after the other settings (`max_length`, `accepted_chars`, `one_line`
/// limit it).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let name = cx.signal(String::new());
/// let _v = textarea(name).placeholder("Your name").one_line(true).max_length(24);
/// cx.dispose();
/// ```
pub fn textarea(text: impl IntoModel<String>) -> WidgetView<Textarea> {
    let model = text.into_model();
    widget_view(Textarea::new).after_children(move |cx, node| {
        bind_model(
            cx,
            node,
            model,
            |e, n, s: String| {
                e.with_widget_mut(n, |t: &mut Textarea, wcx| t.set_text(wcx, &s));
            },
            |e, n, _| text_of(e, n).unwrap_or_default().to_string(),
        );
    })
}

impl WidgetView<Textarea> {
    /// The text shown (greyed) while the field is empty.
    #[must_use]
    pub fn placeholder(self, text: impl IntoText) -> Self {
        let text = text.into_text();
        self.op(move |cx, node| {
            bind_str(
                cx,
                node,
                text,
                |e, n, s| {
                    e.with_widget_mut(n, |t: &mut Textarea, wcx| t.set_placeholder_text_static(wcx, s));
                },
                |e, n, s| {
                    e.with_widget_mut(n, |t: &mut Textarea, wcx| t.set_placeholder_text(wcx, s));
                },
            );
        })
    }

    /// One line: no line breaks, scrolls horizontally, Enter sends `Ready`.
    #[must_use]
    pub fn one_line(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_one_line(cx, on))
    }

    /// Shows bullets instead of the characters (the last typed one briefly).
    #[must_use]
    pub fn password(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_password_mode(cx, on))
    }

    /// The maximum number of characters (0 = unlimited).
    #[must_use]
    pub fn max_length(self, n: impl IntoProp<u32>) -> Self {
        self.bind(n, |t: &mut Textarea, cx, n| t.set_max_length(cx, n))
    }

    /// Accepts only the characters of `chars` (e.g. `"0123456789."`).
    #[must_use]
    pub fn accepted_chars(self, chars: &'static str) -> Self {
        self.op(move |cx, node| {
            cx.engine().with_widget_mut(node, |t: &mut Textarea, wcx| {
                t.set_accepted_chars(wcx, Some(AcceptedChars::Static(chars)));
            });
        })
    }

    /// A click moves the cursor to the clicked character (default on).
    #[must_use]
    pub fn cursor_click_pos(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_cursor_click_pos(cx, on))
    }

    /// Dragging selects text.
    #[must_use]
    pub fn text_selection(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_text_selection(cx, on))
    }

    /// `Ready`: Enter in a one-line field, or the keyboard's OK key.
    #[must_use]
    pub fn on_ready<R>(self, f: impl FnMut() -> R + 'static) -> Self {
        self.on_code(EventCode::Ready, f)
    }

    /// Filters insertions: `f` gets the text about to be inserted and returns `None` to
    /// insert it, or `Some(replacement)` to insert that instead (an empty replacement inserts
    /// nothing). Deletions are not filtered.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// let cx = twine_reactive::create_root();
    /// let code = cx.signal(String::new());
    /// let _v = textarea(code).on_insert(|s| {
    ///     s.chars().any(char::is_lowercase).then(|| s.to_uppercase())
    /// });
    /// cx.dispose();
    /// ```
    #[must_use]
    pub fn on_insert(self, mut f: impl FnMut(&str) -> Option<String> + 'static) -> Self {
        self.op(move |cx, node| {
            cx.engine().with_widget_mut(node, |t: &mut Textarea, _| {
                t.set_insert_filter(Some(Box::new(move |ins: &mut InsertCx<'_>| {
                    if ins.is_delete() {
                        return;
                    }
                    if let Some(r) = f(ins.text()) {
                        ins.replace(&r);
                    }
                })));
            });
        })
    }
}

// ---- Spinbox --------------------------------------------------------------------------------

/// A number field editing `value` (a plain value or a signal kept in sync both ways): keys,
/// the encoder or `increment`/`decrement` change it digit by digit.
///
/// The value is applied after the range and format settings. +/− buttons use a
/// [`NodeRef`]:
///
/// ```
/// use twine_view::prelude::*;
///
/// fn quantity(cx: Scope) -> impl View {
///     let qty = cx.signal(1);
///     let sb: NodeRef<Spinbox> = cx.node_ref();
///     row((
///         button(label("-")).on_click(move || sb.with_mut(|s: &mut Spinbox, cx| s.decrement(cx))),
///         spinbox(qty).range(0..=999).digits(3, 0).node_ref(sb),
///         button(label("+")).on_click(move || sb.with_mut(|s: &mut Spinbox, cx| s.increment(cx))),
///     ))
/// }
/// # let _ = quantity;
/// ```
pub fn spinbox(value: impl IntoModel<i32>) -> WidgetView<Spinbox> {
    let model = value.into_model();
    widget_view(Spinbox::new).after_children(move |cx, node| {
        bind_model(
            cx,
            node,
            model,
            |e, n, v| {
                e.with_widget_mut(n, |s: &mut Spinbox, wcx| s.set_value(wcx, v));
            },
            |e, n, ev| {
                event_value(ev)
                    .or_else(|| e.widget::<Spinbox>(n).map(Spinbox::value))
                    .unwrap_or_default()
            },
        );
    })
}

impl WidgetView<Spinbox> {
    /// The value range.
    #[must_use]
    pub fn range(self, r: impl IntoProp<RangeInclusive<i32>>) -> Self {
        self.bind(r, |s: &mut Spinbox, cx, r: RangeInclusive<i32>| {
            s.set_range(cx, *r.start(), *r.end());
        })
    }

    /// `total` digits (1…10, leading zeros shown) with the decimal point after `sep_pos`
    /// digits (0 = none).
    #[must_use]
    pub fn digits(self, total: u8, sep_pos: u8) -> Self {
        self.op(move |cx, node| {
            cx.engine().with_widget_mut(node, |s: &mut Spinbox, wcx| {
                s.set_digit_format(wcx, total, sep_pos);
            });
        })
    }

    /// The step of one increment (a power of ten: the digit being edited).
    #[must_use]
    pub fn step(self, step: impl IntoProp<u32>) -> Self {
        self.bind(step, |s: &mut Spinbox, cx, step| s.set_step(cx, step))
    }

    /// Wraps around at the range ends instead of stopping.
    #[must_use]
    pub fn rollover(self, on: impl IntoProp<bool>) -> Self {
        self.bind(on, |s: &mut Spinbox, cx, on| s.set_rollover(cx, on))
    }

    /// Called with the new value whenever the user changes it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(i32) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |_, _, ev| event_value(ev), f))
    }
}
