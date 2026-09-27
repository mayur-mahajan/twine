//! `Menu`: pages with a header and a back button, the history, the root back button mode,
//! the sidebar with checked rows, header modes, keypad navigation and cleanup.

mod common;

use common::{Mode, center, class, get, harness_with_group, has_state, with};
use twine_core::Duration;
use twine_engine::{EventCode, Key, NodeId, ObjFlags, State};
use twine_style::{Align, Length};
use twine_testing::EngineHarness;
use twine_widgets::label::Label;
use twine_widgets_ext::menu::{self, Menu, MenuHeaderMode};

struct Scene {
    h: EngineHarness,
    menu: NodeId,
    root: NodeId,
    display: NodeId,
    advanced: NodeId,
    /// Root row → display; display row → advanced.
    rows: [NodeId; 2],
}

fn label(h: &mut EngineHarness, parent: NodeId, text: &'static str) -> NodeId {
    twine_widgets::label::create_with(h.engine_mut(), parent, text).unwrap()
}

fn scene(mode: Mode) -> Scene {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let m = menu::create(e, screen).unwrap();
    e.set_size(m, Length::pct(100), Length::pct(100));
    e.align(m, Align::Center, 0, 0);
    let root = menu::page_create(e, m, None).unwrap();
    let display = menu::page_create(e, m, Some("Display")).unwrap();
    let advanced = menu::page_create(e, m, Some("Advanced")).unwrap();
    let r0 = menu::cont_create(e, root).unwrap();
    let r1 = menu::cont_create(e, display).unwrap();
    let sec = menu::section_create(e, advanced).unwrap();
    let r2 = menu::cont_create(e, sec).unwrap();
    menu::separator_create(e, advanced).unwrap();
    menu::set_load_page_event(e, m, r0, display);
    menu::set_load_page_event(e, m, r1, advanced);
    label(&mut h, r0, "Display");
    label(&mut h, r1, "Advanced");
    label(&mut h, r2, "Nothing here");
    with(&mut h, m, |w: &mut Menu, cx| w.set_page(cx, Some(root)));
    h.run_until_idle();
    Scene {
        h,
        menu: m,
        root,
        display,
        advanced,
        rows: [r0, r1],
    }
}

fn hidden(h: &EngineHarness, n: NodeId) -> bool {
    h.engine().has_flag(n, ObjFlags::HIDDEN)
}

fn title_text(h: &EngineHarness, m: NodeId) -> Option<String> {
    let t = get::<Menu>(h, m).main_header_title();
    (!hidden(h, t)).then(|| get::<Label>(h, t).text().to_string())
}

#[test]
fn menu_structure_and_defaults() {
    let s = scene(Mode::Light);
    let h = &s.h;
    let w = get::<Menu>(h, s.menu);
    assert_eq!(class(h, s.menu), "menu");
    assert_eq!(class(h, w.main_container()), "menu_main_container");
    assert_eq!(class(h, w.main_header()), "menu_main_header_container");
    assert_eq!(class(h, s.root), "menu_page");
    assert_eq!(class(h, s.rows[0]), "menu_cont");
    assert!(hidden(h, w.storage()));
    assert_eq!(
        h.engine().tree().parent(s.display),
        Some(w.storage()),
        "not shown yet"
    );
    assert_eq!(h.engine().tree().parent(s.root), Some(w.main_container()));
    assert_eq!(w.mode_header(), MenuHeaderMode::TopFixed);
    // The root page has no title and no back button: the header is hidden.
    assert!(hidden(h, w.main_header()));
    assert!(hidden(h, w.main_header_back_button()));
}

#[test]
fn menu_load_page_updates_header_and_history() {
    let mut s = scene(Mode::Light);
    let changes = common::count_events(&mut s.h, s.menu, EventCode::ValueChanged);
    s.h.tap(center(&s.h, s.rows[0]));
    s.h.run_until_idle();
    let w = get::<Menu>(&s.h, s.menu);
    assert_eq!(w.cur_main_page(), Some(s.display));
    assert_eq!(w.history(), &[s.root, s.display]);
    assert!(!hidden(&s.h, w.main_header()));
    assert!(!hidden(&s.h, w.main_header_back_button()));
    assert_eq!(title_text(&s.h, s.menu).as_deref(), Some("Display"));
    assert_eq!(
        s.h.engine().tree().parent(s.root),
        Some(w.storage()),
        "the root went back"
    );
    assert_eq!(changes.get(), 1);
    s.h.tap(center(&s.h, s.rows[1]));
    s.h.run_until_idle();
    assert_eq!(get::<Menu>(&s.h, s.menu).history().len(), 3);
    assert_eq!(title_text(&s.h, s.menu).as_deref(), Some("Advanced"));
    s.h.assert_idle();
}

