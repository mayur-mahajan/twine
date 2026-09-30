//! Text entry views: textarea, keyboard, spinbox, button matrix and span group bindings.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_core::Rect;
use twine_hal::Key;
use twine_reactive::runtime_stats;
use twine_testing::{TestUi, by_class, by_id, capture_logs};
use twine_view::prelude::*;
use twine_widgets::buttonmatrix::ButtonMatrix;
use twine_widgets::keyboard::Keyboard;
use twine_widgets::spangroup::SpanGroup;
use twine_widgets::textarea::text_of;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

/// Lets a few frames pass (a focused text field blinks: it is never idle).
fn settle(t: &mut TestUi) {
    t.advance(Duration::ms(100));
}

fn focus(t: &mut TestUi, id: &'static str) {
    let n = node(t, id);
    t.engine_mut().focus(n);
    settle(t);
}

fn ta_text(t: &TestUi, id: &'static str) -> String {
    let n = node(t, id);
    text_of(&t.engine(), n).unwrap_or_default().to_string()
}

/// Taps the key labelled `key` on the keyboard (switching to the map that has it is up to
/// the caller).
fn tap_key(t: &mut TestUi, key: &str) {
    let kb = t.find(by_class("keyboard")).id();
    let area = {
        let e = t.engine();
        let k = e.widget::<Keyboard>(kb).unwrap();
        let m = k.buttonmatrix();
        let idx = (0..m.btn_count())
            .find(|&i| m.btn_text(i) == Some(key))
            .unwrap_or_else(|| panic!("no key {key:?}"));
        m.btn_area(&MeasureCx::new(&e, kb), idx).unwrap()
    };
    t.tap(area.center());
    settle(t);
}

#[test]
fn textarea_view_two_way() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let name = cx.signal(String::from("Ada"));
        cx.provide(name);
        textarea(name).one_line(true).test_id("ta")
    });
    settle(&mut t);
    let name = t.root_scope().expect_context::<Signal<String>>();
    assert_eq!(ta_text(&t, "ta"), "Ada");
    focus(&mut t, "ta");
    t.type_text("m");
    settle(&mut t);
    assert_eq!(name.get_untracked(), "Adam");
    assert_eq!(
        runtime_stats()
            .faults
            .get(twine_reactive::FaultKind::EffectLoopCut),
        0
    );
    // Signal → text (cursor at the end).
    name.set(String::from("Grace"));
    settle(&mut t);
    assert_eq!(ta_text(&t, "ta"), "Grace");
}

#[test]
fn textarea_own_edit_keeps_cursor() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let s = cx.signal(String::from("ac"));
        cx.provide(s);
        textarea(s).one_line(true).test_id("ta")
    });
    settle(&mut t);
    let s = t.root_scope().expect_context::<Signal<String>>();
    focus(&mut t, "ta");
    let n = node(&t, "ta");
    t.engine_mut()
        .with_widget_mut(n, |ta: &mut Textarea, cx| ta.set_cursor_pos(cx, 1));
    t.type_text("b");
    settle(&mut t);
    assert_eq!(s.get_untracked(), "abc");
    let pos = t.engine().widget::<Textarea>(n).unwrap().cursor_pos();
    assert_eq!(pos, 2, "the round trip did not move the cursor to the end");
}

#[test]
fn textarea_settings_apply_before_text() {
    let mut t = TestUi::new(240, 120).mount(|_| {
        textarea(String::from("abc123def"))
            .max_length(5)
            .accepted_chars("abcdef")
            .placeholder("hint")
            .test_id("ta")
    });
    settle(&mut t);
    assert_eq!(ta_text(&t, "ta"), "abcde");
    let n = node(&t, "ta");
    assert_eq!(
        t.engine().widget::<Textarea>(n).unwrap().placeholder_text(),
        "hint"
    );
}

#[test]
fn textarea_on_insert_replaces() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let s = cx.signal(String::new());
        cx.provide(s);
        textarea(s)
            .on_insert(|s| s.chars().any(char::is_lowercase).then(|| s.to_uppercase()))
            .test_id("ta")
    });
    settle(&mut t);
    let s = t.root_scope().expect_context::<Signal<String>>();
    focus(&mut t, "ta");
    t.type_text("ok");
    settle(&mut t);
    assert_eq!(s.get_untracked(), "OK");
}

