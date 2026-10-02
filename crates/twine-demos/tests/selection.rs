//! The `selection` demo: the dropdown and the roller stay in sync both ways, a keypad-only
//! flow picks a city, tabs switch by swipe, the About box opens and closes, and the UI is idle
//! after each interaction.

use twine::core::{Duration, Point};
use twine::engine::NodeId;
use twine::prelude::*;
use twine_demos::selection::{CITIES, Selection, app};
use twine_testing::{TestUi, by_class, by_id};

fn ui() -> (TestUi, Selection) {
    let mut t = TestUi::new(320, 240).mount(app);
    t.run_until_idle();
    let s = t.root_scope().expect_context::<Selection>();
    (t, s)
}

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn center(t: &TestUi, n: NodeId) -> Point {
    t.node(n).coords().center()
}

fn dropdown_sel(t: &TestUi) -> u16 {
    let d = node(t, "dropdown");
    t.engine().widget::<Dropdown>(d).unwrap().selected()
}

fn roller_sel(t: &TestUi) -> u16 {
    let r = node(t, "roller");
    t.engine().widget::<Roller>(r).unwrap().selected()
}

/// The center of option `i` of the open dropdown list.
fn option_point(t: &TestUi, i: i32) -> Point {
    let d = node(t, "dropdown");
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
fn dropdown_and_roller_stay_in_sync() {
    let (mut t, s) = ui();
    assert_eq!((dropdown_sel(&t), roller_sel(&t)), (1, 1));
    // Dropdown → signal → roller.
    let d = node(&t, "dropdown");
    t.tap(center(&t, d));
    t.run_until_idle();
    // The list opens scrolled to the selected city; the next one is in view.
    t.tap(option_point(&t, 2));
    t.run_until_idle();
    assert_eq!(s.city.get_untracked(), 2);
    assert_eq!(roller_sel(&t), 2);
    let city = node(&t, "city");
    assert_eq!(t.node(city).text(), format!("City: {}", CITIES[2]));
    t.assert_idle();
    // Roller → signal → dropdown: tap the row below the band.
    let r = node(&t, "roller");
    let unit = {
        let e = t.engine();
        let m = MeasureCx::new(&e, r);
        i32::from(m.font(Part::Main).line_height) + m.style_i32(Part::Main, PropId::LineSpacing)
    };
    let c = center(&t, r);
    t.tap(Point::new(c.x, c.y + unit));
    t.run_until_idle();
    assert_eq!(s.city.get_untracked(), 3);
    assert_eq!(dropdown_sel(&t), 3);
    t.assert_idle();
    // Infinite roller: past the last city back to the first.
    s.city.set(9);
    t.run_until_idle();
    t.tap(Point::new(c.x, c.y + unit));
    t.run_until_idle();
    assert_eq!(s.city.get_untracked(), 0);
    assert_eq!(dropdown_sel(&t), 0);
    t.assert_idle();
}

#[test]
fn keypad_only_flow_picks_a_city() {
    let (mut t, s) = ui();
    let d = node(&t, "dropdown");
    // Tab (Next) until the dropdown has the focus.
    let mut hops = 0;
    while !t.node(d).state().contains(State::FOCUSED) {
        t.key(Key::Next);
        t.run_until_idle();
        hops += 1;
        assert!(hops < 20, "the dropdown is reachable with Tab");
    }
    t.key(Key::Enter);
    t.run_until_idle();
    assert!(t.engine().widget::<Dropdown>(d).unwrap().list().is_some(), "open");
    t.key(Key::Down);
    t.key(Key::Down);
    t.key(Key::Enter);
    t.run_until_idle();
    assert_eq!(s.city.get_untracked(), 3);
    assert_eq!(roller_sel(&t), 3);
    t.assert_idle();
}

#[test]
fn swipe_switches_tabs() {
    let (mut t, s) = ui();
    let tabs = node(&t, "tabs");
    let content = t.engine().widget::<Tabview>(tabs).unwrap().content();
    let c = center(&t, content);
    t.drag(
        Point::new(c.x + 60, c.y + 40),
        Point::new(c.x - 140, c.y + 40),
        Duration::ms(200),
    );
    t.run_until_idle();
    assert_eq!(s.tab.get_untracked(), 1);
    // Back with the model.
    s.tab.set(2);
    t.run_until_idle();
    assert_eq!(t.engine().widget::<Tabview>(tabs).unwrap().active(), 2);
    t.assert_idle();
}

#[test]
fn about_box_opens_and_closes() {
    let (mut t, s) = ui();
    let about = node(&t, "about");
    t.tap(center(&t, about));
    t.run_until_idle();
    assert_eq!(s.about_opened.get_untracked(), 1);
    assert_eq!(t.find_all(by_class("msgbox")).len(), 1);
    // The backdrop blocks the tabs.
    let close = t.find(by_class("msgbox_header_button")).id();
    t.tap(center(&t, close));
    t.run_until_idle();
    assert!(t.find_all(by_class("msgbox")).is_empty());
    t.assert_idle();
}

#[test]
fn menu_pages_load_from_the_sidebar() {
    let (mut t, s) = ui();
    s.tab.set(1);
    t.run_until_idle();
    let row = node(&t, "menu_sound");
    let menu = node(&t, "menu");
    // The menu is below the list: scroll it into view first.
    t.engine_mut().scroll_to_view_recursive(menu, false);
    t.run_until_idle();
    t.tap(center(&t, row));
    t.run_until_idle();
    let e = t.engine();
    let m = e.widget::<Menu>(menu).unwrap();
    let page = m.cur_main_page().unwrap();
    assert_eq!(e.widget::<MenuPage>(page).unwrap().title(), Some("Sound"));
    assert!(t.node(row).state().contains(State::CHECKED));
}

#[test]
fn heap_stable_after_20_open_close_cycles() {
    use twine_testing::alloc::count_allocs;
    let (mut t, _) = ui();
    let d = node(&t, "dropdown");
    let about = node(&t, "about");
    let cycle = |t: &mut TestUi| {
        t.tap(center(t, d));
        t.run_until_idle();
        t.tap(center(t, d));
        t.run_until_idle();
        t.tap(center(t, about));
        t.run_until_idle();
        let close = t.find(by_class("msgbox_header_button")).id();
        t.tap(center(t, close));
        t.run_until_idle();
    };
    cycle(&mut t);
    cycle(&mut t);
    let nodes = t.engine().tree().len();
    let ((), stats) = count_allocs(|| {
        for _ in 0..20 {
            cycle(&mut t);
        }
    });
    assert_eq!(t.engine().tree().len(), nodes);
    assert!(stats.live <= 0, "heap grew by {} B", stats.live);
}

#[test]
fn snapshot_selection() {
    for (theme, name) in [(DefaultTheme::light(), "light"), (DefaultTheme::dark(), "dark")] {
        let mut t = TestUi::new(320, 240).theme(std::rc::Rc::new(theme)).mount(app);
        t.run_until_idle();
        let s = t.root_scope().expect_context::<Selection>();
        for (tab, tab_name) in [(0usize, "pickers"), (1, "lists"), (2, "tiles")] {
            s.tab.set(tab);
            t.run_until_idle();
            t.assert_snapshot(&format!("selection_{tab_name}_{name}"));
        }
    }
}
