//! [`ToyTree`]: a minimal [`LayoutTree`] for tests.

use alloc::vec::Vec;

use twine_core::{Point, Rect, Size};
use twine_style::{Length, PropId, StyleBuf, StyleProp, StyleValue};

use crate::tree::{AlignTo, LayoutFlags, LayoutTree};

/// One node of a [`ToyTree`].
#[derive(Clone, Debug, Default)]
struct ToyNode {
    parent: Option<usize>,
    children: Vec<usize>,
    style: StyleBuf,
    content: Size,
    coords: Rect,
    flags: LayoutFlags,
    align_to: Option<AlignTo<usize>>,
    scroll: Point,
}

/// A small in-memory [`LayoutTree`]: nodes in a `Vec` (ids are indices, the root is
/// [`ToyTree::ROOT`]), one [`StyleBuf`] per node with inheritance of inheritable properties,
/// a fixed widget content size per node, and a counter of [`LayoutTree::set_coords`] calls.
///
/// Never panics: an id that is not in the tree addresses a detached node outside the tree
/// (reads give its defaults, writes change nothing the layout sees).
///
/// ```
/// # #[cfg(feature = "toy")] {
/// use twine_core::{Rect, Size};
/// use twine_layout::{LayoutTree, ToyTree};
/// use twine_style::{Length, StyleBuf};
///
/// let mut t = ToyTree::new(320, 240);
/// let label = t.add(ToyTree::ROOT, StyleBuf::new().width(Length::pct(50)));
/// t.set_content_size(label, Size::new(0, 16));
/// t.layout();
/// assert_eq!(t.coords(label), Rect::from_xywh(0, 0, 160, 16));
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct ToyTree {
    nodes: Vec<ToyNode>,
    /// What an unknown id addresses.
    detached: ToyNode,
    set_coords_calls: usize,
}

impl ToyTree {
    /// Id of the root node.
    pub const ROOT: usize = 0;

    /// A tree with a `root_w × root_h` root at the origin (sized by `Width`/`Height` styles).
    #[must_use]
    pub fn new(root_w: i32, root_h: i32) -> Self {
        let root = ToyNode {
            parent: None,
            children: Vec::new(),
            style: StyleBuf::new().width(root_w).height(root_h),
            content: Size::new(0, 0),
            coords: Rect::from_xywh(0, 0, root_w, root_h),
            flags: LayoutFlags::empty(),
            align_to: None,
            scroll: Point::new(0, 0),
        };
        Self {
            nodes: alloc::vec![root],
            detached: ToyNode::default(),
            set_coords_calls: 0,
        }
    }

    /// The node `id` (the detached node for an unknown id).
    fn node(&self, id: usize) -> &ToyNode {
        self.nodes.get(id).unwrap_or(&self.detached)
    }

    /// The node `id`, mutably (the detached node for an unknown id).
    fn node_mut(&mut self, id: usize) -> &mut ToyNode {
        self.nodes.get_mut(id).unwrap_or(&mut self.detached)
    }

    /// Adds a child of `parent` (appended last) with the given style; returns its id.
    pub fn add(&mut self, parent: usize, style: StyleBuf) -> usize {
        let id = self.nodes.len();
        self.nodes.push(ToyNode {
            parent: Some(parent),
            children: Vec::new(),
            style,
            content: Size::new(0, 0),
            coords: Rect::ZERO,
            flags: LayoutFlags::empty(),
            align_to: None,
            scroll: Point::new(0, 0),
        });
        self.node_mut(parent).children.push(id);
        id
    }

    /// Sets the widget content size of `id` (what [`LayoutTree::content_size`] returns).
    pub fn set_content_size(&mut self, id: usize, size: Size) {
        self.node_mut(id).content = size;
    }

    /// Sets the layout flags of `id`.
    pub fn set_flags(&mut self, id: usize, flags: LayoutFlags) {
        self.node_mut(id).flags = flags;
    }

    /// Sets or clears the `align_to` relation of `id`.
    pub fn set_align_to(&mut self, id: usize, a: Option<AlignTo<usize>>) {
        self.node_mut(id).align_to = a;
    }

    /// Sets the scroll offset of `id`'s content.
    pub fn set_scroll(&mut self, id: usize, p: Point) {
        self.node_mut(id).scroll = p;
    }

    /// The style of `id`, for changes.
    pub fn style_mut(&mut self, id: usize) -> &mut StyleBuf {
        &mut self.node_mut(id).style
    }

    /// Sets one style property of `id`.
    pub fn set_style(&mut self, id: usize, p: StyleProp) {
        self.node_mut(id).style.set(p);
    }

    /// The children of `id`.
    #[must_use]
    pub fn children_of(&self, id: usize) -> &[usize] {
        &self.node(id).children
    }

    /// Number of nodes (including the root).
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always `false` (the root exists).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// How many times [`LayoutTree::set_coords`] was called.
    #[must_use]
    pub fn set_coords_calls(&self) -> usize {
        self.set_coords_calls
    }

    /// Resets the [`set_coords_calls`](Self::set_coords_calls) counter.
    pub fn reset_set_coords_calls(&mut self) {
        self.set_coords_calls = 0;
    }

    /// Lays out the whole tree, placing the root in a screen of its own `Width`/`Height`
    /// (`Px` values; other lengths count as 0) at the origin.
    pub fn layout(&mut self) {
        let px = |p| match self.node(Self::ROOT).style.get(p).and_then(StyleValue::as_length) {
            Some(Length::Px(v)) => v,
            _ => 0,
        };
        let screen = Rect::from_xywh(0, 0, px(PropId::Width), px(PropId::Height));
        crate::layout_subtree(self, Self::ROOT, screen);
    }
}

impl LayoutTree for ToyTree {
    type Id = usize;

    fn children(&self, id: usize, out: &mut dyn FnMut(usize)) {
        for &c in &self.node(id).children {
            out(c);
        }
    }

    fn style_prop(&self, id: usize, prop: PropId) -> StyleValue {
        let meta = prop.meta();
        let mut n = Some(id);
        while let Some(i) = n {
            if let Some(v) = self.node(i).style.get(prop) {
                return v;
            }
            if !meta.inherited {
                break;
            }
            n = self.node(i).parent;
        }
        meta.default
    }

    fn content_size(&self, id: usize) -> Size {
        self.node(id).content
    }

    fn flags(&self, id: usize) -> LayoutFlags {
        self.node(id).flags
    }

    fn set_coords(&mut self, id: usize, r: Rect) {
        self.set_coords_calls = self.set_coords_calls.saturating_add(1);
        self.node_mut(id).coords = r;
    }

    fn slot(&self, id: usize) -> u32 {
        u32::try_from(id).unwrap_or(u32::MAX)
    }

    fn coords(&self, id: usize) -> Rect {
        self.node(id).coords
    }

    fn align_to(&self, id: usize) -> Option<AlignTo<usize>> {
        self.node(id).align_to
    }

    fn scroll(&self, id: usize) -> Point {
        self.node(id).scroll
    }
}
