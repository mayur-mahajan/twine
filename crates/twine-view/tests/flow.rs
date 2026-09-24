//! Control flow: `when` and `dynamic`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use twine_reactive::debug_stats;
use twine_testing::{TestUi, by_class, by_id, by_text};
use twine_view::prelude::*;

#[test]
fn when_switches_branch_and_disposes_scope() {
    let branch_signal: Rc<RefCell<Option<Signal<i32>>>> = Rc::default();
    let bs = branch_signal.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let on = cx.signal(true);
        cx.provide(on);
        let bs = bs.clone();
        column((
            label("above").test_id("above"),
            when(
                move || on.get(),
                move |cx| {
                    let s = cx.signal(1);
                    *bs.borrow_mut() = Some(s);
                    label(text!("yes {}", s.get())).test_id("yes")
                },
            )
            .otherwise(|_| label("no").test_id("no")),
        ))
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("yes")).text(), "yes 1");
    let s = branch_signal.borrow().unwrap();
    assert!(s.is_alive());
    let on = t.root_scope().expect_context::<Signal<bool>>();
    on.set(false);
    t.run_until_idle();
    assert!(t.find_all(by_id("yes")).is_empty());
    assert_eq!(t.find(by_id("no")).text(), "no");
    assert!(!s.is_alive(), "the branch scope was disposed");
    on.set(true);
    t.run_until_idle();
    assert_eq!(t.find(by_id("yes")).text(), "yes 1");
    assert_eq!(t.find_all(by_id("no")).len(), 0);
}

#[test]
fn when_unchanged_cond_does_not_rebuild() {
    let builds = Rc::new(Cell::new(0));
    let b = builds.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let n = cx.signal(1);
        cx.provide(n);
        let b = b.clone();
        when(
            move || n.get() > 0,
            move |_| {
                b.set(b.get() + 1);
                label("positive").test_id("p")
            },
        )
    });
    t.run_until_idle();
    let id = t.find(by_id("p")).id();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    for v in 2..10 {
        n.set(v);
        t.run_until_idle();
    }
    assert_eq!(builds.get(), 1);
    assert_eq!(t.find(by_id("p")).id(), id, "node ids are stable");
    t.assert_idle();
}

#[test]
fn dynamic_rebuilds_only_on_tracked_change() {
    let builds = Rc::new(Cell::new(0));
    let b = builds.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let mode = cx.signal(0u8);
        let other = cx.signal(0u8);
        cx.provide((mode, other));
        let b = b.clone();
        dynamic(move |_| {
            b.set(b.get() + 1);
            match mode.get() {
                0 => label("zero").into_any(),
                _ => button(label("one")).into_any(),
            }
        })
    });
    t.run_until_idle();
    assert_eq!(builds.get(), 1);
    let (mode, other) = t.root_scope().expect_context::<(Signal<u8>, Signal<u8>)>();
    other.set(3);
    t.run_until_idle();
    assert_eq!(builds.get(), 1, "an untracked signal does not rebuild");
    mode.set(1);
    t.run_until_idle();
    assert_eq!(builds.get(), 2);
    assert_eq!(t.find_all(by_text("zero")).len(), 0);
    assert_eq!(t.find_all(by_class("button")).len(), 1);
}

#[test]
fn child_binding_reads_do_not_rebuild_dynamic() {
    let builds = Rc::new(Cell::new(0));
    let b = builds.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let n = cx.signal(0);
        cx.provide(n);
        let b = b.clone();
        dynamic(move |_| {
            b.set(b.get() + 1);
            label(text!("n = {}", n.get())).test_id("l").into_any()
        })
    });
    t.run_until_idle();
    let n = t.root_scope().expect_context::<Signal<i32>>();
    for v in 1..5 {
        n.set(v);
        t.run_until_idle();
        assert_eq!(t.find(by_id("l")).text(), format!("n = {v}"));
    }
    assert_eq!(builds.get(), 1, "only the label's binding re-ran");
}

#[test]
fn regions_release_everything_on_switch() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let on = cx.signal(true);
        cx.provide(on);
        when(
            move || on.get(),
            |cx| {
                let n = cx.signal(3);
                cx.interval(Duration::ms(100), move || n.update(|v| *v += 1));
                column((label(text!("{}", n.get())), button(label("b"))))
            },
        )
    });
    t.run_until_idle_bounded();
    let on = t.root_scope().expect_context::<Signal<bool>>();
    let nodes_on = t.engine().tree().len();
    let reactive_on = debug_stats().nodes;
    on.set(false);
    t.run_until_idle();
    assert!(t.engine().tree().len() < nodes_on);
    assert!(debug_stats().nodes < reactive_on);
    assert_eq!(
        t.engine().timer_count(),
        0,
        "the interval was removed with its scope"
    );
    t.assert_idle();
}

trait Bounded {
    fn run_until_idle_bounded(&mut self);
}

impl Bounded for TestUi {
    /// Updates a few frames (the interval keeps the UI busy).
    fn run_until_idle_bounded(&mut self) {
        self.advance(Duration::ms(250));
    }
}
