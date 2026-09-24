//! Keyed lists (`for_each`) and `virtual_list`.

use std::cell::Cell;
use std::rc::Rc;

use proptest::prelude::*;
use twine_core::Point;
use twine_engine::NodeId;
use twine_reactive::debug_stats;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_class, by_id, capture_logs};
use twine_view::prelude::*;
use twine_widgets::label::Label;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A list of labels keyed by their number; the list signal is provided in the root scope.
fn list(initial: Vec<u32>) -> TestUi {
    let mut t = TestUi::new(200, 400).mount(move |cx| {
        let items = cx.signal(initial);
        cx.provide(items);
        column(for_each(
            move || items.get(),
            |k| *k,
            |_cx, k| label(format!("{k}")),
        ))
        .gap(0)
        .test_id("col")
    });
    t.run_until_idle();
    t
}

fn items(t: &TestUi) -> Signal<Vec<u32>> {
    t.root_scope().expect_context::<Signal<Vec<u32>>>()
}

/// `(text, node)` of the rows in tree order.
fn rows(t: &TestUi) -> Vec<(String, NodeId)> {
    let e = t.engine();
    let col = t.find(by_id("col")).id();
    let wrapper = e.tree().children(col).next().unwrap();
    e.tree()
        .children(wrapper)
        .map(|n| (e.widget::<Label>(n).unwrap().text().to_owned(), n))
        .collect()
}

/// Sets the items and returns the reconcile log line.
fn set(t: &mut TestUi, v: Vec<u32>) -> String {
    let s = items(t);
    let ((), logs) = capture_logs(|| {
        s.set(v);
        t.run_until_idle();
    });
    logs.into_iter()
        .map(|l| l.message)
        .find(|m| m.starts_with("for_each reconcile"))
        .unwrap_or_default()
}

fn texts(t: &TestUi) -> Vec<String> {
    rows(t).into_iter().map(|r| r.0).collect()
}

#[test]
fn for_each_append_creates_only_new() {
    let mut t = list(vec![1, 2, 3]);
    let before = rows(&t);
    let log = set(&mut t, vec![1, 2, 3, 4]);
    assert_eq!(log, "for_each reconcile: kept=3 moved=0 created=1 deleted=0");
    let after = rows(&t);
    assert_eq!(texts(&t), ["1", "2", "3", "4"]);
    assert_eq!(&after[..3], &before[..], "old rows reused");
}

#[test]
fn for_each_remove_middle_deletes_one() {
    let mut t = list(vec![1, 2, 3, 4]);
    let log = set(&mut t, vec![1, 2, 4]);
    assert_eq!(log, "for_each reconcile: kept=3 moved=0 created=0 deleted=1");
    assert_eq!(texts(&t), ["1", "2", "4"]);
}

#[test]
fn for_each_reverse_moves_n_minus_1() {
    let mut t = list(vec![1, 2, 3, 4, 5]);
    let log = set(&mut t, vec![5, 4, 3, 2, 1]);
    assert_eq!(log, "for_each reconcile: kept=1 moved=4 created=0 deleted=0");
    assert_eq!(texts(&t), ["5", "4", "3", "2", "1"]);
}

#[test]
fn for_each_swap_two_moves_minimal() {
    let mut t = list(vec![1, 2, 3, 4]);
    let log = set(&mut t, vec![1, 3, 2, 4]);
    assert_eq!(log, "for_each reconcile: kept=3 moved=1 created=0 deleted=0");
    assert_eq!(texts(&t), ["1", "3", "2", "4"]);
    // Laid out in the new order.
    let r = rows(&t);
    let e = t.engine();
    assert!(e.coords(r[1].1).y0 < e.coords(r[2].1).y0);
}

#[test]
fn for_each_duplicate_keys_warn() {
    let mut t = list(vec![1, 2]);
    let s = items(&t);
    let ((), logs) = capture_logs(|| {
        s.set(vec![1, 2, 2, 3]);
        t.run_until_idle();
    });
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("duplicate key"))
    );
    assert_eq!(texts(&t), ["1", "2", "3"]);
}

