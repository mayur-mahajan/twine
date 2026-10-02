//! R1.S07: layout convenience modifiers — `fill` / `fill_width` / `fill_height`,
//! `.layout(Layout::…)` and `card(children)` expand to exactly the properties of the explicit
//! forms and lay out identically.

use twine_core::Rect;
use twine_engine::NodeId;
use twine_style::{PropId, StyleValue};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

/// The local `Main` properties of `id` with their values (probed by removing each one, so
/// call it last).
fn local_props(t: &mut TestUi, id: &'static str) -> Vec<(PropId, StyleValue)> {
    let n = node(t, id);
    let mut out = Vec::new();
    for p in PropId::ALL {
        let v = t.engine().style_prop(n, Part::Main, p);
        if t.engine_mut().remove_local_prop(n, p, Selector::MAIN) {
            out.push((p, v));
        }
    }
    out
}

/// The local properties `id` has beyond those of `base` (same widget without the modifier).
fn added_props(t: &mut TestUi, id: &'static str, base: &'static str) -> Vec<(PropId, StyleValue)> {
    let base = local_props(t, base);
    local_props(t, id)
        .into_iter()
        .filter(|p| !base.contains(p))
        .collect()
}

fn coords(t: &TestUi, id: &'static str) -> Rect {
    t.find(by_id(id)).coords()
}

#[test]
fn fill_modifiers_set_exactly_their_sizes() {
    let mut t = TestUi::new(120, 80).mount(|_| {
        container((
            label("x").test_id("base"),
            label("x").fill().test_id("fill"),
            label("x").fill_width().test_id("w"),
            label("x").fill_height().test_id("h"),
            label("x")
                .size(Length::pct(100), Length::pct(100))
                .test_id("explicit"),
        ))
        .size(100, 60)
        .padding(5)
    });
    t.run_until_idle();
    let w = (PropId::Width, StyleValue::Length(Length::Pct(100)));
    let h = (PropId::Height, StyleValue::Length(Length::Pct(100)));
    // 100 % of the parent's content area (padding and border excluded).
    let area = {
        let e = t.engine();
        e.content_area(e.tree().parent(node(&t, "fill")).unwrap())
    };
    assert_eq!(coords(&t, "fill").size(), area.size());
    assert_eq!(coords(&t, "fill"), coords(&t, "explicit"));
    assert_eq!(coords(&t, "w").width(), area.width());
    assert_eq!(coords(&t, "h").height(), area.height());
    assert_eq!(added_props(&mut t, "fill", "base"), vec![w, h]);
    assert_eq!(added_props(&mut t, "w", "base"), vec![w]);
    assert_eq!(added_props(&mut t, "h", "base"), vec![h]);
}

/// Three children in `id`'s content area, relative to it.
fn kids(t: &TestUi, id: &'static str) -> Vec<Rect> {
    let n = node(t, id);
    let e = t.engine();
    let origin = e.content_area(n);
    e.tree()
        .children(n)
        .map(|c| {
            let r = t.node(c).coords();
            Rect::new(
                r.x0 - origin.x0,
                r.y0 - origin.y0,
                r.x1 - origin.x0,
                r.y1 - origin.y0,
            )
        })
        .collect()
}

fn three() -> impl ViewSeq {
    (
        label("a").size(20, 10),
        label("b").size(30, 10),
        label("c").size(20, 10),
    )
}

/// A container with the layout set by raw engine calls (the pattern `.layout(..)` replaces).
fn raw(
    layout: LayoutKind,
    flow: Option<FlexFlow>,
    tracks: Option<(Vec<GridTrack>, Vec<GridTrack>)>,
) -> Container {
    container(three()).op(move |cx, n| {
        let e = cx.engine();
        e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(layout));
        if let Some(f) = flow {
            e.set_local_prop(n, Selector::MAIN, StyleProp::FlexFlow(f));
        }
        if let Some((c, r)) = tracks {
            e.set_grid_tracks(n, c, r);
        }
    })
}

#[test]
fn layout_variants_match_the_explicit_style_props() {
    let cols = || grid_tracks![fr(1), px(30)];
    let rows = || grid_tracks![content, content];
    let mut t = TestUi::new(240, 400).mount(move |_| {
        column((
            container(three())
                .layout(Layout::row())
                .size(80, 40)
                .test_id("lr"),
            raw(LayoutKind::Flex, Some(FlexFlow::ROW), None)
                .size(80, 40)
                .test_id("r"),
            container(three())
                .layout(Layout::none())
                .layout(Layout::row())
                .size(80, 40)
                .test_id("lr2"),
            container(three())
                .layout(Layout::column())
                .size(80, 70)
                .test_id("lc"),
            raw(LayoutKind::Flex, Some(FlexFlow::COLUMN), None)
                .size(80, 70)
                .test_id("c"),
            container(three())
                .layout(Layout::flex(FlexFlow::ROW.wrap(true)))
                .size(80, 60)
                .test_id("lf"),
            raw(LayoutKind::Flex, Some(FlexFlow::ROW.wrap(true)), None)
                .size(80, 60)
                .test_id("f"),
            container(three())
                .layout(Layout::grid(cols(), rows()))
                .size(90, 60)
                .test_id("lg"),
            raw(LayoutKind::Grid, None, Some((cols(), rows())))
                .size(90, 60)
                .test_id("g"),
            container(three())
                .layout(Layout::none())
                .size(80, 40)
                .test_id("ln"),
            raw(LayoutKind::None, None, None).size(80, 40).test_id("n"),
        ))
    });
    t.run_until_idle();
    let pairs = [
        ("lr", "r"),
        ("lr2", "r"),
        ("lc", "c"),
        ("lf", "f"),
        ("lg", "g"),
        ("ln", "n"),
    ];
    let e = t.engine();
    let get = |id, p| e.style_prop(node(&t, id), Part::Main, p);
    for (l, c) in pairs {
        assert_eq!(get(l, PropId::Layout), get(c, PropId::Layout), "{l}");
        assert_eq!(get(l, PropId::FlexFlow), get(c, PropId::FlexFlow), "{l}");
        assert_eq!(
            e.grid_column_tracks(node(&t, l)),
            e.grid_column_tracks(node(&t, c)),
            "{l}"
        );
        assert_eq!(
            e.grid_row_tracks(node(&t, l)),
            e.grid_row_tracks(node(&t, c)),
            "{l}"
        );
    }
    drop(e);
    for (l, c) in pairs {
        assert_eq!(kids(&t, l), kids(&t, c), "{l}");
    }
    // The variants really differ (the comparison is not vacuous).
    assert_ne!(kids(&t, "lr"), kids(&t, "lc"));
    assert_ne!(kids(&t, "lr"), kids(&t, "lg"));
}

