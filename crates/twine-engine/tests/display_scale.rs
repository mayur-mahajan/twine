//! Display scale (R7.S04d): logical lengths resolve to device pixels with the scale of the
//! node's display, device lengths stay, input thresholds scale, and a build without
//! `display-scale` lays a scaled display out at 1× (themes: `twine-widgets/tests/display_scale.rs`).

mod common;

use common::white_screen;
use twine_core::{DisplayScale, Rect};
use twine_engine::{NodeId, Obj};
use twine_style::{GridAlign, GridTrack, LayoutKind, Length, Selector, StyleProp};
use twine_testing::EngineHarness;

/// A scene in logical pixels: a padded flex column with a border and a gap, holding a fixed
/// box, a `Dp` box with a margin, a percentage-wide box and a grid with fixed tracks.
fn scene(e: &mut twine_engine::Engine, screen: NodeId, size: (i32, i32), pad: i32, gap: i32) -> Vec<NodeId> {
    static COLS: [GridTrack; 2] = [GridTrack::Px(20), GridTrack::Px(14)];
    static ROWS: [GridTrack; 1] = [GridTrack::Px(8)];
    let col = e.create(screen, Box::new(Obj)).unwrap();
    e.set_pos(col, 3, 5);
    e.set_size(col, 120, 160);
    for p in [
        StyleProp::PaddingTop(Length::Px(pad).into()),
        StyleProp::PaddingLeft(Length::Px(pad).into()),
        // Symmetric: the content width stays even, so the 50 % child is exact at 1× too.
        StyleProp::PaddingRight(Length::Px(pad).into()),
        StyleProp::RowGap(Length::Px(gap).into()),
        StyleProp::BorderWidth(Length::Px(1).into()),
        StyleProp::Layout(LayoutKind::Flex),
    ] {
        e.set_local_prop(col, Selector::MAIN, p);
    }
    e.set_flex_flow(col, twine_style::FlexFlow::COLUMN);
    let fixed = e.create(col, Box::new(Obj)).unwrap();
    e.set_size(fixed, size.0, size.1);
    let dp = e.create(col, Box::new(Obj)).unwrap();
    e.set_local_prop(dp, Selector::MAIN, StyleProp::Width(Length::dp(32).into()));
    e.set_local_prop(dp, Selector::MAIN, StyleProp::Height(Length::Px(10).into()));
    e.set_local_prop(dp, Selector::MAIN, StyleProp::MarginLeft(Length::Px(4).into()));
    let pct = e.create(col, Box::new(Obj)).unwrap();
    e.set_local_prop(pct, Selector::MAIN, StyleProp::Width(Length::pct(50).into()));
    e.set_local_prop(pct, Selector::MAIN, StyleProp::Height(Length::Px(6).into()));
    let grid = e.create(col, Box::new(Obj)).unwrap();
    e.set_size(grid, 60, 12);
    e.set_grid_tracks(grid, &COLS, &ROWS);
    let cells: Vec<NodeId> = (0..2)
        .map(|c| {
            let n = e.create(grid, Box::new(Obj)).unwrap();
            e.set_grid_cell(n, c, 0, GridAlign::Stretch, GridAlign::Stretch);
            n
        })
        .collect();
    let mut all = vec![col, fixed, dp, pct, grid];
    all.extend(cells);
    all
}

/// The scene laid out on a 200 × 200 logical screen at `scale` (DPI 160: 1 dp = 1 logical
/// px), returning the device coordinates of its nodes.
fn layout(scale: DisplayScale, size: (i32, i32), pad: i32, gap: i32) -> Vec<Rect> {
    let side = u16::try_from(scale.to_device(200)).unwrap();
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(side, side)
        .no_theme()
        .dpi(160)
        .display_scale(scale)
        .mount_engine(|e| {
            let s = white_screen(e);
            ids = scene(e, s, size, pad, gap);
        });
    h.run_until_idle();
    ids.iter().map(|&n| h.engine().coords(n)).collect()
}

#[cfg(feature = "display-scale")]
mod scaled {
    use super::*;
    use common::list;
    use proptest::prelude::*;
    use twine_core::Point;
    use twine_style::{Part, PropId, StyleValue};

