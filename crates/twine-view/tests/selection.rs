//! Selection and container views of Phase 19: dropdown, roller, list, menu, tabview,
//! tileview, window and msgbox bindings.

use std::cell::RefCell;
use std::rc::Rc;

use twine_reactive::runtime_stats;
use twine_testing::{TestUi, by_id};
use twine_view::TextProp;
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
    assert_eq!(
        runtime_stats()
            .faults
            .get(twine_reactive::FaultKind::EffectLoopCut),
        0
    );
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
fn dropdown_fixed_list_view_on_change_and_modifiers() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        dropdown(["Low", "Medium", "High"], 0usize)
            .dir(Side::Right)
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
        assert_eq!(w.options(), "Low\nMedium\nHigh");
        assert_eq!(
            (w.dir(), w.text(), w.selected_highlight()),
            (Side::Right, Some("Level"), false)
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
        i32::from(m.font(Part::Main).line_height) + m.style_i32(Part::Main, PropId::LineSpacing)
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
    assert_eq!(
        runtime_stats()
            .faults
            .get(twine_reactive::FaultKind::EffectLoopCut),
        0
    );
    t.assert_idle();
}

#[test]
fn roller_fixed_list_view_on_change() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut t = TestUi::new(320, 240).mount(move |_| {
        roller(["Red", "Green", "Blue"], 0usize)
            .on_change(move |i| l.borrow_mut().push(i))
            .test_id("r")
    });
    t.run_until_idle();
    let r = node(&t, "r");
    assert_eq!(
        t.engine().widget::<Roller>(r).unwrap().options(),
        "Red\nGreen\nBlue"
    );
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
                |_, k| list_button(Symbol::File, format!("Item {k}")),
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
            list_button((), "Tap me")
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
            menu_cont(label("Display")).loads(display).test_id("row"),
        ))
        .pages(
            menu_page(menu_section(menu_cont(label("Brightness"))))
                .title("Display")
                .page_ref(display),
        )
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
        menu(menu_page(label("Pick a page")))
            .sidebar(menu_page(menu_cont(label("A")).loads(a).test_id("a")).title("Settings"))
            .pages(
                menu_page((label("Page A"), menu_separator()))
                    .title("A")
                    .page_ref(a),
            )
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
        .bar_position(Side::Top)
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
    assert_eq!(
        runtime_stats()
            .faults
            .get(twine_reactive::FaultKind::EffectLoopCut),
        0
    );
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
        let at = cx.signal(TilePos::new(0, 0));
        cx.provide(at);
        tileview(
            at,
            (
                tile(TilePos::new(0, 0), Side::Right, label("A")),
                tile(TilePos::new(1, 0), Sides::LEFT | Sides::BOTTOM, label("B")),
                tile(TilePos::new(1, 1), Side::Top, label("C")),
            ),
        )
        .test_id("tv")
    });
    t.run_until_idle();
    let at = t.root_scope().expect_context::<Signal<TilePos>>();
    let tv = node(&t, "tv");
    let c = center_of(&t, tv);
    t.drag(c, Point::new(c.x - 150, c.y), Duration::ms(150));
    t.run_until_idle();
    assert_eq!(at.get_untracked(), TilePos::new(1, 0));
    at.set(TilePos::new(1, 1));
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
            window_button(Symbol::Close, 40)
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
        let h = cx.show_modal(move |_, _| {
            msgbox("Delete?", "This cannot be undone.")
                .buttons(["Yes", "No"])
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

// ---- Reactive options and translated texts (R1.S03) ---------------------------------------------

/// A two-language table: the text of `key` in language `lang` (0 = English, 1 = German).
fn tr(lang: usize, key: &'static str) -> &'static str {
    match (lang, key) {
        (1, "yes") => "Ja",
        (1, "no") => "Nein",
        (1, "low") => "Niedrig",
        (1, "high") => "Hoch",
        (_, "yes") => "Yes",
        (_, "no") => "No",
        (_, "low") => "Low",
        (_, "high") => "High",
        _ => key,
    }
}

#[test]
fn msgbox_buttons_translated_by_text_fn() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let lang = cx.signal(0usize);
        cx.provide(lang);
        let _ = cx.show_modal(move |_, _| {
            msgbox("?", "Delete?").buttons([
                text!("{}", tr(lang.get(), "yes")),
                text!("{}", tr(lang.get(), "no")),
            ])
        });
        label("behind")
    });
    t.run_until_idle();
    let lang = t.root_scope().expect_context::<Signal<usize>>();
    let labels = |t: &TestUi| -> Vec<String> {
        let e = t.engine();
        t.find_all(twine_testing::by_class("msgbox_footer_button"))
            .iter()
            .map(|b| {
                let l = e.tree().children(b.id()).next().unwrap();
                e.widget::<Label>(l).unwrap().text().to_string()
            })
            .collect()
    };
    assert_eq!(labels(&t), ["Yes", "No"]);
    lang.set(1); // outside the Ui: deferred to the next update
    let runs = runtime_stats().effect_runs;
    t.run_until_idle();
    assert_eq!(labels(&t), ["Ja", "Nein"]);
    assert_eq!(runtime_stats().effect_runs - runs, 2, "one binding per button");
    t.assert_idle();
}

