//! Size resolution: `Px`/`Pct`/`Content` lengths, min/max clamping and content sizes
//! (LVGL `lv_obj_refr_size`, `calc_dynamic_width`, `calc_content_width`).
//!
//! Rules (LVGL 9):
//! - `Pct(p)` of the parent's content size, minus the node's own margins on that axis.
//! - `Content`: the larger of the widget's own content size and the extent of its children,
//!   plus padding and border on both sides.
//! - Min/max are resolved like the size; the result is `max(min, min(size, max))`, so `min`
//!   wins when `min > max`.
//! - A child whose size in effect is a percentage of a content-sized parent contributes
//!   nothing to that parent's content size (otherwise the two would depend on each other);
//!   it is resolved against the parent's final size afterwards.

use twine_core::{Rect, Size};
use twine_style::{LayoutKind, Length, PropId};

use crate::layout::{Item, LayoutScratch, effective_kind};
use crate::tree::{Axis, LayoutTree, margins, max_prop, min_prop, size_prop, spaces, sum};
use crate::{flex, grid, position};

/// Resolves a width or height: `Px(n)` is `n`, `Pct(p)` is `p` % of `parent_content`
/// (truncated like LVGL), `Content` calls `content`.
///
/// ```
/// use twine_layout::resolve_length;
/// use twine_style::Length;
///
/// assert_eq!(resolve_length(Length::Px(40), 200, || 7), 40);
/// assert_eq!(resolve_length(Length::Pct(25), 200, || 7), 50);
/// assert_eq!(resolve_length(Length::Content, 200, || 7), 7);
/// ```
#[must_use]
pub fn resolve_length(len: Length, parent_content: i32, content: impl FnOnce() -> i32) -> i32 {
    match len {
        Length::Content => content(),
        l => l.resolve(parent_content, 0),
    }
}

/// Clamps a resolved size between `min` and `max` (resolved against `parent_content` like
/// sizes). As in LVGL, `min` wins when `min > max`. A `Content` bound does not constrain here
/// (the layout itself resolves content bounds against the node's content).
///
/// ```
/// use twine_layout::clamp_size;
/// use twine_style::Length;
///
/// assert_eq!(clamp_size(500, Length::Px(0), Length::Pct(50), 400), 200);
/// assert_eq!(clamp_size(10, Length::Px(30), Length::Px(20), 400), 30); // min wins
/// ```
#[must_use]
pub fn clamp_size(v: i32, min: Length, max: Length, parent_content: i32) -> i32 {
    let lo = match min {
        Length::Content => i32::MIN,
        l => l.resolve(parent_content, 0),
    };
    let hi = match max {
        Length::Content => i32::MAX,
        l => l.resolve(parent_content, 0),
    };
    lo.max(v.min(hi))
}

/// The content size of `id` along `axis` (LVGL `calc_content_width`/`_height`): the larger of
/// [`LayoutTree::content_size`] and the extent of the children, plus padding and border.
///
/// Children extents: hidden, floating and `align_to` children are skipped; flex and grid
/// containers use their track sizes; children of other nodes count from the content start to
/// their far edge plus their end margin (only start-aligned children, or others with a zero
/// offset). The size of `id` on the other axis is taken from its current coordinates.
/// This allocates a temporary scratch buffer; the layout itself reuses one.
#[must_use]
pub fn content_size_of<T: LayoutTree + ?Sized>(t: &T, id: T::Id, axis: Axis) -> i32 {
    let mut s = LayoutScratch::new();
    let sp = spaces(t, id);
    let c = t.coords(id);
    let avail = Size::new(c.x1 - c.x0 - sp.horizontal(), c.y1 - c.y0 - sp.vertical());
    let sized = [t.is_content_sized(id, Axis::X), t.is_content_sized(id, Axis::Y)];
    measure_content(t, &mut s, id, axis, avail, sized)
}

/// A node's size and the values needed by flex/grid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resolved {
    /// Outer size (without margins).
    pub size: Size,
    /// Resolved min size per axis (`i32::MIN` when the axis was overridden).
    pub min: [i32; 2],
    /// Resolved max size per axis (`i32::MAX` when the axis was overridden).
    pub max: [i32; 2],
    /// The size in effect comes from a percentage (LVGL `w_ignore_size` input).
    pub pct: [bool; 2],
}

