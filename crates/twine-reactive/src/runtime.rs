//! The reactive graph: node and scope storage and the push-pull coloring algorithm.
//!
//! Every method takes `&Runtime` and borrows [`Inner`] only for short, user-code-free sections.
//! **Rule R1**: no borrow of `inner` is held while a user closure (memo, effect, cleanup,
//! channel handler, `Clone`/`Drop`/`PartialEq` of user values) runs. Closures and values are
//! stored behind `Rc` so they can be cloned out of the arena, the borrow dropped, and then
//! called; removed nodes are collected and dropped after the borrow is released.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::{Any, TypeId};
use core::cell::RefCell;

use twine_core::log::{debug, error, trace};
use twine_core::{Arena, Id, SmallVec};

use crate::channel::ChannelReg;
use crate::key_list::KeyList;

/// Key of a reactive node (signal, memo or effect).
pub(crate) type NodeKey = Id<ReactiveNode>;
/// Key of a scope.
pub(crate) type ScopeKey = Id<ScopeData>;

/// Recursion depth of `update_if_necessary` above which sources are no longer checked
/// (the node is conservatively recomputed instead). See the crate docs, "Deep chains".
pub(crate) const MAX_CHECK_DEPTH: u32 = 256;

/// Default for [`Inner::flush_iterations_limit`].
pub(crate) const DEFAULT_FLUSH_LIMIT: u32 = 100;

/// Coloring state of a node (signals are always `Clean`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NodeState {
    /// Up to date.
    Clean,
    /// A transitive source changed; the direct sources must be checked first.
    Check,
    /// A direct source changed; must recompute.
    Dirty,
}

/// Memo body: computes, compares with and stores into the value cell; returns `changed`.
pub(crate) type MemoFn = Rc<dyn Fn(&RefCell<dyn Any>) -> bool>;
/// Effect body, called with the flush context.
pub(crate) type EffectFn = Rc<RefCell<dyn FnMut(&mut dyn Any)>>;
/// A type-erased value cell (`T` for signals, `Option<T>` for memos).
pub(crate) type ValueRc = Rc<RefCell<dyn Any>>;

/// What a computation node runs.
pub(crate) enum Kind {
    /// A memo.
    Memo(MemoFn),
    /// An effect.
    Effect(EffectFn),
}

/// The computation part of a memo or effect (boxed so plain signals stay small).
pub(crate) struct Computation {
    pub(crate) kind: Kind,
    /// What the computation read during its last run, in read order.
    pub(crate) sources: SmallVec<NodeKey, 4>,
}

/// Node flag: the effect is in `pending`.
pub(crate) const QUEUED: u8 = 1;
/// Node flag: the effect is in `deferred` (waits for a real context).
pub(crate) const DEFERRED: u8 = 2;
/// Node flag: the computation is currently running.
pub(crate) const RUNNING: u8 = 4;

/// One signal, memo or effect.
pub(crate) struct ReactiveNode {
    /// Value cell of signals and memos; `None` for effects.
    pub(crate) value: Option<ValueRc>,
    /// Computations that read this node during their last run.
    pub(crate) subscribers: KeyList,
    /// `None` for signals.
    pub(crate) comp: Option<Box<Computation>>,
    /// Owning scope.
    pub(crate) scope: ScopeKey,
    pub(crate) state: NodeState,
    pub(crate) flags: u8,
}

impl ReactiveNode {
    fn is_effect(&self) -> bool {
        matches!(
            self.comp.as_deref(),
            Some(Computation {
                kind: Kind::Effect(_),
                ..
            })
        )
    }
}

/// A scope: owner of nodes, child scopes, cleanups and contexts.
#[derive(Default)]
pub(crate) struct ScopeData {
    pub(crate) parent: Option<ScopeKey>,
    pub(crate) children: Vec<ScopeKey>,
    pub(crate) nodes: Vec<NodeKey>,
    pub(crate) cleanups: Vec<Box<dyn FnOnce()>>,
    pub(crate) contexts: Vec<(TypeId, Rc<dyn Any>)>,
}

/// Counters exposed through [`crate::debug_stats`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counters {
    pub(crate) writes: u64,
    pub(crate) effect_runs: u64,
    pub(crate) memo_runs: u64,
    pub(crate) depth_guard_hits: u64,
    pub(crate) loop_cuts: u64,
}

