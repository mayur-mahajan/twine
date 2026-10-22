//! Keyed lists (`for_each`) and `virtual_list`: reconciliation, item signals updated in place
//! (binding runs and allocations per change), sources read without cloning.

use std::cell::Cell;
use std::rc::Rc;

use proptest::prelude::*;
use twine_core::Point;
use twine_engine::NodeId;
use twine_reactive::Runtime;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_class, by_id, capture_logs};
use twine_view::prelude::*;
use twine_widgets::label::Label;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A list of labels keyed by their number; the list signal is provided in the root scope.
fn list(initial: Vec<u32>) -> TestUi {
    let mut t = TestUi::new(200, 400).mount(move |cx| {
        let items = cx.signal(initial);
        cx.provide(items);
        column(for_each(items, |k| *k, |_cx, k| label(text!("{}", k.get()))))
            .gap(0)
            .test_id("col")
    });
    t.run_until_idle();
    t
}

fn items(t: &TestUi) -> Signal<Vec<u32>> {
    t.root_scope().use_context::<Signal<Vec<u32>>>().unwrap()
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
    assert_eq!(
        log,
        "for_each reconcile: kept=3 moved=0 created=1 deleted=0 updated=0"
    );
    let after = rows(&t);
    assert_eq!(texts(&t), ["1", "2", "3", "4"]);
    assert_eq!(&after[..3], &before[..], "old rows reused");
}

#[test]
fn for_each_remove_middle_deletes_one() {
    let mut t = list(vec![1, 2, 3, 4]);
    let log = set(&mut t, vec![1, 2, 4]);
    assert_eq!(
        log,
        "for_each reconcile: kept=3 moved=0 created=0 deleted=1 updated=0"
    );
    assert_eq!(texts(&t), ["1", "2", "4"]);
}

#[test]
fn for_each_reverse_moves_n_minus_1() {
    let mut t = list(vec![1, 2, 3, 4, 5]);
    let log = set(&mut t, vec![5, 4, 3, 2, 1]);
    assert_eq!(
        log,
        "for_each reconcile: kept=1 moved=4 created=0 deleted=0 updated=0"
    );
    assert_eq!(texts(&t), ["5", "4", "3", "2", "1"]);
}

#[test]
fn for_each_swap_two_moves_minimal() {
    let mut t = list(vec![1, 2, 3, 4]);
    let log = set(&mut t, vec![1, 3, 2, 4]);
    assert_eq!(
        log,
        "for_each reconcile: kept=3 moved=1 created=0 deleted=0 updated=0"
    );
    assert_eq!(texts(&t), ["1", "3", "2", "4"]);
    // Laid out in the new order.
    let r = rows(&t);
    let e = t.engine();
    assert!(e.coords(r[1].1).y0 < e.coords(r[2].1).y0);
}

#[test]
fn for_each_same_keys_is_a_single_pass() {
    let mut t = list(vec![1, 2, 3]);
    let before = rows(&t);
    let log = set(&mut t, vec![1, 2, 3]);
    assert_eq!(
        log,
        "for_each reconcile: kept=3 moved=0 created=0 deleted=0 updated=0"
    );
    assert_eq!(rows(&t), before);
}

#[test]
fn for_each_duplicate_keys_warn() {
    let mut t = list(vec![1, 2]);
    let s = items(&t);
    let ((), logs) = capture_logs(|| {
        s.set(vec![1, 2, 2, 3, 1]);
        t.run_until_idle();
    });
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("duplicate key"))
    );
    assert_eq!(texts(&t), ["1", "2", "3"]);
    // A duplicate of a new key is skipped too, and the rows stay consistent afterwards.
    let ((), _) = capture_logs(|| {
        s.set(vec![7, 7, 1]);
        t.run_until_idle();
    });
    assert_eq!(texts(&t), ["7", "1"]);
    s.set(vec![1]);
    t.run_until_idle();
    assert_eq!(texts(&t), ["1"]);
}

/// Sensor readings: `(row, value)`.
type Readings = Channel<(usize, i32), 8>;

/// A sensor table: `(id, value)` rows, the row label formatting the value in place; readings
/// arrive through a channel (handled inside the update, like a real sensor task's).
struct Table {
    t: TestUi,
    values: Signal<Vec<(u32, i32)>>,
    sensor: &'static Readings,
    builds: Rc<Cell<u32>>,
    formats: Rc<Cell<u32>>,
}

