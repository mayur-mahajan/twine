//! The surface behind text (`TextDsc::background`, R7.S04b): the engine finds the nearest
//! opaque background once per text run so the font's polarity-dependent text gamma
//! (R6.S03) picks the right curve.

use twine_core::{Color, Opa};
use twine_engine::{Engine, EngineConfig, MeasureCx, NodeId, Obj};
use twine_style::{Part, Selector, State, StyleProp};
use twine_text::Polarity;

const DARK_CARD: Color = Color::hex(0x0028_2B30);
const PRIMARY: Color = Color::hex(0x0021_96F3);

fn engine() -> Engine {
    Engine::new(EngineConfig::default()).unwrap()
}

fn bg(e: &mut Engine, n: NodeId, sel: Selector, color: Color, opa: Opa) {
    e.set_local_prop(n, sel, StyleProp::BgColor(color.into()));
    e.set_local_prop(n, sel, StyleProp::BgOpacity(opa.into()));
}

/// A screen (white) → card (dark) → label (transparent) tree.
fn card_tree(e: &mut Engine) -> (NodeId, NodeId, NodeId) {
    let screen = e.create_root(Box::new(Obj)).unwrap();
    bg(e, screen, Selector::MAIN, Color::WHITE, Opa::COVER);
    let card = e.create(screen, Box::new(Obj)).unwrap();
    bg(e, card, Selector::MAIN, DARK_CARD, Opa::COVER);
    let label = e.create(card, Box::new(Obj)).unwrap();
    bg(e, label, Selector::MAIN, Color::RED, Opa::TRANSP);
    (screen, card, label)
}

#[test]
fn nearest_opaque_ancestor_is_the_background() {
    let mut e = engine();
    let (_, card, label) = card_tree(&mut e);
    e.set_local_prop(label, Selector::MAIN, StyleProp::TextColor(PRIMARY.into()));
    let d = e.text_dsc(label, Part::Main, Opa::COVER);
    assert_eq!(d.background, Some(DARK_CARD));
    // Primary blue on a dark card is light-on-dark; from the text alone it was taken as dark
    // text (luminance < 128).
    assert_eq!(Polarity::of(d.color, d.background), Polarity::LightOnDark);
    assert_eq!(Polarity::of(d.color, None), Polarity::DarkOnLight);
    // A translucent card does not count: the screen behind it does.
    bg(&mut e, card, Selector::MAIN, DARK_CARD, Opa::P50);
    assert_eq!(e.text_background(label, Part::Main), Some(Color::WHITE));
}

#[test]
fn own_opaque_background_comes_first() {
    let mut e = engine();
    let (_, _, label) = card_tree(&mut e);
    bg(&mut e, label, Selector::MAIN, Color::YELLOW, Opa::COVER);
    assert_eq!(e.text_background(label, Part::Main), Some(Color::YELLOW));
}

#[test]
fn an_opaque_part_background_is_behind_its_text() {
    let mut e = engine();
    let (_, _, node) = card_tree(&mut e);
    let items = Selector::part(Part::Items);
    bg(&mut e, node, items, PRIMARY, Opa::COVER);
    assert_eq!(e.text_background(node, Part::Items), Some(PRIMARY));
    // In another state (button-matrix buttons): that state's background.
    bg(
        &mut e,
        node,
        items.with_state(State::PRESSED),
        Color::GREEN,
        Opa::COVER,
    );
    let d = e.text_dsc_for_state(node, Part::Items, State::PRESSED, Opa::COVER);
    assert_eq!(d.background, Some(Color::GREEN));
    // A transparent part falls back to the node and its ancestors.
    assert_eq!(e.text_background(node, Part::Indicator), Some(DARK_CARD));
}

#[test]
fn background_follows_state_changes() {
    let mut e = engine();
    let (_, card, label) = card_tree(&mut e);
    bg(
        &mut e,
        card,
        Selector::MAIN.with_state(State::CHECKED),
        Color::WHITE,
        Opa::COVER,
    );
    assert_eq!(e.text_background(label, Part::Main), Some(DARK_CARD));
    e.add_state(card, State::CHECKED);
    assert_eq!(e.text_background(label, Part::Main), Some(Color::WHITE));
}

#[test]
fn recolor_applies_to_the_background_like_to_the_text() {
    let mut e = engine();
    let (_, card, label) = card_tree(&mut e);
    e.set_local_prop(card, Selector::MAIN, StyleProp::Recolor(Color::WHITE.into()));
    e.set_local_prop(card, Selector::MAIN, StyleProp::RecolorOpacity(Opa::P50.into()));
    let expected = Color::mix(Color::WHITE, DARK_CARD, Opa::P50);
    assert_eq!(e.text_background(label, Part::Main), Some(expected));
}

#[test]
fn nothing_opaque_gives_none() {
    let mut e = engine();
    let screen = e.create_root(Box::new(Obj)).unwrap();
    bg(&mut e, screen, Selector::MAIN, Color::WHITE, Opa::TRANSP);
    let label = e.create(screen, Box::new(Obj)).unwrap();
    assert_eq!(e.text_dsc(label, Part::Main, Opa::COVER).background, None);
}

#[test]
fn measuring_does_not_look_for_the_background() {
    let mut e = engine();
    let (_, _, label) = card_tree(&mut e);
    assert_eq!(MeasureCx::new(&e, label).text_dsc(Part::Main).background, None);
}