/// All mutable runtime state.
pub(crate) struct Inner {
    pub(crate) nodes: Arena<ReactiveNode>,
    pub(crate) scopes: Arena<ScopeData>,
    /// The computation currently tracking reads.
    pub(crate) observer: Option<NodeKey>,
    /// The effect currently running (for `defer_current_effect`; unaffected by `untrack`).
    pub(crate) running_effect: Option<NodeKey>,
    pub(crate) batch_depth: u32,
    /// Effects to run, FIFO.
    pub(crate) pending: VecDeque<NodeKey>,
    pub(crate) running_flush: bool,
    pub(crate) flush_iterations_limit: u32,
    /// Effects waiting for a non-`()` flush context.
    pub(crate) deferred: Vec<NodeKey>,
    /// `on_message` registrations, sorted by `id`.
    pub(crate) channels: Vec<ChannelReg>,
    pub(crate) next_channel_id: u64,
    /// Work stack of `mark_stale` (kept to reuse its allocation).
    stack: Vec<(NodeKey, NodeState)>,
    /// Current `update_if_necessary` recursion depth.
    update_depth: u32,
    pub(crate) counters: Counters,
    pub(crate) created_logged: bool,
}

impl Inner {
    const fn new() -> Self {
        Inner {
            nodes: Arena::new(),
            scopes: Arena::new(),
            observer: None,
            running_effect: None,
            batch_depth: 0,
            pending: VecDeque::new(),
            running_flush: false,
            flush_iterations_limit: DEFAULT_FLUSH_LIMIT,
            deferred: Vec::new(),
            channels: Vec::new(),
            next_channel_id: 0,
            stack: Vec::new(),
            update_depth: 0,
            counters: Counters {
                writes: 0,
                effect_runs: 0,
                memo_runs: 0,
                depth_guard_hits: 0,
                loop_cuts: 0,
            },
            created_logged: false,
        }
    }

    /// Colors the subscribers of `key` after a write: direct subscribers `Dirty`, their
    /// descendants `Check`; effects reached are queued once.
    fn mark_stale(&mut self, key: NodeKey) {
        let Inner {
            nodes,
            stack,
            pending,
            ..
        } = self;
        stack.clear();
        if let Some(n) = nodes.get(key) {
            // Reverse so that subscribers are visited (and effects queued) in subscription order.
            stack.extend(
                n.subscribers
                    .as_slice()
                    .iter()
                    .rev()
                    .map(|&s| (s, NodeState::Dirty)),
            );
        }
        while let Some((k, st)) = stack.pop() {
            let Some(node) = nodes.get_mut(k) else {
                continue;
            };
            if node.state >= st {
                continue;
            }
            let was_clean = node.state == NodeState::Clean;
            node.state = st;
            if node.is_effect() {
                if node.flags & (QUEUED | DEFERRED) == 0 {
                    node.flags |= QUEUED;
                    pending.push_back(k);
                }
            } else if was_clean {
                stack.extend(
                    node.subscribers
                        .as_slice()
                        .iter()
                        .rev()
                        .map(|&s| (s, NodeState::Check)),
                );
            }
        }
    }

    /// Removes node `k` from the arena and from its neighbours' edge lists. The caller drops the
    /// returned node after releasing the borrow (R1).
    pub(crate) fn unlink_node(&mut self, k: NodeKey) -> Option<ReactiveNode> {
        let node = self.nodes.remove(k)?;
        if let Some(comp) = &node.comp {
            for &s in &comp.sources {
                if let Some(src) = self.nodes.get_mut(s) {
                    src.subscribers.remove(k);
                }
            }
        }
        for &sub in node.subscribers.as_slice() {
            if let Some(c) = self.nodes.get_mut(sub).and_then(|n| n.comp.as_deref_mut()) {
                c.sources.retain(|&x| x != k);
            }
        }
        Some(node)
    }

    /// Drops `pending`/`deferred` entries of removed nodes.
    pub(crate) fn prune_queues(&mut self) {
        let Inner {
            nodes,
            pending,
            deferred,
            ..
        } = self;
        pending.retain(|&k| nodes.contains(k));
        deferred.retain(|&k| nodes.contains(k));
    }
}

/// The reactive runtime of one thread.
pub(crate) struct Runtime {
    pub(crate) inner: RefCell<Inner>,
}

