//! R1.S06: Rust-native layout types in the view API — the flex flow builder, `MainAlign` /
//! `CrossAlign`, `Axis` scrolling, `Side` gestures, `Align` / `Anchor`, `grid_tracks!`
//! (constant and reactive) and `grid_col` / `grid_row` ranges.

use twine_core::Point;
use twine_engine::{EventCode, EventParam, NodeId, ObjFlags};
use twine_style::{PropId, StyleValue};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn prop(t: &TestUi, id: &'static str, p: PropId) -> StyleValue {
    t.engine().style_prop(node(t, id), Part::Main, p)
}

/// Every one of LVGL's eight `lv_flex_flow_t` values, as the builder writes it.
#[test]
fn flex_builder_matches_the_eight_lvgl_flows() {
    let table: [(FlexDirection, bool, bool, u8); 8] = [
        (FlexDirection::Row, false, false, 0x00),    // LV_FLEX_FLOW_ROW
        (FlexDirection::Column, false, false, 0x01), // LV_FLEX_FLOW_COLUMN
        (FlexDirection::Row, true, false, 0x04),     // LV_FLEX_FLOW_ROW_WRAP
        (FlexDirection::Row, false, true, 0x08),     // LV_FLEX_FLOW_ROW_REVERSE
        (FlexDirection::Row, true, true, 0x0C),      // LV_FLEX_FLOW_ROW_WRAP_REVERSE
        (FlexDirection::Column, true, false, 0x05),  // LV_FLEX_FLOW_COLUMN_WRAP
        (FlexDirection::Column, false, true, 0x09),  // LV_FLEX_FLOW_COLUMN_REVERSE
        (FlexDirection::Column, true, true, 0x0D),   // LV_FLEX_FLOW_COLUMN_WRAP_REVERSE
    ];
    for (dir, wrap, reverse, lvgl) in table {
        let mut t = TestUi::new(100, 80).mount(move |_| {
            flex(dir, (label("a"), label("b")))
                .wrap(wrap)
                .reverse(reverse)
                .test_id("f")
        });
        t.run_until_idle();
        let flow = prop(&t, "f", PropId::FlexFlow).as_flex_flow().unwrap();
        assert_eq!(flow.to_lvgl(), lvgl, "{dir:?} wrap {wrap} reverse {reverse}");
        assert_eq!(flow, FlexFlow::new(dir).wrap(wrap).reverse(reverse));
    }
}

#[test]
fn row_and_column_take_wrap_and_reverse_and_a_later_flow_wins() {
    let mut t = TestUi::new(100, 80).mount(|_| {
        column((
            row(label("a")).wrap(true).test_id("r"),
            column(label("b")).reverse(true).test_id("c"),
            row(label("c"))
                .flex_flow(FlexFlow::COLUMN)
                .wrap(true)
                .test_id("o"),
        ))
    });
    t.run_until_idle();
    let flow = |id| prop(&t, id, PropId::FlexFlow).as_flex_flow().unwrap();
    assert_eq!(flow("r"), FlexFlow::ROW.wrap(true));
    assert_eq!(flow("c"), FlexFlow::COLUMN.reverse(true));
    // The constructor's flow is the first build step: an explicit `flex_flow` overrides it.
    assert_eq!(flow("o"), FlexFlow::COLUMN);
}

#[test]
fn align_items_sets_cross_and_track_align_content_the_tracks() {
    let mut t = TestUi::new(100, 80).mount(|_| {
        column((
            row(label("a")).align_items(CrossAlign::End).test_id("i"),
            row(label("b"))
                .align_items(CrossAlign::Center)
                .align_content(MainAlign::SpaceBetween)
                .justify(MainAlign::SpaceEvenly)
                .test_id("c"),
        ))
    });
    t.run_until_idle();
    assert_eq!(
        prop(&t, "i", PropId::FlexCrossAlign).as_cross_align(),
        Some(CrossAlign::End)
    );
    assert_eq!(
        prop(&t, "i", PropId::FlexTrackAlign).as_main_align(),
        Some(MainAlign::End)
    );
    assert_eq!(
        prop(&t, "c", PropId::FlexCrossAlign).as_cross_align(),
        Some(CrossAlign::Center)
    );
    assert_eq!(
        prop(&t, "c", PropId::FlexTrackAlign).as_main_align(),
        Some(MainAlign::SpaceBetween)
    );
    assert_eq!(
        prop(&t, "c", PropId::FlexMainAlign).as_main_align(),
        Some(MainAlign::SpaceEvenly)
    );
}

#[test]
fn scroll_view_and_scroll_dir_take_an_axis() {
    let mut t = TestUi::new(100, 80).mount(|_| {
        column((
            scroll_view(Axis::Horizontal, label("a")).test_id("h"),
            scroll_view(Axis::Vertical, label("b")).test_id("v"),
            container(label("c")).scroll_dir(Axis::Both).test_id("b"),
        ))
    });
    t.run_until_idle();
    let e = t.engine();
    assert_eq!(e.scroll_dir(node(&t, "h")), Sides::HORIZONTAL);
    assert_eq!(e.scroll_dir(node(&t, "v")), Sides::VERTICAL);
    assert_eq!(e.scroll_dir(node(&t, "b")), Sides::ALL);
    let flow = |id| {
        e.style_prop(node(&t, id), Part::Main, PropId::FlexFlow)
            .as_flex_flow()
    };
    assert_eq!(flow("h"), Some(FlexFlow::ROW));
    assert_eq!(flow("v"), Some(FlexFlow::COLUMN));
}

