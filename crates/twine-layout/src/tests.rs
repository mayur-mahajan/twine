//! Flex and grid tests (numbers follow LVGL 9's `lv_flex.c` / `lv_grid.c` arithmetic).

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Rect, Size};
use twine_style::{
    BaseDir, FlexAlign, FlexFlow, GridAlign, GridTrack, LayoutKind, Length, StyleBuf, StyleProp,
};

use crate::toy::ToyTree;
use crate::{LayoutFlags, LayoutTree};

fn run(t: &mut ToyTree) {
    t.layout();
}

fn origin(t: &ToyTree, id: usize) -> (i32, i32) {
    let r = t.coords(id);
    (r.x0, r.y0)
}

fn xs(t: &ToyTree, ids: &[usize]) -> Vec<i32> {
    ids.iter().map(|&i| t.coords(i).x0).collect()
}

fn ys(t: &ToyTree, ids: &[usize]) -> Vec<i32> {
    ids.iter().map(|&i| t.coords(i).y0).collect()
}

/// A `w × h` flex container at the origin of a large screen.
fn flex(w: i32, h: i32, flow: FlexFlow) -> (ToyTree, usize) {
    let mut t = ToyTree::new(1000, 1000);
    let c = t.add(
        ToyTree::ROOT,
        StyleBuf::new()
            .width(w)
            .height(h)
            .layout(LayoutKind::Flex)
            .flex_flow(flow),
    );
    (t, c)
}

fn items(t: &mut ToyTree, c: usize, sizes: &[(i32, i32)]) -> Vec<usize> {
    sizes
        .iter()
        .map(|&(w, h)| t.add(c, StyleBuf::new().width(w).height(h)))
        .collect()
}

fn row3(place: FlexAlign) -> (ToyTree, Vec<usize>) {
    let (mut t, c) = flex(300, 100, FlexFlow::Row);
    t.set_style(c, StyleProp::PadColumn(10));
    t.set_style(c, StyleProp::FlexMainPlace(place));
    let ids = items(&mut t, c, &[(50, 20); 3]);
    run(&mut t);
    (t, ids)
}

fn tracks(v: &[GridTrack]) -> &'static [GridTrack] {
    Box::leak(v.to_vec().into_boxed_slice())
}

/// A `w × h` grid container at the origin.
fn grid(w: Length, h: Length, cols: &[GridTrack], rows: &[GridTrack]) -> (ToyTree, usize) {
    let mut t = ToyTree::new(1000, 1000);
    let c = t.add(
        ToyTree::ROOT,
        StyleBuf::new()
            .width(w)
            .height(h)
            .layout(LayoutKind::Grid)
            .grid_column_dsc_array(tracks(cols))
            .grid_row_dsc_array(tracks(rows)),
    );
    (t, c)
}

fn cell(t: &mut ToyTree, c: usize, col: i32, row: i32, style: StyleBuf) -> usize {
    t.add(c, style.grid_cell_column_pos(col).grid_cell_row_pos(row))
}

fn stretch() -> StyleBuf {
    StyleBuf::new()
        .grid_cell_x_align(GridAlign::Stretch)
        .grid_cell_y_align(GridAlign::Stretch)
}

mod flex {
    use super::*;

    // ---- Flex I -------------------------------------------------------------------------------

    #[test]
    fn row_start_positions() {
        let (t, ids) = row3(FlexAlign::Start);
        assert_eq!(xs(&t, &ids), [0, 60, 120]);
        assert_eq!(ys(&t, &ids), [0, 0, 0]);
        assert_eq!(t.coords(ids[1]).size(), Size::new(50, 20));
    }

    #[test]
    fn row_end_positions() {
        let (t, ids) = row3(FlexAlign::End);
        assert_eq!(xs(&t, &ids), [130, 190, 250]);
    }

    #[test]
    fn row_center_positions() {
        let (t, ids) = row3(FlexAlign::Center);
        assert_eq!(xs(&t, &ids), [65, 125, 185]);
    }

    #[test]
    fn row_space_evenly() {
        // free = 300 - 170 = 130; extra gap = 130 / 4 = 32 before every item.
        let (t, ids) = row3(FlexAlign::SpaceEvenly);
        assert_eq!(xs(&t, &ids), [32, 124, 216]);
    }

    #[test]
    fn row_space_around() {
        // extra gap = 130 / 3 = 43, half of it before the first item.
        let (t, ids) = row3(FlexAlign::SpaceAround);
        assert_eq!(xs(&t, &ids), [21, 124, 227]);
    }

    #[test]
    fn row_space_between() {
        let (t, ids) = row3(FlexAlign::SpaceBetween);
        assert_eq!(xs(&t, &ids), [0, 125, 250]);
    }

    #[test]
    fn space_between_single_item_is_start() {
        for (place, x) in [
            (FlexAlign::SpaceBetween, 0),
            (FlexAlign::SpaceAround, 125),
            (FlexAlign::SpaceEvenly, 125),
        ] {
            let (mut t, c) = flex(300, 100, FlexFlow::Row);
            t.set_style(c, StyleProp::FlexMainPlace(place));
            let ids = items(&mut t, c, &[(50, 20)]);
            run(&mut t);
            // LVGL: with one item SpaceAround/SpaceEvenly center, SpaceBetween starts.
            assert_eq!(xs(&t, &ids), [x], "{place:?}");
        }
    }