fn table(n: u32) -> Table {
    let builds = Rc::new(Cell::new(0));
    let formats = Rc::new(Cell::new(0));
    let (b, f) = (builds.clone(), formats.clone());
    let sensor: &'static Readings = TestUi::channel();
    let mut t = TestUi::new(200, 400).mount(move |cx| {
        let values = cx.signal((0..n).map(|i| (i, 0i32)).collect::<Vec<_>>());
        cx.provide(values);
        cx.on_message(sensor, move |(row, value)| {
            values.update(|v| {
                if let Some(r) = v.get_mut(row) {
                    r.1 = value;
                }
            });
        });
        let (b, f) = (b.clone(), f.clone());
        column(for_each(
            values,
            |(id, _)| *id,
            move |_cx, v| {
                b.set(b.get() + 1);
                let f = f.clone();
                label(text!("{}", {
                    f.set(f.get() + 1);
                    v.with(|(_, value)| *value)
                }))
            },
        ))
        .test_id("col")
    });
    t.run_until_idle();
    let values = t.root_scope().use_context::<Signal<Vec<(u32, i32)>>>().unwrap();
    Table {
        t,
        values,
        sensor,
        builds,
        formats,
    }
}

impl Table {
    /// Sends the readings and runs the update that handles them.
    fn read(&mut self, readings: &[(usize, i32)]) {
        for r in readings {
            self.sensor.try_send(*r).unwrap();
        }
        self.t.run_until_idle();
    }
}

#[test]
fn for_each_200_values_3_changes_cost_3_binding_runs() {
    let mut tb = table(200);
    assert_eq!((tb.builds.get(), tb.formats.get()), (200, 200));
    let rows_before = rows(&tb.t);
    let runs = rt().stats().effect_runs;
    tb.read(&[(3, 30), (100, 1000), (199, 1990)]);
    // The reconcile itself and exactly the three label bindings.
    assert_eq!(rt().stats().effect_runs - runs, 1 + 3);
    assert_eq!(tb.formats.get(), 203, "3 binding runs");
    assert_eq!(tb.builds.get(), 200, "no row rebuilt");
    let r = rows(&tb.t);
    assert_eq!(
        r.iter().map(|x| x.1).collect::<Vec<_>>(),
        rows_before.iter().map(|x| x.1).collect::<Vec<_>>(),
        "same nodes"
    );
    assert_eq!(
        (r[3].0.as_str(), r[100].0.as_str(), r[199].0.as_str()),
        ("30", "1000", "1990")
    );
    assert_eq!(r[4].0, "0");
}

#[test]
fn for_each_value_changes_allocate_nothing() {
    let mut tb = table(200);
    // Warm up (label buffers, effect queues).
    for k in 1..3 {
        tb.read(&[(7, k), (150, k), (42, k)]);
    }
    // A reading equal to the current value: the list is written (notified) but unchanged.
    let ((), unchanged) = count_allocs(|| tb.read(&[(7, 2)]));
    assert_eq!(unchanged.allocs, 0, "unchanged list: {unchanged:?}");
    let ((), changed) = count_allocs(|| tb.read(&[(7, 5), (150, 9), (42, 1)]));
    assert_eq!(changed.allocs, 0, "3 values changed: {changed:?}");
    assert!(tb.values.with_untracked(|v| v[150].1 == 9));
    assert_eq!(texts(&tb.t)[150], "9");
}

/// An item counting its clones.
#[derive(PartialEq)]
struct Counted {
    id: u32,
    value: u32,
}

thread_local! {
    static CLONES: Cell<u32> = const { Cell::new(0) };
}

impl Clone for Counted {
    fn clone(&self) -> Self {
        CLONES.with(|c| c.set(c.get() + 1));
        Counted {
            id: self.id,
            value: self.value,
        }
    }
}

#[test]
fn for_each_signal_items_are_read_in_place() {
    let mut t = TestUi::new(200, 400).mount(|cx| {
        let items = cx.signal((0..50).map(|id| Counted { id, value: 0 }).collect::<Vec<_>>());
        cx.provide(items);
        column(for_each(
            items,
            |c| c.id,
            |_cx, c| label(text!("{}", c.with(|c| c.value))),
        ))
    });
    t.run_until_idle();
    let items = t.root_scope().use_context::<Signal<Vec<Counted>>>().unwrap();
    let before = CLONES.with(Cell::get);
    items.update(|v| v[10].value = 1);
    t.run_until_idle();
    assert_eq!(
        CLONES.with(Cell::get) - before,
        1,
        "only the changed item is copied (into its row), the list is never cloned"
    );
    // A new item is copied once, into its new row.
    let before = CLONES.with(Cell::get);
    items.update(|v| v.push(Counted { id: 99, value: 0 }));
    t.run_until_idle();
    assert_eq!(CLONES.with(Cell::get) - before, 1);
}