/// Restores `observer`/`running_effect` and clears `RUNNING` after a computation, also on
/// unwinding.
struct RunGuard<'a> {
    rt: &'a Runtime,
    key: NodeKey,
    prev_observer: Option<NodeKey>,
    prev_effect: Option<NodeKey>,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        let mut inner = self.rt.inner.borrow_mut();
        inner.observer = self.prev_observer;
        inner.running_effect = self.prev_effect;
        if let Some(n) = inner.nodes.get_mut(self.key) {
            n.flags &= !RUNNING;
        }
    }
}

/// Decrements `update_depth` on scope exit.
struct DepthGuard<'a>(&'a Runtime);

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        let mut inner = self.0.inner.borrow_mut();
        inner.update_depth = inner.update_depth.saturating_sub(1);
    }
}

/// Clears `running_flush` on scope exit.
struct FlushGuard<'a>(&'a Runtime);

impl Drop for FlushGuard<'_> {
    fn drop(&mut self) {
        self.0.inner.borrow_mut().running_flush = false;
    }
}

/// Panics for an arena that is full (65 535 live nodes or scopes).
#[cold]
fn capacity_panic(what: &str) -> ! {
    error!(target: "twine::reactive", "too many reactive {} (limit 65535)", what);
    panic!("twine-reactive: too many reactive {what} (limit 65535)")
}

impl Runtime {
    /// An empty runtime (usable in `static` and `const` thread-local initializers).
    pub(crate) const fn new() -> Self {
        Runtime {
            inner: RefCell::new(Inner::new()),
        }
    }

    /// Logs `"runtime created"` the first time the runtime is used.
    pub(crate) fn log_created(&self) {
        let mut inner = self.inner.borrow_mut();
        if !inner.created_logged {
            inner.created_logged = true;
            debug!(target: "twine::reactive", "runtime created");
        }
    }

    // ----- scopes -------------------------------------------------------------------------

    /// Creates a scope, optionally as a child of `parent`. Returns `None` if the parent is dead.
    pub(crate) fn create_scope(&self, parent: Option<ScopeKey>) -> Option<ScopeKey> {
        self.log_created();
        let mut inner = self.inner.borrow_mut();
        if let Some(p) = parent {
            if !inner.scopes.contains(p) {
                return None;
            }
        }
        let key = inner
            .scopes
            .insert(ScopeData {
                parent,
                ..ScopeData::default()
            })
            .unwrap_or_else(|_| capacity_panic("scopes"));
        if let Some(p) = parent.and_then(|p| inner.scopes.get_mut(p)) {
            p.children.push(key);
        }
        Some(key)
    }

    /// Whether scope `s` is alive.
    pub(crate) fn scope_alive(&self, s: ScopeKey) -> bool {
        self.inner.borrow().scopes.contains(s)
    }

    /// Disposes scope `s`. The caller wraps this in a batch.
    pub(crate) fn dispose_scope(&self, s: ScopeKey) {
        if !self.scope_alive(s) {
            debug!(target: "twine::reactive", "dispose scope {:?}: already disposed", s);
            return;
        }
        let mut children_disposed = 0usize;
        // Cleanups may register new children or cleanups on `s`; loop until both are empty.
        loop {
            let (children, cleanups) = {
                let mut inner = self.inner.borrow_mut();
                let Some(data) = inner.scopes.get_mut(s) else {
                    return;
                };
                (
                    core::mem::take(&mut data.children),
                    core::mem::take(&mut data.cleanups),
                )
            };
            if children.is_empty() && cleanups.is_empty() {
                break;
            }
            children_disposed += children.len();
            for &c in children.iter().rev() {
                self.dispose_scope(c);
            }
            for f in cleanups.into_iter().rev() {
                f();
            }
        }
        let (data, garbage) = {
            let mut inner = self.inner.borrow_mut();
            let Some(data) = inner.scopes.remove(s) else {
                return;
            };
            if let Some(p) = data.parent.and_then(|p| inner.scopes.get_mut(p)) {
                p.children.retain(|&c| c != s);
            }
            let mut garbage: Vec<ReactiveNode> = Vec::with_capacity(data.nodes.len());
            for &k in &data.nodes {
                if let Some(n) = inner.unlink_node(k) {
                    garbage.push(n);
                }
            }
            inner.prune_queues();
            let channels = crate::channel::take_scope_channels(&mut inner, s);
            drop(inner);
            (data, (garbage, channels))
        };
        debug!(
            target: "twine::reactive",
            "dispose scope {:?}: {} nodes, {} children",
            s,
            garbage.0.len(),
            children_disposed
        );
        // Drop user values and closures outside the borrow (R1): nodes, then contexts.
        drop(garbage);
        drop(data);
    }

