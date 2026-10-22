//! [`for_each`]: a keyed list reusing, moving, inserting and deleting only what changed, with
//! one item signal per row updated in place.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::hash::{BuildHasher, Hash};

use hashbrown::{DefaultHashBuilder, HashTable};
use twine_core::alloc_guard::{self, AllocSite};
use twine_engine::{Engine, NodeId, fmt_node_id};
use twine_reactive::{ReadSignal, Scope, Signal};

use super::items::{IntoItems, Items};
use super::{FOR_EACH_CLASS, Wrapper, dispose_with};
use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::view::{AnyView, IntoAnyView, View};

/// A keyed list: one row per item, identified by `key(item)`; `view` builds a row from a
/// [`ReadSignal`] of its item.
///
/// `items` is any [`IntoItems`]: a `Signal`, `ReadSignal` or `Memo` of a `Vec<T>` (read in
/// place with `with`, the list is never cloned), a closure returning a `Vec<T>`, or a constant
/// `Vec<T>` / `&'static [T]`.
///
/// When the items change, the rows are reconciled with the new keys:
///
/// - a row whose key is still there is **kept** — never rebuilt. When its item changed (`!=`),
///   the new value is written into the row's item signal in place (`clone_from`, which reuses
///   the old value's buffers), so exactly the bindings of that row that read it run;
/// - rows of removed keys are disposed and deleted, rows of new keys are built;
/// - rows out of order are moved — the minimal number of moves (everything on a longest
///   increasing subsequence of the old positions stays where it is).
///
/// So a table of 200 values of which 3 change costs one reconcile (200 key and value
/// comparisons, no allocation) and 3 binding runs.
///
/// **Field projections.** A row showing one field of a larger item can derive it with
/// [`ReadSignal::map`] (a memo owned by the row): its bindings then run only when that field
/// changes, not when another field of the item does.
///
/// Duplicate keys are a user error: the first occurrence wins, the others are skipped (and
/// logged with `warn!`).
///
/// **Bound.** At most [`Limits::max_list_items`](twine_core::Limits::max_list_items) rows
/// (or [`ForEach::max_items`]): the items beyond it are not shown, and a
/// [`CapacityFault::ListItems`](twine_core::fault::CapacityFault::ListItems) fault is raised.
///
/// **Cost.** Per row: a scope and one signal holding a copy of the item. Per change: when the
/// keys are those of the rows, in the same order (values changed, or nothing did), one pass
/// over the items comparing keys and values, allocation-free. Otherwise (insertions, removals,
/// moves) one pass to index the keys and one to place the rows, in a scratch the collection
/// keeps between changes (an index table of `u32` entries — keys are never copied — and a
/// few vectors): once it has reached the size of the largest change, or was reserved for the
/// bound with [`Limits::preallocate`](twine_core::Limits::preallocate), removals and moves
/// allocate nothing, and insertions only what their new rows need.
///
/// ```
/// use twine_view::prelude::*;
///
/// #[derive(Clone, PartialEq)]
/// struct Sensor {
///     id: u32,
///     name: &'static str,
///     value: i32,
/// }
///
/// fn table(cx: Scope) -> impl View {
///     let sensors = cx.signal(vec![
///         Sensor { id: 1, name: "Boiler", value: 64 },
///         Sensor { id: 2, name: "Tank", value: 21 },
///     ]);
///     column(for_each(sensors, |s| s.id, |_cx, s| {
///         // Re-runs only when the `value` of this sensor changes.
///         let value = s.map(|s| s.value);
///         row((label(s.get_untracked().name), label(text!("{} °C", value.get()))))
///     }))
/// }
/// # let _ = table;
/// ```
pub fn for_each<T, K, V, M>(
    items: impl IntoItems<T, M>,
    key: impl Fn(&T) -> K + 'static,
    view: impl Fn(Scope, ReadSignal<T>) -> V + 'static,
) -> ForEach<T, K>
where
    T: Clone + PartialEq + 'static,
    K: Eq + Hash + Clone + 'static,
    V: View,
{
    ForEach {
        items: items.into_items(),
        key: Box::new(key),
        view: Box::new(move |s, t| view(s, t).into_any()),
        max_items: u16::MAX,
    }
}