#[test]
fn for_each_field_projection_runs_only_for_its_field() {
    #[derive(Clone, PartialEq)]
    struct Item {
        id: u32,
        name: &'static str,
        value: i32,
    }
    let name_runs = Rc::new(Cell::new(0));
    let n = name_runs.clone();
    let mut t = TestUi::new(200, 200).mount(move |cx| {
        let items = cx.signal(vec![
            Item {
                id: 1,
                name: "a",
                value: 0,
            },
            Item {
                id: 2,
                name: "b",
                value: 0,
            },
        ]);
        cx.provide(items);
        let n = n.clone();
        column(for_each(
            items,
            |i| i.id,
            move |_cx, item| {
                let name = item.map(|i| i.name);
                let value = item.map(|i| i.value);
                let n = n.clone();
                row((
                    label(move || {
                        n.set(n.get() + 1);
                        name.get()
                    }),
                    label(text!("{}", value.get())),
                ))
            },
        ))
        .test_id("col")
    });
    t.run_until_idle();
    assert_eq!(name_runs.get(), 2);
    let items = t.root_scope().use_context::<Signal<Vec<Item>>>().unwrap();
    items.update(|v| v[0].value = 5);
    t.run_until_idle();
    assert_eq!(name_runs.get(), 2, "the name binding did not run");
    items.update(|v| v[1].name = "B");
    t.run_until_idle();
    assert_eq!(name_runs.get(), 3);
    let labels: Vec<String> = t
        .find_all(by_class("label"))
        .iter()
        .map(twine_testing::NodeHandle::text)
        .collect();
    assert_eq!(labels, ["a", "5", "B", "0"]);
}

#[test]
fn for_each_closure_and_constant_sources() {
    static NAMES: [&str; 3] = ["x", "y", "z"];
    let mut t = TestUi::new(200, 300).mount(|cx| {
        let n = cx.signal(2usize);
        cx.provide(n);
        column((
            for_each(&NAMES, |s| *s, |_cx, s| label(s)),
            for_each(
                move || (0..n.get()).map(|i| i * 10).collect::<Vec<_>>(),
                |i| *i,
                |_cx, i| label(text!("{}", i.get())),
            ),
            for_each(vec![7u8], |i| *i, |_cx, i| label(text!("{}", i.get()))),
        ))
    });
    t.run_until_idle();
    let texts = |t: &TestUi| -> Vec<String> {
        t.find_all(by_class("label"))
            .iter()
            .map(twine_testing::NodeHandle::text)
            .collect()
    };
    assert_eq!(texts(&t), ["x", "y", "z", "0", "10", "7"]);
    t.root_scope().use_context::<Signal<usize>>().unwrap().set(3);
    t.run_until_idle();
    assert_eq!(texts(&t), ["x", "y", "z", "0", "10", "20", "7"]);
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
        let base = rt().stats().scopes;
        let nodes = rt().stats().nodes;
        for l in lists {
            let s = items(&t);
            s.set(l.clone());
            t.run_until_idle();
            let got: Vec<String> = texts(&t);
            let want: Vec<String> = l.iter().map(ToString::to_string).collect();
            prop_assert_eq!(got, want);
            prop_assert_eq!(rt().stats().scopes, base + l.len(), "one scope per row, none leaked");
            // Per row: the item signal and the label binding.
            prop_assert_eq!(rt().stats().nodes, nodes + 2 * l.len(), "no node leaked");
        }
    }
}

