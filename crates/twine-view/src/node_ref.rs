//! [`NodeRef`]: the imperative escape hatch to a built widget.

use core::marker::PhantomData;

use twine_engine::{NodeId, Widget, WidgetCx, fmt_node_id};
use twine_reactive::{Scope, Signal};

use crate::access::EngineAccess;

/// A reference to the node of a widget view, filled when the view is built
/// (`.node_ref(r)`). `Copy`, like a signal (it is one: reading it with [`get`](Self::get)
/// inside a binding subscribes to it).
///
/// [`with_mut`](Self::with_mut) calls the widget's own setters — LVGL-style, for hot paths —
/// from handlers, effects and timer callbacks run by the `Ui`.
///
/// ```
/// use twine_view::prelude::*;
/// use twine_widgets::label::Label;
///
/// fn app(cx: Scope) -> impl View {
///     let r: NodeRef<Label> = cx.node_ref();
///     column((
///         label("0").node_ref(r),
///         button(label("Poke")).on_click(move || {
///             r.with_mut(|l: &mut Label, cx| l.set_text(cx, "poked"));
///         }),
///     ))
/// }
/// # let _ = app;
/// ```
pub struct NodeRef<W: Widget> {
    cell: Signal<Option<NodeId>>,
    _w: PhantomData<fn() -> W>,
}

impl<W: Widget> Clone for NodeRef<W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W: Widget> Copy for NodeRef<W> {}

impl<W: Widget> core::fmt::Debug for NodeRef<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeRef")
            .field("widget", &core::any::type_name::<W>())
            .field("node", &self.cell.try_get().flatten().map(fmt_node_id))
            .finish()
    }
}

impl<W: Widget> NodeRef<W> {
    /// An empty reference owned by `cx` (usually created with `cx.node_ref()`).
    #[must_use]
    pub fn new(cx: Scope) -> Self {
        NodeRef {
            cell: cx.signal(None),
            _w: PhantomData,
        }
    }

    /// The node, once built (a tracked read).
    #[must_use]
    pub fn get(&self) -> Option<NodeId> {
        self.cell.try_get().flatten()
    }

    /// The node without subscribing the running binding.
    #[must_use]
    pub fn get_untracked(&self) -> Option<NodeId> {
        if self.cell.is_alive() {
            self.cell.get_untracked()
        } else {
            None
        }
    }

    /// Sets the node (done by the `.node_ref(r)` modifier).
    pub(crate) fn fill(&self, node: NodeId) {
        if self.cell.is_alive() {
            self.cell.set(Some(node));
        }
    }

    /// Calls `f` with the widget and a widget context for its node, inside handlers, effects
    /// and timer callbacks run by the `Ui` (the engine is taken from [`EngineAccess`]).
    /// Returns `None` and logs `warn!` when no engine is available (outside those scopes),
    /// when the reference is not filled yet or its node was deleted, or when the node holds
    /// another widget type.
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut W, &mut WidgetCx<'_>) -> R) -> Option<R> {
        let Some(node) = self.get_untracked() else {
            twine_core::warn!(target: "twine::view", "NodeRef::with_mut: reference not filled");
            return None;
        };
        if let Some(r) = EngineAccess::with(|e| e.with_widget_mut::<W, R>(node, f)) {
            r
        } else {
            twine_core::warn!(
                target: "twine::view",
                "NodeRef::with_mut on {} outside a Ui handler, effect or timer; ignored",
                fmt_node_id(node)
            );
            None
        }
    }
}
