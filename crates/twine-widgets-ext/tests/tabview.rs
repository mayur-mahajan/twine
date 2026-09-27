//! `Tabview`: the bar of buttons and the snap-scrolling content, switching by click and by
//! swipe, the four bar positions, RTL, keypad / encoder, the switch animation's invalidation
//! and the theme's look.

mod common;

use common::{Mode, center, class, get, harness_with_group, has_state, values, with};
use twine_core::{Duration, Point};
use twine_engine::{Key, NodeId, State};
use twine_style::{BaseDir, Dir, Part, PropId, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::label::Label;
use twine_widgets_ext::tabview::{self, Tabview};

const TABS: [&str; 3] = ["First", "Second", "Third"];

fn scene_with(
    mode: Mode,
    setup: impl FnOnce(&mut EngineHarness, NodeId),
) -> (EngineHarness, NodeId, Vec<NodeId>) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let tv = tabview::create(h.engine_mut(), screen).unwrap();
    setup(&mut h, tv);
    let mut pages = Vec::new();
    for (i, t) in TABS.iter().enumerate() {
        let p = with(&mut h, tv, |w: &mut Tabview, cx| w.add_tab(cx, t)).unwrap();
        let l = twine_widgets::label::create(h.engine_mut(), p).unwrap();
        let text = ["Content of the first tab", "Second page", "Third page"][i];
        h.engine_mut()
            .with_widget_mut(l, |w: &mut Label, cx| w.set_text(cx, text));
        pages.push(p);
    }
    h.run_until_idle();
    (h, tv, pages)
}

fn scene(mode: Mode) -> (EngineHarness, NodeId, Vec<NodeId>) {
    scene_with(mode, |_, _| {})
}

fn active(h: &EngineHarness, tv: NodeId) -> u32 {
    get::<Tabview>(h, tv).active()
}

fn button(h: &EngineHarness, tv: NodeId, i: u32) -> NodeId {
    get::<Tabview>(h, tv).tab_button(h.engine(), i).unwrap()
}

#[test]
fn tabview_structure_and_defaults() {
    let (h, tv, pages) = scene(Mode::Light);
    let w = get::<Tabview>(&h, tv);
    let e = h.engine();
    assert_eq!(class(&h, tv), "tabview");
    assert_eq!(class(&h, w.tab_bar()), "tabview_tab_bar");
    assert_eq!(class(&h, w.content()), "tabview_content");
    assert_eq!(w.tab_count(e), 3);
    assert_eq!(w.tab_bar_position(), Dir::TOP);
    assert_eq!(e.coords(tv).size(), twine_core::Size::new(320, 240));
    // The bar: 100 % wide, dpi / 2 high.
    assert_eq!(e.coords(w.tab_bar()).height(), 130 / 2);
    // Each page fills the content area; the first tab is active and checked.
    let c = e.content_area(w.content());
    assert_eq!(e.coords(pages[0]).size(), c.size());
    assert_eq!(active(&h, tv), 0);
    assert!(has_state(&h, button(&h, tv, 0), State::CHECKED));
    assert!(!has_state(&h, button(&h, tv, 1), State::CHECKED));
}

#[test]
fn tabview_click_tab_switches_and_checks_button() {
    let (mut h, tv, pages) = scene(Mode::Light);
    let changes = values(&mut h, tv);
    h.tap(center(&h, button(&h, tv, 2)));
    h.run_until_idle();
    assert_eq!(active(&h, tv), 2);
    assert!(has_state(&h, button(&h, tv, 2), State::CHECKED));
    assert!(!has_state(&h, button(&h, tv, 0), State::CHECKED));
    let c = get::<Tabview>(&h, tv).content();
    assert_eq!(
        h.engine().coords(pages[2]).x0,
        h.engine().content_area(c).x0,
        "in view"
    );
    assert_eq!(*changes.borrow(), vec![2]);
    // Clicking the active tab changes nothing.
    h.tap(center(&h, button(&h, tv, 2)));
    h.run_until_idle();
    assert_eq!(*changes.borrow(), vec![2]);
    h.assert_idle();
}