    #[test]
    fn row_reverse_takes_last_child_first() {
        let (mut t, c) = flex(300, 100, FlexFlow::RowReverse);
        let ids = items(&mut t, c, &[(10, 5), (20, 5), (30, 5)]);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [50, 30, 0]);
    }

    #[test]
    fn column_reverse_order() {
        let (mut t, c) = flex(100, 300, FlexFlow::ColumnReverse);
        let ids = items(&mut t, c, &[(10, 20), (10, 30), (10, 10)]);
        run(&mut t);
        assert_eq!(ys(&t, &ids), [40, 10, 0]);
        assert_eq!(xs(&t, &ids), [0, 0, 0]);
    }

    #[test]
    fn gap_applied_between_items() {
        let (mut t, c) = flex(300, 100, FlexFlow::Column);
        t.set_style(c, StyleProp::PadRow(7));
        t.set_style(c, StyleProp::PadColumn(99)); // not the item gap of a column
        let ids = items(&mut t, c, &[(10, 20), (10, 30), (10, 10)]);
        run(&mut t);
        assert_eq!(ys(&t, &ids), [0, 27, 64]);
    }

    #[test]
    fn margins_add_to_item_extent() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        let a = t.add(
            c,
            StyleBuf::new()
                .width(50)
                .height(20)
                .margin_left(5)
                .margin_right(3)
                .margin_top(4),
        );
        let b = t.add(c, StyleBuf::new().width(50).height(20));
        run(&mut t);
        assert_eq!(origin(&t, a), (5, 4));
        assert_eq!(origin(&t, b), (58, 0));
    }

    #[test]
    fn grow_distributes_free_space_exactly() {
        let (mut t, c) = flex(301, 100, FlexFlow::Row);
        let ids: Vec<usize> = (1..=3)
            .map(|g| t.add(c, StyleBuf::new().flex_grow(g).width(999).height(10)))
            .collect();
        run(&mut t);
        let widths: Vec<i32> = ids.iter().map(|&i| t.coords(i).width()).collect();
        // round(301 / 6) = 50, round(251 * 2 / 5) = 100, the rest 151.
        assert_eq!(widths, [50, 100, 151]);
        assert_eq!(widths.iter().sum::<i32>(), 301);
        assert_eq!(xs(&t, &ids), [0, 50, 150]);
    }

    #[test]
    fn grow_shares_space_left_by_fixed_items_and_gaps() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        t.set_style(c, StyleProp::PadColumn(10));
        let a = t.add(c, StyleBuf::new().width(40).height(10));
        let g = t.add(c, StyleBuf::new().flex_grow(1).height(10));
        let b = t.add(c, StyleBuf::new().width(60).height(10));
        run(&mut t);
        assert_eq!(t.coords(g), Rect::from_xywh(50, 0, 180, 10));
        assert_eq!(origin(&t, a), (0, 0));
        assert_eq!(origin(&t, b), (240, 0));
    }

    #[test]
    fn grow_respects_max_and_redistributes() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        let a = t.add(c, StyleBuf::new().flex_grow(1).max_width(50).height(10));
        let b = t.add(c, StyleBuf::new().flex_grow(1).height(10));
        let d = t.add(
            c,
            StyleBuf::new().flex_grow(1).min_width(Length::pct(60)).height(10),
        );
        run(&mut t);
        // a clamped to 50, d to its min 180, b gets the remaining 70.
        assert_eq!(t.coords(a).width(), 50);
        assert_eq!(t.coords(d).width(), 180);
        assert_eq!(t.coords(b).width(), 70);
        assert_eq!(xs(&t, &[a, b, d]), [0, 50, 120]);
    }

    #[test]
    fn cross_center_and_end() {
        // LVGL: items are placed within their track, whose cross size is the largest item (here
        // 26 = 20 + margin 6); the track itself is placed by the track placement.
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        t.set_style(c, StyleProp::FlexCrossPlace(FlexAlign::Center));
        let ids = items(&mut t, c, &[(10, 20), (10, 21)]);
        let m = t.add(c, StyleBuf::new().width(10).height(20).margin_top(6));
        run(&mut t);
        assert_eq!(ys(&t, &ids), [3, 2]);
        assert_eq!(origin(&t, m).1, 6);

        // Track centered in the container: the items are centered in the container too.
        t.set_style(c, StyleProp::FlexTrackPlace(FlexAlign::Center));
        run(&mut t);
        assert_eq!(ys(&t, &ids), [40, 39]);
        assert_eq!(origin(&t, m).1, 43);

        t.set_style(c, StyleProp::FlexTrackPlace(FlexAlign::Start));
        t.set_style(c, StyleProp::FlexCrossPlace(FlexAlign::End));
        t.set_style(m, StyleProp::MarginBottom(5));
        run(&mut t);
        // Track cross size 31 (20 + 6 + 5).
        assert_eq!(ys(&t, &ids), [11, 10]);
        assert_eq!(origin(&t, m).1, 6);

        // Space* modes act as Start on the cross axis.
        t.set_style(c, StyleProp::FlexCrossPlace(FlexAlign::SpaceEvenly));
        run(&mut t);
        assert_eq!(ys(&t, &ids), [0, 0]);
    }

    #[test]
    fn content_sized_row_container_fits_children() {
        let mut t = ToyTree::new(1000, 1000);
        let c = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .layout(LayoutKind::Flex)
                .flex_flow(FlexFlow::RowWrap)
                .flex_main_place(FlexAlign::Center)
                .pad_column(5)
                .pad_left(2)
                .pad_right(2)
                .pad_top(2)
                .pad_bottom(2),
        );
        let ids = items(&mut t, c, &[(30, 10), (40, 20)]);
        let g = t.add(c, StyleBuf::new().flex_grow(1).min_width(7).height(3));
        run(&mut t);
        // Width = 30 + 5 + 40 + 5 + 7 (min of the grow item) + 4; no wrapping; height = 20 + 4.
        assert_eq!(t.coords(c).size(), Size::new(91, 24));
        assert_eq!(xs(&t, &ids), [2, 37]);
        assert_eq!(t.coords(g), Rect::from_xywh(82, 2, 7, 3));
    }

    #[test]
    fn content_sized_column_uses_widest_item() {
        let mut t = ToyTree::new(1000, 1000);
        let c = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .layout(LayoutKind::Flex)
                .flex_flow(FlexFlow::Column)
                .pad_row(4)
                .flex_track_place(FlexAlign::Center),
        );
        let ids = items(&mut t, c, &[(30, 10), (50, 20)]);
        let p = t.add(c, StyleBuf::new().width(Length::pct(100)).height(5));
        run(&mut t);
        assert_eq!(t.coords(c).size(), Size::new(50, 43));
        assert_eq!(xs(&t, &ids), [0, 0]);
        // The percentage child is excluded from the content width, then resolved against it.
        assert_eq!(t.coords(p).width(), 50);
    }

    #[test]
    fn hidden_and_floating_children_skipped() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        let a = t.add(c, StyleBuf::new().width(50).height(10));
        let h = t.add(c, StyleBuf::new().width(50).height(10));
        t.set_flags(h, LayoutFlags::HIDDEN);
        let f = t.add(c, StyleBuf::new().width(50).height(10).x(200).y(50));
        t.set_flags(f, LayoutFlags::FLOATING);
        let i = t.add(c, StyleBuf::new().width(50).height(10).x(100).y(70));
        t.set_flags(i, LayoutFlags::IGNORE_LAYOUT);
        let b = t.add(c, StyleBuf::new().width(50).height(10));
        run(&mut t);
        assert_eq!(xs(&t, &[a, b]), [0, 50]);
        // Non-items are positioned by their own x/y.
        assert_eq!(origin(&t, f), (200, 50));
        assert_eq!(origin(&t, i), (100, 70));
        assert_eq!(origin(&t, h), (0, 0));
    }

    // ---- Flex II ------------------------------------------------------------------------------

    #[test]
    fn row_wrap_places_items_on_second_track() {
        let (mut t, c) = flex(200, 100, FlexFlow::RowWrap);
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::PadRow(5));
        let ids = items(&mut t, c, &[(80, 20), (80, 15), (80, 20)]);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 90, 0]);
        assert_eq!(ys(&t, &ids), [0, 0, 25]);
    }

    #[test]
    fn oversized_item_gets_own_track() {
        let (mut t, c) = flex(200, 100, FlexFlow::RowWrap);
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::PadRow(5));
        let ids = items(&mut t, c, &[(50, 20), (250, 20), (50, 20)]);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 0, 0]);
        assert_eq!(ys(&t, &ids), [0, 25, 50]);
    }

    #[test]
    fn flex_in_new_track_forces_break() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        t.set_style(c, StyleProp::PadRow(5));
        let ids = items(&mut t, c, &[(50, 20), (50, 20), (50, 20)]);
        t.set_flags(ids[1], LayoutFlags::FLEX_IN_NEW_TRACK);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 0, 50]);
        assert_eq!(ys(&t, &ids), [0, 25, 25]);
    }

    #[test]
    fn track_place_center_and_space_between() {
        let (mut t, c) = flex(200, 100, FlexFlow::RowWrap);
        t.set_style(c, StyleProp::FlexTrackPlace(FlexAlign::Center));
        let ids = items(&mut t, c, &[(150, 20), (150, 20)]);
        run(&mut t);
        assert_eq!(ys(&t, &ids), [30, 50]);
        t.set_style(c, StyleProp::FlexTrackPlace(FlexAlign::SpaceBetween));
        run(&mut t);
        assert_eq!(ys(&t, &ids), [0, 80]);
        t.set_style(c, StyleProp::FlexTrackPlace(FlexAlign::SpaceEvenly));
        run(&mut t);
        assert_eq!(ys(&t, &ids), [20, 60]);
    }

    #[test]
    fn row_wrap_reverse_track_order() {
        // LVGL: WRAP_REVERSE = wrap + reverse item order; tracks still go top to bottom.
        let (mut t, c) = flex(200, 100, FlexFlow::RowWrapReverse);
        t.set_style(c, StyleProp::PadColumn(10));
        let ids = items(&mut t, c, &[(80, 20), (80, 20), (80, 20)]);
        run(&mut t);
        assert_eq!(origin(&t, ids[2]), (0, 0));
        assert_eq!(origin(&t, ids[1]), (90, 0));
        assert_eq!(origin(&t, ids[0]), (0, 20));
    }

    #[test]
    fn column_wrap_tracks_left_to_right() {
        let (mut t, c) = flex(200, 100, FlexFlow::ColumnWrap);
        t.set_style(c, StyleProp::PadRow(10));
        t.set_style(c, StyleProp::PadColumn(5));
        let ids = items(&mut t, c, &[(30, 40), (20, 40), (30, 40)]);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 0, 35]);
        assert_eq!(ys(&t, &ids), [0, 50, 0]);
    }

    #[test]
    fn rtl_row_mirrors_positions() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        t.set_style(c, StyleProp::BaseDir(BaseDir::Rtl));
        t.set_style(c, StyleProp::PadColumn(10));
        let ids = items(&mut t, c, &[(50, 20), (60, 20)]);
        run(&mut t);
        assert_eq!(xs(&t, &ids), [250, 180]);
        t.set_style(c, StyleProp::FlexMainPlace(FlexAlign::End));
        run(&mut t);
        assert_eq!(xs(&t, &ids), [70, 0]);
    }

    #[test]
    fn rtl_column_places_tracks_from_the_right() {
        let (mut t, c) = flex(300, 100, FlexFlow::ColumnWrap);
        t.set_style(c, StyleProp::BaseDir(BaseDir::Rtl));
        t.set_style(c, StyleProp::PadColumn(10));
        let ids = items(&mut t, c, &[(50, 60), (40, 60)]);
        run(&mut t);
        // The main (vertical) axis is not mirrored; tracks start at the right (LVGL).
        assert_eq!(ys(&t, &ids), [0, 0]);
        assert_eq!(xs(&t, &ids), [250, 200]);
    }

    #[test]
    fn grow_per_track() {
        let (mut t, c) = flex(200, 100, FlexFlow::RowWrap);
        t.set_style(c, StyleProp::PadColumn(10));
        let a = t.add(c, StyleBuf::new().width(80).height(10));
        let g = t.add(c, StyleBuf::new().flex_grow(1).height(10));
        let b = t.add(c, StyleBuf::new().width(150).height(10));
        let g2 = t.add(c, StyleBuf::new().flex_grow(1).height(10));
        run(&mut t);
        assert_eq!(origin(&t, a), (0, 0));
        assert_eq!(t.coords(g), Rect::from_xywh(90, 0, 110, 10));
        assert_eq!(t.coords(b), Rect::from_xywh(0, 10, 150, 10));
        assert_eq!(t.coords(g2), Rect::from_xywh(160, 10, 40, 10));
    }

    #[test]
    fn nested_flex_containers() {
        let (mut t, c) = flex(300, 100, FlexFlow::Row);
        let inner = t.add(
            c,
            StyleBuf::new()
                .layout(LayoutKind::Flex)
                .flex_flow(FlexFlow::Column)
                .pad_row(2),
        );
        let a = t.add(inner, StyleBuf::new().width(20).height(10));
        let b = t.add(inner, StyleBuf::new().width(30).height(10));
        let after = t.add(c, StyleBuf::new().width(5).height(5));
        run(&mut t);
        assert_eq!(t.coords(inner), Rect::from_xywh(0, 0, 30, 22));
        assert_eq!(origin(&t, a), (0, 0));
        assert_eq!(origin(&t, b), (0, 12));
        assert_eq!(origin(&t, after), (30, 0));
    }
}

