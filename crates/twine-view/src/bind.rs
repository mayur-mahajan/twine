//! Bindings: reactive effects that write one property of one node.

use twine_engine::{Engine, NodeId, Widget, WidgetCx, fmt_node_id};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect};

use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::prop::Prop;

/// What a binding run found.
enum Run {
    /// Applied to the live node.
    Applied,
    /// The node was deleted.
    Dead,
}

/// Applies `prop` to `node` with `apply`: a constant once, now; a dynamic value through a
/// binding effect owned by the build scope.
///
/// The binding's first run happens at once (the engine is lent to it). Later runs happen in
/// `Ui`'s effect flush; a run without an engine (a signal written outside `Ui::update`) defers
/// itself to the next flush. A binding whose node was deleted disposes itself.
pub(crate) fn bind_node<T: 'static>(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    prop: Prop<T>,
    apply: impl Fn(&mut Engine, NodeId, T) + 'static,
) {
    match prop {
        Prop::Static(v) => apply(cx.engine(), node, v),
        Prop::Dynamic(f) => {
            let scope = cx.scope();
            cx.provide(|| bind_effect(scope, node, f, apply));
        }
    }
}

/// Creates the binding effect of [`bind_node`] (runs it once at once, with whatever engine is
/// available).
pub(crate) fn bind_effect<T: 'static>(
    scope: Scope,
    node: NodeId,
    f: impl Fn() -> T + 'static,
    apply: impl Fn(&mut Engine, NodeId, T) + 'static,
) {
    scope.effect_with_cx(move |_ctx| {
        if !EngineAccess::available() {
            // Not evaluated: the deferred effect runs (and re-subscribes) at the next flush.
            return defer_current_effect();
        }
        let v = f();
        let run = EngineAccess::with(|e| {
            if e.tree().contains(node) {
                twine_core::trace!(target: "twine::view", "binding run node={}", fmt_node_id(node));
                apply(e, node, v);
                Run::Applied
            } else {
                Run::Dead
            }
        });
        match run {
            Some(Run::Applied) => {}
            Some(Run::Dead) => {
                twine_core::trace!(target: "twine::view", "binding of deleted node {} disposed", fmt_node_id(node));
                dispose_current_effect();
            }
            None => defer_current_effect(),
        }
    });
}

/// [`bind_node`] through a widget setter of `W`.
pub(crate) fn bind_prop<W: Widget, T: 'static>(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    prop: Prop<T>,
    apply: impl Fn(&mut W, &mut WidgetCx<'_>, T) + 'static,
) {
    bind_node(cx, node, prop, move |e, n, v| {
        e.with_widget_mut::<W, _>(n, |w, wcx| apply(w, wcx, v));
    });
}
