//! Selection and container views of Phase 19: dropdown, roller, list, menu, tabview,
//! tileview, window and msgbox bindings.

use std::cell::RefCell;
use std::rc::Rc;

use twine_reactive::debug_stats;
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn center_of(t: &TestUi, id: NodeId) -> Point {
    t.node(id).coords().center()
}

// ---- Dropdown -------------------------------------------------------------------------------

/// The center of option `i` of the open list of dropdown `d`.
fn dd_option_point(t: &TestUi, d: NodeId, i: i32) -> Point {
    let e = t.engine();
    let list = e.widget::<Dropdown>(d).and_then(Dropdown::list).expect("open");
    let td = MeasureCx::new(&e, list).text_dsc(Part::Main);
    let unit = i32::from(td.font.line_height) + td.line_space;
    let c = e.content_area(list);
    Point::new(
        c.x0 + c.width() / 2,
        c.y0 - e.scroll_offset(list).y + i * unit + i32::from(td.font.line_height) / 2,
    )
}

#[test]
fn dropdown_view_two_way() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let sel = cx.signal(1usize);
        cx.provide(sel);
        let names: Vec<String> = ["Oslo", "Lima", "Kyiv", "Pune"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        dropdown(names, sel).test_id("dd").align(Align::TopMid)
    });
    t.run_until_idle();
    let sel = t.root_scope().expect_context::<Signal<usize>>();
    let d = node(&t, "dd");
    assert_eq!(t.engine().widget::<Dropdown>(d).unwrap().selected(), 1);
    assert_eq!(
        t.engine().widget::<Dropdown>(d).unwrap().options(),
        "Oslo\nLima\nKyiv\nPune"
    );
    // User → signal.
    t.tap(center_of(&t, d));
    t.run_until_idle();
    let p = dd_option_point(&t, d, 3);
    t.tap(p);
    t.run_until_idle();
    assert_eq!(sel.get_untracked(), 3);
    assert_eq!(debug_stats().loop_cuts, 0);
    // Signal → widget.
    sel.set(0);
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Dropdown>(d).unwrap().selected(), 0);
    t.assert_idle();
}

#[test]
fn dropdown_view_options_signal_keeps_selection() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let opts = cx.signal(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        cx.provide(opts);
        dropdown(opts, 2usize).test_id("dd")
    });
    t.run_until_idle();
    let opts = t.root_scope().expect_context::<Signal<Vec<String>>>();
    let d = node(&t, "dd");
    opts.set(vec![
        "x".to_string(),
        "y".to_string(),
        "z".to_string(),
        "w".to_string(),
    ]);
    t.run_until_idle();
    let e = t.engine();
    let w = e.widget::<Dropdown>(d).unwrap();
    assert_eq!(w.options(), "x\ny\nz\nw");
    assert_eq!(w.selected(), 2, "kept");
    drop(e);
    opts.set(vec!["only".to_string()]);
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Dropdown>(d).unwrap().selected(), 0, "clamped");
}

#[test]
fn dropdown_static_view_on_change_and_modifiers() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        dropdown_static("Low\nMedium\nHigh", 0usize)
            .dir(Dir::RIGHT)
            .text("Level")
            .highlight(false)
            .on_change(move |i| l.borrow_mut().push(i))
            .test_id("dd")
            .align(Align::LeftMid)
    });
    t.run_until_idle();
    let d = node(&t, "dd");
    {
        let e = t.engine();
        let w = e.widget::<Dropdown>(d).unwrap();
        assert!(matches!(
            w.options_storage(),
            twine_widgets_ext::Options::Static(_)
        ));
        assert_eq!(
            (w.dir(), w.text(), w.selected_highlight()),
            (Dir::RIGHT, Some("Level"), false)
        );
    }
    t.tap(center_of(&t, d));
    t.run_until_idle();
    let p = dd_option_point(&t, d, 2);
    t.tap(p);
    t.run_until_idle();
    assert_eq!(*log.borrow(), vec![2]);
    t.assert_idle();
}