#[test]
fn tabview_swipe_changes_active() {
    let (mut h, tv, _) = scene(Mode::Light);
    let changes = values(&mut h, tv);
    let c = get::<Tabview>(&h, tv).content();
    let p = center(&h, c);
    h.drag(p, Point::new(p.x - 200, p.y), Duration::ms(200));
    h.run_until_idle();
    assert_eq!(active(&h, tv), 1);
    assert!(has_state(&h, button(&h, tv, 1), State::CHECKED));
    assert_eq!(*changes.borrow(), vec![1]);
    assert_eq!(h.engine().scroll_offset(c).x, h.engine().content_area(c).width());
    h.assert_idle();
}

#[test]
fn tabview_set_active_no_anim_instant() {
    let (mut h, tv, _) = scene(Mode::Light);
    let changes = values(&mut h, tv);
    with(&mut h, tv, |w: &mut Tabview, cx| w.set_active(cx, 2, false));
    let c = get::<Tabview>(&h, tv).content();
    assert_eq!(
        h.engine().scroll_offset(c).x,
        2 * h.engine().content_area(c).width()
    );
    assert_eq!(h.engine().anim_count(), 0);
    h.run_until_idle();
    assert!(changes.borrow().is_empty(), "set_active is silent");
    // Animated: the offset moves over several frames.
    with(&mut h, tv, |w: &mut Tabview, cx| w.set_active(cx, 0, true));
    assert!(h.engine().anim_count() > 0);
    h.run_until_idle();
    assert_eq!(h.engine().scroll_offset(c).x, 0);
    h.assert_idle();
}

#[test]
fn tabview_bar_positions_layout() {
    for dir in [Dir::TOP, Dir::BOTTOM, Dir::LEFT, Dir::RIGHT] {
        let (h, tv, pages) = scene_with(Mode::Light, |h, tv| {
            with(h, tv, |w: &mut Tabview, cx| w.set_tab_bar_position(cx, dir));
        });
        let w = get::<Tabview>(&h, tv);
        let e = h.engine();
        let (bar, content) = (e.coords(w.tab_bar()), e.coords(w.content()));
        match dir {
            Dir::TOP => assert!(bar.y1 <= content.y0 && bar.height() == 65, "{dir:?}"),
            Dir::BOTTOM => assert!(bar.y0 >= content.y1 && bar.height() == 65, "{dir:?}"),
            // LVGL keeps the bar size (`dpi / 2`) when the bar moves to a side.
            Dir::LEFT => assert!(bar.x1 <= content.x0 && bar.width() == 65, "{dir:?}"),
            _ => assert!(bar.x0 >= content.x1 && bar.width() == 65, "{dir:?}"),
        }
        // Buttons stack along the bar; pages along the content.
        let (b0, b1) = (e.coords(button(&h, tv, 0)), e.coords(button(&h, tv, 1)));
        let (p0, p1) = (e.coords(pages[0]), e.coords(pages[1]));
        if dir.intersects(Dir::VER) {
            assert!(b1.x0 >= b0.x1 && p1.x0 >= p0.x1, "{dir:?}");
        } else {
            assert!(b1.y0 >= b0.y1 && p1.y0 >= p0.y1, "{dir:?}");
        }
    }
}

#[test]
fn tabview_rtl_order() {
    let (mut h, tv, pages) = scene_with(Mode::Light, |h, tv| {
        h.engine_mut()
            .set_local_prop(tv, Selector::MAIN, StyleProp::BaseDir(BaseDir::Rtl));
    });
    let e = h.engine();
    assert!(
        e.coords(button(&h, tv, 0)).x0 > e.coords(button(&h, tv, 1)).x0,
        "right to left"
    );
    let c = get::<Tabview>(&h, tv).content();
    assert_eq!(
        e.coords(pages[0]).x0,
        e.content_area(c).x0,
        "the first page in view"
    );
    with(&mut h, tv, |w: &mut Tabview, cx| w.set_active(cx, 1, false));
    h.run_until_idle();
    let e = h.engine();
    assert_eq!(e.coords(pages[1]).x0, e.content_area(c).x0);
}

#[test]
fn tabview_keypad_encoder() {
    let (mut h, tv, _) = scene(Mode::Light);
    let changes = values(&mut h, tv);
    let _ = h.keypad_input();
    let b0 = button(&h, tv, 0);
    h.engine_mut().focus(b0);
    h.key(Key::Next);
    h.key(Key::Enter);
    h.run_until_idle();
    assert_eq!(active(&h, tv), 1);
    let _ = h.encoder_input();
    h.encoder(1);
    h.encoder_click();
    h.run_until_idle();
    assert_eq!(active(&h, tv), 2);
    assert_eq!(*changes.borrow(), vec![1, 2]);
    h.assert_idle();
}

