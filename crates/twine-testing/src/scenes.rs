//! Reference scenes for engine tests, benchmarks and examples (feature `engine`), so every
//! buffer mode, rotation and format can be checked against the same content.

use twine_core::{Angle, Color, Opa, Rect};
use twine_engine::{Engine, NodeId, Obj};
use twine_style::{GradDir, Length, Selector, StyleProp};

/// Nodes of the [`engine_boxes`] scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineBoxes {
    /// The screen.
    pub screen: NodeId,
    /// The gradient header bar.
    pub header: NodeId,
    /// The card with shadow and radius.
    pub card: NodeId,
    /// The six boxes inside the card.
    pub card_boxes: [NodeId; 6],
    /// The 50 % opacity group.
    pub group: NodeId,
    /// The rotated box.
    pub rotated: NodeId,
    /// The box moved by the arrow keys.
    pub player: NodeId,
}

/// Sets local `Main` properties.
pub fn set_props(e: &mut Engine, id: NodeId, props: &[StyleProp]) {
    for p in props {
        e.set_local_prop(id, Selector::MAIN, *p);
    }
}

/// Creates an `Obj` under `parent` covering the absolute rectangle `r`, with the given local
/// properties. The position is stored as `X`/`Y` styles relative to the parent's current
/// content area and the size as `Width`/`Height`, so the box stays in place when the layout
/// runs. The layout is updated before (so that area is current) and after (so the box has its
/// coordinates right away).
pub fn styled_box(e: &mut Engine, parent: NodeId, r: Rect, props: &[StyleProp]) -> NodeId {
    e.update_layout();
    let c = e.content_area(parent);
    let b = child_box(e, parent, r.translate(-c.x0, -c.y0), props);
    e.update_layout();
    b
}

/// Creates an `Obj` under `parent` at `r` relative to the parent's content area (`X`, `Y`,
/// `Width`, `Height` styles) with the given local properties.
pub fn child_box(e: &mut Engine, parent: NodeId, r: Rect, props: &[StyleProp]) -> NodeId {
    let b = e.create(parent, Box::new(Obj)).expect("parent exists");
    e.set_pos(b, r.x0, r.y0);
    e.set_size(b, r.width(), r.height());
    set_props(e, b, props);
    b
}