// ---- Roller ---------------------------------------------------------------------------------

#[test]
fn roller_view_two_way() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let sel = cx.signal(2usize);
        cx.provide(sel);
        let hours: Vec<String> = (0..24).map(|i| format!("{i:02}")).collect();
        roller(hours, sel)
            .mode(RollerMode::Infinite)
            .visible_rows(3)
            .test_id("r")
            .align(Align::Center)
    });
    t.run_until_idle();
    let sel = t.root_scope().expect_context::<Signal<usize>>();
    let r = node(&t, "r");
    {
        let e = t.engine();
        let w = e.widget::<Roller>(r).unwrap();
        assert_eq!(
            (w.selected(), w.option_count(), w.mode()),
            (2, 24, RollerMode::Infinite)
        );
        assert_eq!(w.visible_row_count(), 3);
    }
    // User → signal: tap the row below the band.
    let c = center_of(&t, r);
    let unit = {
        let e = t.engine();
        let m = MeasureCx::new(&e, r);
        i32::from(m.font(Part::Main).line_height) + m.style_i32(Part::Main, PropId::TextLineSpace)
    };
    t.tap(Point::new(c.x, c.y + unit));
    t.run_until_idle();
    assert_eq!(sel.get_untracked(), 3);
    // Signal → roller (animated), wrapping forwards from 23 to 0 in infinite mode.
    sel.set(23);
    t.run_until_idle();
    sel.set(0);
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Roller>(r).unwrap().selected(), 0);
    assert_eq!(debug_stats().loop_cuts, 0);
    t.assert_idle();
}

#[test]
fn roller_static_view_on_change() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        roller_static("Red\nGreen\nBlue", 0usize)
            .on_change(move |i| l.borrow_mut().push(i))
            .test_id("r")
    });
    t.run_until_idle();
    let r = node(&t, "r");
    assert!(matches!(
        t.engine().widget::<Roller>(r).unwrap().options_storage(),
        twine_widgets_ext::Options::Static(_)
    ));
    t.engine_mut().focus(r);
    t.key(Key::Down);
    t.key(Key::Enter);
    t.run_until_idle();
    assert_eq!(*log.borrow(), vec![1]);
}

// ---- List -----------------------------------------------------------------------------------

#[test]
fn list_view_for_each_reorder_minimal_moves() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let items = cx.signal(vec![1u32, 2, 3, 4]);
        cx.provide(items);
        list((
            list_text("Numbers"),
            for_each(
                move || items.get(),
                |k| *k,
                |_, k| list_button(Some(ImageSource::Symbol(symbols::FILE)), format!("Item {k}")),
            ),
        ))
        .test_id("list")
    });
    t.run_until_idle();
    let items = t.root_scope().expect_context::<Signal<Vec<u32>>>();
    let ((), logs) = twine_testing::capture_logs(|| {
        items.set(vec![1, 3, 2, 4]);
        t.run_until_idle();
    });
    let line = logs
        .iter()
        .map(|l| l.message.clone())
        .find(|m| m.starts_with("for_each reconcile"))
        .unwrap_or_default();
    assert_eq!(line, "for_each reconcile: kept=3 moved=1 created=0 deleted=0");
    let e = t.engine();
    let l = t.find(by_id("list")).id();
    assert_eq!(e.tree().node(l).unwrap().class().name, "list");
    let texts: Vec<String> = t
        .find_all(twine_testing::by_class("list_button"))
        .into_iter()
        .map(|b| {
            twine_widgets_ext::list::button_text(&e, b.id())
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(texts, ["Item 1", "Item 3", "Item 2", "Item 4"]);
}

#[test]
fn list_view_button_click_and_modifiers() {
    let clicks = Rc::new(std::cell::Cell::new(0));
    let c = clicks.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        list(
            list_button(None, "Tap me")
                .test_id("b")
                .on_click(move || c.set(c.get() + 1)),
        )
    });
    t.run_until_idle();
    let b = node(&t, "b");
    assert_eq!(t.engine().tree().node(b).unwrap().class().name, "list_button");
    t.tap(center_of(&t, b));
    assert_eq!(clicks.get(), 1);
}

