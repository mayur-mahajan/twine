//! Absolute positioning (LVGL `lv_obj_refr_pos`, `lv_obj_align_to`): `X`/`Y`, the 21
//! alignments, translation and right-to-left mirroring.

use twine_core::{Point, Rect, Size};
use twine_style::{Align, Length, PropId};

use crate::layout::Item;
use crate::size::{offset_len, raw_size, translate};
use crate::tree::{
    AlignTo, Axis, LayoutFlags, LayoutTree, content_rect, end, is_rtl, length, spaces, start, style_enum, sum,
};

/// Whether `a` is one of the 12 `Out*` alignments.
#[must_use]
pub const fn is_outside(a: Align) -> bool {
    a as u8 >= Align::OutTopLeft as u8
}

/// The top-left corner of a `child`-sized box aligned in `parent_content` (LVGL formulas,
/// including their truncation: centering is `w / 2 - child_w / 2`).
///
/// Inner alignments place the box inside the rectangle; the `Out*` alignments place it next
/// to the rectangle (as used by `align_to` with the base's outer rectangle). `Default` is
/// `TopLeft` (right-to-left mirroring is applied by the caller).
///
/// ```
/// use twine_core::{Point, Rect, Size};
/// use twine_layout::align_offset;
/// use twine_style::Align;
///
/// let parent = Rect::from_xywh(0, 0, 300, 200);
/// let child = Size::new(100, 50);
/// assert_eq!(align_offset(Align::Center, parent, child), Point::new(100, 75));
/// assert_eq!(align_offset(Align::OutBottomMid, parent, child), Point::new(100, 200));
/// ```
#[must_use]
pub const fn align_offset(align: Align, parent_content: Rect, child: Size) -> Point {
    let r = parent_content;
    let (pw, ph) = (r.x1 - r.x0, r.y1 - r.y0);
    let (w, h) = (child.w, child.h);
    let (x0, y0) = (r.x0, r.y0);
    let mid_x = x0 + pw / 2 - w / 2;
    let mid_y = y0 + ph / 2 - h / 2;
    let (x, y) = match align {
        Align::Default | Align::TopLeft => (x0, y0),
        Align::TopMid => (mid_x, y0),
        Align::TopRight => (x0 + pw - w, y0),
        Align::BottomLeft => (x0, y0 + ph - h),
        Align::BottomMid => (mid_x, y0 + ph - h),
        Align::BottomRight => (x0 + pw - w, y0 + ph - h),
        Align::LeftMid => (x0, mid_y),
        Align::RightMid => (x0 + pw - w, mid_y),
        Align::Center => (mid_x, mid_y),
        Align::OutTopLeft => (x0, y0 - h),
        Align::OutTopMid => (mid_x, y0 - h),
        Align::OutTopRight => (x0 + pw - w, y0 - h),
        Align::OutBottomLeft => (x0, r.y1),
        Align::OutBottomMid => (mid_x, r.y1),
        Align::OutBottomRight => (x0 + pw - w, r.y1),
        Align::OutLeftTop => (x0 - w, y0),
        Align::OutLeftMid => (x0 - w, mid_y),
        Align::OutLeftBottom => (x0 - w, y0 + ph - h),
        Align::OutRightTop => (r.x1, y0),
        Align::OutRightMid => (r.x1, mid_y),
        Align::OutRightBottom => (r.x1, y0 + ph - h),
    };
    Point::new(x, y)
}

/// The alignment used to position a child in its parent (LVGL `lv_obj_refr_pos`): `Default`
/// becomes `TopLeft`, the `Out*` values (meaningful only for `align_to`) act as `TopLeft`, and
/// with a right-to-left parent the left and right variants are swapped (so `Default` is
/// `TopRight`).
///
/// ```
/// use twine_layout::resolve_align;
/// use twine_style::Align;
///
/// assert_eq!(resolve_align(Align::Default, false), Align::TopLeft);
/// assert_eq!(resolve_align(Align::Default, true), Align::TopRight);
/// assert_eq!(resolve_align(Align::RightMid, true), Align::LeftMid);
/// ```
#[must_use]
pub const fn resolve_align(align: Align, rtl: bool) -> Align {
    let a = if matches!(align, Align::Default) || is_outside(align) {
        Align::TopLeft
    } else {
        align
    };
    if !rtl {
        return a;
    }
    match a {
        Align::TopLeft => Align::TopRight,
        Align::TopRight => Align::TopLeft,
        Align::LeftMid => Align::RightMid,
        Align::RightMid => Align::LeftMid,
        Align::BottomLeft => Align::BottomRight,
        Align::BottomRight => Align::BottomLeft,
        other => other,
    }
}

