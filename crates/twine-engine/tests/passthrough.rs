//! `ObjFlags::LAYOUT_PASSTHROUGH` wrappers and `Engine::move_node`.

mod common;

use common::{style, white_screen};
use twine_core::{Color, Opa, Point};
use twine_engine::{Engine, EventCode, EventFilter, EventResult, NodeId, Obj, ObjFlags};
use twine_style::{FlexFlow, LayoutKind, Length, StyleProp};
use twine_testing::EngineHarness;

fn harness() -> EngineHarness {
    let mut h = EngineHarness::new(200, 160).no_theme().mount_engine(|e| {
        white_screen(e);
    });
    h.run_until_idle();
    h
}

/// A flex column (fixed width, content height) with a 4 px gap under the screen.
fn column(e: &mut Engine, parent: NodeId) -> NodeId {
    let c = e.create(parent, Box::new(Obj)).unwrap();
    style(
        e,
        c,
        &[
            StyleProp::Layout(LayoutKind::Flex),
            StyleProp::FlexFlow(FlexFlow::Column),
            StyleProp::PadRow(4),
            StyleProp::Width(Length::Px(120)),
        ],
    );
    c
}

/// A wrapper as the view layer creates it.
fn wrapper(e: &mut Engine, parent: NodeId) -> NodeId {
    let w = e.create(parent, Box::new(Obj)).unwrap();
    e.set_flag(w, ObjFlags::all(), false);
    e.set_flag(w, ObjFlags::LAYOUT_PASSTHROUGH, true);
    w
}

/// A leaf with a percentage width and a fixed height.
fn leaf(e: &mut Engine, parent: NodeId, h: i32, c: Color) -> NodeId {
    let n = e.create(parent, Box::new(Obj)).unwrap();
    style(
        e,
        n,
        &[
            StyleProp::Width(Length::pct(50)),
            StyleProp::Height(Length::Px(h)),
            StyleProp::BgColor(c),
            StyleProp::BgOpa(Opa::COVER),
        ],
    );
    n
}

#[test]
fn passthrough_layout_matches_direct_children() {
    // Direct children.
    let mut h1 = harness();
    let s = h1.screen();
    let (col1, direct) = {
        let e = h1.engine_mut();
        let col = column(e, s);
        let a = leaf(e, col, 10, Color::RED);
        let b = leaf(e, col, 20, Color::GREEN);
        let c = leaf(e, col, 30, Color::BLUE);
        (col, [a, b, c])
    };
    h1.run_until_idle();
    // The same with the middle two inside a wrapper.
    let mut h2 = harness();
    let s = h2.screen();
    let (col2, wrapped, w) = {
        let e = h2.engine_mut();
        let col = column(e, s);
        let a = leaf(e, col, 10, Color::RED);
        let w = wrapper(e, col);
        let b = leaf(e, w, 20, Color::GREEN);
        let c = leaf(e, w, 30, Color::BLUE);
        (col, [a, b, c], w)
    };
    h2.run_until_idle();
    for (d, x) in direct.iter().zip(&wrapped) {
        assert_eq!(h1.engine().coords(*d), h2.engine().coords(*x));
    }
    assert_eq!(h1.engine().coords(col1), h2.engine().coords(col2));
    // The wrapper covers exactly its children.
    let bbox = h2
        .engine()
        .coords(wrapped[1])
        .union(&h2.engine().coords(wrapped[2]));
    assert_eq!(h2.engine().coords(w), bbox);
    assert_eq!(h1.panel_rgb888(), h2.panel_rgb888());

    // A child added to the wrapper later relayouts the column (content height grows).
    let before = h2.engine().coords(col2).height();
    let d = leaf(h2.engine_mut(), w, 5, Color::BLACK);
    h2.run_until_idle();
    assert_eq!(h2.engine().coords(col2).height(), before + 4 + 5);
    assert_eq!(h2.engine().coords(d).y0, h2.engine().coords(wrapped[2]).y1 + 4);
    assert_eq!(h2.engine().coords(w).y1, h2.engine().coords(d).y1);
}

#[test]
fn nested_passthrough_wrappers() {
    let mut h = harness();
    let s = h.screen();
    let (a, b, outer, inner) = {
        let e = h.engine_mut();
        let col = column(e, s);
        let outer = wrapper(e, col);
        let a = leaf(e, outer, 10, Color::RED);
        let inner = wrapper(e, outer);
        let b = leaf(e, inner, 10, Color::GREEN);
        (a, b, outer, inner)
    };
    h.run_until_idle();
    let e = h.engine();
    assert_eq!(e.coords(b).y0, e.coords(a).y1 + 4);
    assert_eq!(e.coords(inner), e.coords(b));
    assert_eq!(e.coords(outer), e.coords(a).union(&e.coords(b)));
}

