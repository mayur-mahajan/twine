//! Text and number entry: [`buttonmatrix`], [`keyboard`], [`textarea`] and [`spinbox`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::RangeInclusive;

use twine_engine::{EventCode, NodeId};
use twine_widgets::buttonmatrix::{BtnCtrl, ButtonMatrix, MapSrc};
use twine_widgets::keyboard::{Keyboard, KeyboardMode};
use twine_widgets::spinbox::Spinbox;
use twine_widgets::textarea::{AcceptedChars, InsertCx, Textarea, text_of};

use crate::bind::bind_effect;
use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, event_value, on_value_changed};
use crate::modifiers::ViewExt;
use crate::node_ref::NodeRef;
use crate::prop::{IntoProp, Prop};
use crate::text::{IntoText, TextProp, TextRef, bind_str};

// ---- Button matrix --------------------------------------------------------------------------

/// A button of a [`buttonmatrix`]: its text and control settings (relative width, checkable,
/// hidden, …). Created with [`btn`].
#[must_use]
pub struct Btn {
    text: TextProp,
    ctrl: BtnCtrl,
}

impl core::fmt::Debug for Btn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Btn")
            .field("text", &self.text)
            .field("ctrl", &self.ctrl)
            .finish()
    }
}

/// A button of a [`buttonmatrix`] showing `text` (any [`IntoText`]; dynamic texts, e.g.
/// translations, update the matrix's map when they change). One width unit by default.
pub fn btn<MT>(text: impl IntoText<MT>) -> Btn {
    Btn {
        text: text.into_text(),
        ctrl: BtnCtrl::empty(),
    }
}

impl Btn {
    /// The relative width in units (1…15, clamped; default 1): a row's width is shared by
    /// units.
    pub fn width(mut self, units: u8) -> Self {
        self.ctrl = (self.ctrl & !BtnCtrl::WIDTH_MASK) | BtnCtrl::width(units);
        self
    }

    fn with(mut self, c: BtnCtrl) -> Self {
        self.ctrl |= c;
        self
    }

    /// Toggles its checked state when clicked.
    pub fn checkable(self) -> Self {
        self.with(BtnCtrl::CHECKABLE)
    }

    /// Starts checked.
    pub fn checked(self) -> Self {
        self.with(BtnCtrl::CHECKED)
    }

    /// Not drawn and not pressable; it still takes its width.
    pub fn hidden(self) -> Self {
        self.with(BtnCtrl::HIDDEN)
    }

    /// Drawn disabled and not pressable.
    pub fn disabled(self) -> Self {
        self.with(BtnCtrl::DISABLED)
    }

    /// No repeated selection while long pressed.
    pub fn no_repeat(self) -> Self {
        self.with(BtnCtrl::NO_REPEAT)
    }

    /// Selected on release instead of on press.
    pub fn click_trig(self) -> Self {
        self.with(BtnCtrl::CLICK_TRIG)
    }

    /// Shows an enlarged copy above the button while pressed (selected on release).
    pub fn popover(self) -> Self {
        self.with(BtnCtrl::POPOVER)
    }

    /// The first application-defined flag (e.g. a theme's control-key look).
    pub fn custom_1(self) -> Self {
        self.with(BtnCtrl::CUSTOM_1)
    }

    /// The second application-defined flag.
    pub fn custom_2(self) -> Self {
        self.with(BtnCtrl::CUSTOM_2)
    }
}

/// The map of a button matrix: the buttons' texts with `"\n"` entries between rows (the
/// widget's compact format), written into `map` (whose strings are reused).
fn write_map(rows: &[Vec<TextProp>], map: &mut Vec<String>) {
    let mut i = 0;
    let mut next = |map: &mut Vec<String>| {
        if map.len() <= i {
            map.push(String::new());
        }
        let s = &mut map[i];
        s.clear();
        i += 1;
        i - 1
    };
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            let k = next(map);
            map[k].push('\n');
        }
        for t in row {
            let k = next(map);
            t.write_to(&mut map[k]);
        }
    }
    map.truncate(i);
}