// ---- Menu -----------------------------------------------------------------------------------

#[test]
fn menu_view_page_refs_resolve_after_build() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        // The row refers to a page defined after it.
        let display = cx.menu_page_ref();
        menu(menu_page(
            None,
            menu_cont(label("Display")).loads(display).test_id("row"),
        ))
        .pages(menu_page(Some("Display"), menu_section(menu_cont(label("Brightness")))).page_ref(display))
        .test_id("menu")
    });
    t.run_until_idle();
    let m = node(&t, "menu");
    let row = node(&t, "row");
    t.tap(center_of(&t, row));
    t.run_until_idle();
    let e = t.engine();
    let w = e.widget::<Menu>(m).unwrap();
    let page = w.cur_main_page().unwrap();
    assert_eq!(e.widget::<MenuPage>(page).unwrap().title(), Some("Display"));
    assert_eq!(w.history().len(), 2);
    let title = w.main_header_title();
    assert_eq!(
        e.widget::<twine_widgets::label::Label>(title).unwrap().text(),
        "Display"
    );
}

#[test]
fn menu_view_sidebar_and_modes() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let a = cx.menu_page_ref();
        menu(menu_page(None, label("Pick a page")))
            .sidebar(menu_page(
                Some("Settings"),
                menu_cont(label("A")).loads(a).test_id("a"),
            ))
            .pages(menu_page(Some("A"), (label("Page A"), menu_separator())).page_ref(a))
            .header_mode(MenuHeaderMode::BottomFixed)
            .root_back_button(false)
            .test_id("menu")
    });
    t.run_until_idle();
    let m = node(&t, "menu");
    assert!(
        t.engine()
            .widget::<Menu>(m)
            .unwrap()
            .sidebar_container()
            .is_some()
    );
    let a = node(&t, "a");
    t.tap(center_of(&t, a));
    t.run_until_idle();
    assert!(t.node(a).state().contains(State::CHECKED));
    assert_eq!(
        t.engine().widget::<Menu>(m).unwrap().mode_header(),
        MenuHeaderMode::BottomFixed
    );
}

// ---- Tabview / tileview -----------------------------------------------------------------------

#[test]
fn tabview_model_two_way() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let tab_idx = cx.signal(0usize);
        cx.provide(tab_idx);
        let title = cx.signal(String::from("Dynamic"));
        cx.provide(title);
        tabview(
            tab_idx,
            (
                tab("One", label("First")),
                tab(move || title.get(), label("Second")),
                tab("Three", label("Third")),
            ),
        )
        .bar_position(Dir::TOP)
        .animated(false)
        .test_id("tv")
    });
    t.run_until_idle();
    let tab_idx = t.root_scope().expect_context::<Signal<usize>>();
    let tv = node(&t, "tv");
    // User → signal.
    let b2 = t
        .engine()
        .widget::<Tabview>(tv)
        .unwrap()
        .tab_button(&t.engine(), 2)
        .unwrap();
    t.tap(center_of(&t, b2));
    t.run_until_idle();
    assert_eq!(tab_idx.get_untracked(), 2);
    // Signal → tabview.
    tab_idx.set(1);
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Tabview>(tv).unwrap().active(), 1);
    // A dynamic title.
    let title = t.root_scope().expect_context::<Signal<String>>();
    title.set(String::from("Renamed"));
    t.run_until_idle();
    let e = t.engine();
    let b1 = e.widget::<Tabview>(tv).unwrap().tab_button(&e, 1).unwrap();
    let l = e.tree().children(b1).next().unwrap();
    assert_eq!(
        e.widget::<twine_widgets::label::Label>(l).unwrap().text(),
        "Renamed"
    );
    drop(e);
    assert_eq!(debug_stats().loop_cuts, 0);
    t.assert_idle();
}