mod grid {
    use super::*;

    // ---- Grid I -------------------------------------------------------------------------------

    #[test]
    fn px_tracks_exact() {
        use GridTrack::Px;
        let (mut t, c) = grid(
            Length::Px(300),
            Length::Px(300),
            &[Px(50), Px(70)],
            &[Px(30), Px(40)],
        );
        let a = cell(&mut t, c, 0, 0, stretch());
        let b = cell(&mut t, c, 1, 1, stretch());
        run(&mut t);
        assert_eq!(t.coords(a), Rect::from_xywh(0, 0, 50, 30));
        assert_eq!(t.coords(b), Rect::from_xywh(50, 30, 70, 40));
    }

    #[test]
    fn fr_tracks_share_free_space_exactly() {
        use GridTrack::Fr;
        let (mut t, c) = grid(Length::Px(100), Length::Px(100), &[Fr(1), Fr(1), Fr(1)], &[Fr(1)]);
        let ids: Vec<usize> = (0..3).map(|i| cell(&mut t, c, i, 0, stretch())).collect();
        run(&mut t);
        let w: Vec<i32> = ids.iter().map(|&i| t.coords(i).width()).collect();
        assert_eq!(w, [33, 34, 33]);
        assert_eq!(xs(&t, &ids), [0, 33, 67]);
    }

