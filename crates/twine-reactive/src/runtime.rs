//! The reactive graph: node and scope storage and the push-pull coloring algorithm.
//!
//! Every method takes `&RuntimeState` and borrows [`Inner`] only for short, user-code-free sections.
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

use twine_core::alloc_guard::{self, AllocSite};
use twine_core::fault::{CapacityFault, FaultCounts, FaultKind};
use twine_core::log::{debug, error, trace};
use twine_core::{Arena, Id, Limits, Partition, SmallVec};

use crate::channel::ChannelReg;
use crate::error::{HandleKind, ReactiveError};
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

/// Memo body: computes, compares with and stores into the value cell; returns `changed`, or
/// `None` when the cell was borrowed (the new value is then dropped).
pub(crate) type MemoFn = Rc<dyn Fn(&RefCell<dyn Any>) -> Option<bool>>;
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
    /// The root of the owning scope: an effect runs with the flush context and the ambient
    /// value only while its root is the active one (see [`Inner::active_root`]).
    pub(crate) root: ScopeKey,
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
    /// The root of this scope's tree (`None`: the scope is a root itself). Copied from the
    /// parent at creation, so finding a scope's root is one lookup.
    pub(crate) root: Option<ScopeKey>,
    pub(crate) children: Vec<ScopeKey>,
    pub(crate) nodes: Vec<NodeKey>,
    pub(crate) cleanups: Vec<Box<dyn FnOnce()>>,
    pub(crate) contexts: Vec<(TypeId, Rc<dyn Any>)>,
}

/// Counters exposed through [`crate::Runtime::stats`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counters {
    pub(crate) writes: u64,
    pub(crate) effect_runs: u64,
    pub(crate) memo_runs: u64,
}

/// The runtime's bounds that can be reached ([`CapacityFault`] codes of its `Capacity`
/// faults), in the order of [`FaultState::capacity`].
pub(crate) const RUNTIME_BOUNDS: [CapacityFault; 3] = [
    CapacityFault::ReactiveNodes,
    CapacityFault::ReactiveScopes,
    CapacityFault::PendingEffects,
];

/// Fault bookkeeping, touched only when a fault is recorded or read: kept out of [`Inner`]
/// (in its own cell) so the hot runtime state stays compact.
pub(crate) struct FaultState {
    /// Faults since start-up.
    pub(crate) total: FaultCounts,
    /// Faults since the last [`crate::Runtime::take_faults`].
    pub(crate) new: FaultCounts,
    /// `Capacity` occurrences per bound ([`RUNTIME_BOUNDS`]) since the last
    /// [`crate::Runtime::take_capacity_faults`].
    pub(crate) capacity: [u32; 3],
    pub(crate) hook: Option<FaultHook>,
}

impl FaultState {
    const fn new() -> Self {
        FaultState {
            total: FaultCounts::new(),
            new: FaultCounts::new(),
            capacity: [0; 3],
            hook: None,
        }
    }
}

/// The runtime's share of [`Limits`] (as `usize`, compared at the insertion points).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeLimits {
    pub(crate) nodes: usize,
    pub(crate) scopes: usize,
    pub(crate) pending: usize,
}

impl RuntimeLimits {
    const fn of(l: &Limits) -> Self {
        RuntimeLimits {
            nodes: l.max_reactive_nodes as usize,
            scopes: l.max_scopes as usize,
            pending: l.max_pending_effects as usize,
        }
    }
}

/// Called for every fault the runtime records: the kind and the number of occurrences.
pub type FaultHook = fn(FaultKind, u32);

/// All mutable runtime state.
pub(crate) struct Inner {
    /// The partition this runtime is the state of (its seal domain).
    pub(crate) part: Partition,
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
    /// The value lent by the innermost active [`provide_ambient`](crate::Runtime::provide_ambient)
    /// (`None` outside one or while it is borrowed).
    pub(crate) ambient: Option<core::ptr::NonNull<dyn Any>>,
    /// The root scope whose update is running ([`Scope::activate`](crate::Scope::activate)):
    /// while set, effects of other roots run without the ambient value and with the `()`
    /// context, and only this root's deferred effects are re-queued by a flush with a context.
    /// `None` outside every UI update (no filtering).
    pub(crate) active_root: Option<ScopeKey>,
    /// The waker of each root scope that has one ([`Scope::set_ui_waker`](crate::Scope::set_ui_waker));
    /// one entry per `Ui`, so a linear search.
    pub(crate) root_wakers: Vec<(ScopeKey, &'static crate::UiWaker)>,
    /// The bounds ([`crate::Runtime::set_limits`]).
    pub(crate) limits: RuntimeLimits,
    /// Effects not queued since the last report because `pending`/`deferred` were full
    /// (reported as `Capacity` faults once the borrow is released).
    pub(crate) pending_overflow: u32,
}

impl Inner {
    const fn new(part: Partition) -> Self {
        Inner {
            part,
            nodes: Arena::new_in(part),
            scopes: Arena::new_in(part),
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
            },
            created_logged: false,
            ambient: None,
            active_root: None,
            root_wakers: Vec::new(),
            limits: RuntimeLimits::of(&Limits::DEFAULT),
            pending_overflow: 0,
        }
    }