#[test]
fn menu_back_pops_history() {
    let mut s = scene(Mode::Light);
    s.h.tap(center(&s.h, s.rows[0]));
    s.h.tap(center(&s.h, s.rows[1]));
    s.h.run_until_idle();
    let back = get::<Menu>(&s.h, s.menu).main_header_back_button();
    s.h.tap(center(&s.h, back));
    s.h.run_until_idle();
    let w = get::<Menu>(&s.h, s.menu);
    assert_eq!(w.cur_main_page(), Some(s.display));
    assert_eq!(w.history(), &[s.root, s.display]);
    s.h.tap(center(&s.h, back));
    s.h.run_until_idle();
    let w = get::<Menu>(&s.h, s.menu);
    assert_eq!(w.cur_main_page(), Some(s.root));
    assert!(hidden(&s.h, w.main_header_back_button()), "no back at the root");
    s.h.assert_idle();
}

#[test]
fn menu_root_back_button_mode() {
    let mut s = scene(Mode::Light);
    with(&mut s.h, s.menu, |w: &mut Menu, cx| {
        w.set_mode_root_back_button(cx, true);
    });
    s.h.run_until_idle();
    let w = get::<Menu>(&s.h, s.menu);
    let back = w.main_header_back_button();
    assert!(!hidden(&s.h, back), "shown on the root");
    assert_eq!(w.history(), &[s.root]);
    // A click on the root's back button goes nowhere; the application sees it as root.
    let seen_root = std::rc::Rc::new(std::cell::Cell::new(false));
    let sr = seen_root.clone();
    let m = s.menu;
    s.h.engine_mut().add_event_handler(
        back,
        twine_engine::EventFilter::Code(EventCode::Clicked),
        move |cx, _| {
            sr.set(
                cx.engine()
                    .widget::<Menu>(m)
                    .unwrap()
                    .back_button_is_root(cx.node()),
            );
            twine_engine::EventResult::Continue
        },
    );
    s.h.tap(center(&s.h, back));
    assert!(seen_root.get());
    assert_eq!(get::<Menu>(&s.h, s.menu).cur_main_page(), Some(s.root));
}

#[test]
fn menu_sidebar_marks_checked() {
    let mut s = scene(Mode::Light);
    let e = s.h.engine_mut();
    let side = menu::page_create(e, s.menu, Some("Settings")).unwrap();
    let a = menu::cont_create(e, side).unwrap();
    let b = menu::cont_create(e, side).unwrap();
    menu::set_load_page_event(e, s.menu, a, s.display);
    menu::set_load_page_event(e, s.menu, b, s.advanced);
    label(&mut s.h, a, "Display");
    label(&mut s.h, b, "Advanced");
    with(&mut s.h, s.menu, |w: &mut Menu, cx| {
        w.set_sidebar_page(cx, Some(side));
    });
    s.h.run_until_idle();
    let w = get::<Menu>(&s.h, s.menu);
    let sb = w.sidebar_container().unwrap();
    assert_eq!(class(&s.h, sb), "menu_sidebar_container");
    assert_eq!(s.h.engine().coords(sb).width(), 320 * 30 / 100);
    assert_eq!(s.h.engine().tree().parent(side), Some(sb));
    assert!(hidden(&s.h, w.sidebar_header_back_button().unwrap()));
    s.h.tap(center(&s.h, a));
    s.h.run_until_idle();
    assert!(has_state(&s.h, a, State::CHECKED));
    assert_eq!(get::<Menu>(&s.h, s.menu).cur_main_page(), Some(s.display));
    assert_eq!(
        get::<Menu>(&s.h, s.menu).history(),
        &[s.display],
        "a sidebar row starts over"
    );
    s.h.tap(center(&s.h, b));
    s.h.run_until_idle();
    assert!(has_state(&s.h, b, State::CHECKED));
    assert!(!has_state(&s.h, a, State::CHECKED));
    // Removing the sidebar deletes its column and keeps the page in storage.
    with(&mut s.h, s.menu, |w: &mut Menu, cx| w.set_sidebar_page(cx, None));
    s.h.run_until_idle();
    assert!(!s.h.engine().tree().contains(sb));
    assert_eq!(
        s.h.engine().tree().parent(side),
        Some(get::<Menu>(&s.h, s.menu).storage())
    );
    s.h.assert_idle();
}