    #[test]
    fn documented_3x3_fr_grid() {
        use GridTrack::Fr;
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Fr(1); 3], &[Fr(1); 3]);
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::PadRow(10));
        let ids: Vec<usize> = (0..9).map(|i| cell(&mut t, c, i % 3, i / 3, stretch())).collect();
        run(&mut t);
        // free = 300 - 2 × 10 = 280 → 93, 94, 93.
        assert_eq!(xs(&t, &ids[..3]), [0, 103, 207]);
        assert_eq!(ys(&t, &[ids[0], ids[3], ids[6]]), [0, 103, 207]);
        assert_eq!(t.coords(ids[4]).size(), Size::new(94, 94));
    }

    #[test]
    fn documented_mixed_px_content_fr_grid() {
        use GridTrack::{Content, Fr, Px};
        let (mut t, c) = grid(
            Length::Px(400),
            Length::Px(100),
            &[Px(60), Content, Fr(1), Fr(2)],
            &[Px(50)],
        );
        t.set_style(c, StyleProp::PadColumn(5));
        let content = cell(&mut t, c, 1, 0, StyleBuf::new().width(45).height(10));
        let ids: Vec<usize> = [0, 2, 3]
            .iter()
            .map(|&i| cell(&mut t, c, i, 0, stretch()))
            .collect();
        run(&mut t);
        // free = 400 - 15 - 105 = 280 → fr 93 and 187.
        assert_eq!(origin(&t, content), (65, 0));
        assert_eq!(xs(&t, &ids), [0, 115, 213]);
        let w: Vec<i32> = ids.iter().map(|&i| t.coords(i).width()).collect();
        assert_eq!(w, [60, 93, 187]);
    }

    #[test]
    fn content_track_fits_largest_child() {
        use GridTrack::{Content, Px};
        let (mut t, c) = grid(
            Length::Px(300),
            Length::Px(300),
            &[Content, Px(10)],
            &[Px(20), Px(20)],
        );
        cell(
            &mut t,
            c,
            0,
            0,
            StyleBuf::new().width(40).height(5).margin_left(3),
        );
        cell(&mut t, c, 0, 1, StyleBuf::new().width(25).height(5));
        // Spanning items do not grow content tracks.
        cell(
            &mut t,
            c,
            0,
            0,
            StyleBuf::new().width(90).height(5).grid_cell_column_span(2),
        );
        let b = cell(&mut t, c, 1, 0, StyleBuf::new().width(5).height(5));
        run(&mut t);
        // The track is the largest width without margins (LVGL).
        assert_eq!(origin(&t, b), (40, 0));
    }

    #[test]
    fn gaps_between_tracks() {
        use GridTrack::Px;
        let (mut t, c) = grid(
            Length::Px(300),
            Length::Px(300),
            &[Px(50), Px(50)],
            &[Px(20), Px(20)],
        );
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::PadRow(4));
        let b = cell(&mut t, c, 1, 1, StyleBuf::new().width(5).height(5));
        run(&mut t);
        assert_eq!(origin(&t, b), (60, 24));
    }

    #[test]
    fn span_covers_tracks_and_inner_gaps() {
        use GridTrack::Px;
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Px(50); 3], &[Px(20); 3]);
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::PadRow(5));
        let a = cell(
            &mut t,
            c,
            1,
            0,
            stretch().grid_cell_column_span(2).grid_cell_row_span(3),
        );
        run(&mut t);
        assert_eq!(t.coords(a), Rect::from_xywh(60, 0, 110, 70));
    }

    #[test]
    fn out_of_range_cell_warns_and_is_clamped() {
        use GridTrack::Px;
        #[cfg(all(feature = "log", not(feature = "defmt")))]
        log_capture::install();
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Px(50); 3], &[Px(20); 3]);
        let a = cell(&mut t, c, 5, -2, stretch());
        let b = cell(
            &mut t,
            c,
            1,
            0,
            stretch().grid_cell_column_span(4).grid_cell_row_span(0),
        );
        run(&mut t);
        // LVGL clamps: column 5 → 2, row −2 → 0, span 4 at 1 → 2, span 0 → 1.
        assert_eq!(t.coords(a), Rect::from_xywh(100, 0, 50, 20));
        assert_eq!(t.coords(b), Rect::from_xywh(50, 0, 100, 20));
        #[cfg(all(feature = "log", not(feature = "defmt")))]
        {
            let warnings = log_capture::take();
            assert!(
                warnings.iter().any(|w| w.contains("column position was 5")),
                "{warnings:?}"
            );
            assert!(
                warnings.iter().any(|w| w.contains("row span was 0")),
                "{warnings:?}"
            );
        }
    }

    #[test]
    fn column_align_center_and_space_between() {
        use GridTrack::Px;
        let (mut t, c) = grid(Length::Px(300), Length::Px(100), &[Px(50), Px(50)], &[Px(20)]);
        t.set_style(c, StyleProp::PadColumn(10));
        t.set_style(c, StyleProp::GridColumnAlign(GridAlign::Center));
        let ids: Vec<usize> = (0..2).map(|i| cell(&mut t, c, i, 0, stretch())).collect();
        run(&mut t);
        assert_eq!(xs(&t, &ids), [95, 155]);
        // Space modes replace the gap.
        t.set_style(c, StyleProp::GridColumnAlign(GridAlign::SpaceBetween));
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 250]);
        t.set_style(c, StyleProp::GridColumnAlign(GridAlign::End));
        t.set_style(c, StyleProp::GridRowAlign(GridAlign::SpaceEvenly)); // one row → centered
        run(&mut t);
        assert_eq!(xs(&t, &ids), [190, 250]);
        assert_eq!(ys(&t, &ids), [40, 40]);
        t.set_style(c, StyleProp::GridColumnAlign(GridAlign::Stretch)); // Start for tracks
        run(&mut t);
        assert_eq!(xs(&t, &ids), [0, 60]);
    }

    #[test]
    fn content_sized_grid_treats_fr_as_content() {
        use GridTrack::Fr;
        let (mut t, c) = grid(Length::Content, Length::Content, &[Fr(1), Fr(2)], &[Fr(1)]);
        t.set_style(c, StyleProp::PadColumn(5));
        t.set_style(c, StyleProp::GridColumnAlign(GridAlign::End)); // ignored when content-sized
        let a = cell(&mut t, c, 0, 0, StyleBuf::new().width(30).height(8));
        let b = cell(&mut t, c, 1, 0, StyleBuf::new().width(40).height(12));
        run(&mut t);
        assert_eq!(t.coords(c).size(), Size::new(75, 12));
        assert_eq!(origin(&t, a), (0, 0));
        assert_eq!(origin(&t, b), (35, 0));
    }

    #[test]
    fn grid_without_templates_places_children_by_position() {
        let mut t = ToyTree::new(1000, 1000);
        let c = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(100).height(100).layout(LayoutKind::Grid),
        );
        let a = t.add(c, StyleBuf::new().width(10).height(10).x(7).y(9));
        run(&mut t);
        assert_eq!(origin(&t, a), (7, 9));
    }

    // ---- Grid II ------------------------------------------------------------------------------

    #[test]
    fn cell_stretch_fills_cell() {
        use GridTrack::Px;
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Px(80)], &[Px(40)]);
        let a = cell(
            &mut t,
            c,
            0,
            0,
            stretch().margin_left(3).margin_right(2).margin_top(1),
        );
        run(&mut t);
        assert_eq!(t.coords(a), Rect::from_xywh(3, 1, 75, 39));
    }

    #[test]
    fn cell_center_and_end() {
        use GridTrack::Px;
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Px(80), Px(80)], &[Px(40)]);
        let a = cell(
            &mut t,
            c,
            0,
            0,
            StyleBuf::new()
                .width(21)
                .height(10)
                .grid_cell_x_align(GridAlign::Center)
                .grid_cell_y_align(GridAlign::End),
        );
        let b = cell(
            &mut t,
            c,
            1,
            0,
            StyleBuf::new()
                .width(20)
                .height(10)
                .margin_right(4)
                .margin_bottom(3)
                .grid_cell_x_align(GridAlign::End)
                .grid_cell_y_align(GridAlign::Center),
        );
        run(&mut t);
        assert_eq!(origin(&t, a), (29, 30));
        // x = 80 + 80 - 20 - 4; y = (40 - 10) / 2 + (0 - 3) / 2 = 15 - 1.
        assert_eq!(origin(&t, b), (136, 14));
    }

    #[test]
    fn stretch_overrides_min_max_like_lvgl() {
        use GridTrack::Px;
        let (mut t, c) = grid(Length::Px(300), Length::Px(300), &[Px(80)], &[Px(40)]);
        let a = cell(&mut t, c, 0, 0, stretch().max_width(50).min_height(60));
        run(&mut t);
        // LVGL sets a stretched item's size directly (w_layout/h_layout), bypassing min/max.
        assert_eq!(t.coords(a).size(), Size::new(80, 40));
    }

    #[test]
    fn rtl_mirrors_columns() {
        use GridTrack::Px;
        let (mut t, c) = grid(
            Length::Px(300),
            Length::Px(100),
            &[Px(50), Px(60), Px(70)],
            &[Px(20)],
        );
        t.set_style(c, StyleProp::BaseDir(BaseDir::Rtl));
        t.set_style(c, StyleProp::PadColumn(10));
        let ids: Vec<usize> = (0..3).map(|i| cell(&mut t, c, i, 0, stretch())).collect();
        let span = cell(&mut t, c, 0, 0, stretch().grid_cell_column_span(2));
        // Children inherit RTL: Start of an item means the right edge of its cell.
        let small = cell(&mut t, c, 2, 0, StyleBuf::new().width(10).height(5));
        run(&mut t);
        assert_eq!(xs(&t, &ids), [250, 180, 100]);
        assert_eq!(t.coords(span), Rect::from_xywh(180, 0, 120, 20));
        assert_eq!(origin(&t, small), (160, 0));
    }

    #[test]
    fn grid_content_size_sum_of_tracks() {
        use GridTrack::{Content, Px};
        let (mut t, c) = grid(
            Length::Content,
            Length::Content,
            &[Px(30), Content, Px(10)],
            &[Px(20), Content],
        );
        t.set_style(c, StyleProp::PadColumn(4));
        t.set_style(c, StyleProp::PadRow(6));
        t.set_style(c, StyleProp::PadLeft(1));
        t.set_style(c, StyleProp::BorderWidth(2));
        cell(&mut t, c, 1, 1, StyleBuf::new().width(25).height(15));
        run(&mut t);
        // x: 30 + 25 + 10 + 2 × 4 + 1 + 2 × 2 = 78; y: 20 + 15 + 6 + 2 × 2 = 45.
        assert_eq!(t.coords(c).size(), Size::new(78, 45));
        assert_eq!(crate::content_size_of(&t, c, crate::Axis::X), 78);
    }
}

