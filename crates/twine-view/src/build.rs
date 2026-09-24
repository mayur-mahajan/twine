//! [`BuildCx`] and the generic widget builder [`WidgetView`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_engine::{
    DisplayId, Engine, EventCode, EventFilter, EventResult, NodeId, Widget, WidgetCx, fmt_node_id,
};
use twine_reactive::Scope;

use crate::access::EngineAccess;
use crate::bind::bind_prop;
use crate::prop::IntoProp;
use crate::view::{View, ViewSeq};

/// The context views are built with: the engine, the parent new nodes go under, the reactive
/// scope that owns the bindings created meanwhile, and the display.
pub struct BuildCx<'a> {
    engine: &'a mut Engine,
    parent: NodeId,
    scope: Scope,
    display: Option<DisplayId>,
}

impl core::fmt::Debug for BuildCx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuildCx")
            .field("parent", &fmt_node_id(self.parent))
            .field("scope", &self.scope)
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

impl<'a> BuildCx<'a> {
    /// A context building under `parent` with bindings owned by `scope`.
    ///
    /// The display is `parent`'s (the engine's default display for detached parents).
    pub fn new(engine: &'a mut Engine, parent: NodeId, scope: Scope) -> Self {
        let display = engine.display_of(parent).or_else(|| engine.default_display());
        Self {
            engine,
            parent,
            scope,
            display,
        }
    }

    /// The node new nodes are created under.
    #[must_use]
    pub fn parent(&self) -> NodeId {
        self.parent
    }

    /// The scope owning the bindings and handlers created while building.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The display the nodes are built for (`None` for detached trees on an engine without
    /// displays).
    #[must_use]
    pub fn display(&self) -> Option<DisplayId> {
        self.display
    }

    /// The engine.
    pub fn engine(&mut self) -> &mut Engine {
        self.engine
    }

    /// Creates `w` as the last child of [`parent`](Self::parent) (the theme styles it, then
    /// `Widget::init` runs). If the parent no longer exists a detached root is created instead
    /// (logged), so building can go on.
    pub fn create<W: Widget>(&mut self, w: W) -> NodeId {
        let id = if let Ok(id) = self.engine.create(self.parent, Box::new(w)) {
            id
        } else {
            twine_core::warn!(
                target: "twine::view",
                "build: parent {} is gone; building detached",
                fmt_node_id(self.parent)
            );
            // Root creation only fails when the arena is full; `create` logged the cause.
            self.engine
                .create_root(Box::new(twine_engine::Obj))
                .unwrap_or(self.parent)
        };
        twine_core::trace!(target: "twine::view", "build {} -> {}", core::any::type_name::<W>(), fmt_node_id(id));
        id
    }