#[test]
fn keyboard_types_into_bound_textarea() {
    let ready = Rc::new(Cell::new(0));
    let r = ready.clone();
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let name = cx.signal(String::new());
        cx.provide(name);
        let ta: NodeRef<Textarea> = cx.node_ref();
        container((
            textarea(name)
                .one_line(true)
                .width(200)
                .node_ref(ta)
                .test_id("ta"),
            keyboard(ta).on_ready(move || r.set(r.get() + 1)),
        ))
        .size(Length::pct(100), Length::pct(100))
    });
    settle(&mut t);
    let name = t.root_scope().expect_context::<Signal<String>>();
    assert!(t.find(by_id("ta")).state().contains(State::FOCUSED), "attached");
    tap_key(&mut t, "h");
    tap_key(&mut t, "i");
    assert_eq!(name.get_untracked(), "hi");
    assert_eq!(ta_text(&t, "ta"), "hi");
    tap_key(&mut t, Symbol::Backspace.as_str());
    assert_eq!(name.get_untracked(), "h");
    tap_key(&mut t, Symbol::Ok.as_str());
    assert_eq!(ready.get(), 1);
}

#[test]
fn keyboard_hidden_by_when_releases_textarea_and_idles() {
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let name = cx.signal(String::new());
        let editing = cx.signal(true);
        cx.provide(editing);
        let ta: NodeRef<Textarea> = cx.node_ref();
        container((
            // First in the default group: it has the input focus, not the textarea.
            button(label("other")).align(Align::TopRight),
            textarea(name).one_line(true).node_ref(ta).test_id("ta"),
            when(
                move || editing.get(),
                move |_| keyboard(ta).on_ready(move || editing.set(false)),
            ),
        ))
        .size(Length::pct(100), Length::pct(100))
    });
    t.advance(Duration::ms(100));
    assert!(t.find(by_id("ta")).state().contains(State::FOCUSED));
    tap_key(&mut t, Symbol::Ok.as_str());
    t.run_until_idle();
    assert!(t.find_all(by_class("keyboard")).is_empty());
    assert!(!t.find(by_id("ta")).state().contains(State::FOCUSED));
    t.assert_idle();
}

#[test]
fn spinbox_view_two_way() {
    let changes = Rc::new(RefCell::new(Vec::new()));
    let c = changes.clone();
    let mut t = TestUi::new(240, 120).mount(move |cx| {
        let v = cx.signal(42);
        cx.provide(v);
        spinbox(v)
            .range(0..=999)
            .digits(3, 0)
            .on_change(move |v| c.borrow_mut().push(v))
            .test_id("sb")
    });
    t.run_until_idle();
    let v = t.root_scope().expect_context::<Signal<i32>>();
    assert_eq!(ta_text(&t, "sb"), "042");
    focus(&mut t, "sb");
    t.key(Key::Up);
    settle(&mut t);
    assert_eq!(v.get_untracked(), 43);
    assert_eq!(*changes.borrow(), [43]);
    v.set(7);
    settle(&mut t);
    assert_eq!(ta_text(&t, "sb"), "007");
    assert_eq!(*changes.borrow(), [43], "programmatic changes are silent");
}

#[test]
fn buttonmatrix_select_and_selected_model() {
    let picked = Rc::new(Cell::new(None));
    let p = picked.clone();
    let mut t = TestUi::new(240, 160).mount(move |cx| {
        let sel = cx.signal(None::<u16>);
        cx.provide(sel);
        buttonmatrix([[btn("1"), btn("2"), btn("3")], [btn("4"), btn("5"), btn("6")]])
            .selected(sel)
            .on_select(move |i| p.set(Some(i)))
            .size(200, 100)
            .test_id("m")
    });
    t.run_until_idle();
    let sel = t.root_scope().expect_context::<Signal<Option<u16>>>();
    let n = node(&t, "m");
    let area = {
        let e = t.engine();
        e.widget::<ButtonMatrix>(n)
            .unwrap()
            .btn_area(&MeasureCx::new(&e, n), 4)
            .unwrap()
    };
    t.tap(area.center());
    t.run_until_idle();
    assert_eq!(picked.get(), Some(4));
    assert_eq!(sel.get_untracked(), Some(4));
    sel.set(Some(1));
    t.run_until_idle();
    assert_eq!(
        t.engine().widget::<ButtonMatrix>(n).unwrap().selected_btn(),
        Some(1)
    );
}