// ---- Scratch reuse --------------------------------------------------------------------------

#[test]
fn scratch_is_reused_and_empty_after_layout() {
    let (mut t, c) = flex(300, 100, FlexFlow::RowWrap);
    for _ in 0..20 {
        let g = t.add(c, StyleBuf::new().width(40).height(10));
        t.add(g, StyleBuf::new().width(4).height(4));
    }
    let mut s = crate::LayoutScratch::new();
    let screen = t.coords(ToyTree::ROOT);
    crate::layout_subtree_with(&mut t, &mut s, ToyTree::ROOT, screen);
    let caps = (s.items.capacity(), s.tracks.capacity(), s.ints.capacity());
    assert!(s.items.is_empty() && s.tracks.is_empty() && s.ints.is_empty());
    crate::layout_subtree_with(&mut t, &mut s, ToyTree::ROOT, screen);
    assert_eq!(caps, (s.items.capacity(), s.tracks.capacity(), s.ints.capacity()));
}

#[test]
fn layout_children_keeps_the_node_rect() {
    let (mut t, c) = flex(300, 100, FlexFlow::Row);
    let a = t.add(c, StyleBuf::new().width(10).height(10));
    t.set_coords(c, Rect::from_xywh(5, 6, 300, 100));
    crate::layout_children(&mut t, c);
    assert_eq!(t.coords(c), Rect::from_xywh(5, 6, 300, 100));
    assert_eq!(origin(&t, a), (5, 6));
}