/// Resolves the size of `id` in a parent whose content area has size `parent`. `ovr` forces
/// the size on an axis (flex grow, grid stretch: LVGL `w_layout`/`h_layout`).
pub(crate) fn resolve_node<T: LayoutTree + ?Sized>(
    t: &T,
    s: &mut LayoutScratch<T::Id>,
    id: T::Id,
    parent: Size,
    ovr: [Option<i32>; 2],
) -> Resolved {
    let m = margins(t, id);
    let sp = spaces(t, id);
    let styles = [Axis::X, Axis::Y]
        .map(|a| [size_prop(a), min_prop(a), max_prop(a)].map(|p| crate::tree::length(t, id, p)));
    let sized = [
        ovr[0].is_none() && styles[0][0] == Length::Content,
        ovr[1].is_none() && styles[1][0] == Length::Content,
    ];
    let needs = |a: usize| ovr[a].is_none() && styles[a].contains(&Length::Content);
    // A content size may depend on the other axis (wrapping flex): resolve that one first.
    let order = if needs(0) && !needs(1) {
        [Axis::Y, Axis::X]
    } else {
        [Axis::X, Axis::Y]
    };
    let mut out = Resolved {
        size: Size::new(0, 0),
        min: [i32::MIN; 2],
        max: [i32::MAX; 2],
        pct: [false; 2],
    };
    let mut known = ovr;
    for axis in order {
        let a = axis.index();
        let v = if let Some(v) = ovr[a] {
            v
        } else {
            let pc = axis.of(parent);
            let msum = sum(m, axis);
            let fixed = |l: Length| match l {
                Length::Content => None,
                Length::Px(v) => Some(v),
                l @ Length::Pct(_) => Some(l.resolve(pc, 0).saturating_sub(msum)),
            };
            let mut content: Option<i32> = None;
            let mut vals = [0; 3];
            for (k, val) in vals.iter_mut().enumerate() {
                *val = match fixed(styles[a][k]) {
                    Some(v) => v,
                    None => *content.get_or_insert_with(|| {
                        let own = fixed(styles[a][0]).map_or(0, |v| v - sum(sp, axis));
                        let other = known[1 - a].map_or(0, |v| v - sum(sp, axis.other()));
                        let avail = match axis {
                            Axis::X => Size::new(own, other),
                            Axis::Y => Size::new(other, own),
                        };
                        measure_content(t, s, id, axis, avail, sized)
                    }),
                };
            }
            let [v, mn, mx] = vals;
            out.min[a] = mn;
            out.max[a] = mx;
            out.pct[a] = size_in_effect_is_pct(v, mn, mx, styles[a]);
            mn.max(v.min(mx))
        };
        match axis {
            Axis::X => out.size.w = v,
            Axis::Y => out.size.h = v,
        }
        known[a] = Some(v);
    }
    out
}

/// Whether the style that sets the final size is a percentage (LVGL `size_in_effect_is_pct`).
fn size_in_effect_is_pct(unclamped: i32, min: i32, max: i32, styles: [Length; 3]) -> bool {
    let is_pct = |l: Length| matches!(l, Length::Pct(_));
    let [size_style, min_style, max_style] = styles;
    if min > max || unclamped < min {
        return is_pct(min_style);
    }
    if unclamped > max {
        return is_pct(max_style);
    }
    let mut p = is_pct(size_style);
    if p && unclamped == min {
        p = is_pct(min_style);
    }
    if p && unclamped == max {
        p = is_pct(max_style);
    }
    p
}

/// Content size of `id` on `axis` including padding and border. `avail` is the node's own
/// content-area size (0 on axes not known yet), `sized` whether each axis is content-sized.
pub(crate) fn measure_content<T: LayoutTree + ?Sized>(
    t: &T,
    s: &mut LayoutScratch<T::Id>,
    id: T::Id,
    axis: Axis,
    avail: Size,
    sized: [bool; 2],
) -> i32 {
    let own = axis.of(t.content_size(id));
    let ext = children_extent(t, s, id, axis, avail, sized);
    sum(spaces(t, id), axis).saturating_add(ext.map_or(own, |e| e.max(own)))
}

/// Extent of the children of `id` on `axis`, measured from its content start; `None` if no
/// child contributes.
fn children_extent<T: LayoutTree + ?Sized>(
    t: &T,
    s: &mut LayoutScratch<T::Id>,
    id: T::Id,
    axis: Axis,
    avail: Size,
    sized: [bool; 2],
) -> Option<i32> {
    let kind = effective_kind(t, id, false);
    let frame = s.items.len();
    {
        let items = &mut s.items;
        t.children(id, &mut |c| items.push(Item::new(c)));
    }
    let end = s.items.len();
    if end == frame {
        return None;
    }
    for i in frame..end {
        let c = s.items[i].id;
        Item::classify(&mut s.items[i], t, kind);
        if !s.items[i].in_flow && !s.items[i].counts_for_content() {
            continue;
        }
        let r = resolve_node(t, s, c, avail, [None; 2]);
        let it = &mut s.items[i];
        it.set_resolved(r);
        it.margin = margins(t, c);
        it.ignore = [sized[0] && r.pct[0], sized[1] && r.pct[1]];
        match kind {
            LayoutKind::Flex => it.grow = flex::grow_of(t, c),
            LayoutKind::Grid => it.cell = grid::cell_of(t, c),
            LayoutKind::None => {}
        }
    }
    let rtl = crate::tree::is_rtl(t, id);
    let ext = match kind {
        LayoutKind::Flex => flex::extent(t, s, id, frame, end, axis, avail, sized, rtl),
        LayoutKind::Grid => grid::extent(t, s, id, frame, end, axis, avail, sized, rtl),
        LayoutKind::None => position::abs_extent_all(t, &s.items[frame..end], axis, avail, sized, rtl),
    };
    s.items.truncate(frame);
    ext
}

