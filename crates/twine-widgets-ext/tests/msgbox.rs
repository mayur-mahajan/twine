//! `Msgbox`: the modal backdrop blocking the input, header / content / footer, the close
//! button, `close_async` from its own handler, the focus group and cleanup.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Mode, center, class, count_events, get, harness_with_group, with};
use twine_core::{Duration, Point};
use twine_engine::{EventCode, EventFilter, EventResult, Key, NodeId};
use twine_image::ImageSource;
use twine_style::{Length, Part, PropId};
use twine_testing::EngineHarness;
use twine_text::Symbol;
use twine_widgets_ext::msgbox::{self, Msgbox};

/// A screen with a button behind, and a modal message box.
fn scene(mode: Mode) -> (EngineHarness, NodeId, NodeId) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let behind = twine_widgets::button::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().set_size(behind, 80, 40);
    let mb = msgbox::create(h.engine_mut(), None).unwrap();
    with(&mut h, mb, |m: &mut Msgbox, cx| {
        m.add_title(cx, "Hello");
        m.add_text(cx, "This is a message box.");
        m.add_footer_button(cx, "Apply");
        m.add_footer_button(cx, "Cancel");
        m.add_close_button(cx);
    });
    h.run_until_idle();
    (h, behind, mb)
}

fn footer_buttons(h: &EngineHarness, mb: NodeId) -> Vec<NodeId> {
    get::<Msgbox>(h, mb).footer_buttons(h.engine()).collect()
}

#[test]
fn msgbox_structure() {
    let (h, _, mb) = scene(Mode::Light);
    let e = h.engine();
    let m = get::<Msgbox>(&h, mb);
    assert_eq!(class(&h, mb), "msgbox");
    let backdrop = m.backdrop().unwrap();
    assert_eq!(class(&h, backdrop), "msgbox_backdrop");
    assert_eq!(e.tree().parent(backdrop), e.top_layer(h.display()));
    assert_eq!(e.coords(backdrop).size(), twine_core::Size::new(320, 240));
    let kids: Vec<&str> = e.tree().children(mb).map(|c| class(&h, c)).collect();
    assert_eq!(kids, ["msgbox_header", "msgbox_content", "msgbox_footer"]);
    assert_eq!(e.coords(mb).width(), 260, "LV_DPI_DEF * 2");
    // Centered.
    let c = e.coords(mb);
    assert_eq!((c.x0 + c.x1) / 2, 160);
    assert_eq!(class(&h, footer_buttons(&h, mb)[0]), "msgbox_footer_button");
    assert_eq!(e.coords(m.footer().unwrap()).height(), 43, "LV_DPI_DEF / 3");
}

#[test]
fn msgbox_modal_blocks_background() {
    let (mut h, behind, mb) = scene(Mode::Light);
    let clicks = count_events(&mut h, behind, EventCode::Clicked);
    h.tap(center(&h, behind));
    assert_eq!(clicks.get(), 0, "the backdrop takes the press");
    assert_eq!(h.find(twine_testing::by_class("msgbox")), mb);
    // Closed, the button behind gets the clicks again.
    msgbox::close(h.engine_mut(), mb);
    h.run_until_idle();
    h.tap(center(&h, behind));
    assert_eq!(clicks.get(), 1);
}

#[test]
fn msgbox_backdrop_style() {
    let (h, _, mb) = scene(Mode::Light);
    let backdrop = get::<Msgbox>(&h, mb).backdrop().unwrap();
    let e = h.engine();
    assert_eq!(
        e.style_opa(backdrop, Part::Main, PropId::BgOpacity),
        twine_core::Opa::P50
    );
    assert_eq!(
        e.style_color(backdrop, Part::Main, PropId::BgColor),
        twine_theme::Palette::Grey.main()
    );
}

#[test]
fn msgbox_footer_button_index() {
    let (mut h, _, mb) = scene(Mode::Light);
    let btns = footer_buttons(&h, mb);
    let hits = Rc::new(Cell::new(None));
    for (i, b) in btns.iter().enumerate() {
        let hits = hits.clone();
        h.engine_mut()
            .add_event_handler(*b, EventFilter::Code(EventCode::Clicked), move |_, _| {
                hits.set(Some(i));
                EventResult::Continue
            });
    }
    h.tap(center(&h, btns[1]));
    assert_eq!(hits.get(), Some(1));
    assert!(h.engine().tree().contains(mb), "footer buttons do not close");
}