/// `X`/`Y` of a child resolved against its parent's content size; a percentage of a
/// content-sized parent is 0 (avoids a circular dependency, LVGL).
fn pos_of<T: LayoutTree + ?Sized>(t: &T, id: T::Id, axis: Axis, parent: Size, sized: [bool; 2]) -> i32 {
    let prop = match axis {
        Axis::X => PropId::X,
        Axis::Y => PropId::Y,
    };
    match length(t, id, prop) {
        Length::Pct(_) if sized[axis.index()] => 0,
        l => offset_len(l, axis.of(parent)),
    }
}

/// The rectangle of a child positioned by its own styles in a parent whose content area is
/// `content` (scrolled) / `unscrolled` (for `FLOATING` children).
pub(crate) fn abs_rect<T: LayoutTree + ?Sized>(
    t: &T,
    id: T::Id,
    size: Size,
    content: Rect,
    unscrolled: Rect,
    rtl: bool,
    sized: [bool; 2],
) -> Rect {
    let origin = if t.flags(id).contains(LayoutFlags::FLOATING) {
        unscrolled
    } else {
        content
    };
    let psize = raw_size(content);
    let (tx, ty) = translate(t, id, size);
    let x = pos_of(t, id, Axis::X, psize, sized) + tx;
    let y = pos_of(t, id, Axis::Y, psize, sized) + ty;
    let align = resolve_align(style_enum::<T, Align>(t, id, PropId::Align), rtl);
    let base = align_offset(align, Rect::new(0, 0, psize.w, psize.h), size);
    // LVGL negates the x offset in right-to-left parents except for left-anchored results.
    let dx = if rtl && !matches!(align, Align::TopLeft | Align::LeftMid | Align::BottomLeft) {
        -x
    } else {
        x
    };
    Rect::from_xywh(origin.x0 + base.x + dx, origin.y0 + base.y + y, size.w, size.h)
}

/// The rectangle of a node aligned to another one (LVGL `lv_obj_align_to`).
pub(crate) fn align_to_rect<T: LayoutTree + ?Sized>(t: &T, id: T::Id, size: Size, a: AlignTo<T::Id>) -> Rect {
    let base = t.coords(a.base);
    let align = match a.align {
        Align::Default if is_rtl(t, a.base) => Align::TopRight,
        Align::Default => Align::TopLeft,
        other => other,
    };
    let reference = if is_outside(align) {
        base
    } else {
        content_rect(base, spaces(t, a.base))
    };
    let p = align_offset(align, reference, size);
    let (tx, ty) = translate(t, id, size);
    Rect::from_xywh(p.x + a.x + tx, p.y + a.y + ty, size.w, size.h)
}

/// How far a child positioned by its own styles reaches from the parent's content start on
/// `axis` (LVGL `calc_content_width`): start-aligned children count up to their far edge plus
/// the end margin; other alignments count only with a zero offset (their size plus margins);
/// a percentage size of a content-sized parent does not count.
fn abs_extent<T: LayoutTree + ?Sized>(
    t: &T,
    it: &Item<T::Id>,
    axis: Axis,
    avail: Size,
    sized: [bool; 2],
    rtl: bool,
) -> Option<i32> {
    let a = axis.index();
    if it.ignore[a] {
        return None;
    }
    let align = style_enum::<T, Align>(t, it.id, PropId::Align);
    let start_aligned = match axis {
        Axis::X => matches!(
            align,
            Align::Default | Align::TopLeft | Align::BottomLeft | Align::LeftMid
        ),
        Axis::Y => matches!(
            align,
            Align::Default | Align::TopLeft | Align::TopMid | Align::TopRight
        ),
    };
    let sz = axis.of(it.size);
    if start_aligned {
        let (tx, ty) = translate(t, it.id, it.size);
        let tr = match axis {
            Axis::X => tx,
            Axis::Y => ty,
        };
        // In right-to-left parents the start is the right edge: the end margin is the left one.
        let end_margin = if rtl && axis == Axis::X {
            start(it.margin, axis)
        } else {
            end(it.margin, axis)
        };
        Some(pos_of(t, it.id, axis, avail, sized) + tr + sz + end_margin)
    } else {
        let prop = match axis {
            Axis::X => PropId::X,
            Axis::Y => PropId::Y,
        };
        (length(t, it.id, prop) == Length::Px(0)).then(|| sz + sum(it.margin, axis))
    }
}