// ---- Property tests ---------------------------------------------------------------------------

mod flex_grid_props {
    use proptest::prelude::*;

    use super::*;

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
    const CELL_ALIGNS: [GridAlign; 4] = [
        GridAlign::Start,
        GridAlign::Center,
        GridAlign::End,
        GridAlign::Stretch,
    ];
    const TRACK_ALIGNS: [GridAlign; 7] = [
        GridAlign::Start,
        GridAlign::Center,
        GridAlign::End,
        GridAlign::Stretch,
        GridAlign::SpaceEvenly,
        GridAlign::SpaceAround,
        GridAlign::SpaceBetween,
    ];

    fn inside(outer: Rect, r: Rect) -> bool {
        r.x0 >= outer.x0 && r.y0 >= outer.y0 && r.x1 <= outer.x1 && r.y1 <= outer.y1
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Items that all fit stay inside the content area and never overlap; without wrapping,
        /// grow items fill the main axis exactly.
        #[test]
        fn flex_invariants(
            flow in 0usize..8,
            main in 0usize..6,
            cross in 0usize..6,
            track in 0usize..6,
            rtl in any::<bool>(),
            (w, h) in (200i32..400, 200i32..400),
            (item_gap, track_gap) in (0i32..=5, 0i32..=5),
            pad in 0i32..10,
            specs in proptest::collection::vec((0i32..=20, 0i32..=20, 0u8..3), 1..=8),
        ) {
            let flow = FLOWS[flow];
            let (mut t, c) = flex(w + 2 * pad, h + 2 * pad, flow);
            for p in [StyleProp::PadLeft(pad), StyleProp::PadRight(pad), StyleProp::PadTop(pad), StyleProp::PadBottom(pad)] {
                t.set_style(c, p);
            }
            t.set_style(c, StyleProp::PadColumn(item_gap));
            t.set_style(c, StyleProp::PadRow(track_gap));
            t.set_style(c, StyleProp::FlexMainPlace(PLACES[main]));
            t.set_style(c, StyleProp::FlexCrossPlace(PLACES[cross]));
            t.set_style(c, StyleProp::FlexTrackPlace(PLACES[track]));
            if rtl {
                t.set_style(c, StyleProp::BaseDir(BaseDir::Rtl));
            }
            let ids: Vec<usize> = specs
                .iter()
                .map(|&(a, b, g)| {
                    let s = StyleBuf::new().width(a).height(b).flex_grow(g);
                    t.add(c, s)
                })
                .collect();
            run(&mut t);
            let content = Rect::from_xywh(pad, pad, w, h);
            let rects: Vec<Rect> = ids.iter().map(|&i| t.coords(i)).collect();
            for r in &rects {
                prop_assert!(inside(content, *r), "{r} outside {content}");
            }
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    prop_assert!(!a.intersects(b), "{a} overlaps {b}");
                }
            }
            if !flow.is_wrap() && specs.iter().any(|s| s.2 > 0) {
                let main_sum: i32 = rects
                    .iter()
                    .map(|r| if flow.is_column() { r.height() } else { r.width() })
                    .sum::<i32>()
                    + if flow.is_column() { track_gap } else { item_gap } * (rects.len() as i32 - 1);
                prop_assert_eq!(main_sum, if flow.is_column() { h } else { w });
            }
        }

        /// Grid items stay inside their cells when they fit; stretched items equal their cell.
        #[test]
        fn grid_invariants(
            cols in proptest::collection::vec(0u8..3, 1..=4),
            rows in proptest::collection::vec(0u8..3, 1..=4),
            track_vals in proptest::collection::vec(1i32..60, 8),
            (col_align, row_align) in (0usize..7, 0usize..7),
            (w, h) in (100i32..400, 100i32..400),
            (gap_c, gap_r) in (0i32..10, 0i32..10),
            rtl in any::<bool>(),
            specs in proptest::collection::vec((0i32..4, 0i32..4, 1i32..3, 1i32..3, 0usize..4, 0usize..4, 0i32..50, 0i32..50), 1..=8),
        ) {
            let mk = |kinds: &[u8], off: usize| -> Vec<GridTrack> {
                kinds
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| match k {
                        0 => GridTrack::Px(track_vals[(i + off) % 8]),
                        1 => GridTrack::Content,
                        _ => GridTrack::Fr((track_vals[(i + off) % 8] % 3 + 1) as u8),
                    })
                    .collect()
            };
            let (ct, rt) = (mk(&cols, 0), mk(&rows, 4));
            let (mut t, c) = grid(Length::Px(w), Length::Px(h), &ct, &rt);
            t.set_style(c, StyleProp::PadColumn(gap_c));
            t.set_style(c, StyleProp::PadRow(gap_r));
            t.set_style(c, StyleProp::GridColumnAlign(TRACK_ALIGNS[col_align]));
            t.set_style(c, StyleProp::GridRowAlign(TRACK_ALIGNS[row_align]));
            if rtl {
                t.set_style(c, StyleProp::BaseDir(BaseDir::Rtl));
            }
            let (nc, nr) = (ct.len() as i32, rt.len() as i32);
            let mut placed = Vec::new();
            for &(cp, rp, cs, rs, xa, ya, iw, ih) in &specs {
                // Keep cells inside the template (clamping is tested separately).
                let (cp, rp) = (cp % nc, rp % nr);
                let (cs, rs) = (cs.min(nc - cp), rs.min(nr - rp));
                let s = StyleBuf::new()
                    .width(iw)
                    .height(ih)
                    .grid_cell_column_pos(cp)
                    .grid_cell_row_pos(rp)
                    .grid_cell_column_span(cs)
                    .grid_cell_row_span(rs)
                    .grid_cell_x_align(CELL_ALIGNS[xa])
                    .grid_cell_y_align(CELL_ALIGNS[ya]);
                let id = t.add(c, s);
                placed.push((id, cp, rp, cs, rs, xa, ya, iw, ih));
            }
            run(&mut t);
            // Cells measured with stretched probes in an identical grid.
            let (mut probe, pc) = grid(Length::Px(w), Length::Px(h), &ct, &rt);
            probe.set_style(pc, StyleProp::PadColumn(gap_c));
            probe.set_style(pc, StyleProp::PadRow(gap_r));
            probe.set_style(pc, StyleProp::GridColumnAlign(TRACK_ALIGNS[col_align]));
            probe.set_style(pc, StyleProp::GridRowAlign(TRACK_ALIGNS[row_align]));
            if rtl {
                probe.set_style(pc, StyleProp::BaseDir(BaseDir::Rtl));
            }
            // The same items (they size `Content` tracks), invisible to nothing else.
            for &(_, cp, rp, cs, rs, _, _, iw, ih) in &placed {
                let s = StyleBuf::new()
                    .width(iw)
                    .height(ih)
                    .grid_cell_column_pos(cp)
                    .grid_cell_row_pos(rp)
                    .grid_cell_column_span(cs)
                    .grid_cell_row_span(rs);
                probe.add(pc, s);
            }
            let probes: Vec<usize> = placed
                .iter()
                .map(|&(_, cp, rp, cs, rs, ..)| {
                    let s = stretch()
                        .width(0)
                        .height(0)
                        .grid_cell_column_pos(cp)
                        .grid_cell_row_pos(rp)
                        .grid_cell_column_span(cs)
                        .grid_cell_row_span(rs);
                    let id = probe.add(pc, s);
                    probe.set_flags(id, LayoutFlags::empty());
                    id
                })
                .collect();
            run(&mut probe);
            for (k, &(id, _, _, _, _, xa, ya, iw, ih)) in placed.iter().enumerate() {
                let cellr = probe.coords(probes[k]);
                let r = t.coords(id);
                let stretch_x = CELL_ALIGNS[xa] == GridAlign::Stretch;
                let stretch_y = CELL_ALIGNS[ya] == GridAlign::Stretch;
                if stretch_x {
                    prop_assert_eq!((r.x0, r.x1), (cellr.x0, cellr.x1));
                } else if iw <= cellr.width() {
                    prop_assert!(r.x0 >= cellr.x0 && r.x1 <= cellr.x1, "x {} not in {}", r, cellr);
                }
                if stretch_y {
                    prop_assert_eq!((r.y0, r.y1), (cellr.y0, cellr.y1));
                } else if ih <= cellr.height() {
                    prop_assert!(r.y0 >= cellr.y0 && r.y1 <= cellr.y1, "y {} not in {}", r, cellr);
                }
            }
        }
    }
}

/// Captures `twine::layout` warnings through the `log` crate.
#[cfg(all(feature = "log", not(feature = "defmt")))]
mod log_capture {
    use std::string::{String, ToString};
    use std::sync::Mutex;
    use std::vec::Vec;

    static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            if r.target() == "twine::layout" && r.level() == log::Level::Warn {
                RECORDS.lock().unwrap().push(r.args().to_string());
            }
        }
        fn flush(&self) {}
    }

    static LOGGER: Capture = Capture;

    pub(super) fn install() {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Warn);
    }

    pub(super) fn take() -> Vec<String> {
        core::mem::take(&mut *RECORDS.lock().unwrap())
    }
}
