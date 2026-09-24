//! [`for_each`]: a keyed list reusing, moving, inserting and deleting only what changed.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::hash::Hash;

use hashbrown::{HashMap, HashSet};
use twine_engine::{Engine, NodeId, fmt_node_id};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect, untrack};

use super::{FOR_EACH_CLASS, Wrapper, dispose_with};
use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::view::{AnyView, IntoAnyView, View};

/// A keyed list: one row per item, identified by `key(item)`.
///
/// When `items()` changes the rows are reconciled with the new keys: rows of kept keys are
/// **reused** (not rebuilt), rows of removed keys are disposed and deleted, rows of new keys
/// are built, and rows out of order are moved — the minimal number of moves (everything on a
/// longest increasing subsequence of the old positions stays where it is). To react to value
/// changes inside a kept row, give items signals (or use
/// [`rebuild_on_change`](ForEach::rebuild_on_change)).
///
/// Duplicate keys are a user error: the first occurrence wins, the others are skipped (and
/// logged with `warn!`).
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let items = cx.signal(vec![(1u32, "one"), (2, "two")]);
///     column(for_each(move || items.get(), |(id, _)| *id, |_cx, (_, name)| label(name)))
/// }
/// # let _ = app;
/// ```
pub fn for_each<T: 'static, K: Eq + Hash + Clone + 'static, V: View>(
    items: impl Fn() -> Vec<T> + 'static,
    key: impl Fn(&T) -> K + 'static,
    view: impl Fn(Scope, T) -> V + 'static,
) -> ForEach<T, K> {
    ForEach {
        items: Box::new(items),
        key: Box::new(key),
        view: Box::new(move |s, t| view(s, t).into_any()),
        keep: None,
    }
}

/// Clones an item and compares two items ([`ForEach::rebuild_on_change`]).
struct Keep<T> {
    clone: fn(&T) -> T,
    eq: fn(&T, &T) -> bool,
}

/// The view of [`for_each`].
#[must_use]
pub struct ForEach<T: 'static, K: 'static> {
    items: Box<dyn Fn() -> Vec<T>>,
    key: Box<dyn Fn(&T) -> K>,
    view: Box<dyn Fn(Scope, T) -> AnyView>,
    keep: Option<Keep<T>>,
}

impl<T, K> core::fmt::Debug for ForEach<T, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ForEach")
    }
}

impl<T: PartialEq + Clone + 'static, K: 'static> ForEach<T, K> {
    /// Also rebuilds a kept row (in place) when its item's value changed. Keeps a copy of
    /// every item.
    pub fn rebuild_on_change(mut self) -> Self {
        self.keep = Some(Keep {
            clone: T::clone,
            eq: T::eq,
        });
        self
    }
}

/// One built row.
struct Row<T, K> {
    key: K,
    node: NodeId,
    scope: Scope,
    /// The item it was built from (only with `rebuild_on_change`).
    value: Option<T>,
}

/// Reconciliation counts (logged).
#[derive(Default, Clone, Copy)]
struct Counts {
    kept: usize,
    moved: usize,
    created: usize,
    deleted: usize,
}

impl<T: 'static, K: Eq + Hash + Clone + 'static> View for ForEach<T, K> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let wrapper = cx.create(Wrapper(&FOR_EACH_CLASS));
        let scope = cx.scope();
        let rows: Rc<RefCell<Vec<Row<T, K>>>> = Rc::default();
        let r = rows.clone();
        cx.on_delete(wrapper, move || {
            for row in r.borrow_mut().drain(..) {
                row.scope.dispose();
            }
        });
        let ForEach {
            items,
            key,
            view,
            keep,
        } = self;
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                let new_items = items();
                match EngineAccess::with(|e| e.tree().contains(wrapper)) {
                    None => return defer_current_effect(),
                    Some(false) => return dispose_current_effect(),
                    Some(true) => {}
                }
                untrack(|| {
                    let c = reconcile(wrapper, scope, &rows, new_items, &*key, &*view, keep.as_ref());
                    twine_core::debug!(
                        target: "twine::view",
                        "for_each reconcile: kept={} moved={} created={} deleted={}",
                        c.kept,
                        c.moved,
                        c.created,
                        c.deleted
                    );
                });
            });
        });
        wrapper
    }
}

/// What to do for one position of the new list.
enum Slot<T> {
    /// Reuse old row `j` (rebuild it in place when `rebuild`).
    Old { j: usize, rebuild: Option<T> },
    /// Build a new row.
    New(T),
}