    /// Runs `f` with `parent` as the parent of new nodes.
    pub fn with_parent<R>(&mut self, parent: NodeId, f: impl FnOnce(&mut BuildCx<'_>) -> R) -> R {
        let old = core::mem::replace(&mut self.parent, parent);
        let r = f(self);
        self.parent = old;
        r
    }

    /// Runs `f` with `scope` owning the bindings created meanwhile.
    pub fn with_scope<R>(&mut self, scope: Scope, f: impl FnOnce(&mut BuildCx<'_>) -> R) -> R {
        let old = core::mem::replace(&mut self.scope, scope);
        let r = f(self);
        self.scope = old;
        r
    }

    /// Builds `v` under the current parent and returns its root node.
    pub fn build(&mut self, v: impl View) -> NodeId {
        v.build(self)
    }

    /// Builds every view of `seq` under the current parent.
    pub fn build_seq(&mut self, seq: impl ViewSeq) {
        seq.build_seq(self);
    }

    /// Runs `f` once when `node` is deleted (an engine `Delete` handler). The engine is
    /// available through [`EngineAccess`] while `f` runs (e.g. for cleanups that remove
    /// timers).
    pub fn on_delete(&mut self, node: NodeId, f: impl FnOnce() + 'static) {
        on_delete(self.engine, node, f);
    }

    /// Runs `f` with the engine lent to [`EngineAccess`] (for code that reaches the engine
    /// indirectly while a build borrows it, e.g. creating effects that apply at once).
    pub fn provide<R>(&mut self, f: impl FnOnce() -> R) -> R {
        EngineAccess::provide(self.engine, f)
    }
}

/// Registers `f` to run once when `node` is deleted (see [`BuildCx::on_delete`]).
pub(crate) fn on_delete(engine: &mut Engine, node: NodeId, f: impl FnOnce() + 'static) {
    let mut f = Some(f);
    engine.add_event_handler(node, EventFilter::Code(EventCode::Delete), move |cx, ev| {
        if ev.target == ev.current_target {
            if let Some(f) = f.take() {
                EngineAccess::provide(cx.engine_mut(), f);
            }
        }
        EventResult::Continue
    });
}

/// A build step run on a freshly created node (see [`WidgetView::op`]).
pub type BuildOp = Box<dyn FnOnce(&mut BuildCx<'_>, NodeId)>;

/// The generic widget builder every widget view wraps: a constructor, build steps
/// (modifiers) run in order right after the node is created, and children built last.
///
/// Building allocates a few boxed closures, once; that is the price of a declarative
/// description and does not recur (views are built once).
///
/// ```
/// use twine_view::prelude::*;
/// use twine_widgets::label::Label;
///
/// let v = widget_view(|| Label::new("hi")).bind(1u8, |_l: &mut Label, _cx, _v| {});
/// # let _ = v;
/// ```
pub struct WidgetView<W: Widget> {
    ctor: Box<dyn FnOnce() -> W>,
    ops: Vec<BuildOp>,
    children: Option<ChildrenFn>,
    post: Vec<BuildOp>,
}

/// Builds the children of a [`WidgetView`].
type ChildrenFn = Box<dyn FnOnce(&mut BuildCx<'_>)>;

impl<W: Widget> core::fmt::Debug for WidgetView<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WidgetView")
            .field("widget", &core::any::type_name::<W>())
            .field("ops", &self.ops.len())
            .field("children", &self.children.is_some())
            .field("post", &self.post.len())
            .finish_non_exhaustive()
    }
}

/// A [`WidgetView`] creating the widget returned by `ctor`.
pub fn widget_view<W: Widget>(ctor: impl FnOnce() -> W + 'static) -> WidgetView<W> {
    WidgetView {
        ctor: Box::new(ctor),
        ops: Vec::new(),
        children: None,
        post: Vec::new(),
    }
}

impl<W: Widget> WidgetView<W> {
    /// Adds a build step: `f(cx, node)` runs right after the node is created, after the
    /// steps added before, before the children are built.
    #[must_use]
    pub fn op(mut self, f: impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static) -> Self {
        self.ops.push(Box::new(f));
        self
    }

    /// Sets the children (replacing earlier ones).
    #[must_use]
    pub fn children(mut self, seq: impl ViewSeq) -> Self {
        self.children = Some(Box::new(move |cx: &mut BuildCx<'_>| seq.build_seq(cx)));
        self
    }

    /// Wires any property value to a widget setter: constants are applied once, signals,
    /// memos and closures through a binding that calls `set` whenever they change.
    ///
    /// `set` should be idempotent (do nothing for an unchanged value), like every widget
    /// setter.
    #[must_use]
    pub fn bind<T: 'static>(
        self,
        prop: impl IntoProp<T>,
        set: impl Fn(&mut W, &mut WidgetCx<'_>, T) + 'static,
    ) -> Self {
        let prop = prop.into_prop();
        self.op(move |cx, node| bind_prop::<W, T>(cx, node, prop, set))
    }

    /// Adds a build step run after the children are built (e.g. to arrange them).
    #[must_use]
    pub fn after_children(mut self, f: impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static) -> Self {
        self.post.push(Box::new(f));
        self
    }

    pub(crate) fn push_op(mut self, op: BuildOp) -> Self {
        self.ops.push(op);
        self
    }
}

impl<W: Widget> View for WidgetView<W> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let id = cx.create((self.ctor)());
        for op in self.ops {
            op(cx, id);
        }
        if let Some(children) = self.children {
            cx.with_parent(id, children);
        }
        for op in self.post {
            op(cx, id);
        }
        id
    }
}

impl<W: Widget> crate::modifiers::ViewExt for WidgetView<W> {
    type Widget = W;

    fn push_op(self, op: BuildOp) -> Self {
        WidgetView::push_op(self, op)
    }
}
