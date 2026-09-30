//! Build failures (R0.S04): a widget that cannot be created gets `DEAD_NODE`, never another
//! node's id; its build steps, bindings and children are skipped; the failure is raised as a
//! `BuildFailed` fault and the runtime constructors return an error.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::ColorFormat;
use twine_engine::{Engine, EngineConfig, Obj};
use twine_hal::DisplayInfo;
use twine_style::StyleValue;
use twine_testing::{EngineHarness, MemoryDisplay, MockClock, TestUi, by_id};
use twine_view::prelude::*;
use twine_view::{BuildFailure, UiCore};

/// A small tree budget, so filling it is cheap (tree mutations are O(n) under `debug-checks`).
fn small() -> EngineConfig {
    EngineConfig {
        max_nodes: 24,
        ..EngineConfig::default()
    }
}

/// Fills the tree with filler nodes under a detached root, leaving room for `free` nodes.
fn fill(e: &mut Engine, free: usize) {
    let holder = e.create_root(Box::new(Obj)).unwrap();
    let mut fillers = Vec::new();
    while let Ok(n) = e.create(holder, Box::new(Obj)) {
        fillers.push(n);
    }
    assert!(
        fillers.len() >= free,
        "not enough room to leave {free} nodes free"
    );
    for n in fillers.into_iter().take(free) {
        e.delete(n).unwrap();
    }
    let _ = e.take_faults();
}

fn build_failed(e: &Engine) -> u32 {
    e.fault_counts().get(FaultKind::BuildFailed)
}

fn bg(e: &Engine, n: NodeId) -> StyleValue {
    e.style_prop(n, Part::Main, PropId::BgColor)
}

#[test]
fn regression_failed_widget_is_not_aliased_to_its_parent() {
    let mut h = EngineHarness::new(100, 60).config(small());
    let screen = h.screen();
    fill(h.engine_mut(), 1); // room for the column only
    let before = build_failed(h.engine());
    let scope = twine_reactive::create_root();
    let (col, child) = {
        let mut cx = BuildCx::new(h.engine_mut(), screen, scope);
        let mut child = None;
        let col = cx.build(
            column((label("x").bg(Color::RED).op(move |_, n| {
                // A build step of the failed label: must never run (it would get the parent).
                panic!("build step ran for {n:?}");
            }),))
            .bg(Color::BLUE),
        );
        cx.with_parent(DEAD_NODE, |cx| child = Some(cx.create(Obj)));
        (col, child.unwrap())
    };
    let e = h.engine();
    assert_ne!(col, DEAD_NODE);
    assert_eq!(child, DEAD_NODE, "creating under a dead parent gives a dead node");
    assert_eq!(
        bg(e, col),
        StyleValue::Color(Color::BLUE),
        "the parent keeps its own style"
    );
    assert_eq!(e.tree().children(col).count(), 0);
    assert_eq!(
        build_failed(e),
        before + 1,
        "one fault for the label, none for the dead child"
    );
    let r = e.last_fault(FaultKind::BuildFailed).unwrap();
    assert_eq!(r.node, Some(col), "the record names the parent");
    assert_eq!(BuildFailure::from_code(r.code), Some(BuildFailure::Capacity));
    assert!(e.pending_faults().contains(FaultKind::BuildFailed));
    assert!(!e.tree().contains(DEAD_NODE));
    scope.dispose();
}

#[test]
fn children_of_a_failed_node_are_not_built_into_the_parent() {
    let mut h = EngineHarness::new(100, 60).config(small());
    let screen = h.screen();
    fill(h.engine_mut(), 1);
    let len = h.engine().tree().len();
    let before = build_failed(h.engine());
    let scope = twine_reactive::create_root();
    let col = {
        let mut cx = BuildCx::new(h.engine_mut(), screen, scope);
        cx.build(column((
            container((
                label("a"),
                label("b"),
                for_each(|| vec![1, 2], |i| *i, |_, _| label("i")),
            )),
            when(|| true, |_| label("w")),
            label("c"),
        )))
    };
    let e = h.engine();
    assert_eq!(e.tree().children(col).count(), 0, "nothing reached the column");
    assert_eq!(e.tree().len(), len + 1, "only the column was created");
    // The container, the `when` wrapper and `c` failed; the container's children were skipped.
    assert_eq!(build_failed(e), before + 3);
    scope.dispose();
}

