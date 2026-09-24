//! The [`LayoutTree`] trait: the boundary between the layout algorithms and a widget tree.

use twine_core::{Insets, Point, Rect, Size};
use twine_style::{Align, BaseDir, BorderSide, Length, PropId, StyleValue};

/// A layout axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Axis {
    /// Horizontal (widths, x coordinates).
    X,
    /// Vertical (heights, y coordinates).
    Y,
}

impl Axis {
    /// The other axis.
    #[must_use]
    pub const fn other(self) -> Axis {
        match self {
            Axis::X => Axis::Y,
            Axis::Y => Axis::X,
        }
    }

    /// `0` for [`Axis::X`], `1` for [`Axis::Y`] (index into `[x, y]` pairs).
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The extent of `s` along this axis.
    ///
    /// ```
    /// use twine_core::Size;
    /// use twine_layout::Axis;
    ///
    /// assert_eq!(Axis::X.of(Size::new(3, 4)), 3);
    /// assert_eq!(Axis::Y.of(Size::new(3, 4)), 4);
    /// ```
    #[must_use]
    pub const fn of(self, s: Size) -> i32 {
        match self {
            Axis::X => s.w,
            Axis::Y => s.h,
        }
    }
}

bitflags::bitflags! {
    /// Per-node flags that influence layout (mapped by the widget tree from its object flags).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct LayoutFlags: u8 {
        /// The node is hidden: it is still sized and positioned, but flex and grid skip it and
        /// it does not contribute to its parent's content size (LVGL `LV_OBJ_FLAG_HIDDEN`).
        const HIDDEN = 1 << 0;
        /// Flex and grid do not position the node; it is placed by its own `X`/`Y`/`Align`
        /// styles but still counts for the parent's content size (`LV_OBJ_FLAG_IGNORE_LAYOUT`).
        const IGNORE_LAYOUT = 1 << 1;
        /// Like `IGNORE_LAYOUT`, and the node neither scrolls with its parent nor counts for
        /// the parent's content size (`LV_OBJ_FLAG_FLOATING`).
        const FLOATING = 1 << 2;
        /// In a flex container, the node starts a new track (`LV_OBJ_FLAG_FLEX_IN_NEW_TRACK`).
        const FLEX_IN_NEW_TRACK = 1 << 3;
        /// The node's content may be scrolled (informational for the widget tree's scroll
        /// bounds; the layout itself only uses [`LayoutTree::scroll`]).
        const SCROLLABLE_CONTENT = 1 << 4;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for LayoutFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "LayoutFlags({=u8:#x})", self.bits());
    }
}

/// Alignment of a node relative to another node (LVGL `lv_obj_align_to`), returned by
/// [`LayoutTree::align_to`].
///
/// The node is placed at `align` relative to `base` (inner alignments use the base's content
/// area, `Out*` alignments its outer rectangle), shifted by `(x, y)` and by the node's own
/// translation. Unlike LVGL, which computes the position once, the relation is kept and
/// re-evaluated on every layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlignTo<I> {
    /// The reference node.
    pub base: I,
    /// Alignment relative to `base`.
    pub align: Align,
    /// Horizontal offset in pixels.
    pub x: i32,
    /// Vertical offset in pixels.
    pub y: i32,
}

/// A tree the layout algorithms can size and position.
///
/// Styles are read through [`style_prop`](Self::style_prop): the implementation returns the
/// resolved value of the node's `Main` part (including inherited properties such as
/// `BaseDir` and defaults for unset properties). Coordinates are absolute, half-open
/// [`Rect`]s; a child's rectangle is independent of its parent's (moving a parent does not
/// move its children until they are laid out).
///
/// Implementations: the engine's widget tree, and `ToyTree` (feature `toy`) for tests.
pub trait LayoutTree {
    /// Node identifier.
    type Id: Copy + Eq;

    /// Calls `out` for each child of `id`, in z-order (first child first).
    fn children(&self, id: Self::Id, out: &mut dyn FnMut(Self::Id));

    /// An integer property of `id` (paddings, margins, gaps, grid cell positions…). The
    /// default implementation reads [`style_prop`](Self::style_prop) and returns 0 for
    /// non-integer values.
    fn style_i32(&self, id: Self::Id, prop: PropId) -> i32 {
        self.style_prop(id, prop).as_i32().unwrap_or(0)
    }

    /// The resolved value of `prop` for the `Main` part of `id`.
    fn style_prop(&self, id: Self::Id, prop: PropId) -> StyleValue;

    /// The size of the widget's own content (text, image…) without padding: LVGL's "self
    /// size". Containers without own content return [`Size::ZERO`](Size).
    fn content_size(&self, id: Self::Id) -> Size;

    /// Layout-relevant flags of `id`.
    fn flags(&self, id: Self::Id) -> LayoutFlags;

    /// Stores the new absolute rectangle of `id`. Only called when the rectangle changes.
    fn set_coords(&mut self, id: Self::Id, r: Rect);

