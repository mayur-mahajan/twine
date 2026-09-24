//! `cargo xtask sim scroll`: scrolling in the engine — drag, momentum, elastic ends, snapping,
//! scroll chaining, scrollbars and scroll-on-focus — on an imperative scene.
//!
//! The screen is a page scrolling vertically (a flex column) with:
//!
//! - a list 200 px high of 100 clickable rows (darker while pressed; clicks are logged) with an
//!   `Auto` scrollbar: drag it, throw it (drag fast and let go), pull it beyond its ends;
//! - a carousel of 8 cards that snaps the nearest card to its center and moves at most one card
//!   per swipe (`SCROLL_ONE`);
//! - a 200 × 150 panel over 600 × 600 px of tiles, scrollable in both directions; at its edges
//!   the drag continues on the page (scroll chaining).
//!
//! Keys: `s` cycles the list's scrollbar mode (Auto → On → Active → Off), `e` toggles its
//! elastic ends, `m` its momentum. Tab / Shift+Tab move the focus through the rows; the focused
//! row is scrolled into view (through the page too). F2 tints every redrawn area: only the
//! scrolled container is refreshed while it moves.
//!
//! `RUST_LOG=twine::scroll=debug,twine::input=debug` logs scroll targets, throws and ends.

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, Opa};
use twine_engine::{Engine, EventCode, EventFilter, EventResult, NodeId, ObjFlags, ScrollbarMode, State};
use twine_examples::scenes::scrollbar_style;
use twine_examples::tile::{PALETTE, tile};
use twine_hal::Key;
use twine_sim::{SimConfig, run_engine};
use twine_style::{Dir, FlexAlign, FlexFlow, LayoutKind, Length, ScrollSnap, Selector, StyleProp};

const W: i32 = 320;
const H: i32 = 240;
/// Padding of the page and gap between its sections.
const GAP: i32 = 8;

fn set(e: &mut Engine, id: NodeId, props: &[StyleProp]) {
    for p in props {
        e.set_local_prop(id, Selector::MAIN, *p);
    }
}

fn pad(p: i32) -> [StyleProp; 4] {
    [
        StyleProp::PadLeft(p),
        StyleProp::PadTop(p),
        StyleProp::PadRight(p),
        StyleProp::PadBottom(p),
    ]
}

/// Flags of an interactive tile (tiles have no flags of their own).
const ITEM_FLAGS: ObjFlags = ObjFlags::CLICKABLE
    .union(ObjFlags::CLICK_FOCUSABLE)
    .union(ObjFlags::PRESS_LOCK)
    .union(ObjFlags::SCROLL_ON_FOCUS)
    .union(ObjFlags::SCROLL_CHAIN)
    .union(ObjFlags::SNAPPABLE)
    .union(ObjFlags::GESTURE_BUBBLE);

/// Logs the scroll events of a container.
fn log_scroll(e: &mut Engine, id: NodeId, name: &'static str) {
    e.add_event_handler(id, EventFilter::All, move |cx, ev| {
        if matches!(
            ev.code,
            EventCode::ScrollBegin | EventCode::ScrollThrowBegin | EventCode::ScrollEnd
        ) && ev.target == id
        {
            let off = cx.engine().scroll_offset(id);
            twine_core::info!(target: "twine::sim", "{:?} on {} at ({}, {})", ev.code, name, off.x, off.y);
        }
        EventResult::Continue
    });
}

/// The list: 100 rows in a 200 px high container.
fn list(e: &mut Engine, page: NodeId) -> NodeId {
    let list = e.create(page, Box::new(twine_engine::Obj)).expect("list");
    e.set_test_id(list, "list");
    e.set_size(list, Length::pct(100), 200);
    e.set_layout(list, LayoutKind::Flex);
    e.set_flex_flow(list, FlexFlow::Column);
    set(
        e,
        list,
        &[StyleProp::BgColor(Color::WHITE), StyleProp::BgOpa(Opa::COVER)],
    );
    set(e, list, &[StyleProp::Radius(6), StyleProp::ClipCorner(true)]);
    scrollbar_style(e, list, 4);
    e.set_scroll_dir(list, Dir::VER);
    let group = e.default_group();
    for i in 0..100 {
        let c = if i % 2 == 0 { 0x42_72_C4 } else { 0x9E_A7_B3 };
        let row = tile(e, list, format!("Row {}", i + 1), Color::hex(c));
        e.set_flag(row, ITEM_FLAGS, true);
        e.set_size(row, Length::pct(100), 40);
        set(e, row, &[StyleProp::Radius(0)]);
        e.set_local_prop(
            row,
            Selector::state(State::PRESSED),
            StyleProp::BgColor(Color::hex(0x1A_23_7E)),
        );
        e.set_local_prop(row, Selector::state(State::FOCUS_KEY), StyleProp::BorderWidth(3));
        e.set_local_prop(
            row,
            Selector::state(State::FOCUS_KEY),
            StyleProp::BorderColor(Color::hex(0xFF_6D_00)),
        );
        e.add_event_handler(row, EventFilter::Code(EventCode::Clicked), move |_, _| {
            twine_core::info!(target: "twine::sim", "Clicked row {}", i + 1);
            EventResult::Continue
        });
        if let Some(g) = group {
            e.group_add(g, row);
        }
    }
    log_scroll(e, list, "list");
    list
}