/// The view of [`for_each`].
#[must_use]
pub struct ForEach<T: 'static, K: 'static> {
    items: Items<T>,
    key: Box<dyn Fn(&T) -> K>,
    view: Box<dyn Fn(Scope, ReadSignal<T>) -> AnyView>,
    /// This collection's row bound ([`max_items`](Self::max_items)).
    max_items: u16,
}

impl<T, K> core::fmt::Debug for ForEach<T, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ForEach")
    }
}

/// One built row.
struct Row<T: 'static, K> {
    key: K,
    node: NodeId,
    scope: Scope,
    /// The row's item (owned by `scope`), lent read-only to the row's view.
    item: Signal<T>,
}

/// Reconciliation counts (logged).
#[derive(Default, Clone, Copy)]
struct Counts {
    kept: usize,
    moved: usize,
    created: usize,
    deleted: usize,
    /// Kept rows whose item signal was written.
    updated: usize,
    /// Items beyond the bound (not shown).
    cut: usize,
}

/// Adds `by` to a count of rows (bounded by the list lengths: it cannot overflow).
#[inline]
fn add(count: &mut usize, by: usize) {
    *count = count.saturating_add(by);
}

/// Writes `item` into a row's item signal when it differs from the value there
/// (`clone_from`: the old value's buffers are reused). Returns whether it was written.
///
/// Inside an effect a write only queues the subscribers: no user code runs here.
#[inline]
pub(super) fn sync_item<T: Clone + PartialEq + 'static>(sig: Signal<T>, item: &T) -> bool {
    if sig.with_untracked(|cur| cur == item) {
        return false;
    }
    sig.update(|cur| cur.clone_from(item));
    true
}

impl<T, K> ForEach<T, K> {
    /// Bounds this collection to `n` rows, below
    /// [`Limits::max_list_items`](twine_core::Limits::max_list_items) (which stays the
    /// ceiling: the bound is the smaller of the two). Default: the `Limits` value.
    ///
    /// With more items than the bound, the first `n` are shown and each reconcile raises a
    /// [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity) fault (code
    /// [`CapacityFault::ListItems`](twine_core::fault::CapacityFault::ListItems), `node` = the
    /// collection). With [`Limits::preallocate`](twine_core::Limits::preallocate) the row list
    /// and the reconcile scratch are reserved for the bound when the collection is built, so
    /// a small list does not reserve for the application-wide maximum. `0` shows nothing.
    /// Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn tabs(cx: Scope) -> impl View {
    ///     let names = cx.signal(vec!["Home", "Logs", "Setup"]);
    ///     row(for_each(names, |n| *n, |_, n| label(n.get_untracked())).max_items(4))
    /// }
    /// # let _ = tabs;
    /// ```
    pub fn max_items(mut self, n: u16) -> Self {
        self.max_items = n;
        self
    }
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + 'static> View for ForEach<T, K> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let wrapper = cx.create(Wrapper(&FOR_EACH_CLASS));
        if wrapper == twine_engine::DEAD_NODE {
            return wrapper; // not created (reported); nothing to keep up to date
        }
        let scope = cx.scope();
        let rt = scope.runtime();
        let limits = *cx.engine().limits();
        let bound = usize::from(self.max_items.min(limits.max_list_items));
        let hasher = DefaultHashBuilder::default();
        let state: Rc<RefCell<State<T, K>>> = Rc::new(RefCell::new(State::new()));
        if limits.preallocate {
            state.borrow_mut().reserve(bound);
        }
        let st = state.clone();
        cx.on_delete(wrapper, move || {
            // Taken out first: disposing runs user cleanups.
            let rows = core::mem::take(&mut st.borrow_mut().rows);
            for row in rows {
                row.scope.dispose();
            }
        });
        let ForEach { items, key, view, .. } = self;
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                match EngineAccess::with(rt, |e| e.tree().contains(wrapper)) {
                    None => return rt.defer_current_effect(),
                    Some(false) => return rt.dispose_current_effect(),
                    Some(true) => {}
                }
                let mut counts = Counts::default();
                let mut structural = false;
                // The read is tracked (the effect re-runs when the list changes); the keys,
                // comparisons and copies of the items are not.
                items.read(|new| {
                    rt.untrack(|| {
                        structural = diff(
                            &mut state.borrow_mut(),
                            new,
                            bound,
                            &*key,
                            hasher,
                            &mut counts,
                            rt.partition(),
                        );
                    });
                });
                if counts.cut > 0 {
                    report_cut(rt, wrapper, counts.cut);
                }
                if structural {
                    rt.untrack(|| apply(wrapper, scope, &state, &*view, &mut counts));
                }
                twine_core::debug!(
                    target: "twine::view",
                    "for_each reconcile: kept={} moved={} created={} deleted={} updated={}",
                    counts.kept,
                    counts.moved,
                    counts.created,
                    counts.deleted,
                    counts.updated
                );
            });
        });
        wrapper
    }
}