/// A matrix of buttons drawn by one widget (no node per button): `rows` of [`btn`]s, each
/// with its text and settings. Rows of equal length can be arrays; rows of different lengths
/// are `Vec`s (any iterable of iterables works).
///
/// The buttons are numbered in reading order (the index [`on_select`](WidgetView::on_select)
/// reports).
///
/// ```
/// use twine_view::prelude::*;
///
/// let _v = buttonmatrix([
///     vec![btn("1"), btn("2"), btn("3")],
///     vec![btn("OK").width(2).checkable()],
/// ])
/// .on_select(|idx| { let _ = idx; });
/// ```
pub fn buttonmatrix<R, I>(rows: R) -> WidgetView<ButtonMatrix>
where
    R: IntoIterator<Item = I>,
    I: IntoIterator<Item = Btn>,
{
    let mut ctrl: Vec<BtnCtrl> = Vec::new();
    let rows: Vec<Vec<TextProp>> = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|b| {
                    ctrl.push(b.ctrl);
                    b.text
                })
                .collect()
        })
        .collect();
    widget_view(|| ButtonMatrix::new(MapSrc::Static(&[]))).op(move |cx, node| {
        let scope = cx.scope();
        let rows: Vec<Vec<TextProp>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(|t| t.resolve(scope)).collect())
            .collect();
        let set_map = |e: &mut twine_engine::Engine, n: NodeId, map: &[String]| {
            e.with_widget_mut(n, |m: &mut ButtonMatrix, wcx| {
                let m_map = m.map();
                let same = m_map.len() == map.len() && map.iter().enumerate().all(|(i, s)| m_map.get(i) == s);
                if !same {
                    m.set_map(wcx, MapSrc::Owned(map.to_vec()));
                }
            });
        };
        if rows.iter().flatten().all(TextProp::is_constant) {
            let mut map = Vec::new();
            write_map(&rows, &mut map);
            set_map(cx.engine(), node, &map);
        } else {
            // One binding for the whole map: the texts are written into reused strings, the
            // widget's map is replaced only when one of them changed.
            let scratch = Rc::new(RefCell::new(Vec::<String>::new()));
            cx.provide(|| {
                bind_effect(
                    scope,
                    node,
                    move || {
                        write_map(&rows, &mut scratch.borrow_mut());
                        scratch.clone()
                    },
                    move |e, n, map: Rc<RefCell<Vec<String>>>| set_map(e, n, &map.borrow()),
                );
            });
        }
        cx.engine()
            .with_widget_mut(node, |m: &mut ButtonMatrix, wcx| m.set_ctrl_map(wcx, &ctrl));
    })
}

