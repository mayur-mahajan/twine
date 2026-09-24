//! Entry points ([`layout_subtree`], [`layout_children`]) and the per-node arrangement that
//! dispatches to absolute positioning, flex and grid.

use alloc::vec::Vec;

use twine_core::log::warn;
use twine_core::{Insets, Rect, Size};
use twine_style::{LayoutKind, PropId};

use crate::flex::Track;
use crate::position::{abs_rect, align_to_rect};
use crate::size::{Resolved, raw_size, resolve_node};
use crate::tree::{Axis, LayoutFlags, LayoutTree, content_rect, is_rtl, place, spaces, style_enum};
use crate::{flex, grid};

/// Reusable buffers of the layout algorithms. Keep one alive (e.g. in the widget tree) and
/// pass it to [`layout_subtree_with`] so that repeated layouts do not allocate once the
/// buffers have grown to the size of the largest container.
///
/// ```
/// use twine_layout::LayoutScratch;
///
/// let scratch: LayoutScratch<u16> = LayoutScratch::new();
/// # drop(scratch);
/// ```
#[derive(Debug)]
pub struct LayoutScratch<I> {
    pub(crate) items: Vec<Item<I>>,
    pub(crate) tracks: Vec<Track>,
    pub(crate) ints: Vec<i32>,
}

impl<I> LayoutScratch<I> {
    /// Empty buffers (no allocation until first use).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            tracks: Vec::new(),
            ints: Vec::new(),
        }
    }
}

impl<I> Default for LayoutScratch<I> {
    fn default() -> Self {
        Self::new()
    }
}

/// A child of the container being measured or arranged.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Item<I> {
    pub id: I,
    pub flags: LayoutFlags,
    /// Positioned by the parent's flex/grid layout.
    pub in_flow: bool,
    /// Positioned by [`LayoutTree::align_to`] (never set for layout items: flex and grid win).
    pub aligned: bool,
    /// Outer size without margins.
    pub size: Size,
    pub min: [i32; 2],
    pub max: [i32; 2],
    pub pct: [bool; 2],
    pub margin: Insets,
    /// Excluded from the parent's content size on an axis (percentage of a content-sized
    /// parent).
    pub ignore: [bool; 2],
    pub grow: u8,
    pub final_main: i32,
    pub clamped: bool,
    /// The size on an axis was set by the parent's layout (grow, stretch).
    pub ovr: [bool; 2],
    pub done: bool,
    /// Grid column, column span, row, row span.
    pub cell: [i32; 4],
}

impl<I: Copy> Item<I> {
    pub(crate) fn new(id: I) -> Self {
        Self {
            id,
            flags: LayoutFlags::empty(),
            in_flow: false,
            aligned: false,
            size: Size::new(0, 0),
            min: [i32::MIN; 2],
            max: [i32::MAX; 2],
            pct: [false; 2],
            margin: Insets::ZERO,
            ignore: [false; 2],
            grow: 0,
            final_main: 0,
            clamped: false,
            ovr: [false; 2],
            done: false,
            cell: [0, 1, 0, 1],
        }
    }

    /// Reads the flags and decides how the child is positioned in a `kind` container.
    pub(crate) fn classify<T: LayoutTree<Id = I> + ?Sized>(&mut self, t: &T, kind: LayoutKind) {
        self.flags = t.flags(self.id);
        self.in_flow = kind != LayoutKind::None
            && !self
                .flags
                .intersects(LayoutFlags::HIDDEN | LayoutFlags::IGNORE_LAYOUT | LayoutFlags::FLOATING);
        self.aligned = !self.in_flow && t.align_to(self.id).is_some();
    }

    /// A non-layout child that counts for the parent's content size.
    pub(crate) fn counts_for_content(&self) -> bool {
        !self.in_flow && !self.aligned && !self.flags.intersects(LayoutFlags::HIDDEN | LayoutFlags::FLOATING)
    }

    pub(crate) fn set_resolved(&mut self, r: Resolved) {
        self.size = r.size;
        self.min = r.min;
        self.max = r.max;
        self.pct = r.pct;
    }
}

/// The layout of `id`'s children; a grid without both templates falls back to `None`
/// (logged when `warn_missing`).
pub(crate) fn effective_kind<T: LayoutTree + ?Sized>(t: &T, id: T::Id, warn_missing: bool) -> LayoutKind {
    let kind = style_enum::<T, LayoutKind>(t, id, PropId::Layout);
    if kind == LayoutKind::Grid && grid::templates(t, id).is_none() {
        if warn_missing {
            warn!(
                target: "twine::layout",
                "grid layout without column and row templates; children placed by x/y/align"
            );
        }
        return LayoutKind::None;
    }
    kind
}

/// Lays out `root` and its whole subtree.
///
/// `parent_content` is the content area `root` is positioned in (its parent's content
/// rectangle, already shifted by the parent's scroll). The root is sized against it and placed
/// by its own `X`/`Y`/`Align` (or [`LayoutTree::align_to`]), with its own base direction
/// standing in for the parent's. Every descendant is then sized and positioned according to
/// its parent's layout ([`LayoutKind`]). [`LayoutTree::set_coords`] is called only for nodes
/// whose rectangle changes.
///
/// This allocates temporary buffers; use [`layout_subtree_with`] to reuse them.
///
/// ```
/// # #[cfg(feature = "toy")] {
/// use twine_core::Rect;
/// use twine_layout::{LayoutTree, ToyTree, layout_subtree};
/// use twine_style::{Align, StyleBuf};
///
/// let mut t = ToyTree::new(200, 100);
/// let child = t.add(ToyTree::ROOT, StyleBuf::new().width(40).height(20).align(Align::Center));
/// let screen = t.coords(ToyTree::ROOT);
/// layout_subtree(&mut t, ToyTree::ROOT, screen);
/// assert_eq!(t.coords(child), Rect::from_xywh(80, 40, 40, 20));
/// # }
/// ```
pub fn layout_subtree<T: LayoutTree + ?Sized>(t: &mut T, root: T::Id, parent_content: Rect) {
    let mut s = LayoutScratch::new();
    layout_subtree_with(t, &mut s, root, parent_content);
}

