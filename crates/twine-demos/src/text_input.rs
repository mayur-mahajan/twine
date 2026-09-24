//! A form: a one-line name field, a password field, multi-line notes, a quantity spinbox
//! with −/+ buttons and a rich-text preview ("Hello, **name**!") that follows the name as it
//! is typed.
//!
//! Tapping (or focusing) a text field opens the on-screen keyboard attached to it; OK or the
//! close key hides it. The screen gets a bottom padding as tall as the keyboard while it is
//! shown and scrolls the field above it. The PC keyboard (keypad) and the encoder (focus the
//! keyboard, click to edit, turn to pick a key, click to type) type into the same fields.

use alloc::string::String;

use twine::engine::Obj;
use twine::prelude::*;
use twine::view::EngineAccess;

/// The height of the on-screen keyboard.
const KB_H: i32 = 120;

/// The three text fields (index into the field references).
const NAME: u8 = 0;
const PASSWORD: u8 = 1;
const NOTES: u8 = 2;

/// The form's state, provided to the scope as context (tests read it).
#[derive(Clone, Copy, Debug)]
pub struct Form {
    /// The name.
    pub name: Signal<String>,
    /// The password.
    pub password: Signal<String>,
    /// The notes.
    pub notes: Signal<String>,
    /// The quantity.
    pub quantity: Signal<i32>,
    /// The field the on-screen keyboard types into (`None`: hidden).
    pub editing: Signal<Option<u8>>,
}

/// The text input demo.
///
/// ```
/// use twine_demos::text_input::{Form, app};
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(320, 240).mount(app);
/// t.run_until_idle();
/// let form = t.root_scope().expect_context::<Form>();
/// form.name.set(String::from("Ada"));
/// t.run_until_idle();
/// let name = t.find(by_id("name")).id();
/// assert_eq!(twine::widgets::textarea::text_of(&t.engine(), name), Some("Ada"));
/// ```
pub fn app(cx: Scope) -> impl View {
    let form = Form {
        name: cx.signal(String::new()),
        password: cx.signal(String::new()),
        notes: cx.signal(String::new()),
        quantity: cx.signal(1),
        editing: cx.signal(None),
    };
    cx.provide(form);
    let fields: [NodeRef<Textarea>; 3] = [cx.node_ref(), cx.node_ref(), cx.node_ref()];
    let spin: NodeRef<Spinbox> = cx.node_ref();
    let page: NodeRef<Obj> = cx.node_ref();
    let edit = move |f: u8| move || form.editing.set_if_changed(Some(f));

    let content = column((
        row((
            label("Profile").font(&fonts::MONTSERRAT_16).flex_grow(1),
            button(label("Clear")).test_id("clear").on_click(move || {
                form.name.set(String::new());
                form.password.set(String::new());
                form.notes.set(String::new());
                form.quantity.set(1);
            }),
        ))
        .width(Length::pct(100))
        .align_items(FlexAlign::Center),
        textarea(form.name)
            .one_line(true)
            .placeholder("Your name")
            .max_length(24)
            .width(Length::pct(100))
            .node_ref(fields[0])
            .on_focus(edit(NAME))
            .on_click(edit(NAME))
            .test_id("name"),
        textarea(form.password)
            .one_line(true)
            .password(true)
            .placeholder("Password")
            .width(Length::pct(100))
            .node_ref(fields[1])
            .on_focus(edit(PASSWORD))
            .on_click(edit(PASSWORD))
            .test_id("password"),
        textarea(form.notes)
            .placeholder("Notes")
            .size(Length::pct(100), 70)
            .node_ref(fields[2])
            .on_focus(edit(NOTES))
            .on_click(edit(NOTES))
            .test_id("notes"),
        row((
            label("Quantity").flex_grow(1),
            button(label("-"))
                .on_click(move || spin.with_mut(|s: &mut Spinbox, cx| s.decrement(cx)))
                .test_id("minus"),
            spinbox(form.quantity)
                .range(0..=99_999)
                .digits(5, 0)
                .width(90)
                .node_ref(spin)
                .test_id("quantity"),
            button(label("+"))
                .on_click(move || spin.with_mut(|s: &mut Spinbox, cx| s.increment(cx)))
                .test_id("plus"),
        ))
        .width(Length::pct(100))
        .gap(6)
        .align_items(FlexAlign::Center),
        spangroup((
            span("Hello, "),
            span(form.name)
                .font(&fonts::MONTSERRAT_16)
                .text_color(Palette::Blue.main()),
            span("!"),
        ))
        .mode(SpanMode::Break)
        .width(Length::pct(100))
        .test_id("preview"),
    ))
    .width(Length::pct(100))
    .gap(8);

    container((
        scroll_view(Dir::VER, content)
            .size(Length::pct(100), Length::pct(100))
            .padding(10)
            .node_ref(page)
            .test_id("page"),
        when(
            move || form.editing.with(Option::is_some),
            move |_| {
                dynamic(move |_| {
                    let f = form.editing.get().unwrap_or(NAME);
                    let ta = fields[usize::from(f)];
                    keyboard(ta)
                        .height(KB_H)
                        .on_ready(move || form.editing.set(None))
                        .on_cancel(move || form.editing.set(None))
                        .op(move |cx, kb| {
                            // Room for the keyboard below the fields, and the field above it.
                            let Some(p) = page.get_untracked() else { return };
                            let e = cx.engine();
                            e.set_local_prop(p, Selector::MAIN, StyleProp::PadBottom(KB_H + 10));
                            e.update_layout();
                            if let Some(t) = ta.get_untracked() {
                                e.scroll_to_view(t, false);
                            }
                            cx.on_delete(kb, move || {
                                EngineAccess::with(|e| {
                                    e.set_local_prop(p, Selector::MAIN, StyleProp::PadBottom(10));
                                });
                            });
                        })
                        .test_id("keyboard")
                        .into_any()
                })
            },
        ),
    ))
    .size(Length::pct(100), Length::pct(100))
    .padding(0)
    .border_width(0)
    .radius(0)
}
