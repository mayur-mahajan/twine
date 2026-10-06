//! [`for_each`]: a keyed list reusing, moving, inserting and deleting only what changed, with
//! one item signal per row updated in place.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::hash::Hash;

use hashbrown::HashMap;
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
/// **Cost.** Per row: a scope and one signal holding a copy of the item. Per change: when the
/// keys are those of the rows, in the same order (values changed, or nothing did), one pass
/// over the items comparing keys and values, allocation-free. Otherwise (insertions, removals,
/// moves) a hash map of the keys and a few vectors for the time of the reconcile — a constant
/// number of allocations, not one per row.
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
    }
}

/// The view of [`for_each`].
#[must_use]
pub struct ForEach<T: 'static, K: 'static> {
    items: Items<T>,
    key: Box<dyn Fn(&T) -> K>,
    view: Box<dyn Fn(Scope, ReadSignal<T>) -> AnyView>,
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

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + 'static> View for ForEach<T, K> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let wrapper = cx.create(Wrapper(&FOR_EACH_CLASS));
        if wrapper == twine_engine::DEAD_NODE {
            return wrapper; // not created (reported); nothing to keep up to date
        }
        let scope = cx.scope();
        let rt = scope.runtime();
        let rows: Rc<RefCell<Vec<Row<T, K>>>> = Rc::default();
        let r = rows.clone();
        cx.on_delete(wrapper, move || {
            for row in r.borrow_mut().drain(..) {
                row.scope.dispose();
            }
        });
        let ForEach { items, key, view } = self;
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                match EngineAccess::with(rt, |e| e.tree().contains(wrapper)) {
                    None => return rt.defer_current_effect(),
                    Some(false) => return rt.dispose_current_effect(),
                    Some(true) => {}
                }
                let mut counts = Counts::default();
                let mut plan = None;
                // The read is tracked (the effect re-runs when the list changes); the keys,
                // comparisons and copies of the items are not.
                items.read(|new| {
                    rt.untrack(|| plan = diff(&rows.borrow(), new, &*key, &mut counts));
                });
                if let Some(plan) = plan {
                    rt.untrack(|| apply(wrapper, scope, &rows, plan, &*view, &mut counts));
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

/// A position of the new list.
enum Slot<T: 'static, K> {
    /// Keep old row `j` (its item is up to date).
    Keep(usize),
    /// Build a row for a new key from a copy of its item.
    New(K, T),
    /// A new row's scope, item signal and view, ready to be built.
    Ready {
        key: K,
        scope: Scope,
        item: Signal<T>,
        view: AnyView,
    },
}

/// What [`apply`] does: the new list's positions and the old rows to delete.
struct Plan<T: 'static, K> {
    slots: Vec<Slot<T, K>>,
    removed: Vec<usize>,
}

/// Marks a key of the index as used by a position of the new list.
const TAKEN: usize = usize::MAX;

/// Compares the new items with the rows. Same keys in the same order (the common case: values
/// changed, or nothing did): the changed items are written into their rows' signals, nothing
/// is allocated and `None` is returned. Otherwise the [`Plan`] of the structural change (the
/// items of kept rows are updated too).
fn diff<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone>(
    rows: &[Row<T, K>],
    new: &[T],
    key: &dyn Fn(&T) -> K,
    counts: &mut Counts,
) -> Option<Plan<T, K>> {
    // Fast path: walk both lists while the keys agree.
    let mut same = 0;
    for (row, item) in rows.iter().zip(new) {
        if key(item) != row.key {
            break;
        }
        counts.updated += usize::from(sync_item(row.item, item));
        same += 1;
    }
    if same == rows.len() && same == new.len() {
        counts.kept = same;
        return None;
    }
    Some(diff_keys(rows, new, key, counts))
}

/// The structural part of [`diff`]: matches the new keys with the old rows (the first
/// occurrence of a duplicate wins).
#[cold]
#[inline(never)]
fn diff_keys<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone>(
    rows: &[Row<T, K>],
    new: &[T],
    key: &dyn Fn(&T) -> K,
    counts: &mut Counts,
) -> Plan<T, K> {
    let mut index: HashMap<K, usize> = HashMap::with_capacity(rows.len() + new.len());
    for (j, r) in rows.iter().enumerate() {
        index.insert(r.key.clone(), j);
    }
    let mut slots = Vec::with_capacity(new.len());
    for item in new {
        let k = key(item);
        match index.get_mut(&k) {
            Some(j) if *j == TAKEN => {
                twine_core::warn!(target: "twine::view", "for_each: duplicate key; item skipped");
            }
            Some(j) => {
                let old = core::mem::replace(j, TAKEN);
                counts.updated += usize::from(sync_item(rows[old].item, item));
                slots.push(Slot::Keep(old));
            }
            None => {
                index.insert(k.clone(), TAKEN);
                slots.push(Slot::New(k, item.clone()));
            }
        }
    }
    let removed = index.into_values().filter(|&j| j != TAKEN).collect();
    Plan { slots, removed }
}