/// The carousel: 8 cards of 160 × 100, centered snapping, one card per swipe.
fn carousel(e: &mut Engine, page: NodeId) -> NodeId {
    let car = e.create(page, Box::new(twine_engine::Obj)).expect("carousel");
    e.set_test_id(car, "carousel");
    e.set_size(car, Length::pct(100), 120);
    e.set_layout(car, LayoutKind::Flex);
    e.set_flex_flow(car, FlexFlow::Row);
    e.set_flex_align(car, FlexAlign::Start, FlexAlign::Center, FlexAlign::Center);
    // Side paddings let the first and the last card reach the center.
    let side = (W - 2 * GAP - 160) / 2;
    set(
        e,
        car,
        &[
            StyleProp::PadLeft(side),
            StyleProp::PadRight(side),
            StyleProp::PadColumn(12),
        ],
    );
    set(
        e,
        car,
        &[
            StyleProp::BgColor(Color::hex(0x26_32_38)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(6),
        ],
    );
    scrollbar_style(e, car, 4);
    e.set_scroll_dir(car, Dir::HOR);
    e.set_scroll_snap_x(car, ScrollSnap::Center);
    e.set_flag(car, ObjFlags::SCROLL_ONE, true);
    for (i, &c) in PALETTE.iter().enumerate() {
        let card = tile(e, car, format!("Card {}", i + 1), Color::hex(c));
        e.set_flag(card, ITEM_FLAGS.difference(ObjFlags::CLICK_FOCUSABLE), true);
        e.set_size(card, 160, 100);
        set(e, card, &[StyleProp::Radius(10)]);
    }
    log_scroll(e, car, "carousel");
    car
}

/// The 2-D panel: 200 × 150 over a 6 × 6 grid of 100 px tiles.
fn panel(e: &mut Engine, page: NodeId) -> NodeId {
    let p = e.create(page, Box::new(twine_engine::Obj)).expect("panel");
    e.set_test_id(p, "panel");
    e.set_size(p, 200, 150);
    set(
        e,
        p,
        &[
            StyleProp::BgColor(Color::WHITE),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::BorderWidth(2),
        ],
    );
    set(e, p, &[StyleProp::BorderColor(Color::hex(0x26_32_38))]);
    scrollbar_style(e, p, 4);
    for i in 0..36 {
        let (x, y) = ((i % 6) * 100, (i / 6) * 100);
        let t = tile(
            e,
            p,
            format!("{},{}", i % 6, i / 6),
            Color::hex(PALETTE[(i + i / 6) % 8]),
        );
        e.set_flag(t, ObjFlags::SCROLL_CHAIN | ObjFlags::GESTURE_BUBBLE, true);
        e.set_pos(t, x as i32 + 4, y as i32 + 4);
        e.set_size(t, 92, 92);
    }
    log_scroll(e, p, "panel");
    p
}

fn scene(e: &mut Engine) {
    e.config_mut().default_font = Some(&MONTSERRAT_14);
    let d = e.default_display().expect("display");
    let page = e.active_screen(d).expect("screen");
    set(
        e,
        page,
        &[
            StyleProp::BgColor(Color::hex(0xEC_EF_F1)),
            StyleProp::BgOpa(Opa::COVER),
        ],
    );
    set(e, page, &pad(GAP));
    set(e, page, &[StyleProp::PadRow(GAP)]);
    e.set_layout(page, LayoutKind::Flex);
    e.set_flex_flow(page, FlexFlow::Column);
    e.set_flex_align(page, FlexAlign::Start, FlexAlign::Center, FlexAlign::Start);
    e.set_scroll_dir(page, Dir::VER);
    scrollbar_style(e, page, 3);
    list(e, page);
    carousel(e, page);
    panel(e, page);
    log_scroll(e, page, "page");
}

/// Finds a node by its test id among the page's children.
fn find(e: &Engine, name: &str) -> Option<NodeId> {
    let d = e.default_display()?;
    let page = e.active_screen(d)?;
    e.tree()
        .children(page)
        .find(|c| e.tree().node(*c).and_then(twine_engine::Node::test_id) == Some(name))
}

fn on_raw_key(e: &mut Engine, key: Key) {
    let Some(list) = find(e, "list") else {
        return;
    };
    match key {
        Key::Char('s') => {
            let next = match e.scrollbar_mode(list) {
                ScrollbarMode::Auto => ScrollbarMode::On,
                ScrollbarMode::On => ScrollbarMode::Active,
                ScrollbarMode::Active => ScrollbarMode::Off,
                ScrollbarMode::Off => ScrollbarMode::Auto,
            };
            e.set_scrollbar_mode(list, next);
            twine_core::info!(target: "twine::sim", "list scrollbar: {:?}", next);
        }
        Key::Char('e') => {
            let on = !e.has_flag(list, ObjFlags::SCROLL_ELASTIC);
            e.set_flag(list, ObjFlags::SCROLL_ELASTIC, on);
            twine_core::info!(target: "twine::sim", "list elastic: {}", on);
        }
        Key::Char('m') => {
            let on = !e.has_flag(list, ObjFlags::SCROLL_MOMENTUM);
            e.set_flag(list, ObjFlags::SCROLL_MOMENTUM, on);
            twine_core::info!(target: "twine::sim", "list momentum: {}", on);
        }
        _ => {}
    }
}

fn main() {
    twine_sim::init_logging();
    let cfg = SimConfig::new(W as u16, H as u16)
        .title("scroll")
        .scale(2)
        .on_raw_key(on_raw_key);
    run_engine(cfg, scene);
}