    /// Applies `limits` (validated by the caller): the bounds, and with
    /// [`Limits::preallocate`] the storage of the arenas, both effect queues and the
    /// propagation stack.
    pub(crate) fn set_limits(&mut self, limits: &Limits) {
        self.limits = RuntimeLimits::of(limits);
        if limits.preallocate {
            self.nodes.reserve(limits.max_reactive_nodes);
            self.scopes.reserve(limits.max_scopes);
            let pending = usize::from(limits.max_pending_effects);
            self.pending
                .reserve_exact(pending.saturating_sub(self.pending.len()));
            self.deferred
                .reserve_exact(pending.saturating_sub(self.deferred.len()));
            let nodes = usize::from(limits.max_reactive_nodes);
            self.stack.reserve_exact(nodes.saturating_sub(self.stack.len()));
        }
    }

    /// Queues effect `k` (flagged `QUEUED`) unless `pending` holds
    /// [`max_pending_effects`](Limits::max_pending_effects) already: then the effect is not
    /// queued and the overflow is counted for a `Capacity` fault. Returns whether it was
    /// queued.
    #[inline]
    fn queue_effect(
        part: Partition,
        pending: &mut VecDeque<NodeKey>,
        limits: RuntimeLimits,
        overflow: &mut u32,
        node: &mut ReactiveNode,
        k: NodeKey,
    ) -> bool {
        if pending.len() >= limits.pending {
            *overflow = overflow.saturating_add(1);
            return false;
        }
        node.flags |= QUEUED;
        alloc_guard::push_back(part, pending, k, AllocSite::EffectQueue);
        true
    }

    /// The overflow counted since the last call, clearing it (`0` almost always: one load).
    #[inline]
    pub(crate) fn take_overflow(&mut self) -> u32 {
        if self.pending_overflow == 0 {
            0
        } else {
            core::mem::take(&mut self.pending_overflow)
        }
    }

    /// What the runtime's heap is used for ([`Runtime::memory`](crate::Runtime::memory)).
    /// O(nodes + scopes); allocates nothing.
    pub(crate) fn memory(&self) -> crate::RuntimeMemory {
        /// Bytes of the `Rc` allocation holding `v`: the two counters and the value, padded
        /// to the allocation's alignment.
        fn rc_bytes<T: ?Sized>(v: &T) -> usize {
            core::alloc::Layout::new::<[usize; 2]>()
                .extend(core::alloc::Layout::for_value(v))
                .map_or(0, |(l, _)| l.pad_to_align().size())
        }
        /// Bytes reserved by a vector's buffer.
        fn vec_bytes<T>(v: &Vec<T>) -> usize {
            core::mem::size_of::<T>().saturating_mul(v.capacity())
        }
        /// Sum of byte counts. The parts are sizes of live allocations in one address space, so
        /// the sum cannot overflow; saturating keeps the report total without a proof per term.
        fn add_bytes(parts: &[usize]) -> usize {
            parts.iter().fold(0, |sum, &b| sum.saturating_add(b))
        }
        let values = self
            .nodes
            .iter()
            .map(|(_, n)| {
                let value = n.value.as_ref().map_or(0, |v| rc_bytes(&**v));
                let comp = n.comp.as_ref().map_or(0, |c| {
                    let f = match &c.kind {
                        Kind::Memo(f) => rc_bytes(&**f),
                        Kind::Effect(f) => rc_bytes(&**f),
                    };
                    add_bytes(&[core::mem::size_of::<Computation>(), c.sources.heap_bytes(), f])
                });
                add_bytes(&[value, comp, n.subscribers.heap_bytes()])
            })
            .fold(0, usize::saturating_add);
        let scope_data = self
            .scopes
            .iter()
            .map(|(_, d)| {
                add_bytes(&[
                    vec_bytes(&d.children),
                    vec_bytes(&d.nodes),
                    vec_bytes(&d.cleanups),
                    d.cleanups
                        .iter()
                        .map(|c| core::mem::size_of_val(&**c))
                        .fold(0, usize::saturating_add),
                    vec_bytes(&d.contexts),
                    d.contexts
                        .iter()
                        .map(|(_, v)| rc_bytes(&**v))
                        .fold(0, usize::saturating_add),
                ])
            })
            .fold(0, usize::saturating_add);
        let queues = add_bytes(&[
            core::mem::size_of::<NodeKey>().saturating_mul(self.pending.capacity()),
            core::mem::size_of::<NodeKey>().saturating_mul(self.deferred.capacity()),
            vec_bytes(&self.channels),
            vec_bytes(&self.stack),
            vec_bytes(&self.root_wakers),
        ]);
        crate::RuntimeMemory {
            nodes: self.nodes.len(),
            scopes: self.scopes.len(),
            arenas: self
                .nodes
                .bytes_reserved()
                .saturating_add(self.scopes.bytes_reserved()),
            values,
            scope_data,
            queues,
        }
    }