/// The largest [`abs_extent`] over the children in `items` positioned by their own styles.
pub(crate) fn abs_extent_all<T: LayoutTree + ?Sized>(
    t: &T,
    items: &[Item<T::Id>],
    axis: Axis,
    avail: Size,
    sized: [bool; 2],
    rtl: bool,
) -> Option<i32> {
    items
        .iter()
        .filter(|it| it.counts_for_content())
        .filter_map(|it| abs_extent(t, it, axis, avail, sized, rtl))
        .max()
}

#[cfg(test)]
mod tests {
    use twine_core::{Point, Rect, Size};
    use twine_style::{Align, BaseDir, Length, StyleBuf};

    use super::*;
    use crate::toy::ToyTree;
    use crate::{AlignTo, layout_subtree};

    fn run(t: &mut ToyTree) {
        let r = t.coords(ToyTree::ROOT);
        layout_subtree(t, ToyTree::ROOT, r);
    }

    #[test]
    fn align_all_21_values() {
        let p = Rect::from_xywh(0, 0, 300, 200);
        let c = Size::new(100, 50);
        let table: [(Align, (i32, i32)); 22] = [
            (Align::Default, (0, 0)),
            (Align::TopLeft, (0, 0)),
            (Align::TopMid, (100, 0)),
            (Align::TopRight, (200, 0)),
            (Align::BottomLeft, (0, 150)),
            (Align::BottomMid, (100, 150)),
            (Align::BottomRight, (200, 150)),
            (Align::LeftMid, (0, 75)),
            (Align::RightMid, (200, 75)),
            (Align::Center, (100, 75)),
            (Align::OutTopLeft, (0, -50)),
            (Align::OutTopMid, (100, -50)),
            (Align::OutTopRight, (200, -50)),
            (Align::OutBottomLeft, (0, 200)),
            (Align::OutBottomMid, (100, 200)),
            (Align::OutBottomRight, (200, 200)),
            (Align::OutLeftTop, (-100, 0)),
            (Align::OutLeftMid, (-100, 75)),
            (Align::OutLeftBottom, (-100, 150)),
            (Align::OutRightTop, (300, 0)),
            (Align::OutRightMid, (300, 75)),
            (Align::OutRightBottom, (300, 150)),
        ];
        for (a, (x, y)) in table {
            assert_eq!(align_offset(a, p, c), Point::new(x, y), "{a:?}");
            // The same through a layout (inner alignments only; `Out*` act as `TopLeft`).
            let mut t = ToyTree::new(300, 200);
            let n = t.add(ToyTree::ROOT, StyleBuf::new().width(100).height(50).align(a));
            run(&mut t);
            let expect = if is_outside(a) {
                Point::new(0, 0)
            } else {
                Point::new(x, y)
            };
            assert_eq!(t.coords(n).origin(), expect, "{a:?}");
        }
        // LVGL truncates each half separately: 301/2 - 100/2 = 100.
        assert_eq!(
            align_offset(Align::Center, Rect::from_xywh(0, 0, 301, 11), Size::new(100, 4)),
            Point::new(100, 3)
        );
    }