fn reconcile<T: 'static, K: Eq + Hash + Clone + 'static>(
    wrapper: NodeId,
    scope: Scope,
    rows: &RefCell<Vec<Row<T, K>>>,
    new_items: Vec<T>,
    key: &dyn Fn(&T) -> K,
    view: &dyn Fn(Scope, T) -> AnyView,
    keep: Option<&Keep<T>>,
) -> Counts {
    let mut counts = Counts::default();
    let mut old: Vec<Option<Row<T, K>>> = rows.borrow_mut().drain(..).map(Some).collect();
    let old_index: HashMap<K, usize> = old
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.as_ref().map(|r| (r.key.clone(), i)))
        .collect();

    // 1. Match the new keys with the old rows (first occurrence of a duplicate wins).
    let mut seen: HashSet<K> = HashSet::with_capacity(new_items.len());
    let mut slots: Vec<(K, Slot<T>)> = Vec::with_capacity(new_items.len());
    for item in new_items {
        let k = key(&item);
        if !seen.insert(k.clone()) {
            twine_core::warn!(target: "twine::view", "for_each: duplicate key; item skipped");
            continue;
        }
        match old_index.get(&k) {
            Some(&j) => {
                let rebuild = keep.and_then(|kp| {
                    let prev = old[j].as_ref().and_then(|r| r.value.as_ref());
                    match prev {
                        Some(p) if (kp.eq)(p, &item) => None,
                        _ => Some(item),
                    }
                });
                slots.push((k, Slot::Old { j, rebuild }));
            }
            None => slots.push((k, Slot::New(item))),
        }
    }
    let mut reused = vec![false; old.len()];
    for (_, s) in &slots {
        if let Slot::Old { j, .. } = s {
            reused[*j] = true;
        }
    }

    // 2. Dispose and delete the rows that are gone (scope first, then node).
    EngineAccess::with(|e| {
        for (j, r) in old.iter_mut().enumerate() {
            if !reused[j] {
                if let Some(row) = r.take() {
                    dispose_with(e, row.scope);
                    if e.tree().contains(row.node) {
                        let _ = e.delete(row.node);
                    }
                    counts.deleted += 1;
                }
            }
        }
    });

    // 3. The rows that stay in place: a longest increasing subsequence of old positions.
    let old_pos: Vec<Option<usize>> = slots
        .iter()
        .map(|(_, s)| match s {
            Slot::Old { j, rebuild: None } => Some(*j),
            _ => None,
        })
        .collect();
    let stay = lis(&old_pos);

    // 4. Views of new (and rebuilt) rows, created while the engine is available to hooks.
    let plan: Vec<(K, Plan<T>)> = slots
        .into_iter()
        .map(|(k, s)| {
            let build = |item: T, replaces: Option<usize>| {
                let child = scope.child();
                let copy = keep.map(|kp| (kp.clone)(&item));
                let view = view(child, item);
                Plan::Build {
                    child,
                    view,
                    copy,
                    replaces,
                }
            };
            let p = match s {
                Slot::Old { j, rebuild: None } => Plan::Keep(j),
                Slot::Old { j, rebuild: Some(t) } => build(t, Some(j)),
                Slot::New(t) => build(t, None),
            };
            (k, p)
        })
        .collect();

    // 5. Walk from the end, inserting before the following row.
    let n = plan.len();
    let mut new_rows: Vec<Option<Row<T, K>>> = (0..n).map(|_| None).collect();
    let mut plan = plan;
    EngineAccess::with(|e| {
        let mut next: Option<NodeId> = None;
        while let Some((k, p)) = plan.pop() {
            let i = plan.len();
            let row = match p {
                Plan::Keep(j) => {
                    let Some(row) = old[j].take() else { continue };
                    if stay.contains(&i) {
                        counts.kept += 1;
                    } else {
                        counts.moved += 1;
                        let _ = e.move_node(row.node, wrapper, next);
                    }
                    row
                }
                Plan::Build {
                    child,
                    view,
                    copy,
                    replaces,
                } => {
                    if let Some(old_row) = replaces.and_then(|j| old[j].take()) {
                        dispose_with(e, old_row.scope);
                        if e.tree().contains(old_row.node) {
                            let _ = e.delete(old_row.node);
                        }
                    }
                    counts.created += 1;
                    build_row(e, wrapper, next, k, child, view, copy)
                }
            };
            next = Some(row.node);
            new_rows[i] = Some(row);
        }
    });
    *rows.borrow_mut() = new_rows.into_iter().flatten().collect();
    counts
}

/// How a position of the new list is filled.
enum Plan<T> {
    /// Reuse old row `j`.
    Keep(usize),
    /// Build a row from `view` in scope `child` (replacing old row `replaces`).
    Build {
        child: Scope,
        view: AnyView,
        copy: Option<T>,
        replaces: Option<usize>,
    },
}

/// Builds a row from its view and inserts it before `next`.
#[allow(clippy::too_many_arguments)]
fn build_row<T, K>(
    e: &mut Engine,
    wrapper: NodeId,
    next: Option<NodeId>,
    key: K,
    scope: Scope,
    view: AnyView,
    value: Option<T>,
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
        value,
    }
}

/// Positions (indices into `seq`) of a longest increasing subsequence of the `Some` values
/// (patience sorting, O(n log n)).
pub(crate) fn lis(seq: &[Option<usize>]) -> HashSet<usize> {
    // tails[l] = index into `seq` of the smallest tail of an increasing run of length l + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
    for (i, v) in seq.iter().enumerate() {
        let Some(v) = *v else { continue };
        let pos = tails.partition_point(|&t| seq[t].is_some_and(|x| x < v));
        if pos > 0 {
            prev[i] = Some(tails[pos - 1]);
        }
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
    }
    let mut out = HashSet::with_capacity(tails.len());
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        out.insert(i);
        cur = prev[i];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::lis;

    #[test]
    fn lis_finds_longest_run() {
        let s = [Some(0), Some(2), Some(1), Some(3)];
        assert_eq!(lis(&s).len(), 3);
        let rev = [Some(3), Some(2), Some(1), Some(0)];
        assert_eq!(lis(&rev).len(), 1);
        let gaps = [None, Some(1), None, Some(0), Some(2)];
        let l = lis(&gaps);
        assert_eq!(l.len(), 2);
        assert!(!l.contains(&0) && !l.contains(&2));
    }
}