    /// Colors the subscribers of `key` after a write: direct subscribers `Dirty`, their
    /// descendants `Check`; effects reached are queued once.
    fn mark_stale(&mut self, key: NodeKey) {
        let Inner {
            part,
            nodes,
            stack,
            pending,
            limits,
            pending_overflow,
            ..
        } = self;
        let part = *part;
        stack.clear();
        if let Some(n) = nodes.get(key) {
            // Reverse so that subscribers are visited (and effects queued) in subscription order.
            let subs = n.subscribers.as_slice();
            alloc_guard::reserve(part, stack, subs.len(), AllocSite::ReactiveScratch);
            stack.extend(subs.iter().rev().map(|&s| (s, NodeState::Dirty)));
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
                    Self::queue_effect(part, pending, *limits, pending_overflow, node, k);
                }
            } else if was_clean {
                let subs = node.subscribers.as_slice();
                alloc_guard::reserve(part, stack, subs.len(), AllocSite::ReactiveScratch);
                stack.extend(subs.iter().rev().map(|&s| (s, NodeState::Check)));
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

    /// The root of scope `s`, or `None` if `s` is dead. One lookup.
    #[inline]
    pub(crate) fn root_of(&self, s: ScopeKey) -> Option<ScopeKey> {
        self.scopes.get(s).map(|d| d.root.unwrap_or(s))
    }

    /// The waker set on root scope `root` ([`Scope::set_ui_waker`](crate::Scope::set_ui_waker)).
    pub(crate) fn root_waker(&self, root: ScopeKey) -> Option<&'static crate::UiWaker> {
        self.root_wakers.iter().find(|(k, _)| *k == root).map(|&(_, w)| w)
    }

    /// Whether effect `k` waits (pending or deferred) and belongs to `root`.
    fn effect_of(&self, k: NodeKey, root: ScopeKey) -> bool {
        self.nodes
            .get(k)
            .and_then(|n| n.comp.as_deref())
            .is_some_and(|c| c.root == root)
    }

    /// Whether effects of root `root` wait to run (pending, or deferred until the root's next
    /// flush with a context). Scans the queues only when they are not empty.
    pub(crate) fn has_pending_effects_of(&self, root: ScopeKey) -> bool {
        self.pending.iter().any(|&k| self.effect_of(k, root))
            || self.deferred.iter().any(|&k| self.effect_of(k, root))
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

/// The state of one partition's reactive runtime (one per partition and thread with `std`, one
/// per partition and process without; reached through
/// [`with_runtime`](crate::global::with_runtime)).
pub(crate) struct RuntimeState {
    /// The partition this state belongs to (fixed at construction: one state per slot).
    pub(crate) part: Partition,
    pub(crate) inner: RefCell<Inner>,
    /// Cold fault state (never borrowed together with `inner` on a hot path).
    pub(crate) faults: RefCell<FaultState>,
}

/// Restores `observer`/`running_effect` and clears `RUNNING` after a computation, also on
/// unwinding.
struct RunGuard<'a> {
    rt: &'a RuntimeState,
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
struct DepthGuard<'a>(&'a RuntimeState);

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        let mut inner = self.0.inner.borrow_mut();
        inner.update_depth = inner.update_depth.saturating_sub(1);
    }
}

/// Calls an effect's body with `ctx` (a re-entered effect is skipped with an error).
#[inline]
fn call_effect(f: &EffectFn, k: NodeKey, ctx: &mut dyn Any) {
    match f.try_borrow_mut() {
        Ok(mut body) => body(ctx),
        Err(_) => {
            error!(target: "twine::reactive", "effect {:?} re-entered; skipped", k);
        }
    }
}