#[test]
fn tab_outside_tabview_warns() {
    let (mut t, logs) =
        twine_testing::capture_logs(|| TestUi::new(200, 100).mount(|_| column(tab("Lost", label("x")))));
    t.run_until_idle();
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("tab outside a tabview"))
    );
}

#[test]
fn tileview_model_two_way() {
    let mut t = TestUi::new(240, 200).mount(|cx| {
        let at = cx.signal((0u8, 0u8));
        cx.provide(at);
        tileview(
            at,
            (
                tile(0, 0, Dir::RIGHT, label("A")),
                tile(1, 0, Dir::LEFT | Dir::BOTTOM, label("B")),
                tile(1, 1, Dir::TOP, label("C")),
            ),
        )
        .test_id("tv")
    });
    t.run_until_idle();
    let at = t.root_scope().expect_context::<Signal<(u8, u8)>>();
    let tv = node(&t, "tv");
    let c = center_of(&t, tv);
    t.drag(c, Point::new(c.x - 150, c.y), Duration::ms(150));
    t.run_until_idle();
    assert_eq!(at.get_untracked(), (1, 0));
    at.set((1, 1));
    t.run_until_idle();
    let e = t.engine();
    let active = e.widget::<Tileview>(tv).unwrap().tile_active().unwrap();
    let tile = e.widget::<Tile>(active).unwrap();
    assert_eq!((tile.col(), tile.row()), (1, 1));
}

// ---- Window / msgbox --------------------------------------------------------------------------

#[test]
fn window_view_builds_header_and_content() {
    let clicks = Rc::new(std::cell::Cell::new(0));
    let c = clicks.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        window(
            "Title",
            window_button(ImageSource::Symbol(symbols::CLOSE), 40)
                .test_id("close")
                .on_click(move || c.set(c.get() + 1)),
            column((label("Body").test_id("body"),)),
        )
        .header_height(50)
        .test_id("win")
    });
    t.run_until_idle();
    let w = node(&t, "win");
    let (header, content) = {
        let e = t.engine();
        let win = e.widget::<Window>(w).unwrap();
        (win.header(), win.content())
    };
    assert_eq!(t.node(header).coords().height(), 50);
    let close = node(&t, "close");
    assert_eq!(t.engine().tree().parent(close), Some(header));
    let body = node(&t, "body");
    assert!(t.engine().tree().ancestors(body).any(|a| a == content));
    t.tap(center_of(&t, close));
    assert_eq!(clicks.get(), 1);
}

#[test]
fn msgbox_view_modal_buttons_and_close() {
    let pressed = Rc::new(RefCell::new(Vec::new()));
    let closed = Rc::new(std::cell::Cell::new(false));
    let (p2, c2) = (pressed.clone(), closed.clone());
    let mut t = TestUi::new(320, 240).mount(move |cx| {
        let p3 = p2.clone();
        let c3 = c2.clone();
        let h = cx.show_modal(move |_| {
            msgbox("Delete?", "This cannot be undone.")
                .buttons(&["Yes", "No"])
                .close_button(true)
                .on_button(move |i| p3.borrow_mut().push(i))
                .on_close(move || c3.set(true))
        });
        cx.provide(h);
        button(label("Behind")).test_id("behind")
    });
    t.run_until_idle();
    let modal = t.root_scope().expect_context::<ModalHandle>();
    assert!(modal.is_open());
    let footer: Vec<NodeId> = t
        .find_all(twine_testing::by_class("msgbox_footer_button"))
        .into_iter()
        .map(|n| n.id())
        .collect();
    assert_eq!(footer.len(), 2);
    t.tap(center_of(&t, footer[1]));
    t.run_until_idle();
    assert_eq!(*pressed.borrow(), vec![1]);
    assert!(modal.is_open(), "the application decides");
    let close = t.find(twine_testing::by_class("msgbox_header_button")).id();
    t.tap(center_of(&t, close));
    t.run_until_idle();
    assert!(closed.get());
    assert!(!modal.is_open());
    assert!(t.find_all(twine_testing::by_class("msgbox")).is_empty());
    t.assert_idle();
}
