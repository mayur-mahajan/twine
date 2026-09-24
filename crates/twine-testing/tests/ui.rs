//! `TestUi`: queries, input, time and reports.

use std::cell::RefCell;
use std::rc::Rc;

use twine_core::Duration;
use twine_hal::Key;
use twine_testing::{TestUi, by_class, by_id, by_text};
use twine_view::prelude::*;

/// The counter of the API guide.
pub fn counter(cx: Scope) -> impl View {
    let count = cx.signal(0u32);

    column((
        label(text!("Clicked {} times", count.get()))
            .font(&twine_assets::fonts::MONTSERRAT_14)
            .test_id("count"),
        button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
        button(label("Reset"))
            .disabled(move || count.get() == 0)
            .on_click(move || count.set(0)),
    ))
    .gap(12)
    .padding(16)
    .align_items(FlexAlign::Center)
    .size(Length::Pct(100), Length::Pct(100))
}

/// The headless test of the API guide.
#[test]
fn counter_increments() {
    let mut t = TestUi::new(320, 240).mount(counter);
    t.find(by_text("Click me")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "Clicked 1 times");
    t.assert_snapshot("counter_clicked_once");
    assert!(
        t.last_frame().dirty_px < 320 * 40,
        "only the label + button should redraw"
    );
}

#[test]
fn find_by_text_and_click() {
    let mut t = TestUi::new(320, 240).mount(counter);
    t.run_until_idle();
    let b = t.find(by_text("Reset"));
    assert!(b.is_visible());
    assert_eq!(t.find_all(by_class("button")).len(), 2);
    assert_eq!(t.find_all(by_class("button").and(by_text("Click me"))).len(), 0);
    // A button's text is its label's.
    let buttons = t.find_all(by_class("button"));
    assert_eq!(buttons[0].text(), "Click me");
    t.find(by_text("Click me")).click();
    t.find(by_text("Click me")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "Clicked 2 times");
    t.find(by_text("Reset")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("count")).text(), "Clicked 0 times");
    t.assert_idle();
}

#[test]
#[should_panic(expected = "2 nodes match")]
fn find_ambiguous_panics_with_tree() {
    let t = TestUi::new(100, 60).mount(|_| column((label("same"), label("same"))));
    let _ = t.find(by_text("same"));
}

#[test]
#[should_panic(expected = "running anims: 1")]
fn run_until_idle_reports_running_anim() {
    let mut t = TestUi::new(100, 60).mount(|cx| {
        let (a, _ctl) = cx.animation(
            Anim::new(0, 100)
                .duration(Duration::ms(500))
                .repeat(Repeat::Infinite),
        );
        label(text!("{}", a.get()))
    });
    t.run_until_idle();
}

#[test]
fn advance_steps_in_refr_period() {
    let ticks = Rc::new(RefCell::new(Vec::new()));
    let tk = ticks.clone();
    let mut t = TestUi::new(100, 60).mount(move |cx| {
        let (a, _c) = cx.animation(Anim::new(0, 1000).duration(Duration::ms(1000)));
        cx.effect(move || tk.borrow_mut().push(a.get()));
        label("x")
    });
    t.update();
    let period = t.engine().config().refr_period;
    let start = ticks.borrow().len();
    t.advance(Duration::us(period.as_micros() * 10));
    let n = ticks.borrow().len() - start;
    // One update (so one animation frame) per `refr_period` step.
    assert!((9..=11).contains(&n), "{n} frames in 10 periods");
    let values = ticks.borrow().clone();
    assert!(values.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn type_text_sends_chars() {
    let got = Rc::new(RefCell::new(String::new()));
    let g = got.clone();
    let mut t = TestUi::new(100, 60).mount(move |_| {
        button(label("k")).on_key(move |k| {
            if let Key::Char(c) = k {
                g.borrow_mut().push(c);
            }
        })
    });
    t.run_until_idle();
    t.type_text("hey");
    t.run_until_idle();
    assert_eq!(*got.borrow(), "hey");
}

#[test]
fn keyboard_type_taps_on_screen_keys() {
    use twine_widgets::keyboard::{self, Keyboard};
    use twine_widgets::textarea;
    let mut ids = None;
    let mut t = TestUi::new(320, 240).mount_engine(|e| {
        let d = e.default_display().unwrap();
        let screen = e.active_screen(d).unwrap();
        let ta = textarea::create(e, screen).unwrap();
        e.set_size(ta, 300, 60);
        let kb = keyboard::create(e, screen).unwrap();
        e.with_widget_mut(kb, |k: &mut Keyboard, cx| k.set_textarea(cx, Some(ta)));
        ids = Some((ta, kb));
    });
    let (ta, kb) = ids.unwrap();
    // The attached textarea's cursor blinks: never idle.
    t.advance(Duration::ms(100));
    // Lower case, upper case (mode switch) and digits (special layout), a space and a line.
    t.keyboard_type(kb, "Hi 42\nok");
    assert_eq!(textarea::text_of(&t.engine(), ta), Some("Hi 42\nok"));
}