/// Raises the `ListItems` capacity fault of a collection that cut `n` items.
#[cold]
#[inline(never)]
pub(super) fn report_cut(rt: twine_reactive::Runtime, node: NodeId, n: usize) {
    twine_core::warn!(
        target: "twine::view",
        "{}: {} item(s) beyond the list bound not shown",
        fmt_node_id(node),
        n
    );
    EngineAccess::with(rt, |e| {
        e.raise_fault(
            twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
                .node(node)
                .code(twine_core::fault::CapacityFault::ListItems.code()),
        );
    });
}

/// Index-table / plan entry: the index part (rows and positions are bounded by
/// `Limits::max_list_items <= 65 535`, far below it).
const INDEX: u32 = (1 << 30) - 1;
/// Entry flag: the index is one of [`Scratch::fresh`] (a key not among the rows), not a row.
const NEW: u32 = 1 << 30;
/// Index-table flag: the row's key was matched by a position of the new list (a second match
/// is a duplicate key). Plan flag (same bit, other array): the row stays where it is.
const TAKEN: u32 = 1 << 31;
/// Plan flag: the kept row lies on the longest increasing run of old positions: it stays.
const STAY: u32 = 1 << 31;
/// No position (end of a LIS predecessor chain).
const NONE: u32 = u32::MAX;

/// An index as an entry (`INDEX` for an index that does not fit; never reached).
#[inline]
fn entry(i: usize) -> u32 {
    u32::try_from(i).map_or(INDEX, |i| i.min(INDEX))
}

/// The index part of an entry.
#[inline]
fn index_of(e: u32) -> usize {
    (e & INDEX) as usize
}

/// A collection's rows and its reconcile scratch, kept between reconciles (their capacity is
/// the high-water mark, or the bound when preallocated).
struct State<T: 'static, K> {
    rows: Vec<Row<T, K>>,
    scratch: Scratch<T, K>,
}

/// What a structural reconcile works with. Every buffer is cleared, not freed, after use:
/// once it reached its size (or was reserved for the bound), a keyed change allocates
/// nothing but the rows it creates.
struct Scratch<T: 'static, K> {
    /// Keys → rows (and keys → `fresh` entries, to find duplicate new keys). It stores
    /// entries only (a `u32` each), never a key: a key is compared through its row.
    index: HashTable<u32>,
    /// One entry per position of the new list (a row index, or `NEW | fresh index`),
    /// followed by the indices of the rows to delete.
    plan: Vec<u32>,
    /// The number of positions of the new list (the head of `plan`).
    positions: usize,
    /// Key and item copy of each new row, in position order.
    fresh: Vec<(K, T)>,
    /// The rows while they are put in the new order (`None` once placed or deleted).
    old: Vec<Option<Row<T, K>>>,
    /// Longest-increasing-run scratch: run tails and predecessors.
    tails: Vec<u32>,
    prev: Vec<u32>,
}

impl<T: 'static, K> State<T, K> {
    /// Empty; allocates nothing.
    fn new() -> Self {
        State {
            rows: Vec::new(),
            scratch: Scratch {
                index: HashTable::new(),
                plan: Vec::new(),
                positions: 0,
                fresh: Vec::new(),
                old: Vec::new(),
                tails: Vec::new(),
                prev: Vec::new(),
            },
        }
    }

    /// Reserves everything a reconcile of up to `bound` rows needs: at most `bound` rows on
    /// each side, so `2 · bound` index and plan entries (old rows plus new keys).
    fn reserve(&mut self, bound: usize) {
        let both = bound.saturating_mul(2);
        let s = &mut self.scratch;
        self.rows.reserve_exact(bound);
        // The table is empty: nothing to rehash.
        s.index.reserve(both, NO_REHASH);
        s.plan.reserve_exact(both);
        s.fresh.reserve_exact(bound);
        s.old.reserve_exact(bound);
        s.tails.reserve_exact(bound);
        s.prev.reserve_exact(bound);
    }
}