/// Restores the ambient value and the active root hidden from another root's effect.
struct UnhideGuard<'a> {
    rt: &'a RuntimeState,
    hidden: (Option<core::ptr::NonNull<dyn Any>>, Option<ScopeKey>),
}

impl Drop for UnhideGuard<'_> {
    fn drop(&mut self) {
        let mut inner = self.rt.inner.borrow_mut();
        inner.ambient = self.hidden.0;
        inner.active_root = self.hidden.1;
    }
}

/// Clears `running_flush` on scope exit.
struct FlushGuard<'a>(&'a RuntimeState);

impl Drop for FlushGuard<'_> {
    fn drop(&mut self) {
        self.0.inner.borrow_mut().running_flush = false;
    }
}

impl RuntimeState {
    /// Records `n` occurrences of `kind` and calls the fault hook (without any borrow held:
    /// the hook is foreign code, rule R1). Cold and out of line: faults are rare, and keeping
    /// this out of `flush`/`update_if_necessary` keeps their hot paths compact.
    #[cold]
    #[inline(never)]
    pub(crate) fn record_fault(&self, kind: FaultKind, n: u32) {
        let hook = {
            let mut f = self.faults.borrow_mut();
            f.total.add(kind, n);
            f.new.add(kind, n);
            f.hook
        };
        if let Some(hook) = hook {
            hook(kind, n);
        }
    }

    /// Records `n` `Capacity` faults of the runtime's bound `bound` (one of
    /// [`RUNTIME_BOUNDS`]): counted per bound for [`crate::Runtime::take_capacity_faults`],
    /// then as [`record_fault`](Self::record_fault). Cold.
    #[cold]
    #[inline(never)]
    pub(crate) fn record_capacity(&self, bound: CapacityFault, n: u32) {
        {
            let mut f = self.faults.borrow_mut();
            if let Some(i) = RUNTIME_BOUNDS.iter().position(|b| *b == bound) {
                if let Some(c) = f.capacity.get_mut(i) {
                    *c = c.saturating_add(n);
                }
            }
        }
        self.record_fault(FaultKind::Capacity, n);
    }

    /// Reports `n` effects that found their queue full (no borrow held: the hook runs).
    #[cold]
    #[inline(never)]
    fn report_overflow(&self, n: u32) {
        error!(
            target: "twine::reactive",
            "{} effect(s) not queued: Limits::max_pending_effects reached",
            n
        );
        self.record_capacity(CapacityFault::PendingEffects, n);
    }