#[test]
fn passthrough_invisible_to_hit_test() {
    let mut h = harness();
    let s = h.screen();
    let (w, btn) = {
        let e = h.engine_mut();
        let col = column(e, s);
        let w = wrapper(e, col);
        let btn = leaf(e, w, 20, Color::RED);
        (w, btn)
    };
    h.run_until_idle();
    let d = h.display();
    let c = h.engine().coords(btn).center();
    assert_eq!(h.engine_mut().hit_test(d, c), Some(btn));
    // Outside the child but inside the column: the wrapper never catches the press.
    let e = h.engine();
    let beside = Point::new(e.coords(btn).x1 + 5, c.y);
    assert_ne!(h.engine_mut().hit_test(d, beside), Some(w));
}

#[test]
fn passthrough_draws_nothing_and_does_not_clip() {
    let mut h = harness();
    let s = h.screen();
    {
        let e = h.engine_mut();
        let w = wrapper(e, s);
        // A background on the wrapper is ignored; its child is outside the wrapper's style
        // size (the box follows the child).
        style(
            e,
            w,
            &[StyleProp::BgColor(Color::RED), StyleProp::BgOpa(Opa::COVER)],
        );
        let n = e.create(w, Box::new(Obj)).unwrap();
        e.set_pos(n, 40, 40);
        e.set_size(n, 10, 10);
        style(
            e,
            n,
            &[StyleProp::BgColor(Color::BLUE), StyleProp::BgOpa(Opa::COVER)],
        );
    }
    h.run_until_idle();
    assert_eq!(h.pixel(45, 45), Color::BLUE);
    assert_eq!(h.pixel(2, 2), Color::WHITE);
}

#[test]
fn move_node_reorders_and_relayouts() {
    let mut h = harness();
    let s = h.screen();
    let (col, kids) = {
        let e = h.engine_mut();
        let col = column(e, s);
        let kids: Vec<NodeId> = (0..4).map(|i| leaf(e, col, 10 + i, Color::RED)).collect();
        (col, kids)
    };
    h.run_until_idle();
    let changed = std::rc::Rc::new(std::cell::Cell::new(0));
    let c2 = changed.clone();
    h.engine_mut()
        .add_event_handler(col, EventFilter::Code(EventCode::ChildChanged), move |_, _| {
            c2.set(c2.get() + 1);
            EventResult::Continue
        });
    // Last to first.
    h.engine_mut().move_node(kids[3], col, Some(kids[0])).unwrap();
    h.run_until_idle();
    let order: Vec<NodeId> = h.engine().tree().children(col).collect();
    assert_eq!(order, [kids[3], kids[0], kids[1], kids[2]]);
    assert_eq!(h.engine().coords(kids[3]).y0, h.engine().coords(col).y0);
    assert_eq!(h.engine().coords(kids[0]).y0, h.engine().coords(kids[3]).y1 + 4);
    assert_eq!(changed.get(), 1);
    // Moving to where it is does nothing.
    h.engine_mut().move_node(kids[3], col, Some(kids[0])).unwrap();
    assert_eq!(changed.get(), 1);
    h.assert_idle();
    // To the end, and into another parent.
    h.engine_mut().move_node(kids[3], col, None).unwrap();
    h.run_until_idle();
    assert_eq!(h.engine().tree().children(col).last(), Some(kids[3]));
    let other = column(h.engine_mut(), s);
    h.engine_mut().move_node(kids[0], other, None).unwrap();
    h.run_until_idle();
    assert_eq!(h.engine().tree().parent(kids[0]), Some(other));
    assert_eq!(h.engine().tree().node(col).unwrap().child_count(), 3);
    // Errors.
    assert!(h.engine_mut().move_node(col, kids[1], None).is_err()); // into its own subtree
    assert!(h.engine_mut().move_node(kids[1], col, Some(kids[0])).is_err()); // not a child
    h.engine().tree().check_invariants().unwrap();
}

#[test]
fn passthrough_never_covers_the_gaps_between_its_children() {
    let mut h = harness();
    let s = h.screen();
    {
        let e = h.engine_mut();
        let col = column(e, s);
        let w = wrapper(e, col);
        // An opaque background on the wrapper must not hide what is between its children.
        style(
            e,
            w,
            &[StyleProp::BgColor(Color::RED), StyleProp::BgOpa(Opa::COVER)],
        );
        leaf(e, w, 10, Color::BLUE);
        leaf(e, w, 10, Color::BLUE);
    }
    h.run_until_idle();
    // Row 12 is in the 4 px gap between the two leaves (0..10, 14..24).
    assert_eq!(h.pixel(5, 12), Color::WHITE);
    assert_eq!(h.pixel(5, 5), Color::BLUE);
}