    /// The current absolute rectangle of `id`.
    fn coords(&self, id: Self::Id) -> Rect;

    /// The node `id` is aligned to, if any (see [`AlignTo`]). Default: none.
    fn align_to(&self, id: Self::Id) -> Option<AlignTo<Self::Id>> {
        let _ = id;
        None
    }

    /// How far the content of `id` is scrolled: children (except `FLOATING` ones) are placed
    /// at their content position minus this offset. Default: not scrolled.
    fn scroll(&self, id: Self::Id) -> Point {
        let _ = id;
        Point::new(0, 0)
    }

    /// Whether the `Width` (for [`Axis::X`]) or `Height` style of `id` is
    /// [`Length::Content`].
    fn is_content_sized(&self, id: Self::Id, axis: Axis) -> bool {
        length(self, id, size_prop(axis)) == Length::Content
    }
}

/// `Width` or `Height`.
pub(crate) const fn size_prop(axis: Axis) -> PropId {
    match axis {
        Axis::X => PropId::Width,
        Axis::Y => PropId::Height,
    }
}

/// `MinWidth` or `MinHeight`.
pub(crate) const fn min_prop(axis: Axis) -> PropId {
    match axis {
        Axis::X => PropId::MinWidth,
        Axis::Y => PropId::MinHeight,
    }
}

/// `MaxWidth` or `MaxHeight`.
pub(crate) const fn max_prop(axis: Axis) -> PropId {
    match axis {
        Axis::X => PropId::MaxWidth,
        Axis::Y => PropId::MaxHeight,
    }
}

/// A `Length` property, falling back to the property's default for mismatched values.
pub(crate) fn length<T: LayoutTree + ?Sized>(t: &T, id: T::Id, prop: PropId) -> Length {
    t.style_prop(id, prop)
        .as_length()
        .or_else(|| prop.meta().default.as_length())
        .unwrap_or_default()
}

/// A typed enum property, falling back to the property's default and then to `T::default()`.
pub(crate) fn style_enum<T, V>(t: &T, id: T::Id, prop: PropId) -> V
where
    T: LayoutTree + ?Sized,
    V: twine_style::PropValue + Default,
{
    t.style_prop(id, prop)
        .get::<V>()
        .or_else(|| prop.meta().default.get::<V>())
        .unwrap_or_default()
}

/// Whether the node's base direction is right-to-left.
pub(crate) fn is_rtl<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> bool {
    style_enum::<T, BaseDir>(t, id, PropId::BaseDir) == BaseDir::Rtl
}

/// Padding plus border width on each side (LVGL `lv_obj_get_style_space_*`: the border
/// counts only on the sides listed in `BorderSide`).
pub(crate) fn spaces<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> Insets {
    let bw = t.style_i32(id, PropId::BorderWidth);
    let side = t
        .style_prop(id, PropId::BorderSide)
        .get::<BorderSide>()
        .unwrap_or(BorderSide::FULL);
    let b = |s: BorderSide| if side.contains(s) { bw } else { 0 };
    Insets::new(
        t.style_i32(id, PropId::PadLeft) + b(BorderSide::LEFT),
        t.style_i32(id, PropId::PadTop) + b(BorderSide::TOP),
        t.style_i32(id, PropId::PadRight) + b(BorderSide::RIGHT),
        t.style_i32(id, PropId::PadBottom) + b(BorderSide::BOTTOM),
    )
}

/// The four margins.
pub(crate) fn margins<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> Insets {
    Insets::new(
        t.style_i32(id, PropId::MarginLeft),
        t.style_i32(id, PropId::MarginTop),
        t.style_i32(id, PropId::MarginRight),
        t.style_i32(id, PropId::MarginBottom),
    )
}

/// Start (left/top) inset along `axis`.
pub(crate) const fn start(i: Insets, axis: Axis) -> i32 {
    match axis {
        Axis::X => i.left,
        Axis::Y => i.top,
    }
}

/// End (right/bottom) inset along `axis`.
pub(crate) const fn end(i: Insets, axis: Axis) -> i32 {
    match axis {
        Axis::X => i.right,
        Axis::Y => i.bottom,
    }
}

/// Start + end inset along `axis`.
pub(crate) const fn sum(i: Insets, axis: Axis) -> i32 {
    start(i, axis) + end(i, axis)
}

/// Sets the coordinates of `id` only if they change.
pub(crate) fn place<T: LayoutTree + ?Sized>(t: &mut T, id: T::Id, r: Rect) {
    if t.coords(id) != r {
        t.set_coords(id, r);
    }
}

/// The content rectangle of a node's outer rectangle.
pub(crate) const fn content_rect(r: Rect, sp: Insets) -> Rect {
    Rect::new(r.x0 + sp.left, r.y0 + sp.top, r.x1 - sp.right, r.y1 - sp.bottom)
}
