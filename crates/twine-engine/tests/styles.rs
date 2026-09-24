//! Styles on nodes: precedence, parts, inheritance, local properties (P3), states, the style
//! cache and shared style changes.

mod common;

use std::rc::Rc;

use proptest::prelude::*;
use twine_core::{Color, Opa, Rect};
use twine_engine::{Engine, EngineConfig, NodeId, Obj};
use twine_style::{Part, PropId, Selector, State, Style, StyleBuf, StyleProp, StyleRef, StyleValue};
use twine_testing::EngineHarness;

static BLUE_BG: Style = Style::new(&[StyleProp::BgColor(Color::BLUE)]);
static RED_BG: Style = Style::new(&[StyleProp::BgColor(Color::RED)]);
static GREEN_BG: Style = Style::new(&[StyleProp::BgColor(Color::GREEN)]);

fn engine_with_node() -> (Engine, NodeId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let n = e.create_root(Box::new(Obj)).unwrap();
    (e, n)
}

fn bg(e: &Engine, n: NodeId) -> Color {
    e.style_color(n, Part::Main, PropId::BgColor)
}

#[test]
fn precedence_higher_state_weight_wins() {
    let (mut e, n) = engine_with_node();
    e.add_style(n, &RED_BG, Selector::state(State::PRESSED));
    e.add_style(n, &BLUE_BG, Selector::state(State::FOCUSED));
    e.add_state(n, State::PRESSED | State::FOCUSED);
    // PRESSED (0x80) weighs more than FOCUSED (0x08), even though FOCUSED was added later.
    assert_eq!(bg(&e, n), Color::RED);
    e.clear_state(n, State::PRESSED);
    assert_eq!(bg(&e, n), Color::BLUE);
}

#[test]
fn precedence_later_added_normal_wins_on_tie() {
    let (mut e, n) = engine_with_node();
    e.add_style(n, &RED_BG, Selector::MAIN);
    e.add_style(n, &BLUE_BG, Selector::MAIN);
    assert_eq!(bg(&e, n), Color::BLUE);
}

#[test]
fn local_beats_normal() {
    let (mut e, n) = engine_with_node();
    e.set_local_prop(n, Selector::MAIN, StyleProp::BgColor(Color::GREEN));
    e.add_style(n, &BLUE_BG, Selector::MAIN);
    assert_eq!(bg(&e, n), Color::GREEN);
}

#[test]
fn normal_beats_theme() {
    let (mut e, n) = engine_with_node();
    e.add_style(n, &BLUE_BG, Selector::MAIN);
    e.add_theme_style(n, &RED_BG, Selector::MAIN);
    assert_eq!(bg(&e, n), Color::BLUE);
    e.remove_style(n, Some(&StyleRef::Static(&BLUE_BG)), None);
    assert_eq!(bg(&e, n), Color::RED);
    e.remove_all_styles(n);
    assert_eq!(bg(&e, n), Color::WHITE); // LVGL default
}

#[test]
fn part_selectors_are_independent() {
    let (mut e, n) = engine_with_node();
    e.add_style(n, &RED_BG, Selector::part(Part::Indicator));
    assert_eq!(e.style_color(n, Part::Indicator, PropId::BgColor), Color::RED);
    assert_eq!(bg(&e, n), Color::WHITE);
}

#[test]
fn inherited_prop_comes_from_parent_main() {
    let (mut e, parent) = engine_with_node();
    let child = e.create(parent, Box::new(Obj)).unwrap();
    e.set_local_prop(parent, Selector::MAIN, StyleProp::TextColor(Color::RED));
    assert_eq!(e.style_color(child, Part::Main, PropId::TextColor), Color::RED);
    assert_eq!(
        e.style_color(child, Part::Indicator, PropId::TextColor),
        Color::RED
    );
}