#[test]
fn span_text_binding_redraws_only_the_group() {
    let mut t = TestUi::new(240, 120).mount(|cx| {
        let name = cx.signal(String::from("Ada"));
        cx.provide(name);
        column((
            spangroup((
                span("Hello, "),
                span(name)
                    .text_color(Color::hex(0x21_96_F3))
                    .text_decoration(TextDecor::UNDERLINE),
                span("!"),
            ))
            .mode(SpanMode::Break)
            .width(200)
            .test_id("g"),
            label("unrelated"),
        ))
    });
    t.run_until_idle();
    let name = t.root_scope().expect_context::<Signal<String>>();
    let n = node(&t, "g");
    let texts = |t: &TestUi| -> Vec<String> {
        let e = t.engine();
        let g = e.widget::<SpanGroup>(n).unwrap();
        g.span_ids()
            .map(|id| g.span(id).unwrap().text().to_string())
            .collect()
    };
    assert_eq!(texts(&t), ["Hello, ", "Ada", "!"]);
    let area = t.find(by_id("g")).coords();
    name.set(String::from("Grace"));
    let runs = runtime_stats().effect_runs;
    let period = t.engine().config().refr_period;
    t.advance(period);
    assert_eq!(runtime_stats().effect_runs - runs, 1);
    assert_eq!(texts(&t), ["Hello, ", "Grace", "!"]);
    let inv: Vec<Rect> = t.invalidations().iter().map(|(r, _)| *r).collect();
    for r in &inv {
        assert!(area.contains_rect(r), "{r:?} outside the group {area:?}");
    }
    t.run_until_idle();
    t.assert_idle();
}

#[test]
fn span_outside_group_warns_and_builds_a_label() {
    let (mut t, logs) = capture_logs(|| TestUi::new(200, 80).mount(|_| column(span("lonely"))));
    t.run_until_idle();
    assert!(
        logs.iter()
            .any(|l| l.message.contains("span outside a spangroup")),
        "{logs:?}"
    );
    assert_eq!(t.find(by_class("label")).text(), "lonely");
}

#[test]
fn keyboard_removal_keeps_group_focus_of_textarea() {
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let name = cx.signal(String::new());
        let editing = cx.signal(true);
        let ta: NodeRef<Textarea> = cx.node_ref();
        container((
            textarea(name).one_line(true).node_ref(ta).test_id("ta"),
            when(
                move || editing.get(),
                move |_| keyboard(ta).on_ready(move || editing.set(false)),
            ),
        ))
        .size(Length::pct(100), Length::pct(100))
    });
    settle(&mut t);
    tap_key(&mut t, Symbol::Ok.as_str());
    assert!(t.find_all(by_class("keyboard")).is_empty());
    // The textarea has the keypad focus: it stays focused (keypad typing goes on there).
    assert!(t.find(by_id("ta")).state().contains(State::FOCUSED));
    t.type_text("x");
    settle(&mut t);
    assert_eq!(ta_text(&t, "ta"), "x");
}

#[test]
fn buttonmatrix_builder_maps_rows_widths_and_controls() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let ok = cx.signal(String::from("OK"));
        cx.provide(ok);
        buttonmatrix([
            vec![btn("1"), btn("").hidden(), btn("3").disabled()],
            vec![
                btn(ok).width(2).checkable().checked(),
                btn("x").no_repeat().click_trig(),
            ],
            vec![btn("p").popover().custom_1().custom_2()],
        ])
        .size(200, 120)
        .test_id("m")
    });
    t.run_until_idle();
    let ok = t.root_scope().expect_context::<Signal<String>>();
    let n = node(&t, "m");
    {
        let e = t.engine();
        let m = e.widget::<ButtonMatrix>(n).unwrap();
        assert_eq!((m.btn_count(), m.row_count()), (6, 3));
        let texts: Vec<_> = (0..6).map(|i| m.btn_text(i).unwrap()).collect();
        assert_eq!(
            texts,
            ["1", "", "3", "OK", "x", "p"],
            "an empty text is a button, not the end"
        );
        let ctrl: Vec<_> = (0..6).map(|i| m.btn_ctrl(i).unwrap()).collect();
        assert_eq!(
            ctrl,
            [
                BtnCtrl::empty(),
                BtnCtrl::HIDDEN,
                BtnCtrl::DISABLED,
                BtnCtrl::width(2) | BtnCtrl::CHECKABLE | BtnCtrl::CHECKED,
                BtnCtrl::NO_REPEAT | BtnCtrl::CLICK_TRIG,
                BtnCtrl::POPOVER | BtnCtrl::CUSTOM_1 | BtnCtrl::CUSTOM_2,
            ]
        );
        // "OK" is twice as wide as "x" (rows share their width by units).
        let mcx = MeasureCx::new(&e, n);
        let (a, b) = (m.btn_area(&mcx, 3).unwrap(), m.btn_area(&mcx, 4).unwrap());
        assert!((a.width() - 2 * b.width()).abs() <= 2, "{a:?} {b:?}");
    }
    // A dynamic text updates the map; the control bits stay.
    ok.set(String::from("Done"));
    t.run_until_idle();
    let e = t.engine();
    let m = e.widget::<ButtonMatrix>(n).unwrap();
    assert_eq!(m.btn_text(3), Some("Done"));
    assert!(m.has_btn_ctrl(3, BtnCtrl::CHECKED));
}