#[test]
fn no_binding_of_a_failed_widget_touches_its_parent() {
    let mut show = None;
    let mut opa = None;
    let mut t = TestUi::new(100, 60).config(small()).mount(|cx| {
        let s = cx.signal(false);
        let o = cx.signal(Opa::COVER);
        show = Some(s);
        opa = Some(o);
        column(when(
            move || s.get(),
            move |_| label("x").bg(Color::RED).bg_opacity(move || o.get()).padding(9),
        ))
        .bg(Color::BLUE)
        .test_id("col")
    });
    let (show, opa) = (show.unwrap(), opa.unwrap());
    let col = t.find(by_id("col")).id();
    let wrapper = t.engine().tree().children(col).next().unwrap();
    let snapshot = |t: &TestUi| {
        let e = t.engine();
        [col, wrapper].map(|n| {
            [PropId::BgColor, PropId::BgOpacity, PropId::PaddingTop].map(|p| e.style_prop(n, Part::Main, p))
        })
    };
    t.run_until_idle();
    let styles = snapshot(&t);
    fill(t.engine_mut(), 0);
    let len = t.engine().tree().len();
    let before = build_failed(&t.engine());

    show.set(true); // the `when` content cannot be created now
    t.run_until_idle();
    assert_eq!(build_failed(&t.engine()), before + 1);
    assert_eq!(
        t.engine().last_fault(FaultKind::BuildFailed).and_then(|r| r.node),
        Some(wrapper)
    );
    opa.set(Opa::P10); // the failed label's binding: must not reach the wrapper or the column
    t.run_until_idle();
    assert_eq!(snapshot(&t), styles, "the parents' styles are unchanged");
    assert_eq!(t.engine().tree().len(), len);
    assert_eq!(bg(&t.engine(), col), StyleValue::Color(Color::BLUE));
}

static BUILD_FAULTS: AtomicU32 = AtomicU32::new(0);

fn count_build_faults(r: &FaultRecord) {
    if r.kind == FaultKind::BuildFailed {
        BUILD_FAULTS.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn try_build_reports_a_build_failure() {
    let display = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    let r = Ui::builder(display)
        .clock(MockClock::new())
        .config(EngineConfig {
            max_nodes: 8,
            ..EngineConfig::default()
        })
        .fault_hook(count_build_faults)
        .try_build(|_| column((0..12).map(|_| label("x")).collect::<Vec<_>>()));
    let Err(UiError::Build(e)) = r else {
        panic!("expected a build error, got {r:?}");
    };
    assert_eq!(e.cause, BuildFailure::Capacity);
    assert!(e.failures >= 1);
    assert!(e.parent.is_some());
    assert!(BUILD_FAULTS.load(Ordering::Relaxed) >= e.failures);
    assert!(UiError::Build(e).to_string().starts_with("build: "));
}

#[test]
fn try_build_succeeds_within_the_budget() {
    let display = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    let mut ui = Ui::builder(display)
        .clock(MockClock::new())
        .config(small())
        .try_build(|_| column((label("a"), label("b"))))
        .unwrap();
    assert!(!ui.take_faults().contains(FaultKind::BuildFailed));
}

#[test]
fn failed_mount_leaves_the_engine_as_it_was() {
    let mut h = EngineHarness::new(100, 60).config(small());
    let d = h.display();
    let len = h.engine().tree().len();
    let screen = h.screen();
    let r = UiCore::mount(h.engine_mut(), d, |_| {
        column((0..40).map(|_| label("x")).collect::<Vec<_>>())
    });
    let e = r.unwrap_err();
    assert_eq!(e.cause, BuildFailure::Capacity);
    assert!(e.failures >= 1);
    assert_eq!(h.engine().tree().len(), len, "the partial build was removed");
    assert_eq!(h.engine().tree().children(screen).count(), 0);
    // The engine still works: a smaller application mounts.
    let core = UiCore::mount(h.engine_mut(), d, |_| label("ok"));
    assert!(core.is_ok());
}

#[test]
fn test_ui_try_mount_reports_the_error() {
    let r = TestUi::new(100, 60)
        .config(EngineConfig {
            max_nodes: 6,
            ..EngineConfig::default()
        })
        .try_mount(|_| column((0..10).map(|_| label("x")).collect::<Vec<_>>()));
    assert!(r.is_err());
}