    /// `r` with every coordinate multiplied by `k`: the device rectangle of a logical one at an
    /// integer scale (the ends are exclusive).
    fn times(r: Rect, k: i32) -> Rect {
        Rect::new(r.x0 * k, r.y0 * k, r.x1 * k, r.y1 * k)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        /// At an integer scale every node's device rectangle is exactly `k` × its 1× rectangle:
        /// sizes, positions, paddings, gaps, borders, margins, `Dp`, percentages and fixed grid
        /// tracks all scale together. (A percentage of an odd size truncates once, in device
        /// pixels, so it can be up to `k − 1` px larger than `k` × its 1× value; the scene keeps
        /// its percentage base even.)
        #[test]
        fn integer_scales_multiply_every_coordinate(
            k in 2i32..=3,
            w in 1i32..80,
            h in 1i32..30,
            pad in 0i32..10,
            gap in 0i32..8,
        ) {
            let one = layout(DisplayScale::ONE, (w, h), pad, gap);
            let scale = DisplayScale::from_raw_256(u16::try_from(k * 256).unwrap());
            let many = layout(scale, (w, h), pad, gap);
            for (a, b) in one.iter().zip(&many) {
                prop_assert_eq!(*b, times(*a, k));
            }
        }
    }

    #[test]
    fn resolved_lengths_are_device_pixels() {
        let mut n = None;
        let mut h = EngineHarness::new(300, 300)
            .no_theme()
            .dpi(160)
            .display_scale(DisplayScale::pct(150))
            .mount_engine(|e| {
                let s = white_screen(e);
                let b = e.create(s, Box::new(Obj)).unwrap();
                for p in [
                    StyleProp::Width(Length::Px(10).into()),
                    StyleProp::PaddingTop(Length::Px(3).into()),
                    StyleProp::PaddingLeft(Length::dp(4).into()),
                    StyleProp::BorderWidth(Length::dev(1).into()),
                    StyleProp::DropShadowOffsetX(Length::Px(2).into()),
                ] {
                    e.set_local_prop(b, Selector::MAIN, p);
                }
                n = Some(b);
            });
        h.run_until_idle();
        let (e, n) = (h.engine(), n.unwrap());
        let dev = |v| StyleValue::Length(Length::Dev(v));
        assert_eq!(e.style_prop(n, Part::Main, PropId::Width), dev(15));
        // 4.5 rounds half away from zero.
        assert_eq!(e.style_prop(n, Part::Main, PropId::PaddingTop), dev(5));
        assert_eq!(e.style_prop(n, Part::Main, PropId::PaddingLeft), dev(6));
        assert_eq!(e.style_prop(n, Part::Main, PropId::BorderWidth), dev(1));
        assert_eq!(e.style_i32(n, Part::Main, PropId::DropShadowOffsetX), 3);
        assert_eq!(e.coords(n).width(), 15);
        assert_eq!(
            (e.node_scale(n), e.node_to_device(n, 10), e.node_dpx(n, 10)),
            (DisplayScale::pct(150), 15, 15)
        );
    }

    #[test]
    fn drag_threshold_is_logical() {
        // 2×: the list and its rows are twice as large in device pixels, and so is the scroll
        // limit (10 logical px = 20 device px).
        let mut ids = None;
        let mut h = EngineHarness::new(400, 400)
            .no_theme()
            .display_scale(DisplayScale::X2)
            .mount_engine(|e| {
                let s = white_screen(e);
                ids = Some(list(e, s, Rect::from_xywh(20, 20, 100, 100), 10, 40));
            });
        h.run_until_idle();
        let (cont, _) = ids.unwrap();
        let period = h.engine().config().read_period;
        h.press(Point::new(140, 160));
        for y in [150, 142] {
            h.clock().advance(period);
            h.move_to(Point::new(140, y));
            assert_eq!(
                h.engine().scroll_offset(cont).y,
                0,
                "18 device px are below the limit"
            );
        }
        // 30 device px: scrolling starts with this read's movement (12 device px), as at 1×.
        h.clock().advance(period);
        h.move_to(Point::new(140, 130));
        assert_eq!(h.engine().scroll_offset(cont).y, 12);
    }
}

