//! Bindings: reactive effects that write one property of one node.

use core::any::Any;

use twine_engine::{Engine, NodeId, Widget, WidgetCx, fmt_node_id};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect, with_ambient};

use crate::access::engine_ready;
use crate::build::BuildCx;
use crate::prop::Prop;

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
///
/// This function is instantiated once per binding closure type, so only the calls of `f` and
/// `apply` are generic: the engine checks, the liveness test, tracing and the effect's
/// defer / dispose bookkeeping live in the non-generic [`engine_ready`], [`binding_target`] and
/// [`finish_run`] (direct calls, no indirection), which keeps every binding a few dozen bytes
/// of code on firmware. A run is on the per-frame path (signal-driven animations), so it reads
/// the ambient slot once to check ([`twine_reactive::ambient_is`], no take / restore) and
/// once to borrow the engine, and finds the engine and the node in a single call.
pub(crate) fn bind_effect<T: 'static>(
    scope: Scope,
    node: NodeId,
    f: impl Fn() -> T + 'static,
    apply: impl Fn(&mut Engine, NodeId, T) + 'static,
) {
    scope.effect_with_cx(move |_ctx| {
        if !engine_ready() {
            return;
        }
        let v = f();
        let found = with_ambient(|a| match binding_target(a, node) {
            Target::Live(e) => {
                apply(e, node, v);
                Found::Applied
            }
            Target::Deleted => Found::Deleted,
            Target::NoEngine => Found::NoEngine,
        });
        finish_run(node, found);
    });
}

/// What a binding run finds in the ambient slot.
enum Target<'a> {
    /// No engine: none provided, or borrowed by an enclosing `EngineAccess::with`.
    NoEngine,
    /// The binding's node was deleted.
    Deleted,
    /// The engine; the node exists.
    Live(&'a mut Engine),
}

/// How a binding run ended ([`Target`] without the engine).
#[derive(Clone, Copy)]
enum Found {
    NoEngine,
    Deleted,
    Applied,
}

/// The engine of a binding run and whether its node still exists (one call for the downcast,
/// the liveness test and the trace).
#[inline(never)]
fn binding_target(a: Option<&mut dyn Any>, node: NodeId) -> Target<'_> {
    let Some(e) = a.and_then(|a| a.downcast_mut::<Engine>()) else {
        return Target::NoEngine;
    };
    if !e.tree().contains(node) {
        return Target::Deleted;
    }
    twine_core::trace!(target: "twine::view", "binding run node={}", fmt_node_id(node));
    Target::Live(e)
}

/// Ends a binding run: disposes the binding of a deleted node, defers a run that found no
/// engine.
#[inline(never)]
fn finish_run(node: NodeId, found: Found) {
    match found {
        Found::Applied => {}
        Found::Deleted => {
            twine_core::trace!(target: "twine::view", "binding of deleted node {} disposed", fmt_node_id(node));
            dispose_current_effect();
        }
        Found::NoEngine => defer_current_effect(),
    }
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