    /// Registers `f` as a cleanup of `s`; returns it back if the scope is dead.
    pub(crate) fn add_cleanup(&self, s: ScopeKey, f: Box<dyn FnOnce()>) -> Result<(), Box<dyn FnOnce()>> {
        let mut inner = self.inner.borrow_mut();
        match inner.scopes.get_mut(s) {
            Some(d) => {
                d.cleanups.push(f);
                Ok(())
            }
            None => Err(f),
        }
    }

    // ----- nodes --------------------------------------------------------------------------

    /// Inserts a node owned by scope `s`. `None` if the scope is dead.
    pub(crate) fn create_node(
        &self,
        s: ScopeKey,
        value: Option<ValueRc>,
        comp: Option<Box<Computation>>,
    ) -> Option<NodeKey> {
        let mut inner = self.inner.borrow_mut();
        if !inner.scopes.contains(s) {
            return None;
        }
        let state = if comp.is_some() {
            NodeState::Dirty
        } else {
            NodeState::Clean
        };
        let key = inner
            .nodes
            .insert(ReactiveNode {
                value,
                subscribers: KeyList::new(),
                comp,
                scope: s,
                state,
                flags: 0,
            })
            .unwrap_or_else(|_| capacity_panic("nodes"));
        if let Some(d) = inner.scopes.get_mut(s) {
            d.nodes.push(key);
        }
        Some(key)
    }

    /// Whether node `k` is alive.
    pub(crate) fn node_alive(&self, k: NodeKey) -> bool {
        self.inner.borrow().nodes.contains(k)
    }

    /// Scope owning node `k`.
    pub(crate) fn node_scope(&self, k: NodeKey) -> Option<ScopeKey> {
        self.inner.borrow().nodes.get(k).map(|n| n.scope)
    }