#[test]
fn msgbox_close_button_deletes() {
    let (mut h, _, mb) = scene(Mode::Light);
    let backdrop = get::<Msgbox>(&h, mb).backdrop().unwrap();
    let header = get::<Msgbox>(&h, mb).header().unwrap();
    let close = h.engine().tree().children(header).last().unwrap();
    assert_eq!(class(&h, close), "msgbox_header_button");
    h.tap(center(&h, close));
    assert!(!h.engine().tree().contains(mb));
    assert!(!h.engine().tree().contains(backdrop));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn msgbox_close_async_inside_handler_safe() {
    let (mut h, _, mb) = scene(Mode::Light);
    let b = footer_buttons(&h, mb)[0];
    h.engine_mut()
        .add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |cx, _| {
            msgbox::close_async(cx.engine_mut(), mb);
            // Still there during the handler.
            assert!(cx.engine().tree().contains(mb));
            EventResult::Continue
        });
    h.tap(center(&h, b));
    h.advance(Duration::ms(40));
    assert!(!h.engine().tree().contains(mb));
    h.engine().tree().check_invariants().unwrap();
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn msgbox_focus_group_restored() {
    let mut h = harness_with_group(320, 240, Mode::Light);
    let g0 = h.engine().default_group().unwrap();
    let (kp, _) = h.keypad_input();
    let screen = h.screen();
    let behind = twine_widgets::button::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().focus(behind);
    let mb = msgbox::create(h.engine_mut(), None).unwrap();
    with(&mut h, mb, |m: &mut Msgbox, cx| {
        m.add_footer_button(cx, "OK");
        m.add_footer_button(cx, "No");
    });
    let g = h.engine().default_group().unwrap();
    assert_ne!(g, g0, "a group of its own");
    assert_eq!(h.engine().input_group(kp), Some(g));
    assert_eq!(h.engine().group_count(g), 2, "the footer buttons");
    h.key(Key::Next);
    assert_eq!(h.engine().focused(g), Some(footer_buttons(&h, mb)[1]));
    msgbox::close(h.engine_mut(), mb);
    assert_eq!(h.engine().default_group(), Some(g0));
    assert_eq!(h.engine().input_group(kp), Some(g0));
    assert_eq!(h.engine().focused(g0), Some(behind));
    h.run_until_idle();
}

#[test]
fn msgbox_long_text_scrolls() {
    let mut h = harness_with_group(320, 240, Mode::Light);
    let mb = msgbox::create(h.engine_mut(), None).unwrap();
    let long = (0..40)
        .map(|i| format!("Line {i}."))
        .collect::<Vec<_>>()
        .join("\n");
    with(&mut h, mb, |m: &mut Msgbox, cx| {
        m.add_title(cx, "Long");
        m.add_text(cx, &long);
        m.add_footer_button(cx, "OK");
    });
    // A fixed height: the content takes the rest and scrolls (LVGL's size-changed handler).
    h.engine_mut().set_height(mb, Length::Px(200));
    h.run_until_idle();
    let content = get::<Msgbox>(&h, mb).content();
    assert_eq!(h.engine().style_i32(content, Part::Main, PropId::FlexGrow), 1);
    assert!(h.engine().scroll_bottom(content) > 0);
    let p = center(&h, content);
    h.drag(p, Point::new(p.x, p.y - 60), Duration::ms(200));
    h.run_until_idle();
    assert!(h.engine().scroll_offset(content).y > 0);
}

#[test]
fn msgbox_open_close_100_leaves_nothing() {
    let mut h = harness_with_group(320, 240, Mode::Light);
    let g0 = h.engine().default_group();
    let _ = h.keypad_input();
    let open_close = |h: &mut EngineHarness| {
        let mb = msgbox::create(h.engine_mut(), None).unwrap();
        with(h, mb, |m: &mut Msgbox, cx| {
            m.add_title(cx, "Hi");
            m.add_text(cx, "Text");
            m.add_footer_button(cx, "OK");
            m.add_header_button(cx, Some(ImageSource::symbol(Symbol::Settings)));
        });
        h.run_until_idle();
        msgbox::close(h.engine_mut(), mb);
        h.run_until_idle();
    };
    open_close(&mut h);
    let nodes = h.engine().tree().len();
    for _ in 0..100 {
        open_close(&mut h);
    }
    assert_eq!(h.engine().tree().len(), nodes);
    assert_eq!(h.engine().default_group(), g0);
    h.assert_idle();
}

#[test]
fn msgbox_in_a_parent_is_not_modal() {
    let mut h = harness_with_group(320, 240, Mode::Light);
    let screen = h.screen();
    let g0 = h.engine().default_group();
    let mb = msgbox::create(h.engine_mut(), Some(screen)).unwrap();
    assert_eq!(get::<Msgbox>(&h, mb).backdrop(), None);
    assert_eq!(h.engine().default_group(), g0);
    msgbox::close(h.engine_mut(), mb);
    assert!(!h.engine().tree().contains(mb));
}

#[test]
fn snapshot_msgbox() {
    for m in Mode::ALL {
        let (mut h, _, _) = scene(m);
        h.assert_snapshot(&format!("msgbox_{}", m.suffix()));
    }
    let mut h = harness_with_group(320, 240, Mode::Light);
    let mb = msgbox::create(h.engine_mut(), None).unwrap();
    with(&mut h, mb, |m: &mut Msgbox, cx| {
        m.add_text(cx, "No header, just a text and a button.");
        m.add_footer_button(cx, "OK");
    });
    h.run_until_idle();
    h.assert_snapshot("msgbox_no_header");
}
