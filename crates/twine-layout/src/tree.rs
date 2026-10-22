//! The [`LayoutTree`] trait: the boundary between the layout algorithms and a widget tree.

use twine_core::{Insets, Point, Rect, Size};
use twine_style::{Anchor, BaseDir, BorderSide, GridTrack, Length, PropId, StyleValue};

use crate::LayoutScratch;

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

    /// This axis's entry of an `[x, y]` pair (no bounds check: the pair has an entry per
    /// axis).
    #[inline]
    pub(crate) const fn pick<T: Copy>(self, pair: [T; 2]) -> T {
        let [x, y] = pair;
        match self {
            Axis::X => x,
            Axis::Y => y,
        }
    }

    /// This axis's entry of an `[x, y]` pair, mutably.
    #[inline]
    pub(crate) fn pick_mut<T>(self, pair: &mut [T; 2]) -> &mut T {
        let [x, y] = pair;
        match self {
            Axis::X => x,
            Axis::Y => y,
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
        /// The node is docked to the bottom of its parent (not in LVGL; e.g. an on-screen
        /// keyboard reserving its height): it gets the full width of the parent's content
        /// area and its own height, and sits at the bottom of that area without scrolling
        /// with the parent. The parent's other children are then sized and positioned in the
        /// area above it, as if the parent's bottom padding were that much larger: `Pct`
        /// sizes, `Align`, flex and grid all use the reduced area. The node is neither a
        /// flex/grid item nor `align_to`-positioned, and it does not count for the parent's
        /// content size. Several docked children stack upwards in child order; a `HIDDEN`
        /// one reserves nothing and is placed like any hidden child.
        const DOCK_BOTTOM = 1 << 5;
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
/// The node is placed at `anchor` relative to `base` ([`Anchor::Inside`] uses the base's
/// content area, the other anchors its outer rectangle), shifted by `(x, y)` and by the node's own
/// translation. Unlike LVGL, which computes the position once, the relation is kept and
/// re-evaluated on every layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AlignTo<I> {
    /// The reference node.
    pub base: I,
    /// Where the node is placed relative to `base`.
    pub anchor: Anchor,
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

    /// An integer property of `id` in pixels (paddings, margins, gaps, grid cell positions…).
    /// The default implementation reads [`style_prop`](Self::style_prop) and returns
    /// integers and pixel lengths ([`StyleValue::as_px`]), 0 for anything else.
    fn style_i32(&self, id: Self::Id, prop: PropId) -> i32 {
        self.style_prop(id, prop).as_px().unwrap_or(0)
    }

    /// The resolved value of `prop` for the `Main` part of `id`.
    fn style_prop(&self, id: Self::Id, prop: PropId) -> StyleValue;

    /// The size of the widget's own content (text, image…) without padding: LVGL's "self
    /// size". Containers without own content return [`Size::ZERO`](Size).
    ///
    /// It may depend on the node's styles and content and on the sizes of the node's own and
    /// its parent's content areas (e.g. a label wrapping at its width), not on other nodes'
    /// rectangles: one layout call remembers each content-sized node's measurement and
    /// measures again only after that node or its parent was placed (which keeps measuring
    /// nested content-sized nodes linear in time).
    fn content_size(&self, id: Self::Id) -> Size;

    /// A number identifying `id` among the tree's live nodes (e.g. its arena index), small
    /// and dense: the index of the layout call's measurement memo (8 bytes per slot up to the
    /// largest one measured, kept by [`LayoutScratch`](crate::LayoutScratch)). Two live nodes
    /// never share a slot; a slot may be reused once its node is removed.
    fn slot(&self, id: Self::Id) -> u32;

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

    /// The grid column ([`Axis::X`]) or row ([`Axis::Y`]) template of `id`. The default
    /// implementation reads the `GridColumnTracks`/`GridRowTracks` style properties and knows
    /// only `'static` templates ([`TracksRef::Static`](twine_style::TracksRef::Static)): a
    /// resolved value carries only the identity of a shared template, so a tree with shared
    /// templates (the engine) borrows them from the style that holds them here, typically
    /// with [`twine_style::resolve_grid_tracks`] (the cascade lookup that returns the
    /// template's slice for any [`TracksRef`](twine_style::TracksRef)).
    fn grid_tracks(&self, id: Self::Id, axis: Axis) -> Option<&[GridTrack]> {
        let prop = match axis {
            Axis::X => PropId::GridColumnTracks,
            Axis::Y => PropId::GridRowTracks,
        };
        self.style_prop(id, prop)
            .as_grid_tracks()
            .and_then(twine_style::TracksRef::as_static)
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
    // Style values are any `i32`: the sums saturate (as every coordinate sum of the layout).
    Insets::new(
        t.style_i32(id, PropId::PaddingLeft)
            .saturating_add(b(BorderSide::LEFT)),
        t.style_i32(id, PropId::PaddingTop)
            .saturating_add(b(BorderSide::TOP)),
        t.style_i32(id, PropId::PaddingRight)
            .saturating_add(b(BorderSide::RIGHT)),
        t.style_i32(id, PropId::PaddingBottom)
            .saturating_add(b(BorderSide::BOTTOM)),
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

/// Start + end inset along `axis` (saturating).
pub(crate) const fn sum(i: Insets, axis: Axis) -> i32 {
    start(i, axis).saturating_add(end(i, axis))
}

/// Sets the coordinates of `id` only if they change, then forgets the measurements that may
/// depend on them: those of `id` and of its children (see [`LayoutTree::content_size`]).
pub(crate) fn place<T: LayoutTree + ?Sized>(t: &mut T, s: &mut LayoutScratch<T::Id>, id: T::Id, r: Rect) {
    if t.coords(id) != r {
        t.set_coords(id, r);
        if !s.memo.is_empty() {
            let t = &*t;
            let memo = &mut s.memo;
            memo.forget(t.slot(id));
            t.children(id, &mut |c| memo.forget(t.slot(c)));
        }
    }
}

/// The content rectangle of a node's outer rectangle.
pub(crate) const fn content_rect(r: Rect, sp: Insets) -> Rect {
    Rect::new(
        r.x0.saturating_add(sp.left),
        r.y0.saturating_add(sp.top),
        r.x1.saturating_sub(sp.right),
        r.y1.saturating_sub(sp.bottom),
    )
}
