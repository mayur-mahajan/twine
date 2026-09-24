//! `cargo xtask sim flex_layout`: an interactive flex layout playground.
//!
//! A container (90 % × 90 %, centered) with six labelled children of different sizes: fixed
//! sizes, one with a percentage width (`3`), one sized by its label (`5`), one growing (`6`).
//! The window title shows the current configuration (also logged). Keys:
//!
//! - `f`: cycle the 8 flex flows (row / column, wrap, reverse).
//! - `m` / `c` / `t`: cycle the main / cross / track placement.
//! - `+` / `-`: add / remove a child.
//! - `g`: toggle `flex_grow` on child `2`.
//! - `r`: toggle right-to-left base direction.
//! - `p`: cycle padding and gap presets.
//!
//! Only what moves is redrawn: press F2 to see the refreshed areas, F3 for the layout bounds.

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId};
use twine_examples::tile::{PALETTE, tile};
use twine_hal::Key;
use twine_sim::{SimConfig, run_engine};
use twine_style::{Align, BaseDir, FlexAlign, FlexFlow, LayoutKind, Length, Selector, StyleProp};

const FLOWS: [FlexFlow; 8] = [
    FlexFlow::Row,
    FlexFlow::Column,
    FlexFlow::RowWrap,
    FlexFlow::RowReverse,
    FlexFlow::RowWrapReverse,
    FlexFlow::ColumnWrap,
    FlexFlow::ColumnReverse,
    FlexFlow::ColumnWrapReverse,
];

const PLACES: [FlexAlign; 6] = [
    FlexAlign::Start,
    FlexAlign::End,
    FlexAlign::Center,
    FlexAlign::SpaceEvenly,
    FlexAlign::SpaceAround,
    FlexAlign::SpaceBetween,
];

/// `(padding, gap)` presets.
const PADS: [(i32, i32); 4] = [(8, 6), (0, 0), (16, 2), (4, 16)];

/// Sizes of the children (cycled when adding more): `None` = sized by the label.
const SIZES: [Option<(Length, Length)>; 6] = [
    Some((Length::Px(60), Length::Px(40))),
    Some((Length::Px(40), Length::Px(64))),
    Some((Length::Pct(20), Length::Px(30))),
    Some((Length::Px(50), Length::Px(50))),
    None,
    Some((Length::Px(36), Length::Px(24))),
];

/// The playground's configuration.
#[derive(Debug, Clone, Copy, Default)]
struct Config {
    flow: usize,
    main: usize,
    cross: usize,
    track: usize,
    pad: usize,
    grow2: bool,
    rtl: bool,
}

impl Config {
    fn describe(&self, children: usize) -> String {
        format!(
            "flex_layout — {:?} main={:?} cross={:?} track={:?} pad/gap={:?} grow2={} {} ({} children)",
            FLOWS[self.flow],
            PLACES[self.main],
            PLACES[self.cross],
            PLACES[self.track],
            PADS[self.pad],
            self.grow2,
            if self.rtl { "RTL" } else { "LTR" },
            children
        )
    }

    /// Applies the configuration to the container (every setter is idempotent: only what
    /// changed marks the layout).
    fn apply(&self, e: &mut Engine, cont: NodeId) {
        e.set_flex_flow(cont, FLOWS[self.flow]);
        e.set_flex_align(cont, PLACES[self.main], PLACES[self.cross], PLACES[self.track]);
        let (pad, gap) = PADS[self.pad];
        for p in [
            StyleProp::PadLeft(pad),
            StyleProp::PadRight(pad),
            StyleProp::PadTop(pad),
            StyleProp::PadBottom(pad),
            StyleProp::PadRow(gap),
            StyleProp::PadColumn(gap),
            StyleProp::BaseDir(if self.rtl { BaseDir::Rtl } else { BaseDir::Ltr }),
        ] {
            e.set_local_prop(cont, Selector::MAIN, p);
        }
        if let Some(c2) = e.tree().child(cont, 1) {
            e.set_flex_grow(c2, u8::from(self.grow2));
        }
        let n = e.tree().children(cont).count();
        twine_sim::set_title(&self.describe(n));
    }
}