#[test]
fn menu_header_modes_layout() {
    let mut s = scene(Mode::Light);
    s.h.tap(center(&s.h, s.rows[0]));
    s.h.run_until_idle();
    let (main, header) = {
        let w = get::<Menu>(&s.h, s.menu);
        (w.main_container(), w.main_header())
    };
    let first = |h: &EngineHarness| h.engine().tree().children(main).next();
    assert_eq!(first(&s.h), Some(header));
    let hc = s.h.engine().coords(header);
    let pc = s.h.engine().coords(s.display);
    assert!(hc.y1 <= pc.y0);
    assert_eq!(
        pc.y1,
        s.h.engine().content_area(main).y1,
        "top fixed: the page fills the rest"
    );
    with(&mut s.h, s.menu, |w: &mut Menu, cx| {
        w.set_mode_header(cx, MenuHeaderMode::BottomFixed);
    });
    s.h.run_until_idle();
    assert_eq!(s.h.engine().tree().children(main).last(), Some(header));
    assert!(s.h.engine().coords(header).y0 >= s.h.engine().coords(s.display).y1);
    with(&mut s.h, s.menu, |w: &mut Menu, cx| {
        w.set_mode_header(cx, MenuHeaderMode::TopUnfixed);
    });
    s.h.run_until_idle();
    assert_eq!(first(&s.h), Some(header));
    assert!(
        s.h.engine().coords(s.display).height()
            < s.h.engine().content_area(main).height() - s.h.engine().coords(header).height(),
        "top unfixed: the page is content high"
    );
    s.h.assert_idle();
}

#[test]
fn menu_clear_history() {
    let mut s = scene(Mode::Light);
    s.h.tap(center(&s.h, s.rows[0]));
    s.h.run_until_idle();
    with(&mut s.h, s.menu, |w: &mut Menu, _| w.clear_history());
    assert!(get::<Menu>(&s.h, s.menu).history().is_empty());
    with(&mut s.h, s.menu, |w: &mut Menu, cx| w.set_page(cx, Some(s.root)));
    assert_eq!(get::<Menu>(&s.h, s.menu).history(), &[s.root]);
    with(&mut s.h, s.menu, |w: &mut Menu, cx| w.set_page(cx, None));
    assert!(get::<Menu>(&s.h, s.menu).history().is_empty());
    assert_eq!(get::<Menu>(&s.h, s.menu).cur_main_page(), None);
}

#[test]
fn menu_keypad_navigation_through_conts() {
    let mut s = scene(Mode::Light);
    let _ = s.h.keypad_input();
    let g = s.h.engine().default_group().unwrap();
    // Rows that load a page are clickable and focusable through the group.
    s.h.engine_mut().group_add(g, s.rows[0]);
    s.h.engine_mut().focus(s.rows[0]);
    s.h.key(Key::Enter);
    s.h.run_until_idle();
    assert_eq!(get::<Menu>(&s.h, s.menu).cur_main_page(), Some(s.display));
    s.h.assert_idle();
}

#[test]
fn menu_rows_validate_parents() {
    let mut s = scene(Mode::Light);
    let screen = s.h.screen();
    assert!(menu::cont_create(s.h.engine_mut(), screen).is_err());
    assert!(menu::section_create(s.h.engine_mut(), screen).is_err());
}

#[test]
fn menu_navigate_50_times_leaves_no_nodes() {
    let mut s = scene(Mode::Light);
    let back = get::<Menu>(&s.h, s.menu).main_header_back_button();
    let cycle = |s: &mut Scene| {
        s.h.tap(center(&s.h, s.rows[0]));
        s.h.run_until_idle();
        s.h.tap(center(&s.h, s.rows[1]));
        s.h.run_until_idle();
        s.h.tap(center(&s.h, back));
        s.h.run_until_idle();
        s.h.tap(center(&s.h, back));
        s.h.run_until_idle();
    };
    cycle(&mut s);
    let nodes = s.h.engine().tree().len();
    for _ in 0..50 {
        cycle(&mut s);
    }
    assert_eq!(s.h.engine().tree().len(), nodes);
    assert_eq!(get::<Menu>(&s.h, s.menu).history(), &[s.root]);
    s.h.advance(Duration::ms(10));
}

#[test]
fn snapshot_menu() {
    for m in Mode::ALL {
        let mut s = scene(m);
        s.h.assert_snapshot(&format!("menu_root_{}", m.suffix()));
        if m == Mode::Light {
            s.h.tap(center(&s.h, s.rows[0]));
            s.h.run_until_idle();
            s.h.assert_snapshot("menu_subpage");
        }
        let e = s.h.engine_mut();
        let side = menu::page_create(e, s.menu, Some("Settings")).unwrap();
        let a = menu::cont_create(e, side).unwrap();
        let b = menu::cont_create(e, side).unwrap();
        menu::set_load_page_event(e, s.menu, a, s.display);
        menu::set_load_page_event(e, s.menu, b, s.advanced);
        label(&mut s.h, a, "Display");
        label(&mut s.h, b, "Advanced");
        with(&mut s.h, s.menu, |w: &mut Menu, cx| {
            w.set_sidebar_page(cx, Some(side));
        });
        s.h.run_until_idle();
        s.h.tap(center(&s.h, a));
        s.h.run_until_idle();
        s.h.assert_snapshot(&format!("menu_sidebar_{}", m.suffix()));
    }
}