/// `Size` of a (possibly degenerate) rectangle without clamping negative extents to 0.
pub(crate) const fn raw_size(r: Rect) -> Size {
    Size::new(r.x1 - r.x0, r.y1 - r.y0)
}

/// `Pct` of `base` like LVGL, `Px` as is, `Content` as 0 (positions and translations).
pub(crate) fn offset_len(l: Length, base: i32) -> i32 {
    match l {
        Length::Content => 0,
        l => l.resolve(base, 0),
    }
}

/// The translation of `id` (`TranslateX/Y`; `Pct` is relative to the node's own size).
pub(crate) fn translate<T: LayoutTree + ?Sized>(t: &T, id: T::Id, size: Size) -> (i32, i32) {
    (
        offset_len(crate::tree::length(t, id, PropId::TranslateX), size.w),
        offset_len(crate::tree::length(t, id, PropId::TranslateY), size.h),
    )
}

#[cfg(test)]
mod tests {
    use twine_core::{Rect, Size};
    use twine_style::{Length, StyleBuf};

    use super::*;
    use crate::toy::ToyTree;
    use crate::{LayoutFlags, layout_subtree};

    fn pad(s: StyleBuf, v: i32) -> StyleBuf {
        s.pad_left(v).pad_right(v).pad_top(v).pad_bottom(v)
    }

    fn run(t: &mut ToyTree) {
        let r = t.coords(ToyTree::ROOT);
        layout_subtree(t, ToyTree::ROOT, r);
    }

    #[test]
    fn px_size_is_exact() {
        let mut t = ToyTree::new(300, 200);
        let a = t.add(ToyTree::ROOT, StyleBuf::new().width(37).height(11));
        run(&mut t);
        assert_eq!(t.coords(a).size(), Size::new(37, 11));
    }