    /// Disposes a single node (an effect via `EffectId::dispose`).
    pub(crate) fn dispose_node(&self, k: NodeKey) {
        let garbage = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.unlink_node(k) else {
                return;
            };
            if let Some(d) = inner.scopes.get_mut(node.scope) {
                d.nodes.retain(|&x| x != k);
            }
            inner.prune_queues();
            node
        };
        drop(garbage);
    }

    /// The value cell of `k`, registering a dependency of the current observer when `track`.
    /// `None` if the node is dead.
    pub(crate) fn read_node(&self, k: NodeKey, track: bool) -> Option<ValueRc> {
        if !track {
            return self.inner.borrow().nodes.get(k)?.value.clone();
        }
        let mut inner = self.inner.borrow_mut();
        let value = inner.nodes.get(k)?.value.clone();
        if let Some(o) = inner.observer {
            if o != k {
                let observer_alive = match inner.nodes.get_mut(o).and_then(|n| n.comp.as_deref_mut()) {
                    Some(c) => {
                        if !c.sources.contains(&k) {
                            c.sources.push(k);
                        }
                        true
                    }
                    None => false,
                };
                if observer_alive {
                    if let Some(n) = inner.nodes.get_mut(k) {
                        n.subscribers.insert(o);
                    }
                }
            }
        }
        value
    }

    /// Records a write to `k` and propagates it; flushes when outside batch and flush.
    pub(crate) fn notify(&self, k: NodeKey) {
        let flush = {
            let mut inner = self.inner.borrow_mut();
            inner.counters.writes += 1;
            inner.mark_stale(k);
            inner.batch_depth == 0 && !inner.running_flush && !inner.pending.is_empty()
        };
        if flush {
            self.flush(&mut ());
        }
    }

    // ----- computations -------------------------------------------------------------------

    /// Brings computation `k` up to date. Effects that need to run are run
    /// with `ctx`; memos ignore it.
    pub(crate) fn update_if_necessary(&self, k: NodeKey, ctx: &mut dyn Any) {
        let (state, depth) = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.nodes.get(k) else {
                return;
            };
            if node.comp.is_none() {
                return;
            }
            if node.flags & RUNNING != 0 && !node.is_effect() {
                drop(inner);
                error!(target: "twine::reactive", "cycle: memo {:?} reads itself", k);
                panic!("twine-reactive: cycle detected (a memo reads itself, directly or indirectly)");
            }
            if node.state == NodeState::Clean {
                return;
            }
            let state = node.state;
            inner.update_depth += 1;
            (state, inner.update_depth)
        };
        let _depth = DepthGuard(self);
        if state == NodeState::Check {
            if depth > MAX_CHECK_DEPTH {
                let mut inner = self.inner.borrow_mut();
                inner.counters.depth_guard_hits += 1;
                if let Some(n) = inner.nodes.get_mut(k) {
                    n.state = NodeState::Dirty;
                }
                drop(inner);
                error!(
                    target: "twine::reactive",
                    "memo chain deeper than {}: recomputing {:?} without checking its sources",
                    MAX_CHECK_DEPTH,
                    k
                );
            } else {
                let mut i = 0;
                loop {
                    let src = {
                        let inner = self.inner.borrow();
                        let Some(node) = inner.nodes.get(k) else {
                            return;
                        };
                        if node.state != NodeState::Check {
                            break;
                        }
                        match node.comp.as_deref().and_then(|c| c.sources.get(i)) {
                            Some(&s) => s,
                            None => break,
                        }
                    };
                    self.update_if_necessary(src, &mut ());
                    i += 1;
                }
            }
        }
        let dirty = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.nodes.get_mut(k) else {
                return;
            };
            if node.state == NodeState::Dirty {
                true
            } else {
                node.state = NodeState::Clean;
                false
            }
        };
        if dirty {
            self.recompute(k, ctx);
        }
    }

    /// Re-runs computation `k`: re-tracks its sources and, for a memo whose value changed,
    /// marks its subscribers `Dirty`.
    fn recompute(&self, k: NodeKey, ctx: &mut dyn Any) {
        enum Job {
            Memo(MemoFn, ValueRc),
            Effect(EffectFn),
        }
        let (job, guard) = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.nodes.get_mut(k) else {
                return;
            };
            let value = node.value.clone();
            let Some(comp) = node.comp.as_deref_mut() else {
                return;
            };
            let job = match (&comp.kind, value) {
                (Kind::Memo(f), Some(v)) => Job::Memo(f.clone(), v),
                (Kind::Effect(f), _) => Job::Effect(f.clone()),
                (Kind::Memo(_), None) => return,
            };
            let mut old = core::mem::take(&mut comp.sources);
            // Clean before running: a write during the run re-marks (and re-queues) it.
            node.state = NodeState::Clean;
            node.flags |= RUNNING;
            for &s in &old {
                if let Some(src) = inner.nodes.get_mut(s) {
                    src.subscribers.remove(k);
                }
            }
            old.clear();
            if let Some(c) = inner.nodes.get_mut(k).and_then(|n| n.comp.as_deref_mut()) {
                c.sources = old; // reuse a spilled buffer
            }
            let prev_observer = inner.observer.replace(k);
            let prev_effect = inner.running_effect;
            match job {
                Job::Memo(..) => inner.counters.memo_runs += 1,
                Job::Effect(_) => {
                    inner.counters.effect_runs += 1;
                    inner.running_effect = Some(k);
                }
            }
            let guard = RunGuard {
                rt: self,
                key: k,
                prev_observer,
                prev_effect,
            };
            (job, guard)
        };
        match job {
            Job::Memo(f, value) => {
                let changed = f(&value);
                drop(guard);
                if changed {
                    let mut inner = self.inner.borrow_mut();
                    let count = inner.nodes.get(k).map_or(0, |n| n.subscribers.len());
                    for i in 0..count {
                        let Some(&s) = inner.nodes.get(k).and_then(|n| n.subscribers.as_slice().get(i))
                        else {
                            break;
                        };
                        if let Some(n) = inner.nodes.get_mut(s) {
                            if n.state == NodeState::Check {
                                n.state = NodeState::Dirty;
                            }
                        }
                    }
                }
            }
            Job::Effect(f) => {
                trace!(target: "twine::reactive", "run effect {:?}", k);
                match f.try_borrow_mut() {
                    Ok(mut body) => body(ctx),
                    Err(_) => {
                        error!(target: "twine::reactive", "effect {:?} re-entered; skipped", k);
                    }
                }
                drop(guard);
            }
        }
    }

    /// Runs a freshly created effect once (outside a flush) with a `()` context, as if in a
    /// flush, then flushes whatever it queued (unless batching).
    pub(crate) fn run_new_effect(&self, k: NodeKey) {
        let queue_only = {
            let mut inner = self.inner.borrow_mut();
            if inner.running_flush {
                if let Some(n) = inner.nodes.get_mut(k) {
                    n.flags |= QUEUED;
                }
                inner.pending.push_back(k);
                true
            } else {
                inner.running_flush = true;
                false
            }
        };
        if queue_only {
            return;
        }
        {
            let _flush = FlushGuard(self);
            self.update_if_necessary(k, &mut ());
        }
        let flush = {
            let inner = self.inner.borrow();
            inner.batch_depth == 0 && !inner.pending.is_empty()
        };
        if flush {
            self.flush(&mut ());
        }
    }

    /// Runs pending effects with `ctx` until none are left.
    pub(crate) fn flush(&self, ctx: &mut dyn Any) {
        {
            let mut inner = self.inner.borrow_mut();
            if inner.running_flush {
                return;
            }
            inner.running_flush = true;
            if !ctx.is::<()>() && !inner.deferred.is_empty() {
                let Inner {
                    nodes,
                    deferred,
                    pending,
                    ..
                } = &mut *inner;
                for k in deferred.drain(..) {
                    if let Some(n) = nodes.get_mut(k) {
                        n.flags &= !DEFERRED;
                        if n.flags & QUEUED == 0 {
                            n.flags |= QUEUED;
                            pending.push_back(k);
                        }
                    }
                }
            }
        }
        let _flush = FlushGuard(self);
        // A round is the set of effects queued before it started; effects queued while a round
        // runs form the next one. More than `flush_iterations_limit` rounds means a loop.
        let mut rounds: u32 = 1;
        let mut left_in_round = self.inner.borrow().pending.len();
        loop {
            let key = {
                let mut inner = self.inner.borrow_mut();
                if left_in_round == 0 {
                    if inner.pending.is_empty() {
                        break;
                    }
                    rounds += 1;
                    left_in_round = inner.pending.len();
                    if rounds > inner.flush_iterations_limit {
                        let limit = inner.flush_iterations_limit;
                        let dropped = inner.pending.len();
                        inner.counters.loop_cuts += 1;
                        let Inner { nodes, pending, .. } = &mut *inner;
                        for k in pending.drain(..) {
                            if let Some(n) = nodes.get_mut(k) {
                                n.flags &= !QUEUED;
                                n.state = NodeState::Clean;
                            }
                        }
                        drop(inner);
                        error!(
                            target: "twine::reactive",
                            "effect loop exceeded {} iterations; dropping {} pending effects",
                            limit,
                            dropped
                        );
                        break;
                    }
                }
                left_in_round -= 1;
                let Some(k) = inner.pending.pop_front() else {
                    break;
                };
                let Some(node) = inner.nodes.get_mut(k) else {
                    continue;
                };
                node.flags &= !QUEUED;
                if node.state == NodeState::Clean || node.flags & DEFERRED != 0 {
                    continue;
                }
                k
            };
            self.update_if_necessary(key, ctx);
        }
    }

    /// Moves the running effect to `deferred`. Returns `false` outside an
    /// effect.
    pub(crate) fn defer_current_effect(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(k) = inner.running_effect else {
            return false;
        };
        let Some(node) = inner.nodes.get_mut(k) else {
            return false;
        };
        node.state = NodeState::Dirty;
        if node.flags & DEFERRED == 0 {
            node.flags |= DEFERRED;
            inner.deferred.push(k);
        }
        true
    }

    /// Clears every node, scope and registration (test helper behind [`crate::reset`]).
    /// Arena generations are kept, so old handles stay dead.
    pub(crate) fn reset(&self) {
        let garbage = {
            let mut inner = self.inner.borrow_mut();
            let node_keys: Vec<NodeKey> = inner.nodes.iter().map(|(k, _)| k).collect();
            let scope_keys: Vec<ScopeKey> = inner.scopes.iter().map(|(k, _)| k).collect();
            let nodes: Vec<ReactiveNode> = node_keys
                .into_iter()
                .filter_map(|k| inner.nodes.remove(k))
                .collect();
            let scopes: Vec<ScopeData> = scope_keys
                .into_iter()
                .filter_map(|k| inner.scopes.remove(k))
                .collect();
            let channels = core::mem::take(&mut inner.channels);
            inner.pending.clear();
            inner.deferred.clear();
            inner.observer = None;
            inner.running_effect = None;
            inner.batch_depth = 0;
            inner.running_flush = false;
            inner.update_depth = 0;
            inner.flush_iterations_limit = DEFAULT_FLUSH_LIMIT;
            inner.counters = Counters::default();
            (nodes, scopes, channels)
        };
        drop(garbage);
    }
}