#[test]
fn layout_row_matches_the_row_container() {
    // `row()` is transparent; the same `.layout(..)` on a transparent container is identical.
    let mut t = TestUi::new(200, 100).mount(|_| {
        column((
            container(three())
                .layout(Layout::row())
                .padding(0)
                .border_width(0)
                .gap(0)
                .size(80, 40)
                .test_id("l"),
            row(three()).gap(0).size(80, 40).test_id("r"),
        ))
    });
    t.run_until_idle();
    assert_eq!(kids(&t, "l"), kids(&t, "r"));
}

#[test]
fn layout_sets_exactly_the_container_props() {
    let mut t = TestUi::new(200, 100).mount(|_| {
        column((
            container(label("a")).test_id("base"),
            container(label("a")).layout(Layout::column()).test_id("col"),
            container(label("a")).layout(Layout::none()).test_id("none"),
            container(label("a"))
                .layout(Layout::grid(grid_tracks![fr(1)], grid_tracks![content]))
                .test_id("grid"),
        ))
    });
    t.run_until_idle();
    let g = t.find(by_id("grid")).id();
    assert_eq!(t.engine().grid_column_tracks(g), Some(&[GridTrack::Fr(1)][..]));
    assert_eq!(t.engine().grid_row_tracks(g), Some(&[GridTrack::Content][..]));
    assert_eq!(
        added_props(&mut t, "col", "base"),
        vec![
            (PropId::Layout, StyleValue::from(LayoutKind::Flex)),
            (PropId::FlexFlow, StyleValue::from(FlexFlow::COLUMN)),
        ]
    );
    assert_eq!(
        added_props(&mut t, "none", "base"),
        vec![(PropId::Layout, StyleValue::from(LayoutKind::None))]
    );
    // A grid adds its tracks as local style properties too (R2.S01, rework F4).
    let grid = added_props(&mut t, "grid", "base");
    assert_eq!(grid[0], (PropId::Layout, StyleValue::from(LayoutKind::Grid)));
    assert_eq!(
        grid.iter().map(|p| p.0).collect::<Vec<_>>(),
        [PropId::Layout, PropId::GridColumnTracks, PropId::GridRowTracks]
    );
}

#[test]
fn a_dynamic_layout_switches_between_row_and_column() {
    let mut wide = None;
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let w = cx.signal(true);
        wide = Some(w);
        container(three())
            .layout(move || if w.get() { Layout::row() } else { Layout::column() })
            .size(100, 80)
            .test_id("c")
    });
    t.run_until_idle();
    let row_kids = kids(&t, "c");
    assert!(row_kids[1].x0 > row_kids[0].x0 && row_kids[1].y0 == row_kids[0].y0);
    wide.unwrap().set(false);
    t.run_until_idle();
    let col_kids = kids(&t, "c");
    assert!(col_kids[1].y0 > col_kids[0].y0 && col_kids[1].x0 == col_kids[0].x0);
}

#[test]
fn card_has_the_theme_card_look_and_a_column() {
    let mut t = TestUi::new(200, 200).mount(|_| {
        column((
            container(label("a")).test_id("container"),
            card(three()).test_id("card"),
            container(three()).layout(Layout::column()).test_id("explicit"),
        ))
    });
    t.run_until_idle();
    let e = t.engine();
    let get = |id, p| e.style_prop(node(&t, id), Part::Main, p);
    // The theme's look of a plain object (the default theme's card style), not transparent.
    for p in [
        PropId::BgColor,
        PropId::BgOpacity,
        PropId::BorderWidth,
        PropId::BorderColor,
        PropId::Radius,
        PropId::PaddingTop,
        PropId::PaddingLeft,
        PropId::RowGap,
    ] {
        assert_eq!(get("card", p), get("container", p), "{p:?}");
    }
    assert_ne!(get("card", PropId::BgOpacity), StyleValue::Opa(Opa::TRANSP));
    assert_eq!(get("card", PropId::Layout), StyleValue::from(LayoutKind::Flex));
    assert_eq!(get("card", PropId::FlexFlow), StyleValue::from(FlexFlow::COLUMN));
    drop(e);
    assert_eq!(kids(&t, "card"), kids(&t, "explicit"));
    assert_eq!(coords(&t, "card").size(), coords(&t, "explicit").size());
}