    #[test]
    fn pct_of_parent_content() {
        let mut t = ToyTree::new(300, 200);
        t.style_mut(ToyTree::ROOT)
            .set(twine_style::StyleProp::PadLeft(10));
        t.style_mut(ToyTree::ROOT)
            .set(twine_style::StyleProp::PadRight(40));
        t.style_mut(ToyTree::ROOT)
            .set(twine_style::StyleProp::BorderWidth(5));
        let a = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(Length::pct(50)).height(Length::pct(33)),
        );
        // Margins are subtracted from percentage sizes (LVGL).
        let b = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(Length::pct(100)).margin_left(6).height(1),
        );
        run(&mut t);
        // content width = 300 - 10 - 40 - 2 * 5 = 240; height = 200 - 2 * 5 = 190.
        assert_eq!(t.coords(a).size(), Size::new(120, 62));
        assert_eq!(t.coords(b).size(), Size::new(234, 1));
        assert_eq!(t.coords(a).origin(), twine_core::Point::new(15, 5));
    }

    #[test]
    fn pct_over_100() {
        let mut t = ToyTree::new(200, 100);
        let a = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(Length::pct(150)).height(Length::pct(250)),
        );
        run(&mut t);
        assert_eq!(t.coords(a).size(), Size::new(300, 250));
    }

    #[test]
    fn min_max_clamp() {
        let mut t = ToyTree::new(400, 400);
        let a = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .width(500)
                .max_width(Length::pct(50))
                .height(5)
                .min_height(30),
        );
        run(&mut t);
        assert_eq!(t.coords(a).size(), Size::new(200, 30));
        assert_eq!(clamp_size(500, Length::Px(0), Length::Pct(50), 400), 200);
        assert_eq!(clamp_size(-5, Length::Px(0), Length::Content, 400), 0);
    }

    #[test]
    fn min_wins_over_max() {
        let mut t = ToyTree::new(400, 400);
        let a = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(50).min_width(80).max_width(60).height(1),
        );
        run(&mut t);
        assert_eq!(t.coords(a).width(), 80);
        assert_eq!(clamp_size(10, Length::Px(30), Length::Px(20), 0), 30);
    }

    #[test]
    fn content_size_uses_children_bbox_plus_padding() {
        let mut t = ToyTree::new(400, 400);
        let p = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .pad_left(3)
                .pad_right(4)
                .pad_top(5)
                .pad_bottom(6)
                .border_width(1),
        );
        t.add(
            p,
            StyleBuf::new()
                .width(50)
                .height(20)
                .x(10)
                .y(7)
                .margin_right(2)
                .margin_bottom(9),
        );
        t.add(p, StyleBuf::new().width(30).height(60));
        run(&mut t);
        // x: max(10 + 50 + 2, 30) + 3 + 4 + 2 = 71; y: max(7 + 20 + 9, 60) + 5 + 6 + 2 = 73.
        assert_eq!(t.coords(p).size(), Size::new(71, 73));
        assert_eq!(content_size_of(&t, p, Axis::X), 71);
        assert_eq!(content_size_of(&t, p, Axis::Y), 73);
    }

    #[test]
    fn content_size_uses_widget_content_if_larger() {
        let mut t = ToyTree::new(400, 400);
        let p = t.add(ToyTree::ROOT, pad(StyleBuf::new(), 2));
        t.set_content_size(p, Size::new(80, 10));
        t.add(p, StyleBuf::new().width(50).height(20));
        run(&mut t);
        assert_eq!(t.coords(p).size(), Size::new(84, 24));
    }

    #[test]
    fn pct_child_ignored_for_content_parent_then_resolved() {
        let mut t = ToyTree::new(400, 400);
        let p = t.add(ToyTree::ROOT, StyleBuf::new());
        let fixed = t.add(p, StyleBuf::new().width(60).height(40));
        let pct = t.add(p, StyleBuf::new().width(Length::pct(50)).height(Length::pct(200)));
        run(&mut t);
        assert_eq!(t.coords(p).size(), Size::new(60, 40));
        assert_eq!(t.coords(fixed).size(), Size::new(60, 40));
        assert_eq!(t.coords(pct).size(), Size::new(30, 80));
    }

    #[test]
    fn floating_child_ignored_for_content() {
        let mut t = ToyTree::new(400, 400);
        let p = t.add(ToyTree::ROOT, StyleBuf::new());
        t.add(p, StyleBuf::new().width(20).height(10));
        let f = t.add(p, StyleBuf::new().width(200).height(200));
        t.set_flags(f, LayoutFlags::FLOATING);
        let ig = t.add(p, StyleBuf::new().width(30).height(5));
        t.set_flags(ig, LayoutFlags::IGNORE_LAYOUT);
        run(&mut t);
        // IGNORE_LAYOUT children still count (LVGL).
        assert_eq!(t.coords(p).size(), Size::new(30, 10));
        assert_eq!(t.coords(f).size(), Size::new(200, 200));
    }

    #[test]
    fn hidden_child_ignored_for_content() {
        let mut t = ToyTree::new(400, 400);
        let p = t.add(ToyTree::ROOT, StyleBuf::new());
        t.add(p, StyleBuf::new().width(20).height(10));
        let h = t.add(p, StyleBuf::new().width(200).height(200));
        t.set_flags(h, LayoutFlags::HIDDEN);
        run(&mut t);
        assert_eq!(t.coords(p).size(), Size::new(20, 10));
        // Hidden nodes are still sized (LVGL).
        assert_eq!(t.coords(h), Rect::from_xywh(0, 0, 200, 200));
    }

    #[test]
    fn content_bounds_and_non_start_aligned_children() {
        let mut t = ToyTree::new(400, 400);
        // Width 10 but at least the content (a 50 px child).
        let p = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(10).min_width(Length::Content).height(30),
        );
        t.add(
            p,
            StyleBuf::new()
                .width(50)
                .height(5)
                .align(twine_style::Align::Center),
        );
        // A centered child with an offset does not count.
        t.add(
            p,
            StyleBuf::new()
                .width(90)
                .height(5)
                .x(3)
                .align(twine_style::Align::Center),
        );
        run(&mut t);
        assert_eq!(t.coords(p).width(), 50);
    }

    #[test]
    fn nested_content_parents() {
        let mut t = ToyTree::new(400, 400);
        let outer = t.add(ToyTree::ROOT, pad(StyleBuf::new(), 5));
        let inner = t.add(outer, pad(StyleBuf::new(), 1));
        t.add(inner, StyleBuf::new().width(10).height(20));
        run(&mut t);
        assert_eq!(t.coords(inner).size(), Size::new(12, 22));
        assert_eq!(t.coords(outer).size(), Size::new(22, 32));
        assert_eq!(t.coords(inner).origin(), twine_core::Point::new(5, 5));
    }
}