#[test]
fn for_each_rebuild_on_change_rebuilds_changed_only() {
    let builds = Rc::new(Cell::new(0));
    let b = builds.clone();
    let mut t = TestUi::new(200, 200).mount(move |cx| {
        let items = cx.signal(vec![(1u32, "a"), (2, "b"), (3, "c")]);
        cx.provide(items);
        let b = b.clone();
        column(
            for_each(
                move || items.get(),
                |(k, _)| *k,
                move |_cx, (_, v)| {
                    b.set(b.get() + 1);
                    label(v)
                },
            )
            .rebuild_on_change(),
        )
        .test_id("col")
    });
    t.run_until_idle();
    assert_eq!(builds.get(), 3);
    let items = t
        .root_scope()
        .expect_context::<Signal<Vec<(u32, &'static str)>>>();
    items.set(vec![(1, "a"), (2, "B"), (3, "c")]);
    t.run_until_idle();
    assert_eq!(builds.get(), 4, "only the changed row was rebuilt");
    assert_eq!(texts(&t), ["a", "B", "c"]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn for_each_matches_model(lists in prop::collection::vec(
        prop::collection::btree_set(0u32..12, 0..8).prop_map(|s| {
            let mut v: Vec<u32> = s.into_iter().collect();
            // Some order other than sorted.
            v.reverse();
            let mid = v.len() / 2;
            if v.len() > 2 { v.swap(0, mid); }
            v
        }),
        1..6,
    )) {
        let mut t = list(Vec::new());
        let base = debug_stats().scopes;
        for l in lists {
            let s = items(&t);
            s.set(l.clone());
            t.run_until_idle();
            let got: Vec<String> = texts(&t);
            let want: Vec<String> = l.iter().map(ToString::to_string).collect();
            prop_assert_eq!(got, want);
            prop_assert_eq!(debug_stats().scopes, base + l.len(), "one scope per row, none leaked");
        }
    }
}

fn big_list(_cx: Scope) -> impl View {
    virtual_list(|| 10_000, 30, |_cx, i| label(format!("Row {i}")))
        .size(200, 240)
        .test_id("vl")
}

#[test]
fn virtual_list_builds_visible_rows_only() {
    let mut t = TestUi::new(200, 240).mount(big_list);
    t.run_until_idle();
    let labels = t.find_all(by_class("label"));
    assert!(!labels.is_empty() && labels.len() <= 10, "{} rows", labels.len());
    assert_eq!(labels[0].text(), "Row 0");
}

#[test]
fn virtual_list_scroll_recycles() {
    let mut t = TestUi::new(200, 240).mount(big_list);
    t.run_until_idle();
    let vl = t.find(by_id("vl")).id();
    t.engine_mut().scroll_to_y(vl, 30 * 500, false);
    t.run_until_idle();
    let labels: Vec<String> = t
        .find_all(by_class("label"))
        .iter()
        .map(twine_testing::NodeHandle::text)
        .collect();
    assert!(labels.len() <= 10);
    assert!(labels.contains(&"Row 500".to_string()), "{labels:?}");
    assert!(!labels.contains(&"Row 0".to_string()));
    // Drag scrolling works too.
    t.drag(Point::new(100, 200), Point::new(100, 50), Duration::ms(300));
    t.run_until_idle();
    assert!(t.find_all(by_class("label")).len() <= 10);
}

#[test]
fn virtual_list_memory_constant() {
    let mut t = TestUi::new(200, 240).mount(big_list);
    t.run_until_idle();
    let vl = t.find(by_id("vl")).id();
    // Warm up the pools (row vectors, caches).
    for i in 0..5 {
        t.engine_mut().scroll_to_y(vl, 30 * i * 10, false);
        t.run_until_idle();
    }
    t.engine_mut().scroll_to_y(vl, 0, false);
    t.run_until_idle();
    let ((), stats) = count_allocs(|| {
        let mut y = 0;
        while y < 30 * 10_000 {
            t.engine_mut().scroll_to_y(vl, y, false);
            // One frame per step (the test engine logs invalidations until a frame starts).
            t.advance(Duration::ms(40));
            y += 240 * 5;
        }
        t.engine_mut().scroll_to_y(vl, 0, false);
        t.run_until_idle();
    });
    assert!(stats.live.abs() <= 1024, "heap grew by {} bytes", stats.live);
}
