//! R5.S03 completion: a sealed UI changes within its limits without allocating — keyed lists
//! remove and move rows (the reconcile scratch is reserved from `Limits::max_list_items`),
//! a navigator pops back to built pages (disposal allocates nothing), and dynamic texts of
//! labels, dropdowns and button matrices are rewritten within `Limits::max_text_len` — and a
//! keyed collection longer than its bound shows the first rows with a `ListItems` fault.
//!
//! req: SAF-010

use std::cell::Cell;

use twine_core::alloc_guard;
use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal};
use twine_testing::{TestUi, by_class};
use twine_view::prelude::*;
use twine_widgets::buttonmatrix::ButtonMatrix;
use twine_widgets_ext::dropdown::Dropdown;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A product's limits, reserved at start-up.
fn product_limits() -> Limits {
    Limits::DEFAULT
        .max_nodes(256)
        .max_reactive_nodes(512)
        .max_scopes(128)
        .max_pending_effects(64)
        .max_text_len(48)
        .max_nav_depth(4)
        .max_list_items(16)
        .preallocated()
}

fn labels(t: &TestUi) -> Vec<String> {
    t.find_all(by_class("label"))
        .iter()
        .map(twine_testing::NodeHandle::text)
        .collect()
}

#[test]
fn sealed_for_each_removes_moves_and_updates_rows_without_allocating() {
    let mut t = TestUi::new(200, 400)
        .app_config(AppConfig::new().limits(product_limits()))
        .mount(|cx| {
            let items = cx.signal((0..12u32).map(|k| (k, k * 10)).collect::<Vec<_>>());
            cx.provide(items);
            column(for_each(
                items,
                |(k, _)| *k,
                |_, it| label(text!("{}", it.get().1)),
            ))
            .gap(0)
        });
    t.run_until_idle();
    let items = t.root_scope().use_context::<Signal<Vec<(u32, u32)>>>().unwrap();
    t.seal();
    assert_no_alloc_after_seal(|| {
        items.update(|v| v.reverse());
        t.run_until_idle();
        items.update(|v| v.swap(0, 7));
        t.run_until_idle();
        items.update(|v| v.retain(|(k, _)| k % 3 != 0));
        t.run_until_idle();
        items.update(|v| v.rotate_left(2));
        t.run_until_idle();
        items.update(|v| {
            for (_, x) in v.iter_mut() {
                *x += 1;
            }
        });
        t.run_until_idle();
    });
    alloc_guard::unseal();
    let expected: Vec<String> = items.with(|v| v.iter().map(|(_, x)| x.to_string()).collect());
    assert_eq!(labels(&t), expected);
    assert_eq!(t.engine().fault_counts().get(FaultKind::AllocAfterSeal), 0);
}

#[test]
fn for_each_beyond_its_bound_shows_the_first_rows_with_a_fault() {
    let mut t = TestUi::new(200, 400)
        .app_config(AppConfig::new().limits(twine_testing::HARNESS_LIMITS.max_list_items(4)))
        .mount(|cx| {
            let items = cx.signal((0..6u32).collect::<Vec<_>>());
            cx.provide(items);
            column((
                for_each(items, |k| *k, |_, k| label(text!("a{}", k.get()))),
                // A per-collection bound below the `Limits` ceiling.
                for_each(items, |k| *k, |_, k| label(text!("b{}", k.get()))).max_items(2),
            ))
        });
    t.run_until_idle();
    assert_eq!(labels(&t), ["a0", "a1", "a2", "a3", "b0", "b1"]);
    let r = *t.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(CapacityFault::from_code(r.code), Some(CapacityFault::ListItems));
    assert!(r.node.is_some());
    let faults = t.engine().fault_counts().get(FaultKind::Capacity);
    // Within the bounds again: no new fault, and the rows follow the items.
    let items = t.root_scope().use_context::<Signal<Vec<u32>>>().unwrap();
    items.set(vec![5, 1]);
    t.run_until_idle();
    assert_eq!(labels(&t), ["a5", "a1", "b5", "b1"]);
    assert_eq!(t.engine().fault_counts().get(FaultKind::Capacity), faults);
}

