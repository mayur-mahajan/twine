//! [`vector_canvas`]: vector graphics (paths, fills, strokes, gradients) redrawn from signals
//! (feature `vector`).

use alloc::rc::Rc;
use core::cell::RefCell;

use twine_engine::NodeId;
use twine_vector::VectorScene;
use twine_widgets_ext::vector_view::VectorView;

use crate::bind::bind_effect;
use crate::build::{BuildCx, BuildOp, WidgetView, widget_view};
use crate::modifiers::ViewExt;
use crate::view::View;

/// A canvas of vector graphics: `draw` fills a [`VectorScene`] (in the canvas's coordinates,
/// (0, 0) = top-left corner) and runs again whenever a signal it reads changes. Only the
/// bounds of the old and the new scene are redrawn, and nothing happens when the new scene
/// equals the old one.
///
/// The scene passed to `draw` is cleared but keeps its storage: build paths with
/// [`VectorScene::path_mut`] and an animated scene allocates nothing per frame.
///
/// Size the canvas with modifiers (default: the extent of the scene).
///
/// ```
/// use twine_core::{Color, Fx};
/// use twine_vector::{FxPoint, VectorDsc};
/// use twine_view::prelude::*;
///
/// fn gauge(cx: Scope) -> impl View {
///     let r = cx.signal(10);
///     vector_canvas(move |scene| {
///         let (path, dsc) = scene.path_mut();
///         path.circle(FxPoint::from_int(50, 50), Fx::from_int(r.get()));
///         *dsc = VectorDsc::fill(Color::BLUE);
///     })
///     .size(100, 100)
/// }
/// # let _ = gauge;
/// ```
pub fn vector_canvas(draw: impl Fn(&mut VectorScene) + 'static) -> VectorCanvas {
    VectorCanvas(
        widget_view(VectorView::new).op(move |cx: &mut BuildCx<'_>, node: NodeId| {
            let scope = cx.scope();
            // The scene being built; after a swap it holds the previous scene's storage.
            let scratch = Rc::new(RefCell::new(VectorScene::new()));
            cx.provide(|| {
                bind_effect(
                    scope,
                    node,
                    move || {
                        {
                            let mut s = scratch.borrow_mut();
                            s.clear();
                            draw(&mut s);
                        }
                        scratch.clone()
                    },
                    |e, n, s: Rc<RefCell<VectorScene>>| {
                        e.with_widget_mut(n, |v: &mut VectorView, wcx| {
                            v.swap_scene(wcx, &mut s.borrow_mut());
                        });
                    },
                );
            });
        }),
    )
}

/// The view of [`vector_canvas`].
#[derive(Debug)]
#[must_use]
pub struct VectorCanvas(WidgetView<VectorView>);

impl View for VectorCanvas {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for VectorCanvas {
    type Widget = VectorView;

    fn push_op(self, op: BuildOp) -> Self {
        VectorCanvas(self.0.push_op(op))
    }
}