#[test]
fn a_swipe_reports_the_side_it_moved_towards() {
    let mut t = TestUi::new(200, 150).mount(|cx| {
        let got = cx.signal(None::<Side>);
        cx.provide(got);
        container(())
            .size(200, 150)
            .scrollable(false)
            .flag(ObjFlags::GESTURE_BUBBLE, false)
            .on_gesture(move |s| got.set(Some(s)))
            .test_id("g")
    });
    t.run_until_idle();
    t.drag(Point::new(180, 75), Point::new(20, 75), Duration::ms(200));
    t.run_until_idle();
    let got = t.root_scope().expect_context::<Signal<Option<Side>>>();
    assert_eq!(got.get(), Some(Side::Left));
    // The engine event carries the same `Side`.
    let n = node(&t, "g");
    t.engine_mut()
        .send_event(n, EventCode::Gesture, EventParam::Dir(Side::Top));
    t.run_until_idle();
    assert_eq!(got.get(), Some(Side::Top));
}

#[test]
fn align_to_takes_an_anchor_or_an_align() {
    let mut t = TestUi::new(200, 150).mount(|cx| {
        let base = cx.node_ref::<twine_engine::Obj>();
        container((
            container(())
                .size(60, 40)
                .pos(50, 50)
                .node_ref(base)
                .test_id("base"),
            container(())
                .size(10, 10)
                .align_to(base, Anchor::BelowLeft, 0, 2)
                .test_id("below"),
            container(())
                .size(10, 10)
                .align_to(base, Align::Center, 0, 0)
                .test_id("center"),
        ))
        .size(200, 150)
        .padding(0)
        .border_width(0)
    });
    t.run_until_idle();
    let base = t.find(by_id("base")).coords();
    let below = t.find(by_id("below")).coords();
    let center = t.find(by_id("center")).coords();
    assert_eq!((below.x0, below.y0), (base.x0, base.y1 + 2));
    assert_eq!(center.center(), base.center());
}

#[test]
fn grid_col_and_grid_row_take_indices_and_ranges() {
    let mut t = TestUi::new(200, 150).mount(|_| {
        grid(
            grid_tracks![fr(1), fr(1), fr(1)],
            grid_tracks![fr(1), fr(1), fr(1)],
            (
                label("a").grid_col(0..2).grid_row(1).test_id("a"),
                label("b").grid_col(1..=2).grid_row(0..3).test_id("b"),
                label("c").grid_col(2u8).grid_row(2usize).test_id("c"),
            ),
        )
    });
    t.run_until_idle();
    let cell = |id| {
        [
            PropId::GridCellColumn,
            PropId::GridCellColumnSpan,
            PropId::GridCellRow,
            PropId::GridCellRowSpan,
        ]
        .map(|p| prop(&t, id, p).as_px().unwrap())
    };
    assert_eq!(cell("a"), [0, 2, 1, 1]);
    assert_eq!(cell("b"), [1, 2, 0, 3]);
    assert_eq!(cell("c"), [2, 1, 2, 1]);
}

#[test]
fn grid_align_places_the_item_in_its_cell() {
    let mut t = TestUi::new(200, 150).mount(|_| {
        grid(
            grid_tracks![px(100)],
            grid_tracks![px(60)],
            container(())
                .size(20, 10)
                .grid_col(0)
                .grid_row(0)
                .grid_align(GridAlign::End, GridAlign::Center)
                .test_id("i"),
        )
        .test_id("g")
    });
    t.run_until_idle();
    let g = t.engine().content_area(node(&t, "g"));
    let i = t.find(by_id("i")).coords();
    assert_eq!((i.x0 - g.x0, i.y0 - g.y0), (80, 25));
}

#[derive(Clone, Copy)]
struct Tracks(Signal<Vec<GridTrack>>);

#[test]
fn reactive_grid_tracks_relayout_only_on_change() {
    let mut t = TestUi::new(200, 150).mount(|cx| {
        let cols = cx.signal(grid_tracks![px(50), px(60)]);
        cx.provide(Tracks(cols));
        grid(
            cols,
            grid_tracks![px(20)],
            (
                label("a").grid_col(0).grid_row(0).test_id("a"),
                label("b").grid_col(1).grid_row(0).test_id("b"),
            ),
        )
        .gap(0)
        .test_id("g")
    });
    t.run_until_idle();
    let x = |t: &TestUi| t.find(by_id("b")).coords().x0 - t.engine().content_area(node(t, "g")).x0;
    assert_eq!(x(&t), 50);
    assert_eq!(
        t.engine().grid_column_tracks(node(&t, "g")),
        Some(&[GridTrack::Px(50), GridTrack::Px(60)][..])
    );

    let Tracks(cols) = t.root_scope().expect_context::<Tracks>();
    cols.set(grid_tracks![px(70), content]);
    t.run_until_idle();
    assert_eq!(x(&t), 70);

    // The same tracks again: the binding runs, the engine setter does nothing (idempotence
    // is checked in the engine's unit test).
    cols.set(grid_tracks![px(70), content]);
    t.run_until_idle();
    assert_eq!(x(&t), 70);
}

#[test]
fn deleting_a_grid_drops_its_tracks() {
    let mut t = TestUi::new(200, 150).mount(|cx| {
        let on = cx.signal(true);
        cx.provide(on);
        when(
            move || on.get(),
            |_| grid(grid_tracks![fr(1)], grid_tracks![fr(1)], label("x")).test_id("g"),
        )
    });
    t.run_until_idle();
    let g = node(&t, "g");
    assert!(t.engine().grid_row_tracks(g).is_some());
    t.root_scope().expect_context::<Signal<bool>>().set(false);
    t.run_until_idle();
    assert!(!t.engine().tree().contains(g));
    assert_eq!(t.engine().grid_row_tracks(g), None);
}