/// Adds child number `i` (1-based label).
fn add_child(e: &mut Engine, cont: NodeId, i: usize) {
    let t = tile(
        e,
        cont,
        i.to_string(),
        Color::hex(PALETTE[(i - 1) % PALETTE.len()]),
    );
    match SIZES[(i - 1) % SIZES.len()] {
        Some((w, h)) => e.set_size(t, w, h),
        None => e.set_local_prop(t, Selector::MAIN, StyleProp::Height(Length::Px(28))),
    }
    if i == 6 {
        e.set_flex_grow(t, 1);
    }
}

fn scene(e: &mut Engine) -> NodeId {
    e.config_mut().default_font = Some(&MONTSERRAT_14);
    let d = e.default_display().expect("display");
    let screen = e.active_screen(d).expect("screen");
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgColor(Color::hex(0xEC_EF_F1)));
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
    let cont = e.create(screen, Box::new(twine_engine::Obj)).expect("container");
    e.set_size(cont, Length::pct(90), Length::pct(90));
    e.align(cont, Align::Center, 0, 0);
    e.set_layout(cont, LayoutKind::Flex);
    for p in [
        StyleProp::BgColor(Color::WHITE),
        StyleProp::BgOpa(Opa::COVER),
        StyleProp::Radius(8),
        StyleProp::BorderWidth(1),
        StyleProp::BorderColor(Color::hex(0xB0_BE_C5)),
    ] {
        e.set_local_prop(cont, Selector::MAIN, p);
    }
    for i in 1..=6 {
        add_child(e, cont, i);
    }
    Config::default().apply(e, cont);
    cont
}

/// Handles the example's keys; returns whether the configuration changed.
fn on_key(e: &mut Engine, cont: NodeId, cfg: &mut Config, key: Key) -> bool {
    let Key::Char(ch) = key else {
        return false;
    };
    match ch {
        'f' => cfg.flow = (cfg.flow + 1) % FLOWS.len(),
        'm' => cfg.main = (cfg.main + 1) % PLACES.len(),
        'c' => cfg.cross = (cfg.cross + 1) % PLACES.len(),
        't' => cfg.track = (cfg.track + 1) % PLACES.len(),
        'p' => cfg.pad = (cfg.pad + 1) % PADS.len(),
        'g' => cfg.grow2 = !cfg.grow2,
        'r' => cfg.rtl = !cfg.rtl,
        '+' | '=' => {
            let n = e.tree().children(cont).count();
            if n < 40 {
                add_child(e, cont, n + 1);
            }
        }
        '-' => {
            if let Some(last) = e.tree().child(cont, -1) {
                let _ = e.delete(last);
            }
        }
        _ => return false,
    }
    cfg.apply(e, cont);
    true
}

fn main() {
    twine_sim::init_logging();
    let cont = std::rc::Rc::new(std::cell::Cell::new(None));
    let c2 = cont.clone();
    let mut cfg = Config::default();
    let sim = SimConfig::new(320, 240)
        .title("flex_layout")
        .scale(2)
        .on_raw_key(move |e, k| {
            if let Some(c) = c2.get() {
                on_key(e, c, &mut cfg, k);
            }
        });
    run_engine(sim, move |e| cont.set(Some(scene(e))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cycle_the_configuration() {
        let mut e = Engine::new(twine_engine::EngineConfig::default()).unwrap();
        let root = e.create_root(Box::new(twine_engine::Obj)).unwrap();
        e.place(root, twine_core::Rect::from_xywh(0, 0, 320, 240));
        let cont = e.create(root, Box::new(twine_engine::Obj)).unwrap();
        for i in 1..=6 {
            add_child(&mut e, cont, i);
        }
        let mut cfg = Config::default();
        for _ in 0..8 {
            assert!(on_key(&mut e, cont, &mut cfg, Key::Char('f')));
        }
        assert_eq!(cfg.flow, 0);
        assert!(on_key(&mut e, cont, &mut cfg, Key::Char('+')));
        assert_eq!(e.tree().children(cont).count(), 7);
        assert!(on_key(&mut e, cont, &mut cfg, Key::Char('-')));
        assert!(on_key(&mut e, cont, &mut cfg, Key::Char('-')));
        assert_eq!(e.tree().children(cont).count(), 5);
        assert!(!on_key(&mut e, cont, &mut cfg, Key::Char('z')));
        assert!(cfg.describe(5).contains("Row"));
    }
}