    #[test]
    fn x_y_pct_of_parent() {
        let mut t = ToyTree::new(300, 200);
        let n = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .width(10)
                .height(10)
                .x(Length::pct(10))
                .y(Length::pct(-25)),
        );
        let m = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .width(10)
                .height(10)
                .x(Length::pct(10))
                .align(Align::TopRight),
        );
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(30, -50));
        assert_eq!(t.coords(m).origin(), Point::new(320, 0));
    }

    #[test]
    fn translate_does_not_affect_siblings() {
        let mut t = ToyTree::new(300, 200);
        t.style_mut(ToyTree::ROOT)
            .set(twine_style::StyleProp::Layout(twine_style::LayoutKind::Flex));
        let a = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(50).height(10).translate_x(7).translate_y(3),
        );
        let b = t.add(ToyTree::ROOT, StyleBuf::new().width(50).height(10));
        let c = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(20).height(10).x(5).translate_x(100),
        );
        t.set_flags(c, crate::LayoutFlags::IGNORE_LAYOUT);
        run(&mut t);
        assert_eq!(t.coords(a).origin(), Point::new(7, 3));
        assert_eq!(t.coords(b).origin(), Point::new(50, 0));
        assert_eq!(t.coords(c).origin(), Point::new(105, 0));
    }

    #[test]
    fn translate_pct_of_own_size() {
        let mut t = ToyTree::new(300, 200);
        let n = t.add(
            ToyTree::ROOT,
            StyleBuf::new()
                .width(80)
                .height(40)
                .align(Align::Center)
                .translate_x(Length::pct(-50))
                .translate_y(Length::pct(25)),
        );
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(110 - 40, 80 + 10));
    }

    #[test]
    fn align_to_out_bottom_mid_of_base() {
        let mut t = ToyTree::new(300, 200);
        let base = t.add(ToyTree::ROOT, StyleBuf::new().width(100).height(40).x(20).y(30));
        let n = t.add(ToyTree::ROOT, StyleBuf::new().width(50).height(10));
        t.set_align_to(
            n,
            Some(AlignTo {
                base,
                align: Align::OutBottomMid,
                x: 0,
                y: 5,
            }),
        );
        run(&mut t);
        assert_eq!(t.coords(n), Rect::from_xywh(20 + 50 - 25, 30 + 40 + 5, 50, 10));
        // Inner alignments use the base's content area.
        t.style_mut(base).set(twine_style::StyleProp::PadLeft(10));
        t.set_align_to(
            n,
            Some(AlignTo {
                base,
                align: Align::TopLeft,
                x: 1,
                y: 2,
            }),
        );
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(31, 32));
    }

    #[test]
    fn align_to_base_laid_out_first() {
        let mut t = ToyTree::new(300, 200);
        // `a` is aligned to `b`, which is itself aligned to `base`; both come before their base
        // in child order.
        let a = t.add(ToyTree::ROOT, StyleBuf::new().width(10).height(10));
        let b = t.add(ToyTree::ROOT, StyleBuf::new().width(20).height(20));
        let base = t.add(ToyTree::ROOT, StyleBuf::new().width(40).height(40).x(100).y(100));
        t.set_align_to(
            a,
            Some(AlignTo {
                base: b,
                align: Align::OutRightTop,
                x: 0,
                y: 0,
            }),
        );
        t.set_align_to(
            b,
            Some(AlignTo {
                base,
                align: Align::OutBottomLeft,
                x: 0,
                y: 0,
            }),
        );
        run(&mut t);
        assert_eq!(t.coords(b).origin(), Point::new(100, 140));
        assert_eq!(t.coords(a).origin(), Point::new(120, 140));
        // Moving the base moves both on the next layout.
        t.style_mut(base).set(twine_style::StyleProp::X(Length::Px(0)));
        run(&mut t);
        assert_eq!(t.coords(a).origin(), Point::new(20, 140));
    }

    #[test]
    fn rtl_default_align_is_top_right() {
        let mut t = ToyTree::new(300, 200);
        t.style_mut(ToyTree::ROOT)
            .set(twine_style::StyleProp::BaseDir(BaseDir::Rtl));
        let n = t.add(ToyTree::ROOT, StyleBuf::new().width(100).height(50).x(10).y(5));
        let m = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(100).height(50).x(10).align(Align::TopRight),
        );
        let c = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(100).height(50).x(10).align(Align::Center),
        );
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(190, 5));
        // LVGL: TopRight mirrors to TopLeft and keeps the offset's sign.
        assert_eq!(t.coords(m).origin(), Point::new(10, 0));
        assert_eq!(t.coords(c).origin(), Point::new(90, 75));
    }

    #[test]
    fn set_coords_called_only_on_change() {
        let mut t = ToyTree::new(300, 200);
        let a = t.add(ToyTree::ROOT, StyleBuf::new().width(10).height(10));
        t.add(a, StyleBuf::new().width(5).height(5));
        t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(20).height(10).align(Align::Center),
        );
        run(&mut t);
        assert_eq!(t.set_coords_calls(), 3);
        t.reset_set_coords_calls();
        run(&mut t);
        assert_eq!(t.set_coords_calls(), 0);
        t.style_mut(a).set(twine_style::StyleProp::Width(Length::Px(11)));
        run(&mut t);
        assert_eq!(t.set_coords_calls(), 1);
    }

    #[test]
    fn floating_child_ignores_scroll() {
        let mut t = ToyTree::new(300, 200);
        t.set_scroll(ToyTree::ROOT, Point::new(0, 40));
        let n = t.add(ToyTree::ROOT, StyleBuf::new().width(10).height(10));
        let f = t.add(ToyTree::ROOT, StyleBuf::new().width(10).height(10));
        t.set_flags(f, crate::LayoutFlags::FLOATING);
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(0, -40));
        assert_eq!(t.coords(f).origin(), Point::new(0, 0));
    }

    #[test]
    fn margins_ignored_by_absolute_positioning() {
        let mut t = ToyTree::new(300, 200);
        let n = t.add(
            ToyTree::ROOT,
            StyleBuf::new().width(10).height(10).margin_left(9).margin_top(9),
        );
        run(&mut t);
        assert_eq!(t.coords(n).origin(), Point::new(0, 0));
    }
}