/// Without the feature a scaled display is laid out at 1× (and warned about once).
#[cfg(not(feature = "display-scale"))]
#[test]
fn without_the_feature_a_scaled_display_is_laid_out_at_1x() {
    let one = layout(DisplayScale::ONE, (30, 10), 4, 2);
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(400, 400)
        .no_theme()
        .dpi(160)
        .display_scale(DisplayScale::X2)
        .mount_engine(|e| {
            let s = white_screen(e);
            ids = scene(e, s, (30, 10), 4, 2);
        });
    h.run_until_idle();
    let d = h.display();
    assert_eq!(h.engine().display_scale(d), DisplayScale::ONE);
    let coords: Vec<Rect> = ids.iter().map(|&n| h.engine().coords(n)).collect();
    assert_eq!(coords, one);
}

/// R7.S04f: a display changing its scale at run time (a desktop window moved to another
/// monitor) resolves, lays out and draws everything again at the new scale.
#[cfg(feature = "display-scale")]
mod live {
    use twine_core::fault::FaultKind;
    use twine_core::{ColorFormat, DisplayScale, Instant};
    use twine_engine::{BufferMode, BufferSpec, DisplayControlFault, Engine, EngineConfig, Obj};
    use twine_hal::DisplayInfo;
    use twine_style::{Length, Selector, StyleProp};
    use twine_testing::MemoryDisplay;

    /// A 60 × 40 logical display at 1× with a 20 × 10 box at (5, 4) padded by 3.
    fn scene(resizable: bool) -> (Engine, twine_engine::DisplayId, twine_engine::NodeId) {
        let mut e = Engine::new(EngineConfig::default()).unwrap();
        let panel = MemoryDisplay::new(DisplayInfo::new(60, 40, ColorFormat::Rgb565));
        let panel = if resizable { panel.with_resize() } else { panel };
        let d = e
            .add_display(panel, BufferMode::alloc(BufferSpec::default()), None)
            .unwrap();
        let screen = e.active_screen(d).unwrap();
        let n = e.create(screen, Box::new(Obj)).unwrap();
        e.set_pos(n, 5, 4);
        e.set_size(n, 20, 10);
        e.set_local_prop(n, Selector::MAIN, StyleProp::PaddingTop(Length::Px(3).into()));
        e.step(Instant::from_millis(0));
        (e, d, n)
    }

    #[test]
    fn a_scale_change_relayouts_at_the_new_scale() {
        let (mut e, d, n) = scene(true);
        assert_eq!(e.coords(n), twine_core::Rect::from_xywh(5, 4, 20, 10));
        e.set_display_scale(d, DisplayScale::X2).unwrap();
        e.step(Instant::from_millis(20));
        let info = e.display_info(d).unwrap();
        assert_eq!((info.width, info.height, info.scale), (120, 80, DisplayScale::X2));
        assert_eq!(e.coords(e.active_screen(d).unwrap()).width(), 120);
        assert_eq!(e.coords(n), twine_core::Rect::from_xywh(10, 8, 40, 20));
        assert_eq!(
            e.style_i32(n, twine_style::Part::Main, twine_style::PropId::PaddingTop),
            6
        );
        // And back: 1.5× at the same logical size.
        e.set_display_scale(d, DisplayScale::pct(150)).unwrap();
        e.step(Instant::from_millis(40));
        assert_eq!(e.coords(n), twine_core::Rect::from_xywh(8, 6, 30, 15));
    }

    #[test]
    fn a_panel_that_cannot_resize_keeps_its_scale() {
        let (mut e, d, n) = scene(false);
        e.set_display_scale(d, DisplayScale::X2).unwrap();
        e.step(Instant::from_millis(20));
        assert_eq!(e.display_scale(d), DisplayScale::ONE);
        assert_eq!(e.coords(n).width(), 20);
        let f = e
            .last_fault(FaultKind::DisplayControl)
            .expect("a DisplayControl fault");
        assert_eq!(
            DisplayControlFault::from_code(f.code),
            Some(DisplayControlFault::ResizeFailed)
        );
    }

    #[test]
    fn invalid_requests_are_refused() {
        let (mut e, _, _) = scene(true);
        // A chunked display has no driver to resize.
        let info = DisplayInfo::new(60, 40, ColorFormat::Rgb565);
        let chunked = e.add_chunked_display(info, 60 * 2 * 8, None).unwrap();
        assert!(e.set_display_scale(chunked, DisplayScale::X2).is_err());
    }
}