/// The hasher an index-table operation passes for a rehash. One type for every call (a
/// `dyn` reference), so the table's growth code (`reserve_rehash`) is instantiated once for
/// all collections and call sites rather than once per closure (flash); the dynamic call
/// only runs while the table grows, never on a lookup.
type Rehash<'a> = &'a dyn Fn(&u32) -> u64;

/// The [`Rehash`] of a reserve on an empty table (nothing to rehash).
const NO_REHASH: Rehash<'static> = &|_| 0;

/// The key of index-table entry `e`: a row's, or a new key's.
#[inline]
fn key_of<'a, T, K>(rows: &'a [Row<T, K>], fresh: &'a [(K, T)], e: u32) -> Option<&'a K> {
    if e & NEW == 0 {
        rows.get(index_of(e)).map(|r| &r.key)
    } else {
        fresh.get(index_of(e)).map(|f| &f.0)
    }
}

/// Compares the new items (the first `bound` of them) with the rows. Same keys in the same
/// order (the common case: values changed, or nothing did): the changed items are written
/// into their rows' signals, nothing is allocated and `false` is returned. Otherwise the
/// structural change is planned in the scratch (the items of kept rows are updated too) and
/// `true` is returned.
fn diff<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone>(
    st: &mut State<T, K>,
    new: &[T],
    bound: usize,
    key: &dyn Fn(&T) -> K,
    hasher: DefaultHashBuilder,
    counts: &mut Counts,
    part: twine_core::Partition,
) -> bool {
    let new = match new.get(..bound) {
        Some(head) if head.len() < new.len() => {
            counts.cut = new.len().saturating_sub(bound);
            head
        }
        _ => new,
    };
    // Fast path: walk both lists while the keys agree.
    let mut same = 0usize;
    for (row, item) in st.rows.iter().zip(new) {
        if key(item) != row.key {
            break;
        }
        add(&mut counts.updated, usize::from(sync_item(row.item, item)));
        add(&mut same, 1);
    }
    if same == st.rows.len() && same == new.len() {
        counts.kept = same;
        return false;
    }
    diff_keys(st, new, key, hasher, counts, part);
    true
}

/// The structural part of [`diff`]: matches the new keys with the rows (the first occurrence
/// of a duplicate wins) and writes the plan: the new positions, then the rows to delete (in
/// row order). Allocates only when the scratch is smaller than this change (noted as
/// `Reconcile` after a seal).
#[cold]
#[inline(never)]
fn diff_keys<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone>(
    st: &mut State<T, K>,
    new: &[T],
    key: &dyn Fn(&T) -> K,
    hasher: DefaultHashBuilder,
    counts: &mut Counts,
    part: twine_core::Partition,
) {
    let State {
        rows,
        scratch:
            Scratch {
                index,
                plan,
                positions,
                fresh,
                ..
            },
    } = st;
    index.clear();
    plan.clear();
    fresh.clear();
    let entries = rows.len().saturating_add(new.len());
    if index.capacity() < entries {
        twine_core::alloc_guard::grew(part, AllocSite::Reconcile);
        // Empty (cleared): nothing to rehash.
        index.reserve(entries, NO_REHASH);
    }
    alloc_guard::reserve(part, plan, entries, AllocSite::Reconcile);
    let hash = |k: Option<&K>| k.map_or(0, |k| hasher.hash_one(k));
    for (j, r) in rows.iter().enumerate() {
        let rehash: Rehash<'_> = &|&e| hash(key_of(rows, fresh, e));
        index.insert_unique(hasher.hash_one(&r.key), entry(j), rehash);
    }
    for item in new {
        let k = key(item);
        let h = hasher.hash_one(&k);
        match index.find_mut(h, |&e| key_of(rows, fresh, e) == Some(&k)) {
            Some(e) if *e & (TAKEN | NEW) != 0 => {
                twine_core::warn!(target: "twine::view", "for_each: duplicate key; item skipped");
            }
            Some(e) => {
                *e |= TAKEN;
                let j = *e & INDEX;
                if let Some(row) = rows.get(index_of(j)) {
                    add(&mut counts.updated, usize::from(sync_item(row.item, item)));
                }
                alloc_guard::push(part, plan, j, AllocSite::Reconcile);
            }
            None => {
                let f = entry(fresh.len());
                alloc_guard::push(part, fresh, (k, item.clone()), AllocSite::Reconcile);
                let rehash: Rehash<'_> = &|&e| hash(key_of(rows, fresh, e));
                index.insert_unique(h, NEW | TAKEN | f, rehash);
                alloc_guard::push(part, plan, NEW | f, AllocSite::Reconcile);
            }
        }
    }
    *positions = plan.len();
    // The rows no position took, in row order.
    for (j, r) in rows.iter().enumerate() {
        let me = entry(j);
        let taken = index
            .find(hasher.hash_one(&r.key), |&e| e & (NEW | INDEX) == me)
            .is_some_and(|&e| e & TAKEN != 0);
        if !taken {
            alloc_guard::push(part, plan, me, AllocSite::Reconcile);
        }
    }
    index.clear();
}