/// [`layout_subtree`] with caller-provided buffers.
pub fn layout_subtree_with<T: LayoutTree + ?Sized>(
    t: &mut T,
    s: &mut LayoutScratch<T::Id>,
    root: T::Id,
    parent_content: Rect,
) {
    let r = resolve_node(&*t, s, root, raw_size(parent_content), [None; 2]);
    let rect = match t.align_to(root) {
        Some(a) => align_to_rect(&*t, root, r.size, a),
        None => abs_rect(
            &*t,
            root,
            r.size,
            parent_content,
            parent_content,
            is_rtl(&*t, root),
            [false; 2],
        ),
    };
    place(t, root, rect);
    arrange(t, s, root, [false; 2]);
}

/// Lays out the descendants of `id` keeping its current rectangle (for nodes whose rectangle
/// is decided elsewhere, e.g. screens or items of a parent that is not being laid out).
pub fn layout_children<T: LayoutTree + ?Sized>(t: &mut T, id: T::Id) {
    let mut s = LayoutScratch::new();
    layout_children_with(t, &mut s, id);
}

/// [`layout_children`] with caller-provided buffers.
pub fn layout_children_with<T: LayoutTree + ?Sized>(t: &mut T, s: &mut LayoutScratch<T::Id>, id: T::Id) {
    arrange(t, s, id, [false; 2]);
}

/// Sizes and positions the children of `id` (whose rectangle is final) and recurses. `ovr`
/// tells on which axes the size of `id` was set by its parent's layout (then it is not
/// content-sized on that axis even if its style says so, as in LVGL).
fn arrange<T: LayoutTree + ?Sized>(t: &mut T, s: &mut LayoutScratch<T::Id>, id: T::Id, ovr: [bool; 2]) {
    let raw = content_rect(t.coords(id), spaces(&*t, id));
    let sc = t.scroll(id);
    let content = Rect::new(raw.x0 - sc.x, raw.y0 - sc.y, raw.x1 - sc.x, raw.y1 - sc.y);
    let csize = raw_size(raw);
    let sized = [
        !ovr[0] && t.is_content_sized(id, Axis::X),
        !ovr[1] && t.is_content_sized(id, Axis::Y),
    ];
    let rtl = is_rtl(&*t, id);
    let kind = effective_kind(&*t, id, true);

    let frame = s.items.len();
    {
        let items = &mut s.items;
        t.children(id, &mut |c| items.push(Item::new(c)));
    }
    let end = s.items.len();
    if end == frame {
        return;
    }
    for i in frame..end {
        Item::classify(&mut s.items[i], &*t, kind);
    }

    match kind {
        LayoutKind::Flex => flex::arrange(t, s, id, frame, end, content, sized, rtl),
        LayoutKind::Grid => grid::arrange(t, s, id, frame, end, content, sized, rtl),
        LayoutKind::None => {}
    }

    // Children not positioned by flex/grid: x/y/align.
    for i in frame..end {
        let it = s.items[i];
        if it.in_flow || it.aligned {
            continue;
        }
        let r = resolve_node(&*t, s, it.id, csize, [None; 2]);
        let rect = abs_rect(&*t, it.id, r.size, content, raw, rtl, sized);
        place(t, it.id, rect);
    }
    for i in frame..end {
        let it = s.items[i];
        if !it.aligned {
            arrange(t, s, it.id, it.ovr);
        }
    }

    // `align_to` children last, each after its base when the base is an aligned sibling.
    loop {
        let mut pending = false;
        let mut progress = false;
        for i in frame..end {
            let it = s.items[i];
            if !it.aligned || it.done {
                continue;
            }
            pending = true;
            let Some(a) = t.align_to(it.id) else { continue };
            let base_pending = (frame..end)
                .any(|j| j != i && s.items[j].id == a.base && s.items[j].aligned && !s.items[j].done);
            if base_pending {
                continue;
            }
            place_aligned(t, s, i, csize);
            progress = true;
        }
        if !pending {
            break;
        }
        if !progress {
            // A cycle of align_to relations: break it at the first pending child.
            if let Some(i) = (frame..end).find(|&i| s.items[i].aligned && !s.items[i].done) {
                place_aligned(t, s, i, csize);
            }
        }
    }
    s.items.truncate(frame);
}

/// Positions the `align_to` child at `s.items[i]` and lays out its subtree.
fn place_aligned<T: LayoutTree + ?Sized>(t: &mut T, s: &mut LayoutScratch<T::Id>, i: usize, csize: Size) {
    let c = s.items[i].id;
    s.items[i].done = true;
    let r = resolve_node(&*t, s, c, csize, [None; 2]);
    if let Some(a) = t.align_to(c) {
        let rect = align_to_rect(&*t, c, r.size, a);
        place(t, c, rect);
    }
    arrange(t, s, c, [false; 2]);
}