    /// An empty runtime of partition `part` (usable in `static` and `const` thread-local
    /// initializers).
    pub(crate) const fn new(part: Partition) -> Self {
        RuntimeState {
            part,
            inner: RefCell::new(Inner::new(part)),
            faults: RefCell::new(FaultState::new()),
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

    /// Creates a scope, optionally as a child of `parent`: `Disposed(Scope)` if the parent is
    /// dead, `Capacity(Scope)` if [`Limits::max_scopes`] scopes are live.
    pub(crate) fn create_scope(&self, parent: Option<ScopeKey>) -> Result<ScopeKey, ReactiveError> {
        self.log_created();
        let mut inner = self.inner.borrow_mut();
        if let Some(p) = parent {
            if !inner.scopes.contains(p) {
                return Err(ReactiveError::Disposed(HandleKind::Scope));
            }
        }
        if inner.scopes.len() >= inner.limits.scopes {
            return Err(ReactiveError::Capacity(HandleKind::Scope));
        }
        let root = parent.and_then(|p| inner.root_of(p));
        let key = inner
            .scopes
            .insert(ScopeData {
                parent,
                root,
                ..ScopeData::default()
            })
            .map_err(|_| ReactiveError::Capacity(HandleKind::Scope))?;
        if let Some(p) = parent.and_then(|p| inner.scopes.get_mut(p)) {
            alloc_guard::push(self.part, &mut p.children, key, AllocSite::ReactiveScope);
        }
        Ok(key)
    }

    /// Whether scope `s` is alive.
    pub(crate) fn scope_alive(&self, s: ScopeKey) -> bool {
        self.inner.borrow().scopes.contains(s)
    }

    /// Disposes scope `s` and its descendants. The caller wraps this in a batch.
    ///
    /// Order: a scope's children first (most recent first, each with its own subtree), then
    /// its cleanups (most recent first; a cleanup may create children or cleanups on the
    /// scope, which are disposed or run before the scope goes), then its nodes and channel
    /// registrations, then its contexts.
    ///
    /// **Iterative and allocation-free.** The walk follows the scope tree's own links — a
    /// scope's `children` list down, its `parent` link back up — so it needs no recursion
    /// (constant stack, whatever the tree depth) and no work list. Nodes and channel
    /// registrations are released one at a time, each dropped after the borrow is released
    /// (rule R1), so no garbage buffer is needed either: disposal allocates nothing, before
    /// or after a seal. User code that runs meanwhile (a cleanup, a `Drop`) may dispose
    /// scopes itself, including ones on the walk's path: the walk then restarts from `s`
    /// (still alive) or ends (`s` disposed).
    pub(crate) fn dispose_scope(&self, s: ScopeKey) {
        if !self.scope_alive(s) {
            debug!(target: "twine::reactive", "dispose scope {:?}: already disposed", s);
            return;
        }
        let mut cur = s;
        loop {
            let cleanups = {
                let mut inner = self.inner.borrow_mut();
                let Some(data) = inner.scopes.get_mut(cur) else {
                    // `cur` was disposed by user code meanwhile.
                    if cur == s || !inner.scopes.contains(s) {
                        return;
                    }
                    cur = s;
                    continue;
                };
                if let Some(&child) = data.children.last() {
                    if inner.scopes.contains(child) {
                        cur = child;
                    } else if let Some(data) = inner.scopes.get_mut(cur) {
                        // Not reachable (a removed scope leaves its parent's list); keeps
                        // the walk finite should it happen.
                        data.children.pop();
                    }
                    continue;
                }
                core::mem::take(&mut data.cleanups)
            };
            if !cleanups.is_empty() {
                // Outside the borrow (R1); then look at `cur` again: a cleanup may have
                // added children or cleanups.
                for f in cleanups.into_iter().rev() {
                    f();
                }
                continue;
            }
            // No children, no cleanups: release `cur` and go back up.
            let parent = self.release_scope(cur);
            if cur == s {
                return;
            }
            // A dead parent (disposed by a `Drop` meanwhile) restarts the walk from `s`.
            cur = parent.unwrap_or(s);
        }
    }

    /// Removes scope `s` (which has no children and no cleanups left) from the arena and its
    /// parent's child list, then releases its nodes, its channel registrations and finally
    /// its contexts, each dropped without the borrow (rule R1). Returns the parent. Allocates
    /// nothing.
    fn release_scope(&self, s: ScopeKey) -> Option<ScopeKey> {
        let data = {
            let mut inner = self.inner.borrow_mut();
            let data = inner.scopes.remove(s)?;
            if let Some(p) = data.parent.and_then(|p| inner.scopes.get_mut(p)) {
                // The disposal walk removes the last child: O(1) then.
                if p.children.last() == Some(&s) {
                    p.children.pop();
                } else {
                    p.children.retain(|&c| c != s);
                }
            }
            if data.parent.is_none() && !inner.root_wakers.is_empty() {
                inner.root_wakers.retain(|&(k, _)| k != s);
            }
            data
        };
        // One node at a time: the statement's borrow ends before the node's user values and
        // closures (its value cell and computation) are dropped. Only those two leave the
        // borrow: the node's bookkeeping (its subscriber list) is freed inside it, where no
        // user code runs (and the whole node is not moved out).
        for &k in &data.nodes {
            let user = self.inner.borrow_mut().unlink_node(k).map(|n| (n.value, n.comp));
            drop(user);
        }
        // Dead keys in the queues are skipped by a flush (none runs inside the caller's
        // batch); pruning once frees their room.
        self.inner.borrow_mut().prune_queues();
        loop {
            let reg = crate::channel::take_scope_channel(&mut self.inner.borrow_mut(), s);
            let Some(reg) = reg else {
                break;
            };
            crate::channel::release_channel(self.part, reg);
        }
        debug!(target: "twine::reactive", "dispose scope {:?}: {} nodes", s, data.nodes.len());
        let parent = data.parent;
        drop(data); // contexts: user values
        parent
    }

    /// Registers `f` as a cleanup of `s`; returns it back if the scope is dead.
    pub(crate) fn add_cleanup(&self, s: ScopeKey, f: Box<dyn FnOnce()>) -> Result<(), Box<dyn FnOnce()>> {
        let mut inner = self.inner.borrow_mut();
        match inner.scopes.get_mut(s) {
            Some(d) => {
                if core::mem::size_of_val(&*f) != 0 {
                    alloc_guard::note(self.part, AllocSite::ReactiveCleanup);
                }
                alloc_guard::push(self.part, &mut d.cleanups, f, AllocSite::ReactiveCleanup);
                Ok(())
            }
            None => Err(f),
        }
    }

    // ----- nodes --------------------------------------------------------------------------

    /// Inserts a node owned by scope `s`, taking `value` and `kind` out of the caller's
    /// options: `Disposed(Scope)` if the scope is dead, `Capacity(what)` if
    /// [`Limits::max_reactive_nodes`] nodes are live. On failure `value` and `kind` stay with
    /// the caller, which drops them after the borrow is released (rule R1).
    ///
    /// Out of line, and taking the options by reference (the result is a key or an error, no
    /// values travel back), so that the runtime access around it inlines into the caller.
    #[inline]
    pub(crate) fn create_node(
        &self,
        s: ScopeKey,
        value: &mut Option<ValueRc>,
        kind: &mut Option<Kind>,
        what: HandleKind,
    ) -> Result<NodeKey, ReactiveError> {
        let mut inner = self.inner.borrow_mut();
        let Some(root) = inner.root_of(s) else {
            return Err(ReactiveError::Disposed(HandleKind::Scope));
        };
        if inner.nodes.len() >= inner.limits.nodes || inner.nodes.is_full() {
            return Err(ReactiveError::Capacity(what));
        }
        // A node always comes with a heap value: its value cell or its computation.
        alloc_guard::note(self.part, AllocSite::ReactiveNode);
        let comp = kind.take().map(|kind| {
            Box::new(Computation {
                kind,
                sources: SmallVec::new(),
                root,
            })
        });
        let state = if comp.is_some() {
            NodeState::Dirty
        } else {
            NodeState::Clean
        };
        let node = ReactiveNode {
            value: value.take(),
            subscribers: KeyList::new(),
            comp,
            scope: s,
            state,
            flags: 0,
        };
        // Cannot fail: the arena is not full (checked above, before the node was built, so a
        // refused value is handed back and dropped outside this borrow, rule R1). Should it
        // fail anyway, `insert` drops the node and the creation reports the capacity error.
        let Ok(key) = inner.nodes.insert(node) else {
            return Err(ReactiveError::Capacity(what));
        };
        if let Some(d) = inner.scopes.get_mut(s) {
            alloc_guard::push(self.part, &mut d.nodes, key, AllocSite::ReactiveScope);
        }
        Ok(key)
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
        let part = self.part;
        if let Some(o) = inner.observer {
            if o != k {
                let observer_alive = match inner.nodes.get_mut(o).and_then(|n| n.comp.as_deref_mut()) {
                    Some(c) => {
                        if !c.sources.contains(&k) {
                            c.sources.push_in(part, k);
                        }
                        true
                    }
                    None => false,
                };
                if observer_alive {
                    if let Some(n) = inner.nodes.get_mut(k) {
                        n.subscribers.insert(part, o);
                    }
                }
            }
        }
        value
    }

    /// Records a write to `k` and propagates it; flushes when outside batch and flush.
    pub(crate) fn notify(&self, k: NodeKey) {
        let (flush, overflow) = {
            let mut inner = self.inner.borrow_mut();
            // Statistics: a `u64` count never wraps in a product's lifetime.
            inner.counters.writes = inner.counters.writes.wrapping_add(1);
            inner.mark_stale(k);
            let flush = inner.batch_depth == 0 && !inner.running_flush && !inner.pending.is_empty();
            (flush, inner.take_overflow())
        };
        if overflow != 0 {
            self.report_overflow(overflow);
        }
        if flush {
            self.flush(&mut ());
        }
    }

    // ----- computations -------------------------------------------------------------------

    /// Brings computation `k` up to date. Effects that need to run are run
    /// with `ctx`; memos ignore it.
    ///
    /// [`ReactiveError::Cycle`] when `k` is a memo whose computation is running (it reads
    /// itself), or when bringing one of its sources up to date hit such a memo; `k` is then
    /// left as it was (not recomputed, still to be checked). A dead or plain node is `Ok`.
    pub(crate) fn update_if_necessary(&self, k: NodeKey, ctx: &mut dyn Any) -> Result<(), ReactiveError> {
        let (state, depth) = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.nodes.get(k) else {
                return Ok(());
            };
            if node.comp.is_none() {
                return Ok(());
            }
            if node.flags & RUNNING != 0 && !node.is_effect() {
                drop(inner);
                error!(target: "twine::reactive", "cycle: memo {:?} reads itself", k);
                return Err(ReactiveError::Cycle);
            }
            if node.state == NodeState::Clean {
                return Ok(());
            }
            let state = node.state;
            inner.update_depth = inner.update_depth.saturating_add(1);
            (state, inner.update_depth)
        };
        let _depth = DepthGuard(self);
        if state == NodeState::Check {
            if depth > MAX_CHECK_DEPTH {
                let mut inner = self.inner.borrow_mut();
                if let Some(n) = inner.nodes.get_mut(k) {
                    n.state = NodeState::Dirty;
                }
                drop(inner);
                self.record_fault(FaultKind::DepthGuard, 1);
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
                            return Ok(());
                        };
                        if node.state != NodeState::Check {
                            break;
                        }
                        match node.comp.as_deref().and_then(|c| c.sources.get(i)) {
                            Some(&s) => s,
                            None => break,
                        }
                    };
                    self.update_if_necessary(src, &mut ())?;
                    i = i.saturating_add(1);
                }
            }
        }
        let dirty = {
            let mut inner = self.inner.borrow_mut();
            let Some(node) = inner.nodes.get_mut(k) else {
                return Ok(());
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
        Ok(())
    }

    /// Re-runs computation `k`: re-tracks its sources and, for a memo whose value changed,
    /// marks its subscribers `Dirty`.
    fn recompute(&self, k: NodeKey, ctx: &mut dyn Any) {
        enum Job {
            Memo(MemoFn, ValueRc),
            /// The body, and whether the effect belongs to another root than the active one.
            Effect(EffectFn, bool),
        }
        let (job, guard) = {
            let mut inner = self.inner.borrow_mut();
            let active = inner.active_root;
            let Some(node) = inner.nodes.get_mut(k) else {
                return;
            };
            let value = node.value.clone();
            let Some(comp) = node.comp.as_deref_mut() else {
                return;
            };
            let job = match (&comp.kind, value) {
                (Kind::Memo(f), Some(v)) => Job::Memo(f.clone(), v),
                (Kind::Effect(f), _) => Job::Effect(f.clone(), active.is_some_and(|a| a != comp.root)),
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
                Job::Memo(..) => inner.counters.memo_runs = inner.counters.memo_runs.wrapping_add(1),
                Job::Effect(..) => {
                    inner.counters.effect_runs = inner.counters.effect_runs.wrapping_add(1);
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
                let Some(changed) = changed else {
                    // The value cell is borrowed by a `with` of this memo further up the stack:
                    // the new value was dropped. Recompute on the next read.
                    if let Some(n) = self.inner.borrow_mut().nodes.get_mut(k) {
                        n.state = NodeState::Dirty;
                    }
                    ReactiveError::Borrowed(HandleKind::Memo).skip(self.part, None);
                    return;
                };
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
            Job::Effect(f, foreign) => {
                trace!(target: "twine::reactive", "run effect {:?}", k);
                if foreign {
                    self.run_foreign_effect(&f, k);
                } else {
                    call_effect(&f, k, ctx);
                }
                drop(guard);
            }
        }
    }

    /// Runs effect `k` of another root than the active one (see [`Inner::active_root`]): with
    /// the ambient value hidden, no active root and the `()` context, so it cannot reach the
    /// context lent to the active root's update (another UI's engine). A view-layer binding
    /// then defers itself to its own root's next update ([`defer_current_effect`](Self::defer_current_effect)
    /// wakes that root); a plain effect runs as usual. Cold: only with several roots.
    #[cold]
    #[inline(never)]
    fn run_foreign_effect(&self, f: &EffectFn, k: NodeKey) {
        let hidden = {
            let mut inner = self.inner.borrow_mut();
            (inner.ambient.take(), inner.active_root.take())
        };
        // Put back when the effect returns (also on unwinding). The providing frame of the
        // ambient value is further down this call stack and still active, and nothing reached
        // the value meanwhile (the slot was empty), so restoring it is like the end of a
        // `with_ambient` borrow that did not use the value.
        let _restore = UnhideGuard { rt: self, hidden };
        call_effect(f, k, &mut ());
    }

    /// Runs a freshly created effect once (outside a flush) with a `()` context, as if in a
    /// flush, then flushes whatever it queued (unless batching).
    pub(crate) fn run_new_effect(&self, k: NodeKey) {
        let (queue_only, overflow) = {
            let mut inner = self.inner.borrow_mut();
            if inner.running_flush {
                let Inner {
                    nodes,
                    pending,
                    limits,
                    pending_overflow,
                    ..
                } = &mut *inner;
                if let Some(n) = nodes.get_mut(k) {
                    Inner::queue_effect(self.part, pending, *limits, pending_overflow, n, k);
                }
                (true, inner.take_overflow())
            } else {
                inner.running_flush = true;
                (false, 0)
            }
        };
        if queue_only {
            if overflow != 0 {
                self.report_overflow(overflow);
            }
            return;
        }
        {
            let _flush = FlushGuard(self);
            if let Err(e) = self.update_if_necessary(k, &mut ()) {
                e.skip(self.part, None);
            }
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
                Self::requeue_deferred(&mut inner);
                let overflow = inner.take_overflow();
                if overflow != 0 {
                    drop(inner);
                    self.report_overflow(overflow);
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
                    rounds = rounds.saturating_add(1);
                    left_in_round = inner.pending.len();
                    if rounds > inner.flush_iterations_limit {
                        let limit = inner.flush_iterations_limit;
                        let dropped = inner.pending.len();
                        let Inner { nodes, pending, .. } = &mut *inner;
                        for k in pending.drain(..) {
                            if let Some(n) = nodes.get_mut(k) {
                                n.flags &= !QUEUED;
                                n.state = NodeState::Clean;
                            }
                        }
                        drop(inner);
                        self.record_fault(FaultKind::EffectLoopCut, 1);
                        error!(
                            target: "twine::reactive",
                            "effect loop exceeded {} iterations; dropping {} pending effects",
                            limit,
                            dropped
                        );
                        break;
                    }
                }
                // Not 0 here (a new round started above otherwise).
                left_in_round = left_in_round.saturating_sub(1);
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
            if let Err(e) = self.update_if_necessary(key, ctx) {
                e.skip(self.part, None);
            }
        }
    }

    /// Moves the deferred effects of the active root (all of them when no root is active) to
    /// `pending`, in the order they were deferred; other roots' effects stay deferred for
    /// their own root's flush.
    fn requeue_deferred(inner: &mut Inner) {
        let active = inner.active_root;
        let part = inner.part;
        let Inner {
            nodes,
            deferred,
            pending,
            limits,
            pending_overflow,
            ..
        } = inner;
        deferred.retain(|&k| {
            let Some(n) = nodes.get_mut(k) else {
                return false;
            };
            if active.is_some_and(|a| n.comp.as_deref().is_some_and(|c| c.root != a)) {
                return true;
            }
            if n.flags & QUEUED == 0 && !Inner::queue_effect(part, pending, *limits, pending_overflow, n, k) {
                // No room: stays deferred for the next flush with a context.
                return true;
            }
            n.flags &= !DEFERRED;
            false
        });
    }

    /// Disposes the running effect. Returns `false` outside an effect.
    pub(crate) fn dispose_current_effect(&self) -> bool {
        let Some(k) = self.inner.borrow().running_effect else {
            return false;
        };
        self.dispose_node(k);
        true
    }

    /// Moves the running effect to `deferred`. Returns `false` outside an effect.
    ///
    /// An effect deferred outside its own root's update (no root active, or another one)
    /// wakes its root's waker, so the UI that can run it updates.
    pub(crate) fn defer_current_effect(&self) -> bool {
        let (wake, full) = {
            let mut inner = self.inner.borrow_mut();
            let Some(k) = inner.running_effect else {
                return false;
            };
            let active = inner.active_root;
            let max = inner.limits.pending;
            let Inner { nodes, deferred, .. } = &mut *inner;
            let Some(node) = nodes.get_mut(k) else {
                return false;
            };
            node.state = NodeState::Dirty;
            let root = node.comp.as_deref().map(|c| c.root);
            let mut full = false;
            if node.flags & DEFERRED == 0 {
                if deferred.len() < max {
                    node.flags |= DEFERRED;
                    alloc_guard::push(self.part, deferred, k, AllocSite::EffectQueue);
                } else {
                    full = true;
                }
            }
            (
                root.filter(|&r| Some(r) != active)
                    .and_then(|r| inner.root_waker(r)),
                full,
            )
        };
        if full {
            self.report_overflow(1);
        }
        // Outside the borrow: `wake` may call the platform's notify function (rule R1).
        if let Some(w) = wake {
            w.wake();
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
            inner.ambient = None;
            inner.active_root = None;
            inner.root_wakers.clear();
            inner.limits = RuntimeLimits::of(&Limits::DEFAULT);
            inner.pending_overflow = 0;
            (nodes, scopes, channels)
        };
        *self.faults.borrow_mut() = FaultState::new();
        let (nodes, scopes, channels) = garbage;
        drop(nodes);
        for reg in channels {
            crate::channel::release_channel(self.part, reg);
        }
        drop(scopes);
    }
}