#[test]
fn tabview_rename_and_size() {
    let (mut h, tv, _) = scene(Mode::Light);
    with(&mut h, tv, |w: &mut Tabview, cx| {
        w.rename_tab(cx, 1, "Renamed");
        w.set_tab_bar_size(cx, 40);
    });
    h.run_until_idle();
    let b = button(&h, tv, 1);
    let l = h.engine().tree().children(b).next().unwrap();
    assert_eq!(get::<Label>(&h, l).text(), "Renamed");
    assert_eq!(h.engine().coords(get::<Tabview>(&h, tv).tab_bar()).height(), 40);
    with(&mut h, tv, |w: &mut Tabview, cx| w.set_tab_bar_size(cx, 40));
    h.assert_idle();
}

#[test]
fn tabview_switch_then_idle() {
    let (mut h, tv, _) = scene(Mode::Light);
    h.tap(center(&h, button(&h, tv, 1)));
    let t = h.run_until_idle();
    assert!(t > Duration::ZERO, "animated");
    h.assert_idle();
}

#[test]
fn tabview_anim_frames_invalidate_content_only() {
    let (mut h, tv, _) = scene(Mode::Light);
    let (bar, content) = {
        let w = get::<Tabview>(&h, tv);
        (w.tab_bar(), w.content())
    };
    let changed = [button(&h, tv, 0), button(&h, tv, 1)];
    h.tap(center(&h, changed[1]));
    let e = h.engine();
    let ca = e.coords(content);
    let allowed_buttons: Vec<_> = changed
        .iter()
        .map(|b| {
            e.coords(*b)
                .expand(e.style_i32(*b, Part::Main, PropId::OutlineWidth) + 8)
        })
        .collect();
    let mut frames = 0;
    for _ in 0..30 {
        h.advance(Duration::ms(10));
        for (a, reason) in h.invalidations() {
            let ok = ca.contains_rect(a) || allowed_buttons.iter().any(|b| b.contains_rect(a));
            assert!(ok, "{a:?} ({reason:?}) outside the content and the two buttons");
        }
        if !h.invalidations().is_empty() {
            frames += 1;
        }
    }
    assert!(frames > 2);
    let _ = bar;
    h.run_until_idle();
}

#[test]
fn tabview_theme_styles() {
    let (h, tv, pages) = scene(Mode::Light);
    let e = h.engine();
    let b = button(&h, tv, 0);
    // The checked tab: primary text over a muted primary, a primary bottom border.
    assert_eq!(
        e.style_color(b, Part::Main, PropId::TextColor),
        twine_theme::Palette::Blue.main()
    );
    assert_eq!(
        e.style_i32(b, Part::Main, PropId::BorderWidth),
        2 * twine_theme::dpx(2, 130)
    );
    // Pages are padded (`pad_normal`: `PAD_DEF`, 16 dpx on a small display).
    assert_eq!(
        e.style_i32(pages[0], Part::Main, PropId::PadLeft),
        twine_theme::dpx(16, 130)
    );
}

#[test]
fn snapshot_tabview() {
    for m in Mode::ALL {
        let (mut h, _, _) = scene(m);
        h.assert_snapshot(&format!("tabview_top_{}", m.suffix()));
    }
    let (mut h, _, _) = scene_with(Mode::Light, |h, tv| {
        with(h, tv, |w: &mut Tabview, cx| w.set_tab_bar_position(cx, Dir::LEFT));
    });
    h.assert_snapshot("tabview_left");
    let (mut h, tv, _) = scene(Mode::Light);
    let c = get::<Tabview>(&h, tv).content();
    let p = center(&h, c);
    h.press(p);
    for k in 1..=5 {
        h.advance(Duration::ms(30));
        h.move_to(Point::new(p.x - 20 * k, p.y));
    }
    h.advance(Duration::ms(30));
    h.assert_panel_snapshot("tabview_mid_swipe");
    h.release();
    h.run_until_idle();
}
