//! [`VectorScene`]: a list of paths with their descriptors, drawn together (LVGL
//! `lv_vector_dsc` + `lv_draw_vector`).

use alloc::vec::Vec;

use twine_core::{Rect, Transform};
use twine_render::Painter;

use crate::draw::{DrawParams, PainterVectorExt, VectorDsc, union_rect};
use crate::path::Path;

// NOTE(P20.S09): the `Canvas` widget's `with_painter` draws scenes and single paths into its
// buffer through `PainterVectorExt` (`p.vector(path, dsc)`), like `VectorView` does on screen.

/// Paths with their descriptors, drawn in order.
///
/// [`clear`](Self::clear) keeps the item storage, and [`path_mut`](Self::path_mut) gives a
/// path back for rebuilding, so a scene rebuilt every frame reuses its allocations.
///
/// ```
/// use twine_core::{Color, Fx};
/// use twine_vector::{FxPoint, Path, VectorDsc, VectorScene};
///
/// let mut scene = VectorScene::new();
/// let mut p = Path::new();
/// p.circle(FxPoint::from_int(10, 10), Fx::from_int(5));
/// scene.add(p, VectorDsc::fill(Color::RED));
/// assert_eq!(scene.len(), 1);
/// assert_eq!(scene.bounds(), Some(twine_core::Rect::new(4, 4, 16, 16)));
/// ```
#[derive(Clone, Debug, Default)]
pub struct VectorScene {
    items: Vec<(Path, VectorDsc)>,
    /// Number of live items (`items[len..]` are spare paths kept for reuse; `len ≤
    /// items.len()`).
    len: usize,
}

impl PartialEq for VectorScene {
    /// Scenes are equal when their live items are equal (spare storage is ignored).
    fn eq(&self, o: &Self) -> bool {
        self.live() == o.live()
    }
}

impl Eq for VectorScene {}

impl VectorScene {
    /// An empty scene.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            len: 0,
        }
    }

    /// The live items (`len ≤ items.len()`, so all of them).
    fn live(&self) -> &[(Path, VectorDsc)] {
        // SAFE-INDEX: `len ≤ items.len()` (every method keeps it); the fallback is never
        // taken.
        self.items.get(..self.len).unwrap_or(&self.items)
    }

    /// Appends `path` drawn with `dsc`.
    pub fn add(&mut self, path: Path, dsc: VectorDsc) {
        match self.items.get_mut(self.len) {
            Some(slot) => *slot = (path, dsc),
            None => self.items.push((path, dsc)),
        }
        // SAFE-ARITH: `len < items.len() ≤ isize::MAX` after the line above.
        self.len = self.len.wrapping_add(1);
    }

    /// Appends an item and returns its (cleared, capacity-keeping) path and descriptor for
    /// filling in place: the allocation-free way to rebuild a scene. A spare item left by
    /// [`clear`](Self::clear) is reused; otherwise the storage grows by one item.
    ///
    /// Returns `None` (and appends nothing) when the storage cannot grow: the allocation of a
    /// new item failed. Never panics.
    ///
    /// ```
    /// use twine_core::{Color, Fx};
    /// use twine_vector::{FxPoint, VectorDsc, VectorScene};
    ///
    /// let mut scene = VectorScene::new();
    /// let Some((path, dsc)) = scene.path_mut() else {
    ///     return; // out of memory: the scene keeps its items, nothing is appended
    /// };
    /// path.circle(FxPoint::from_int(10, 10), Fx::from_int(5));
    /// *dsc = VectorDsc::fill(Color::RED);
    /// assert_eq!(scene.len(), 1);
    /// // Rebuilding reuses the item: no allocation.
    /// scene.clear();
    /// assert!(scene.path_mut().is_some_and(|(path, _)| path.is_empty()));
    /// ```
    pub fn path_mut(&mut self) -> Option<(&mut Path, &mut VectorDsc)> {
        if self.len == self.items.len() {
            self.items.try_reserve(1).ok()?;
            self.items.push((Path::new(), VectorDsc::default()));
        }
        let i = self.len;
        let (p, d) = self.items.get_mut(i)?;
        // SAFE-ARITH: item `i` exists, so `len < items.len() ≤ isize::MAX`.
        self.len = i.wrapping_add(1);
        p.clear();
        *d = VectorDsc::default();
        Some((p, d))
    }

    /// Removes every item (the storage is kept for reuse).
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Number of items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the scene has no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The items.
    pub fn items(&self) -> impl Iterator<Item = (&Path, &VectorDsc)> + '_ {
        self.live().iter().map(|(p, d)| (p, d))
    }

    /// Screen bounds of all items (`None` when nothing is drawn).
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds_with(&Transform::IDENTITY)
    }

    /// Screen bounds when drawn with [`draw`](Self::draw)`(…, t)`.
    #[must_use]
    pub fn bounds_with(&self, t: &Transform) -> Option<Rect> {
        let mut b = None;
        for (p, d) in self.items() {
            if d.is_visible() {
                let d = VectorDsc {
                    transform: d.transform.then(*t),
                    ..d.clone()
                };
                b = union_rect(b, d.bounds(p));
            }
        }
        b
    }

    /// Draws every item, with `t` applied after each item's own transform. Items outside the
    /// painter's clip are skipped cheaply; nothing is allocated once the painter's
    /// [`VectorCaches`](crate::VectorCaches) have grown.
    pub fn draw(&self, p: &mut Painter<'_>, t: &Transform) {
        self.draw_with(p, &DrawParams::transform(*t));
    }

    /// Draws every item with scene-wide `params` (transform, opacity, recolor).
    pub fn draw_with(&self, p: &mut Painter<'_>, params: &DrawParams) {
        for (path, dsc) in self.items() {
            p.vector_params(path, dsc, params);
        }
    }
}