/// Applies the planned structural change: deletes the removed rows, then walks the new
/// positions from the first, placing each row right after the previous one — kept rows on a
/// longest increasing run of old positions stay, the other kept rows move, new rows are
/// built (their views with the engine lent to hooks).
fn apply<T: 'static, K>(
    wrapper: NodeId,
    scope: Scope,
    state: &RefCell<State<T, K>>,
    view: &dyn Fn(Scope, ReadSignal<T>) -> AnyView,
    counts: &mut Counts,
) {
    let rt = scope.runtime();
    let part = scope.partition();
    // Taken out for the time of the apply (moving the buffers allocates nothing): user code
    // runs meanwhile (cleanups, views) and may even delete the collection.
    let mut st = core::mem::replace(&mut *state.borrow_mut(), State::new());
    {
        let State {
            rows,
            scratch:
                Scratch {
                    plan,
                    positions,
                    fresh,
                    old,
                    tails,
                    prev,
                    ..
                },
        } = &mut st;
        let n = *positions;
        alloc_guard::reserve(part, old, rows.len(), AllocSite::Reconcile);
        old.extend(rows.drain(..).map(Some));

        // 1. Dispose and delete the rows that are gone (scope first, then node).
        EngineAccess::with(rt, |e| {
            for &j in plan.get(n..).unwrap_or(&[]) {
                if let Some(row) = old.get_mut(index_of(j)).and_then(Option::take) {
                    delete_row(e, row.scope, row.node);
                    add(&mut counts.deleted, 1);
                }
            }
        });

        // 2. The rows that stay in place.
        if let Some(head) = plan.get_mut(..n) {
            mark_stay(head, tails, prev, part);
        }

        // 3. Walk the positions, placing each row right after the previous one.
        alloc_guard::reserve(part, rows, n, AllocSite::Reconcile);
        EngineAccess::with(rt, |e| {
            let mut fresh = fresh.drain(..);
            let mut placed: Option<NodeId> = None;
            for &p in plan.get(..n).unwrap_or(&[]) {
                let before = match placed {
                    Some(prev) => e.tree().node(prev).and_then(twine_engine::Node::next_sibling),
                    None => e.tree().node(wrapper).and_then(twine_engine::Node::first_child),
                };
                let row = if p & NEW == 0 {
                    let Some(row) = old.get_mut(index_of(p)).and_then(Option::take) else {
                        continue;
                    };
                    if p & STAY != 0 || before == Some(row.node) {
                        add(&mut counts.kept, 1);
                    } else {
                        add(&mut counts.moved, 1);
                        let _ = e.move_node(row.node, wrapper, before);
                    }
                    row
                } else {
                    // `fresh` holds one entry per `NEW` position, in position order.
                    let Some((key, item)) = fresh.next() else {
                        continue;
                    };
                    add(&mut counts.created, 1);
                    build_row(e, wrapper, before, scope, key, item, view)
                };
                placed = Some(row.node);
                rows.push(row);
            }
        });
        // Empty unless the engine was unavailable (then the rows left are dropped).
        old.clear();
        fresh.clear();
        plan.clear();
    }
    *state.borrow_mut() = st;
}

