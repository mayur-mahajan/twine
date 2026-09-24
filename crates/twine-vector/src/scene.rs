//! [`VectorScene`]: a list of paths with their descriptors, drawn together (LVGL
//! `lv_vector_dsc` + `lv_draw_vector`).

use alloc::vec::Vec;

use twine_core::{Rect, Transform};
use twine_render::Painter;

use crate::draw::{PainterVectorExt, VectorDsc, union_rect};
use crate::path::Path;

// NOTE(P23.S06): the `VectorView` widget (twine-widgets-ext) and the `vector_canvas` view draw a
// `VectorScene` and invalidate `bounds_with` of the old and new scene.

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
    /// Number of live items (`items[len..]` are spare paths kept for reuse).
    len: usize,
}

impl PartialEq for VectorScene {
    /// Scenes are equal when their live items are equal (spare storage is ignored).
    fn eq(&self, o: &Self) -> bool {
        self.items[..self.len] == o.items[..o.len]
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

    /// Appends `path` drawn with `dsc`.
    pub fn add(&mut self, path: Path, dsc: VectorDsc) {
        if self.len < self.items.len() {
            self.items[self.len] = (path, dsc);
        } else {
            self.items.push((path, dsc));
        }
        self.len += 1;
    }

    /// Appends an item and returns its (cleared, capacity-keeping) path and descriptor for
    /// filling in place: the allocation-free way to rebuild a scene.
    pub fn path_mut(&mut self) -> (&mut Path, &mut VectorDsc) {
        if self.len == self.items.len() {
            self.items.push((Path::new(), VectorDsc::default()));
        }
        let (p, d) = &mut self.items[self.len];
        p.clear();
        *d = VectorDsc::default();
        self.len += 1;
        (p, d)
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
        self.items[..self.len].iter().map(|(p, d)| (p, d))
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
    /// painter's clip are skipped cheaply.
    pub fn draw(&self, p: &mut Painter<'_>, t: &Transform) {
        for (path, dsc) in self.items() {
            if t.is_identity() {
                p.vector(path, dsc);
            } else {
                let d = VectorDsc {
                    transform: dsc.transform.then(*t),
                    ..dsc.clone()
                };
                p.vector(path, &d);
            }
        }
    }
}