/// The `engine_boxes` example scene on the default display's active screen, laid out for 320 × 240
/// (it works on any size): a gradient header, a card with shadow, radius and border holding
/// six boxes, a 50 % opacity group, a rotated box and the "player" box.
pub fn engine_boxes(e: &mut Engine) -> EngineBoxes {
    let d = e.default_display().expect("a display");
    let screen = e.active_screen(d).expect("a screen");
    set_props(
        e,
        screen,
        &[
            StyleProp::BgColor(Color::hex(0xE8_EC_F2)),
            StyleProp::BgOpa(Opa::COVER),
        ],
    );
    let header = child_box(
        e,
        screen,
        Rect::new(0, 0, 320, 36),
        &[
            StyleProp::BgColor(Color::hex(0x3F_51_B5)),
            StyleProp::BgGradColor(Color::hex(0x1A_23_7E)),
            StyleProp::BgGradDir(GradDir::Ver),
            StyleProp::BgOpa(Opa::COVER),
        ],
    );
    let card = child_box(
        e,
        screen,
        Rect::from_xywh(16, 50, 196, 170),
        &[
            StyleProp::BgColor(Color::WHITE),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(12),
            StyleProp::BorderWidth(1),
            StyleProp::BorderColor(Color::hex(0xC5_CD_DA)),
            StyleProp::ShadowWidth(20),
            StyleProp::ShadowOffsetY(6),
            StyleProp::ShadowColor(Color::hex(0x1A_23_7E)),
            StyleProp::ShadowOpa(Opa::P40),
        ],
    );
    let palette = [
        0xE5_39_35, 0xFB_8C_00, 0xFD_D8_35, 0x43_A0_47, 0x1E_88_E5, 0x8E_24_AA,
    ];
    let mut card_boxes = [screen; 6];
    for (i, c) in palette.iter().enumerate() {
        let (col, row) = ((i % 3) as i32, (i / 3) as i32);
        // Relative to the card's content area (inside its 1 px border).
        let r = Rect::from_xywh(11 + col * 60, 13 + row * 76, 52, 64);
        let mut props = vec![
            StyleProp::BgColor(Color::hex(*c)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(6),
        ];
        match i {
            1 => props.extend([
                StyleProp::BorderWidth(3),
                StyleProp::BorderColor(Color::hex(0x5D_40_37)),
            ]),
            2 => props.extend([
                StyleProp::BgGradColor(Color::hex(0xFF_6F_00)),
                StyleProp::BgGradDir(GradDir::Hor),
            ]),
            4 => props.extend([
                StyleProp::OutlineWidth(2),
                StyleProp::OutlinePad(2),
                StyleProp::OutlineColor(Color::hex(0x0D_47_A1)),
            ]),
            5 => props.push(StyleProp::Radius(26)),
            _ => {}
        }
        card_boxes[i] = child_box(e, card, r, &props);
    }
    let group = child_box(
        e,
        screen,
        Rect::from_xywh(228, 50, 80, 84),
        &[StyleProp::OpaLayered(Opa::P50)],
    );
    child_box(
        e,
        group,
        Rect::from_xywh(0, 0, 56, 56),
        &[
            StyleProp::BgColor(Color::RED),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(8),
        ],
    );
    child_box(
        e,
        group,
        Rect::from_xywh(24, 28, 56, 56),
        &[
            StyleProp::BgColor(Color::BLUE),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(8),
        ],
    );
    let rotated = child_box(
        e,
        screen,
        Rect::from_xywh(238, 160, 56, 28),
        &[
            StyleProp::BgColor(Color::hex(0x00_89_7B)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(4),
            StyleProp::TransformRotation(Angle::deg(25)),
            StyleProp::TransformPivotX(Length::Pct(50)),
            StyleProp::TransformPivotY(Length::Pct(50)),
        ],
    );
    let player = child_box(
        e,
        screen,
        Rect::from_xywh(228, 204, 24, 24),
        &[
            StyleProp::BgColor(Color::hex(0xF4_51_1E)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(12),
            StyleProp::ShadowWidth(8),
            StyleProp::ShadowOpa(Opa::P50),
        ],
    );
    e.set_test_id(player, "player");
    e.set_test_id(card, "card");
    EngineBoxes {
        screen,
        header,
        card,
        card_boxes,
        group,
        rotated,
        player,
    }
}

/// Styles a node's `Scrollbar` part like a theme would: `width` px thick, 2 px from the
/// edges, rounded, dark gray; blue while the node is scrolled (`State::SCROLLED`).
pub fn scrollbar_style(e: &mut Engine, id: NodeId, width: i32) {
    let sb = Selector::part(twine_style::Part::Scrollbar);
    for p in [
        StyleProp::Width(Length::Px(width)),
        StyleProp::BgColor(Color::hex(0x55_55_55)),
        StyleProp::BgOpa(Opa::P60),
        StyleProp::Radius(width / 2),
        StyleProp::PadRight(2),
        StyleProp::PadBottom(2),
        StyleProp::PadTop(2),
        StyleProp::PadLeft(2),
    ] {
        e.set_local_prop(id, sb, p);
    }
    let scrolled = sb.with_state(twine_style::State::SCROLLED);
    e.set_local_prop(id, scrolled, StyleProp::BgColor(Color::hex(0x20_60_FF)));
    e.set_local_prop(id, scrolled, StyleProp::BgOpa(Opa::COVER));
}

/// A scrolling list: a white container under `parent` covering the absolute rectangle `r`
/// holding `rows` full-width rows of `row_h` pixels in alternating blue / gray, each
/// clickable and darker while pressed, with a styled vertical scrollbar (see
/// [`scrollbar_style`]). Returns the container and the rows.
pub fn scroll_list(
    e: &mut Engine,
    parent: NodeId,
    r: Rect,
    rows: usize,
    row_h: i32,
) -> (NodeId, Vec<NodeId>) {
    let list = styled_box(
        e,
        parent,
        r,
        &[StyleProp::BgColor(Color::WHITE), StyleProp::BgOpa(Opa::COVER)],
    );
    scrollbar_style(e, list, 4);
    let mut ids = Vec::with_capacity(rows);
    for i in 0..rows {
        let c = if i % 2 == 0 {
            Color::hex(0x42_72_C4)
        } else {
            Color::hex(0x9E_A7_B3)
        };
        let row = child_box(
            e,
            list,
            Rect::from_xywh(0, i as i32 * row_h, r.width(), row_h),
            &[StyleProp::BgColor(c), StyleProp::BgOpa(Opa::COVER)],
        );
        e.set_local_prop(
            row,
            Selector::state(twine_style::State::PRESSED),
            StyleProp::BgColor(Color::hex(0x1A_23_7E)),
        );
        ids.push(row);
    }
    e.update_layout();
    (list, ids)
}