/// Flags (`STAY`) the positions of `plan` whose kept rows lie on a longest increasing run of
/// old positions (patience sorting, O(n log n)); `NEW` positions are not on any run. `tails`
/// and `prev` are the retained scratch.
fn mark_stay(plan: &mut [u32], tails: &mut Vec<u32>, prev: &mut Vec<u32>, part: twine_core::Partition) {
    tails.clear();
    prev.clear();
    alloc_guard::reserve(part, tails, plan.len(), AllocSite::Reconcile);
    alloc_guard::reserve(part, prev, plan.len(), AllocSite::Reconcile);
    let old_at = |plan: &[u32], i: u32| {
        plan.get(i as usize)
            .filter(|&&e| e & NEW == 0)
            .map(|&e| e & INDEX)
    };
    for i in 0..plan.len() {
        let me = entry(i);
        let mut before = NONE;
        if let Some(v) = old_at(plan, me) {
            // tails[l] = position of the smallest tail of an increasing run of length l + 1.
            let pos = tails.partition_point(|&t| old_at(plan, t).is_some_and(|x| x < v));
            if let Some(&b) = pos.checked_sub(1).and_then(|q| tails.get(q)) {
                before = b;
            }
            // `pos <= tails.len()`: replace a tail, or extend by one.
            match tails.get_mut(pos) {
                Some(t) => *t = me,
                None => tails.push(me),
            }
        }
        prev.push(before);
    }
    let mut cur = tails.last().copied().unwrap_or(NONE);
    while let Some(e) = plan.get_mut(cur as usize) {
        *e |= STAY;
        cur = prev.get(cur as usize).copied().unwrap_or(NONE);
    }
}

/// Disposes a row's scope (with the engine lent to its cleanups), then deletes its node.
pub(super) fn delete_row(e: &mut Engine, scope: Scope, node: NodeId) {
    dispose_with(e, scope);
    if e.tree().contains(node) {
        let _ = e.delete(node);
    }
}

/// Builds a new row for `key` and `item` — its scope, item signal and view (created with the
/// engine lent to the view's hooks) — and inserts it before `next` (`None`: last).
fn build_row<T: 'static, K>(
    e: &mut Engine,
    wrapper: NodeId,
    next: Option<NodeId>,
    parent: Scope,
    key: K,
    item: T,
    view: &dyn Fn(Scope, ReadSignal<T>) -> AnyView,
) -> Row<T, K> {
    let scope = parent.child();
    let item = scope.signal(item);
    let v = EngineAccess::provide(parent, e, || view(scope, item.read_only()));
    let node = {
        let mut bcx = BuildCx::new(e, wrapper, scope);
        v.build(&mut bcx)
    };
    if next.is_some() {
        // The engine logs the reason.
        if e.move_node(node, wrapper, next).is_err() {
            twine_core::warn!(target: "twine::view", "for_each: cannot place row {}", fmt_node_id(node));
        }
    }
    Row {
        key,
        node,
        scope,
        item,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{NEW, STAY, mark_stay};

    /// The positions flagged `STAY` for `s` (`None` = a new row).
    fn run(s: &[Option<u32>]) -> Vec<bool> {
        let mut plan: Vec<u32> = s.iter().map(|v| v.unwrap_or(NEW)).collect();
        mark_stay(
            &mut plan,
            &mut Vec::new(),
            &mut Vec::new(),
            twine_core::Partition::MAIN,
        );
        plan.iter().map(|e| e & STAY != 0).collect()
    }

    #[test]
    fn mark_stay_finds_longest_run() {
        let s = [Some(0), Some(2), Some(1), Some(3)];
        assert_eq!(run(&s).iter().filter(|b| **b).count(), 3);
        let rev = [Some(3), Some(2), Some(1), Some(0)];
        assert_eq!(run(&rev).iter().filter(|b| **b).count(), 1);
        let gaps = [None, Some(1), None, Some(0), Some(2)];
        let l = run(&gaps);
        assert_eq!(l.iter().filter(|b| **b).count(), 2);
        assert!(!l[0] && !l[2]);
        assert!(run(&[]).is_empty());
    }
}