#[test]
fn virtual_list_beyond_its_bound_builds_the_first_visible_rows_with_a_fault() {
    let mut t = TestUi::new(200, 240)
        .app_config(AppConfig::new().limits(twine_testing::HARNESS_LIMITS.max_list_items(3)))
        .mount(|_| {
            let rows: Vec<u32> = (0..100).collect();
            virtual_list(rows, |i| *i, 30, |_, i| label(text!("Row {}", i.get()))).size(200, 240)
        });
    t.run_until_idle();
    assert_eq!(labels(&t), ["Row 0", "Row 1", "Row 2"]);
    let r = *t.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(CapacityFault::from_code(r.code), Some(CapacityFault::ListItems));
}

#[derive(Clone, PartialEq, Debug)]
struct Page(u32);

thread_local! {
    static NAV: Cell<Option<Navigator<Page>>> = const { Cell::new(None) };
}

#[test]
fn sealed_navigator_pops_back_to_built_pages_without_allocating() {
    let mut t = TestUi::new(200, 100)
        .app_config(AppConfig::new().limits(product_limits()))
        .mount(|cx| {
            navigator(cx, Page(0), |cx, p: Page| {
                NAV.with(|n| n.set(use_navigator::<Page>(cx).ok()));
                let n = cx.signal(p.0);
                label(text!("page {}", n.get()))
            })
        });
    t.run_until_idle();
    let nav = NAV.with(Cell::get).unwrap();
    for i in 1..4 {
        assert_eq!(nav.push(Page(i), ScreenAnim::None), NavOutcome::Accepted);
        t.run_until_idle();
    }
    t.seal();
    assert_no_alloc_after_seal(|| {
        for _ in 0..3 {
            assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::Accepted);
            t.run_until_idle();
        }
    });
    alloc_guard::unseal();
    assert_eq!(nav.depth(), 1);
    assert_eq!(labels(&t), ["page 0"]);
}

#[test]
fn sealed_text_edits_within_limits_allocate_nothing() {
    static NAMES: [&str; 4] = ["Low", "Medium", "High", "Turbo"];
    let mut t = TestUi::new(240, 200)
        .app_config(AppConfig::new().limits(product_limits()))
        .mount(|cx| {
            let n = cx.signal(0u32);
            // The application's list has room for every option (its own allocation).
            let mut list = Vec::with_capacity(NAMES.len());
            list.extend_from_slice(&NAMES[..2]);
            let opts = cx.signal(list);
            cx.provide(n);
            cx.provide(opts);
            column((
                label(text!("reading {} of {}", n.get(), 1 << 20)),
                dropdown(opts, 0),
                buttonmatrix([[btn(text!("{}", n.get())), btn("OK")]]),
            ))
        });
    t.run_until_idle();
    let n = t.root_scope().use_context::<Signal<u32>>().unwrap();
    let opts = t.root_scope().use_context::<Signal<Vec<&'static str>>>().unwrap();
    // Warm-up: the same updates once before sealing size the `debug-checks` invalidation
    // logs (test instrumentation, not built into a product).
    for i in 0..20u32 {
        n.set(i * 997 + 1);
        t.run_until_idle();
    }
    t.seal();
    assert_no_alloc_after_seal(|| {
        for i in 0..20u32 {
            n.set(i * 997);
            t.run_until_idle();
        }
        // Options change in place (the list's capacity holds four).
        opts.update(|v| v.push(NAMES[2]));
        t.run_until_idle();
        opts.update(|v| {
            v.clear();
            v.extend_from_slice(&NAMES[1..]);
        });
        t.run_until_idle();
    });
    alloc_guard::unseal();
    let d = t.find(by_class("dropdown")).id();
    assert_eq!(
        t.engine().widget::<Dropdown>(d).unwrap().options(),
        "Medium\nHigh\nTurbo"
    );
    let m = t.find(by_class("buttonmatrix")).id();
    assert_eq!(
        t.engine().widget::<ButtonMatrix>(m).unwrap().btn_text(0),
        Some("18943")
    );
}
