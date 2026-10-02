//! [`BuildCx`] and the generic widget builder [`WidgetView`].

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::Cell;

use twine_core::fault::FaultKind;
use twine_engine::{
    DEAD_NODE, DisplayId, Engine, EngineError, EventCode, EventFilter, EventResult, FaultRecord, NodeId,
    Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_reactive::{Runtime, Scope};

use crate::access::EngineAccess;
use crate::bind::bind_prop;
use crate::error::{BuildFailure, BuildFault, BuildReport};
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

    /// The reactive runtime the build runs in (the scope's: [`Scope::runtime`]), for
    /// [`EngineAccess`] and the runtime-wide operations of custom views. Free (zero-sized);
    /// never panics.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_view::{BuildCx, EngineAccess};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// let rt = twine_reactive::Runtime::take().unwrap();
    /// let scope = rt.create_root();
    /// let cx = BuildCx::new(&mut e, root, scope);
    /// // e.g. a custom view checking whether an enclosing `Ui` lends its engine:
    /// assert!(!EngineAccess::available(cx.runtime()));
    /// # scope.dispose();
    /// ```
    #[must_use]
    #[inline]
    pub fn runtime(&self) -> Runtime {
        self.scope.runtime()
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
    /// `Widget::init` runs) and returns its id.
    ///
    /// # Failure
    ///
    /// When the node cannot be created (the tree is full, or the parent was deleted) this
    /// returns [`DEAD_NODE`], never another node: every engine call given it is ignored, so
    /// later build steps and bindings cannot style or change the wrong node. The failure is
    /// logged (`warn!`, with the widget type) and raised as a
    /// [`FaultKind::BuildFailed`](twine_core::fault::FaultKind::BuildFailed) fault with the
    /// parent as its node and the [`BuildFailure`] as its code (telemetry), and recorded in the
    /// [`BuildReport`] of the mount in progress if the scope belongs to an application being
    /// mounted (also from nested builds, e.g. the first rows of a `for_each`); the runtime
    /// constructors then fail with a [`BuildError`](crate::BuildError). Creating under a dead
    /// parent returns [`DEAD_NODE`] at once, without a fault of its own (the parent's failure
    /// was reported).
    ///
    /// Custom views that do more than run [`WidgetView`] steps should stop early when the
    /// returned id [`is_dead`](Self::is_dead) (as [`WidgetView`] does), rather than
    /// registering handlers and effects for a node that does not exist.
    ///
    /// ```
    /// use twine_engine::{DEAD_NODE, Engine, EngineConfig, Obj};
    /// use twine_view::BuildCx;
    ///
    /// let mut e = Engine::new(EngineConfig { max_nodes: 1, ..EngineConfig::default() }).unwrap();
    /// let root = e.create_root(Box::new(Obj)).unwrap();
    /// let scope = twine_reactive::Runtime::take().unwrap().create_root();
    /// let mut cx = BuildCx::new(&mut e, root, scope);
    /// let n = cx.create(Obj); // the tree is full
    /// assert_eq!(n, DEAD_NODE);
    /// assert!(BuildCx::is_dead(n));
    /// # scope.dispose();
    /// ```
    pub fn create<W: Widget>(&mut self, w: W) -> NodeId {
        // The class names the widget in diagnostics: a static the widget links anyway
        // (`core::any::type_name` would add one string per widget type to the firmware).
        let class = w.class();
        if self.parent == DEAD_NODE {
            twine_core::trace!(target: "twine::view", "build {}: parent failed; skipped", class.name);
            return DEAD_NODE;
        }
        match self.engine.create(self.parent, Box::new(w)) {
            Ok(id) => {
                twine_core::trace!(target: "twine::view", "build {} -> {}", class.name, fmt_node_id(id));
                id
            }
            Err(e) => {
                self.fail(class, &e);
                DEAD_NODE
            }
        }
    }

    /// Reports a widget of `class` that could not be created (cold path, kept out of
    /// `create`).
    #[cold]
    #[inline(never)]
    fn fail(&mut self, class: &'static WidgetClass, e: &EngineError) {
        let cause = BuildFailure::of(e);
        twine_core::warn!(
            target: "twine::view",
            "build: cannot create {} under {}: {}",
            class.name,
            fmt_node_id(self.parent),
            cause
        );
        self.engine.raise_fault(
            FaultRecord::new(FaultKind::BuildFailed)
                .node(self.parent)
                .code(cause.code()),
        );
        if let Some(report) = self.scope.use_context::<MountReport>() {
            report.record(BuildFault::new(cause, Some(self.parent)));
        }
    }

    /// Whether `id` is [`DEAD_NODE`] (a widget whose creation failed; see
    /// [`create`](Self::create)).
    #[must_use]
    #[inline]
    pub fn is_dead(id: NodeId) -> bool {
        id == DEAD_NODE
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
        on_delete(self.scope.runtime(), self.engine, node, f);
    }

    /// Runs `f` with the engine lent to [`EngineAccess`] (for code that reaches the engine
    /// indirectly while a build borrows it, e.g. creating effects that apply at once).
    pub fn provide<R>(&mut self, f: impl FnOnce() -> R) -> R {
        EngineAccess::provide(self.scope, self.engine, f)
    }
}

/// Registers `f` to run once when `node` is deleted (see [`BuildCx::on_delete`]).
pub(crate) fn on_delete(rt: Runtime, engine: &mut Engine, node: NodeId, f: impl FnOnce() + 'static) {
    let mut f = Some(f);
    engine.add_event_handler(node, EventFilter::Code(EventCode::Delete), move |cx, ev| {
        if ev.target == ev.current_target {
            if let Some(f) = f.take() {
                EngineAccess::provide(rt, cx.engine_mut(), f);
            }
        }
        EventResult::Continue
    });
}

/// The [`BuildReport`] of a mount in progress, provided on the application's root scope so
/// that every [`BuildCx`] under it — also those of nested builds run by effects during the
/// mount — records its failures there ([`BuildCx::create`]). Only the mount reads it, and it
/// stops recording when the mount [finishes](Self::finish): later failures (a `when` branch
/// built into a full tree) are faults only.
#[derive(Clone)]
pub(crate) struct MountReport(Rc<Cell<Option<BuildReport>>>);

impl MountReport {
    /// Starts recording the build of the application rooted at `root`.
    pub(crate) fn begin(root: Scope) -> MountReport {
        let report = MountReport(Rc::new(Cell::new(Some(BuildReport::new()))));
        root.provide(report.clone());
        report
    }

    /// Records one failure (nothing once the mount finished).
    pub(crate) fn record(&self, fault: BuildFault) {
        if let Some(mut r) = self.0.get() {
            r.record(fault);
            self.0.set(Some(r));
        }
    }

    /// Stops recording and returns the report.
    pub(crate) fn finish(&self) -> BuildReport {
        self.0.take().unwrap_or_default()
    }
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
/// let v = widget_view(|| Label::new("hi")).bind(1, |_l: &mut Label, _cx, _v: u8| {});
/// # let _ = v;
/// ```
pub struct WidgetView<W: Widget> {
    ctor: Box<dyn FnOnce() -> W>,
    ops: Vec<BuildOp>,
    children: Option<ChildrenFn>,
    post: Vec<BuildOp>,
    /// Settings shared by a widget view's builder methods and its build steps (see
    /// [`WidgetView::shared`]).
    shared: Option<Rc<dyn Any>>,
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
        shared: None,
    }
}

impl<W: Widget> WidgetView<W> {
    /// Adds a build step: `f(cx, node)` runs right after the node is created, after the
    /// steps added before, before the children are built. When the node cannot be created
    /// (see [`BuildCx::create`]) no step runs and no child is built.
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
    /// setter. Its value parameter fixes `T` (any [`IntoProp<T, _>`](IntoProp) is accepted, so
    /// `T` is not inferred from `prop`): call a widget setter with it, or annotate it
    /// (`|w: &mut MyWidget, cx, v: u8| …`).
    #[must_use]
    pub fn bind<T: 'static, M>(
        self,
        prop: impl IntoProp<T, M>,
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

    /// Like [`bind`](Self::bind), applied **late**: the value is first set (and its binding
    /// created) after every other build step and the children, in the order of the
    /// `bind_after_children` calls, wherever in the builder chain they were made.
    ///
    /// For a value whose setter depends on other settings: a bar's or a gauge's value is
    /// clamped into its range, so `bar(150).range(0..=200)` must apply the range first even
    /// though `bar(..)` comes first in the chain. The built-in value widgets (`bar`, `led`,
    /// `animimg`, …) use exactly this; views of custom widgets do the same. Later changes of a
    /// dynamic value run the binding like any other (the order only matters for the first
    /// application while building). Costs the same as [`bind`](Self::bind): one boxed build
    /// step, and one reactive binding for a dynamic value.
    ///
    /// ```
    /// use core::ops::RangeInclusive;
    /// use twine_view::prelude::*;
    /// use twine_widgets::bar::Bar;
    ///
    /// /// A bar view whose value is applied after its range.
    /// fn level(v: i32, r: RangeInclusive<i32>) -> WidgetView<Bar> {
    ///     widget_view(Bar::new)
    ///         .bind_after_children(v, |b: &mut Bar, cx, v| b.set_value(cx, v, false))
    ///         .bind(r, |b: &mut Bar, cx, r: RangeInclusive<i32>| {
    ///             b.set_range(cx, *r.start(), *r.end());
    ///         })
    /// }
    ///
    /// let mut t = twine_testing::TestUi::new(120, 60).mount(|_| level(150, 0..=200).test_id("b"));
    /// let id = t.find(twine_testing::by_id("b")).id();
    /// assert_eq!(t.engine().widget::<Bar>(id).unwrap().value(), 150); // not clamped to 100
    /// ```
    #[must_use]
    pub fn bind_after_children<T: 'static, M>(
        self,
        prop: impl IntoProp<T, M>,
        set: impl Fn(&mut W, &mut WidgetCx<'_>, T) + 'static,
    ) -> Self {
        let prop = prop.into_prop();
        self.after_children(move |cx, node| bind_prop::<W, T>(cx, node, prop, set))
    }

    /// The view's shared settings of type `T` (created with `T::default()` on first use).
    ///
    /// Builder methods write them and build steps read them when the view is built, after
    /// every builder method ran (e.g. `bar(v).animated(d)`: the value binding created by
    /// `bar` animates). A view has settings of one type; another type replaces them.
    pub(crate) fn shared<T: Default + 'static>(&mut self) -> Rc<T> {
        if let Some(t) = self.shared.clone().and_then(|a| a.downcast::<T>().ok()) {
            return t;
        }
        let t = Rc::new(T::default());
        self.shared = Some(t.clone());
        t
    }

    pub(crate) fn push_op(mut self, op: BuildOp) -> Self {
        self.ops.push(op);
        self
    }
}

impl<W: Widget> View for WidgetView<W> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let id = cx.create((self.ctor)());
        if id == DEAD_NODE {
            // Nothing to style, bind or put children under: skip the steps and children (a
            // failed widget's subtree is never built into its parent).
            return id;
        }
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
