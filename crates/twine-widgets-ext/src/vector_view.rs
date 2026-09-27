//! [`VectorView`]: a widget drawing a [`VectorScene`] (paths with fills, strokes and
//! gradients; LVGL `lv_draw_vector` in a canvas), feature `vector`.

use alloc::boxed::Box;

use twine_core::{Fx, Point, Rect, Size, Transform};
use twine_engine::{
    DrawCx, Engine, EngineError, MeasureCx, NodeId, OBJ_FLAGS, Widget, WidgetClass, WidgetCx,
};
use twine_style::Part;
use twine_vector::VectorScene;

use crate::util::log_set;

/// The class of [`VectorView`].
pub static VECTOR_VIEW_CLASS: WidgetClass = WidgetClass::new("vector_view").default_flags(OBJ_FLAGS);

/// Draws a [`VectorScene`] in its own coordinates: scene point (0, 0) is the top-left corner
/// of the widget; drawing is clipped to the widget.
///
/// Changing the scene invalidates only the bounds of the old and the new scene, so an animated
/// path redraws its own area and nothing else. [`swap_scene`](Self::swap_scene) exchanges the
/// scene with a caller's buffer: a scene rebuilt every frame into the other buffer (with
/// [`VectorScene::path_mut`]) is shown without allocating.
///
/// ```
/// use twine_core::{Color, Fx};
/// use twine_testing::EngineHarness;
/// use twine_vector::{FxPoint, Path, VectorDsc, VectorScene};
/// use twine_widgets_ext::vector_view::{self, VectorView};
///
/// let mut h = EngineHarness::new(64, 64);
/// let screen = h.screen();
/// let v = vector_view::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().set_size(v, 40, 40);
/// let mut scene = VectorScene::new();
/// let mut p = Path::new();
/// p.circle(FxPoint::from_int(20, 20), Fx::from_int(15));
/// scene.add(p, VectorDsc::fill(Color::RED));
/// h.engine_mut().with_widget_mut(v, |w: &mut VectorView, cx| w.set_scene(cx, scene));
/// h.run_until_idle();
/// assert_eq!(h.pixel(20, 20), Color::RED);
/// ```
#[derive(Debug, Default)]
pub struct VectorView {
    scene: VectorScene,
    /// Bounds of `scene` in scene coordinates (cached for invalidation).
    bounds: Option<Rect>,
}

impl VectorView {
    /// An empty view.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scene: VectorScene::new(),
            bounds: None,
        }
    }

    /// The scene.
    #[must_use]
    pub fn scene(&self) -> &VectorScene {
        &self.scene
    }

    /// Bounds of the scene in scene coordinates (`None` when nothing is drawn).
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// Replaces the scene. Idempotent: an equal scene changes nothing; otherwise the old and
    /// the new bounds are invalidated.
    pub fn set_scene(&mut self, cx: &mut WidgetCx<'_>, mut scene: VectorScene) {
        self.swap_scene(cx, &mut scene);
    }

    /// Exchanges the scene with `scene` (which receives the old one, storage included).
    /// Idempotent like [`set_scene`](Self::set_scene): an equal scene is not swapped.
    pub fn swap_scene(&mut self, cx: &mut WidgetCx<'_>, scene: &mut VectorScene) {
        if *scene == self.scene {
            return;
        }
        log_set(VECTOR_VIEW_CLASS.name, cx.node(), "scene");
        let old = self.bounds;
        core::mem::swap(&mut self.scene, scene);
        self.bounds = self.scene.bounds();
        let origin = cx.coords();
        let clip = origin;
        for r in [old, self.bounds].into_iter().flatten() {
            let abs = Rect::new(
                r.x0 + origin.x0,
                r.y0 + origin.y0,
                r.x1 + origin.x0,
                r.y1 + origin.y0,
            );
            if let Some(a) = abs.intersection(&clip) {
                cx.invalidate_area(a);
            }
        }
        let extent = |b: Option<Rect>| b.map_or(Size::ZERO, |r| Size::new(r.x1.max(0), r.y1.max(0)));
        if extent(old) != extent(self.bounds) {
            cx.mark_layout();
        }
    }
}

impl Widget for VectorView {
    fn class(&self) -> &'static WidgetClass {
        &VECTOR_VIEW_CLASS
    }

    /// The extent of the scene from (0, 0).
    fn content_size(&self, _cx: &MeasureCx<'_>) -> Size {
        self.bounds
            .map_or(Size::ZERO, |r| Size::new(r.x1.max(0), r.y1.max(0)))
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        if self.scene.is_empty() {
            return;
        }
        let c = cx.coords();
        let t = Transform::translate(Fx::from_int(c.x0), Fx::from_int(c.y0));
        let _ = cx.with_clip(c, |cx| self.scene.draw(cx.painter(), &t));
    }
}

/// Creates an empty vector view as the last child of `parent`.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(VectorView::new()))
}

/// The scene origin of a vector view: its top-left corner (for hit testing in scene
/// coordinates).
#[must_use]
pub fn origin(engine: &Engine, id: NodeId) -> Point {
    let c = engine.coords(id);
    Point::new(c.x0, c.y0)
}