/// Applies a structural [`Plan`]: deletes the removed rows, builds the new ones and moves the
/// kept rows that are out of order.
fn apply<T: 'static, K>(
    wrapper: NodeId,
    scope: Scope,
    rows: &RefCell<Vec<Row<T, K>>>,
    plan: Plan<T, K>,
    view: &dyn Fn(Scope, ReadSignal<T>) -> AnyView,
    counts: &mut Counts,
) {
    let rt = scope.runtime();
    let Plan { mut slots, removed } = plan;
    let mut old: Vec<Option<Row<T, K>>> = rows.borrow_mut().drain(..).map(Some).collect();

    // 1. Dispose and delete the rows that are gone (scope first, then node).
    EngineAccess::with(rt, |e| {
        for j in removed {
            if let Some(row) = old.get_mut(j).and_then(Option::take) {
                delete_row(e, row.scope, row.node);
                counts.deleted += 1;
            }
        }
    });

    // 2. The rows that stay in place: a longest increasing subsequence of old positions.
    let stay = lis(slots.len(), |i| match slots[i] {
        Slot::Keep(j) => Some(j),
        _ => None,
    });

    // 3. Views of new rows, created while the engine is available to hooks.
    for s in &mut slots {
        if let Slot::New(..) = s {
            let Slot::New(key, item) = core::mem::replace(s, Slot::Keep(usize::MAX)) else {
                continue;
            };
            let child = scope.child();
            let item = child.signal(item);
            let view = view(child, item.read_only());
            *s = Slot::Ready {
                key,
                scope: child,
                item,
                view,
            };
        }
    }

    // 4. Walk from the end, inserting before the following row.
    let mut new_rows: Vec<Row<T, K>> = Vec::with_capacity(slots.len());
    EngineAccess::with(rt, |e| {
        let mut next: Option<NodeId> = None;
        while let Some(s) = slots.pop() {
            let i = slots.len();
            let row = match s {
                Slot::Keep(j) => {
                    let Some(row) = old.get_mut(j).and_then(Option::take) else {
                        continue;
                    };
                    if stay[i] {
                        counts.kept += 1;
                    } else {
                        counts.moved += 1;
                        let _ = e.move_node(row.node, wrapper, next);
                    }
                    row
                }
                Slot::Ready {
                    key,
                    scope,
                    item,
                    view,
                } => {
                    counts.created += 1;
                    build_row(e, wrapper, next, key, scope, item, view)
                }
                // Turned into `Ready` above.
                Slot::New(..) => continue,
            };
            next = Some(row.node);
            new_rows.push(row);
        }
    });
    new_rows.reverse();
    *rows.borrow_mut() = new_rows;
}

/// Disposes a row's scope (with the engine lent to its cleanups), then deletes its node.
pub(super) fn delete_row(e: &mut Engine, scope: Scope, node: NodeId) {
    dispose_with(e, scope);
    if e.tree().contains(node) {
        let _ = e.delete(node);
    }
}

/// Builds a row from its view and inserts it before `next`.
fn build_row<T: 'static, K>(
    e: &mut Engine,
    wrapper: NodeId,
    next: Option<NodeId>,
    key: K,
    scope: Scope,
    item: Signal<T>,
    view: AnyView,
) -> Row<T, K> {
    let node = {
        let mut bcx = BuildCx::new(e, wrapper, scope);
        view.build(&mut bcx)
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

/// Which positions `0..len` lie on a longest increasing subsequence of the `Some` values of
/// `at` (patience sorting, O(n log n)).
pub(crate) fn lis(len: usize, at: impl Fn(usize) -> Option<usize>) -> Vec<bool> {
    // tails[l] = position of the smallest tail of an increasing run of length l + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<usize> = vec![usize::MAX; len];
    for (i, p) in prev.iter_mut().enumerate() {
        let Some(v) = at(i) else { continue };
        let pos = tails.partition_point(|&t| at(t).is_some_and(|x| x < v));
        if pos > 0 {
            *p = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
    }
    let mut out = vec![false; len];
    let mut cur = tails.last().copied().unwrap_or(usize::MAX);
    while let Some(flag) = out.get_mut(cur) {
        *flag = true;
        cur = prev[cur];
    }
    out
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::lis;

    fn run(s: &[Option<usize>]) -> Vec<bool> {
        lis(s.len(), |i| s[i])
    }

    #[test]
    fn lis_finds_longest_run() {
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
