//! The selection and container widgets: a window whose header has an "i" button (a modal
//! "About" message box) and whose content is a tabview:
//!
//! - **Pickers**: a dropdown and an infinite roller of the same ten cities, both bound to one
//!   signal (they stay in sync), and a label showing the city.
//! - **Lists**: a list of twenty buttons with symbols (built by `for_each`) and a settings
//!   menu with a sidebar (Display / Sound pages with switches and sliders).
//! - **Tiles**: a 2 × 2 tileview; each tile shows where it can be swiped to.
//!
//! Touch, the PC keyboard (keypad) and the mouse wheel / encoder work everywhere.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use twine::prelude::*;

/// The cities of the pickers.
pub const CITIES: [&str; 10] = [
    "Amsterdam",
    "Berlin",
    "Cairo",
    "Delhi",
    "Lima",
    "Madrid",
    "Oslo",
    "Paris",
    "Rome",
    "Tokyo",
];

/// The demo's state, provided to the scope as context (tests read it).
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    /// The selected city (index into [`CITIES`]), shared by the dropdown and the roller.
    pub city: Signal<usize>,
    /// The active tab.
    pub tab: Signal<usize>,
    /// The tile in view.
    pub tile: Signal<TilePos>,
    /// The number of times the About box was opened.
    pub about_opened: Signal<u32>,
}

/// The height of the window header and of the tab bar.
const BAR_H: i32 = 40;

/// The selection demo.
///
/// ```
/// use twine_demos::selection::{Selection, app};
/// use twine_testing::TestUi;
///
/// let mut t = TestUi::new(320, 240).mount(app);
/// t.run_until_idle();
/// let s = t.root_scope().expect_context::<Selection>();
/// s.city.set(3);
/// t.run_until_idle();
/// assert_eq!(s.city.get_untracked(), 3);
/// ```
pub fn app(cx: Scope) -> impl View {
    let s = Selection {
        city: cx.signal(1),
        tab: cx.signal(0),
        tile: cx.signal(TilePos::new(0, 0)),
        about_opened: cx.signal(0),
    };
    cx.provide(s);
    window(
        "Selection",
        window_button(ImageSource::Symbol("i"), BAR_H)
            .test_id("about")
            .on_click(move || {
                s.about_opened.update(|n| *n += 1);
                let _ = cx.show_modal(|_, _| {
                    msgbox(
                        "About",
                        "Dropdown, roller, list, menu, tabview, tileview, window and msgbox.",
                    )
                    .buttons(["OK"])
                    .close_button(true)
                    .test_id("about_box")
                });
            }),
        tabview(
            s.tab,
            (
                tab("Pickers", pickers(s)),
                // The heavier pages exist only while shown: the heap holds the largest page,
                // not all of them (an RP2040 has a 96 KiB heap).
                tab("Lists", when(move || s.tab.get() == 1, lists)),
                tab("Tiles", when(move || s.tab.get() == 2, move |_| tiles(s))),
            ),
        )
        .bar_size(BAR_H)
        .fill()
        .test_id("tabs"),
    )
    .header_height(BAR_H)
    .content_padding(0)
}

/// The Pickers tab: a dropdown and a roller sharing `city`.
fn pickers(s: Selection) -> impl View {
    row((
        column((
            dropdown(CITIES, s.city).width(120).test_id("dropdown"),
            label(move || {
                let i = s.city.get();
                format!("City: {}", CITIES.get(i).copied().unwrap_or("?"))
            })
            .test_id("city"),
        ))
        .gap(12),
        roller(CITIES, s.city)
            .mode(RollerMode::Infinite)
            .visible_rows(3)
            .test_id("roller"),
    ))
    .gap(16)
    .size(Length::pct(100), Length::Content)
    .align_items(CrossAlign::Start)
}

/// The symbols of the list buttons, cycled.
const ICONS: [Symbol; 10] = [
    Symbol::File,
    Symbol::Directory,
    Symbol::Save,
    Symbol::Image,
    Symbol::Audio,
    Symbol::Video,
    Symbol::Bluetooth,
    Symbol::Wifi,
    Symbol::Usb,
    Symbol::Gps,
];

/// The Lists tab: a list of 20 buttons and a settings menu with a sidebar.
fn lists(cx: Scope) -> impl View {
    let items: Signal<Vec<u32>> = cx.signal((0..20).collect());
    let display = cx.menu_page_ref();
    let sound = cx.menu_page_ref();
    let row_switch = |text: &'static str, on: bool| menu_cont((label(text).flex_grow(1), switch(on)));
    column((
        list((
            list_text("Files"),
            for_each(
                move || items.get(),
                |i| *i,
                |_, i| list_button(ICONS[i as usize % ICONS.len()], format!("Item {}", i + 1)),
            ),
        ))
        .size(Length::pct(100), 120)
        .test_id("list"),
        menu(menu_page(label("Pick a page").padding(8)))
            .sidebar(
                menu_page((
                    menu_cont(label("Display")).loads(display).test_id("menu_display"),
                    menu_cont(label("Sound")).loads(sound).test_id("menu_sound"),
                ))
                .title("Settings"),
            )
            .pages((
                menu_page(menu_section((
                    row_switch("Dark mode", false),
                    menu_cont(slider(60).flex_grow(1)),
                )))
                .title("Display")
                .page_ref(display),
                menu_page(menu_section((
                    row_switch("Mute", false),
                    menu_cont(slider(30).flex_grow(1)),
                )))
                .title("Sound")
                .page_ref(sound),
            ))
            .size(Length::pct(100), 180)
            .test_id("menu"),
    ))
    .size(Length::pct(100), Length::Content)
    .gap(8)
}

/// The Tiles tab: a 2 × 2 tileview; each tile names the directions it can be swiped to.
fn tiles(s: Selection) -> impl View {
    let t = |col: u8, row: u8, dirs: Sides, text: String| {
        tile(TilePos::new(col, row), dirs, label(text).align(Align::Center))
    };
    tileview(
        s.tile,
        (
            t(
                0,
                0,
                Sides::RIGHT | Sides::BOTTOM,
                format!("Tile 1  {}  {}", Symbol::Right, Symbol::Down),
            ),
            t(
                1,
                0,
                Sides::LEFT | Sides::BOTTOM,
                format!("{}  Tile 2  {}", Symbol::Left, Symbol::Down),
            ),
            t(
                0,
                1,
                Sides::TOP | Sides::RIGHT,
                format!("{}  Tile 3  {}", Symbol::Up, Symbol::Right),
            ),
            t(
                1,
                1,
                Sides::TOP | Sides::LEFT,
                format!("{}  Tile 4  {}", Symbol::Left, Symbol::Up),
            ),
        ),
    )
    .fill()
    .scrollbar(ScrollbarMode::Off)
    .test_id("tiles")
}