#[test]
fn non_inherited_prop_uses_default() {
    let (mut e, parent) = engine_with_node();
    let child = e.create(parent, Box::new(Obj)).unwrap();
    e.set_local_prop(parent, Selector::MAIN, StyleProp::BgColor(Color::RED));
    assert_eq!(bg(&e, child), Color::WHITE);
    assert_eq!(e.style_opa(child, Part::Main, PropId::BgOpa), Opa::TRANSP);
}

#[test]
fn default_font_from_config() {
    let font: &'static twine_text::Font = &twine_assets::fonts::MONTSERRAT_14;
    let mut e = Engine::new(EngineConfig {
        default_font: Some(font),
        ..EngineConfig::default()
    })
    .unwrap();
    let n = e.create_root(Box::new(Obj)).unwrap();
    assert!(core::ptr::eq(e.style_font(n, Part::Main), font));
    assert!(core::ptr::eq(e.cached_main(n).font, font));
    let (e2, n2) = engine_with_node();
    assert!(core::ptr::eq(
        e2.style_font(n2, Part::Main),
        &raw const twine_text::EMPTY_FONT
    ));
}

fn harness_box() -> (EngineHarness, NodeId) {
    let mut id = None;
    let mut h = EngineHarness::new(64, 64).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        id = Some(common::boxed(e, s, Rect::from_xywh(10, 10, 20, 20), Color::RED));
    });
    h.run_until_idle();
    (h, id.unwrap())
}

#[test]
fn set_local_prop_same_value_is_noop() {
    let (mut h, b) = harness_box();
    let _ = h.engine().cached_main(b);
    assert!(h.engine().style_cache_valid(b));
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::RED));
    assert!(
        h.engine().invalidation_log().is_empty(),
        "{:?}",
        h.engine().invalidation_log()
    );
    assert!(h.engine().style_cache_valid(b));
    h.assert_idle();
    // A different value invalidates exactly the box.
    h.engine_mut()
        .set_local_prop(b, Selector::MAIN, StyleProp::BgColor(Color::BLUE));
    assert!(
        h.engine()
            .invalidation_log()
            .iter()
            .all(|(r, _)| *r == Rect::from_xywh(10, 10, 20, 20))
    );
    assert!(!h.engine().style_cache_valid(b));
}

#[test]
fn state_change_without_state_styles_is_noop() {
    let (mut h, b) = harness_box();
    h.engine_mut().add_state(b, State::PRESSED);
    assert!(h.engine().invalidation_log().is_empty());
    assert_eq!(h.engine().tree().node(b).unwrap().state(), State::PRESSED);
    h.assert_idle();
    // With a PRESSED style, the state change redraws the box.
    h.engine_mut()
        .add_style(b, &GREEN_BG, Selector::state(State::PRESSED));
    h.run_until_idle();
    h.engine_mut().clear_state(b, State::PRESSED);
    assert!(!h.engine().invalidation_log().is_empty());
    h.run_until_idle();
    assert_eq!(h.pixel(15, 15), Color::RED);
}

#[test]
fn inherited_change_bumps_epoch_and_children_see_new_value() {
    let (mut e, parent) = engine_with_node();
    let child = e.create(parent, Box::new(Obj)).unwrap();
    let before = e.cached_main(child).text_color;
    assert_eq!(before, Color::BLACK);
    let epoch = e.tree().style_epoch();
    e.set_local_prop(parent, Selector::MAIN, StyleProp::TextColor(Color::RED));
    assert_ne!(e.tree().style_epoch(), epoch);
    assert_eq!(e.cached_main(child).text_color, Color::RED);
    // A non-inherited change does not bump the epoch.
    let epoch = e.tree().style_epoch();
    e.set_local_prop(parent, Selector::MAIN, StyleProp::BgColor(Color::RED));
    assert_eq!(e.tree().style_epoch(), epoch);
}