fn big_list(_cx: Scope) -> impl View {
    let rows: Vec<u32> = (0..10_000).collect();
    virtual_list(rows, |i| *i, 30, |_cx, i| label(text!("Row {}", i.get())))
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

/// A row height below 1 px (a divisor of the visible-row lookup) is logged and taken as 1 px:
/// no division by zero, the list still builds and scrolls (R5.S10).
/// req: SAF-033
#[test]
fn regression_virtual_list_zero_row_height_is_one_px() {
    for height in [0, -5] {
        let (mut t, logs) = capture_logs(|| {
            let mut t = TestUi::new(200, 240).mount(move |_cx| {
                let rows: Vec<u32> = (0..1_000).collect();
                virtual_list(rows, |i| *i, height, |_cx, i| label(text!("Row {}", i.get())))
                    .size(200, 240)
                    .test_id("vl")
            });
            t.run_until_idle();
            t
        });
        assert!(
            logs.iter()
                .any(|l| l.message.contains("row height") && l.message.contains("1 px")),
            "{logs:?}"
        );
        // One row per pixel of the viewport (plus one) exists.
        let n = t.find_all(by_class("label")).len();
        assert!(n > 0 && n <= 241, "{n} rows");
        let vl = t.find(by_id("vl")).id();
        t.engine_mut().scroll_to_y(vl, 500, false);
        t.run_until_idle();
        assert!(!t.find_all(by_class("label")).is_empty());
    }
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

/// A virtual list of `(id, value)` items in a signal (also updated through `sensor`), counting
/// row builds.
/// `(id, value)` items.
type Items = Signal<Vec<(u32, i32)>>;

fn keyed_virtual_list() -> (TestUi, Items, &'static Readings, Rc<Cell<u32>>) {
    let builds = Rc::new(Cell::new(0));
    let b = builds.clone();
    let sensor: &'static Readings = TestUi::channel();
    let mut t = TestUi::new(200, 240).mount(move |cx| {
        let items = cx.signal((0..1000u32).map(|i| (i, 0i32)).collect::<Vec<_>>());
        cx.provide(items);
        cx.on_message(sensor, move |(row, value)| items.update(|v| v[row].1 = value));
        let b = b.clone();
        virtual_list(
            items,
            |(id, _)| *id,
            30,
            move |_cx, item| {
                b.set(b.get() + 1);
                label(text!("{}:{}", item.with(|i| i.0), item.with(|i| i.1)))
            },
        )
        .size(200, 240)
        .test_id("vl")
    });
    t.run_until_idle();
    let items = t.root_scope().use_context::<Signal<Vec<(u32, i32)>>>().unwrap();
    (t, items, sensor, builds)
}

/// `(text, node, y)` of the built rows, top to bottom.
fn vrows(t: &TestUi) -> Vec<(String, NodeId, i32)> {
    let mut r: Vec<_> = t
        .find_all(by_class("label"))
        .iter()
        .map(|n| (n.text(), n.id(), t.engine().coords(n.id()).y0))
        .collect();
    r.sort_by_key(|x| x.2);
    r
}

#[test]
fn virtual_list_value_change_updates_the_row_in_place() {
    let (mut t, _items, sensor, builds) = keyed_virtual_list();
    let built = builds.get();
    let before = vrows(&t);
    let runs = rt().stats().effect_runs;
    sensor.try_send((2, 7)).unwrap();
    t.run_until_idle();
    assert_eq!(builds.get(), built, "nothing rebuilt");
    assert_eq!(
        rt().stats().effect_runs - runs,
        1 + 1,
        "the sync and one label binding"
    );
    let after = vrows(&t);
    assert_eq!(after[2].0, "2:7");
    assert_eq!(after[2].1, before[2].1, "same node");
    // A change outside the visible rows runs no binding.
    let runs = rt().stats().effect_runs;
    sensor.try_send((900, 1)).unwrap();
    t.run_until_idle();
    assert_eq!(rt().stats().effect_runs - runs, 1);
    assert_eq!(builds.get(), built);
}

#[test]
fn virtual_list_insert_keeps_rows_by_key() {
    let (mut t, items, _sensor, builds) = keyed_virtual_list();
    let built = builds.get();
    let before = vrows(&t);
    // Insert at the top: every visible key moves down one row.
    items.update(|v| v.insert(0, (5000, 0)));
    t.run_until_idle();
    let after = vrows(&t);
    assert_eq!(after[0].0, "5000:0");
    assert_eq!(after[1].0, "0:0");
    assert_eq!(after[1].1, before[0].1, "the row of key 0 was kept and moved");
    assert_eq!(after[1].2, before[1].2, "one row lower");
    assert_eq!(builds.get(), built + 1, "only the new key's row was built");
    // Removing it moves them back.
    items.update(|v| {
        v.remove(0);
    });
    t.run_until_idle();
    let back = vrows(&t);
    assert_eq!(back[0].1, before[0].1);
    assert_eq!(back[0].2, before[0].2);
    assert!(back.len() <= before.len() + 1);
}