impl WidgetView<ButtonMatrix> {
    /// At most one checkable button is checked at a time.
    #[must_use]
    pub fn one_checked<M>(self, on: impl IntoProp<bool, M>) -> Self {
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
        let rt = cx.runtime();
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
            crate::access::EngineAccess::with(rt, |e| {
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
    pub fn mode<M>(self, m: impl IntoProp<KeyboardMode, M>) -> Self {
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
    pub fn popovers<M>(self, on: impl IntoProp<bool, M>) -> Self {
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
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
    pub fn placeholder<MT>(self, text: impl IntoText<MT>) -> Self {
        self.placeholder_prop(text.into_text())
    }

    /// [`placeholder`](Self::placeholder) after the conversion (not generic).
    fn placeholder_prop(self, text: TextProp) -> Self {
        self.op(move |cx, node| {
            bind_str(cx, node, text, |e, n, s| {
                e.with_widget_mut(n, |t: &mut Textarea, wcx| match s {
                    TextRef::Static(s) => t.set_placeholder_text_static(wcx, s),
                    TextRef::Borrowed(s) => t.set_placeholder_text(wcx, s),
                });
            });
        })
    }

    /// One line: no line breaks, scrolls horizontally, Enter sends `Ready`.
    #[must_use]
    pub fn one_line<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_one_line(cx, on))
    }

    /// Shows bullets instead of the characters (the last typed one briefly).
    #[must_use]
    pub fn password<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_password_mode(cx, on))
    }

    /// The maximum number of characters (0 = unlimited). An `i32` like every count of the view
    /// API, so that literals infer (see [`IntoProp`] § Integer literals); a negative value is
    /// warned about and counts as 0.
    #[must_use]
    pub fn max_length<M>(self, n: impl IntoProp<i32, M>) -> Self {
        self.bind(n, |t: &mut Textarea, cx, n: i32| {
            t.set_max_length(cx, super::count_u32("max_length", n, 0));
        })
    }

    /// Accepts only the characters of `chars` (e.g. `"0123456789."`; any [`IntoText`]).
    #[must_use]
    pub fn accepted_chars<MT>(self, chars: impl IntoText<MT>) -> Self {
        self.accepted_chars_prop(chars.into_text())
    }

    /// [`accepted_chars`](Self::accepted_chars) after the conversion (not generic).
    fn accepted_chars_prop(self, chars: TextProp) -> Self {
        self.op(move |cx, node| {
            bind_str(cx, node, chars, |e, n, s| {
                e.with_widget_mut(n, |t: &mut Textarea, wcx| match s {
                    TextRef::Static(s) => t.set_accepted_chars(wcx, Some(AcceptedChars::Static(s))),
                    // Compared first: no copy for an unchanged list.
                    TextRef::Borrowed(s) => {
                        if t.accepted_chars().map(AcceptedChars::as_str) != Some(s) {
                            t.set_accepted_chars(wcx, Some(AcceptedChars::Owned(s.into())));
                        }
                    }
                });
            });
        })
    }

    /// A click moves the cursor to the clicked character (default on).
    #[must_use]
    pub fn cursor_click_pos<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |t: &mut Textarea, cx, on| t.set_cursor_click_pos(cx, on))
    }

    /// Dragging selects text.
    #[must_use]
    pub fn text_selection<M>(self, on: impl IntoProp<bool, M>) -> Self {
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
    /// let cx = twine_reactive::Runtime::take().unwrap().create_root();
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
    pub fn range<M>(self, r: impl IntoProp<RangeInclusive<i32>, M>) -> Self {
        self.bind(r, |s: &mut Spinbox, cx, r: RangeInclusive<i32>| {
            s.set_range(cx, *r.start(), *r.end());
        })
    }

    /// `total` digits (1…10, leading zeros shown) with the decimal point after `sep_pos`
    /// digits (0 = none).
    #[must_use]
    pub fn digits<M1, M2>(self, total: impl IntoProp<u8, M1>, sep_pos: impl IntoProp<u8, M2>) -> Self {
        match (total.into_prop(), sep_pos.into_prop()) {
            (Prop::Static(total), Prop::Static(sep)) => self.op(move |cx, node| {
                cx.engine().with_widget_mut(node, |s: &mut Spinbox, wcx| {
                    s.set_digit_format(wcx, total, sep);
                });
            }),
            (total, sep) => self
                .bind(total, |s: &mut Spinbox, cx, total| {
                    let sep = s.dec_point_pos();
                    s.set_digit_format(cx, total, sep);
                })
                .bind(sep, |s: &mut Spinbox, cx, sep| {
                    let total = s.digit_count();
                    s.set_digit_format(cx, total, sep);
                }),
        }
    }

    /// The step of one increment (a power of ten: the digit being edited). An `i32` like the
    /// value, so that literals infer; a value below 1 is warned about and counts as 1.
    #[must_use]
    pub fn step<M>(self, step: impl IntoProp<i32, M>) -> Self {
        self.bind(step, |s: &mut Spinbox, cx, step: i32| {
            s.set_step(cx, super::count_u32("step", step, 1));
        })
    }

    /// Wraps around at the range ends instead of stopping.
    #[must_use]
    pub fn rollover<M>(self, on: impl IntoProp<bool, M>) -> Self {
        self.bind(on, |s: &mut Spinbox, cx, on| s.set_rollover(cx, on))
    }

    /// Called with the new value whenever the user changes it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(i32) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |_, _, ev| event_value(ev), f))
    }
}