#[test]
fn dropdown_items_are_texts_translated_in_one_binding() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let lang = cx.signal(0usize);
        cx.provide(lang);
        dropdown(
            [
                TextProp::StaticFn(Box::new(move || tr(lang.get(), "low"))),
                TextProp::Static("--"),
                TextProp::StaticFn(Box::new(move || tr(lang.get(), "high"))),
            ],
            2usize,
        )
        .test_id("dd")
    });
    t.run_until_idle();
    let lang = t.root_scope().expect_context::<Signal<usize>>();
    let d = node(&t, "dd");
    assert_eq!(
        t.engine().widget::<Dropdown>(d).unwrap().options(),
        "Low\n--\nHigh"
    );
    lang.set(1); // outside the Ui: deferred to the next update
    let runs = runtime_stats().effect_runs;
    t.run_until_idle();
    let e = t.engine();
    let w = e.widget::<Dropdown>(d).unwrap();
    assert_eq!(w.options(), "Niedrig\n--\nHoch");
    assert_eq!(w.selected(), 2, "the selection is kept");
    assert_eq!(runtime_stats().effect_runs - runs, 1, "one binding for all items");
}

#[test]
fn roller_options_from_a_closure_and_a_memo() {
    let mut t = TestUi::new(320, 240).mount(|cx| {
        let n = cx.signal(3u32);
        cx.provide(n);
        let names = cx.memo(move || (0..n.get()).map(|i| format!("#{i}")).collect::<Vec<_>>());
        row((
            roller(
                move || (0..n.get()).map(|i| i.to_string()).collect::<Vec<_>>(),
                1usize,
            )
            .mode(RollerMode::Infinite)
            .test_id("closure"),
            roller(names, 0usize).test_id("memo"),
            // A fixed list from an iterator.
            roller((1..=2).map(|i| format!("{i} h")), 0usize).test_id("iter"),
        ))
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<u32>>();
    let opts = |t: &TestUi, id| {
        t.engine()
            .widget::<Roller>(node(t, id))
            .unwrap()
            .options()
            .to_string()
    };
    assert_eq!(opts(&t, "closure"), "0\n1\n2");
    assert_eq!(opts(&t, "memo"), "#0\n#1\n#2");
    assert_eq!(opts(&t, "iter"), "1 h\n2 h");
    n.set(4);
    t.run_until_idle();
    assert_eq!(opts(&t, "closure"), "0\n1\n2\n3");
    assert_eq!(opts(&t, "memo"), "#0\n#1\n#2\n#3");
    let e = t.engine();
    let r = e.widget::<Roller>(node(&t, "closure")).unwrap();
    assert_eq!((r.mode(), r.selected()), (RollerMode::Infinite, 1));
}