#[test]
fn report_style_change_refreshes_users_only() {
    let mut users = Vec::new();
    let shared = Rc::new(StyleBuf::new().bg_color(Color::RED).bg_opa(Opa::COVER));
    let mut h = EngineHarness::new(64, 64).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..3 {
            let b = e.create(s, Box::new(Obj)).unwrap();
            e.set_pos(b, i * 20, 0);
            e.set_size(b, 10, 10);
            if i < 2 {
                e.add_style(b, StyleRef::Shared(shared.clone()), Selector::MAIN);
                users.push(b);
            }
        }
    });
    h.run_until_idle();
    h.engine_mut().report_style_change(&shared);
    let log: Vec<Rect> = h.engine().invalidation_log().iter().map(|(r, _)| *r).collect();
    assert!(!log.is_empty());
    assert!(
        log.iter()
            .all(|r| *r == Rect::from_xywh(0, 0, 10, 10) || *r == Rect::from_xywh(20, 0, 10, 10)),
        "{log:?}"
    );
    assert_eq!(users.len(), 2);
}

fn random_style(bits: u8, color: u32) -> StyleBuf {
    let mut s = StyleBuf::new();
    if bits & 1 != 0 {
        s.set(StyleProp::BgColor(Color::hex(color)));
    }
    if bits & 2 != 0 {
        s.set(StyleProp::Radius(i32::from(bits)));
    }
    if bits & 4 != 0 {
        s.set(StyleProp::PadTop(i32::from(bits) + 1));
    }
    if bits & 8 != 0 {
        s.set(StyleProp::TextColor(Color::hex(color ^ 0xFF)));
    }
    if bits & 16 != 0 {
        s.set(StyleProp::Opa(Opa(color as u8)));
    }
    if bits & 32 != 0 {
        s.set(StyleProp::BorderWidth(3));
    }
    s
}

const STATES: [State; 4] = [State::DEFAULT, State::PRESSED, State::FOCUSED, State::CHECKED];

proptest! {
    #[test]
    fn cache_matches_full_resolution_randomized(
        entries in proptest::collection::vec((any::<u8>(), any::<u32>(), 0usize..4, any::<bool>()), 0..8),
        parent_entries in proptest::collection::vec((any::<u8>(), any::<u32>(), 0usize..4), 0..4),
        state_ops in proptest::collection::vec((0usize..4, any::<bool>()), 0..6),
    ) {
        let (mut e, parent) = engine_with_node();
        let child = e.create(parent, Box::new(Obj)).unwrap();
        for (bits, c, st, _) in &parent_entries.iter().map(|(a, b, c)| (*a, *b, *c, false)).collect::<Vec<_>>() {
            e.add_style(parent, random_style(*bits, *c), Selector::state(STATES[*st]));
        }
        for (bits, c, st, local) in &entries {
            if *local {
                e.set_local_prop(child, Selector::state(STATES[*st]), StyleProp::BgColor(Color::hex(*c)));
            } else {
                e.add_style(child, random_style(*bits, *c), Selector::state(STATES[*st]));
            }
        }
        for (st, on) in state_ops {
            let _ = e.cached_main(child);
            e.set_state(child, STATES[st], on);
            let m = e.cached_main(child);
            prop_assert_eq!(m.bg_color, e.style_color(child, Part::Main, PropId::BgColor));
            prop_assert_eq!(m.radius, e.style_i32(child, Part::Main, PropId::Radius));
            prop_assert_eq!(m.pad.top, e.style_i32(child, Part::Main, PropId::PadTop));
            prop_assert_eq!(m.text_color, e.style_color(child, Part::Main, PropId::TextColor));
            prop_assert_eq!(m.opa, e.style_opa(child, Part::Main, PropId::Opa));
            prop_assert_eq!(m.border_width, e.style_i32(child, Part::Main, PropId::BorderWidth));
            // The parent's state changes the child's inherited text color.
            let _ = e.cached_main(child);
            e.set_state(parent, STATES[st], !on);
            prop_assert_eq!(e.cached_main(child).text_color, e.style_color(child, Part::Main, PropId::TextColor));
            prop_assert_eq!(e.style_prop(child, Part::Main, PropId::BgColor).as_color().is_some(), true);
            let _ = StyleValue::None;
        }
    }
}
