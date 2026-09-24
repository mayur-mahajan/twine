//! Views, sequences and the build context.

use std::cell::RefCell;
use std::rc::Rc;

use twine_engine::{Engine, NodeId, Obj};
use twine_testing::EngineHarness;
use twine_view::EngineAccess;
use twine_view::prelude::*;
use twine_widgets::label::Label;

/// Builds `seq` under the screen of a fresh harness; returns the harness and the texts of the
/// screen's children in order.
fn build_seq(seq: impl ViewSeq) -> (EngineHarness, Vec<String>) {
    let mut h = EngineHarness::new(200, 100);
    let screen = h.screen();
    let scope = twine_reactive::create_root();
    let mut cx = BuildCx::new(h.engine_mut(), screen, scope);
    cx.build_seq(seq);
    let texts = texts(h.engine(), screen);
    (h, texts)
}

fn texts(e: &Engine, parent: NodeId) -> Vec<String> {
    e.tree()
        .children(parent)
        .filter_map(|c| e.widget::<Label>(c).map(|l| l.text().to_owned()))
        .collect()
}

#[test]
fn tuple_seq_builds_children_in_order() {
    let (_h, t) = build_seq((label("a"),));
    assert_eq!(t, ["a"]);
    let (_h, t) = build_seq((label("a"), label("b")));
    assert_eq!(t, ["a", "b"]);
    let (_h, t) = build_seq((
        label("0"),
        label("1"),
        label("2"),
        label("3"),
        label("4"),
        label("5"),
        label("6"),
        label("7"),
        label("8"),
        label("9"),
        label("10"),
        label("11"),
        label("12"),
        label("13"),
        label("14"),
        label("15"),
    ));
    let want: Vec<String> = (0..16).map(|i| i.to_string()).collect();
    assert_eq!(t, want);
    // Nested tuples flatten.
    let (_h, t) = build_seq(((label("a"), label("b")), label("c")));
    assert_eq!(t, ["a", "b", "c"]);
}

#[test]
fn vec_option_array_unit_seq() {
    let (_h, t) = build_seq(vec![label("x"), label("y")]);
    assert_eq!(t, ["x", "y"]);
    let (_h, t) = build_seq(Some(label("s")));
    assert_eq!(t, ["s"]);
    let (_h, t) = build_seq(None::<twine_view::WidgetView<Label>>);
    assert!(t.is_empty());
    let (_h, t) = build_seq([label("p"), label("q"), label("r")]);
    assert_eq!(t, ["p", "q", "r"]);
    let (h, t) = build_seq(());
    assert!(t.is_empty());
    assert_eq!(h.engine().tree().children(h.screen()).count(), 0);
}

#[test]
fn any_view_builds_same_as_concrete() {
    let (h1, t1) = build_seq(button(label("go")).padding(3));
    let (h2, t2) = build_seq(button(label("go")).padding(3).into_any());
    assert_eq!(t1, t2);
    assert_eq!(h1.tree_dump(), h2.tree_dump());
    let (_h, t) = build_seq(vec![label("a").into_any(), label("b").into_any()]);
    assert_eq!(t, ["a", "b"]);
}

#[test]
fn ops_run_after_create_before_children() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
    let v = widget_view(|| Obj)
        .op(move |cx, n| {
            l1.borrow_mut()
                .push(format!("op1 children={}", cx.engine().tree().children(n).count()));
        })
        .children(label("child").op(move |_, _| l3.borrow_mut().push("child".into())))
        .op(move |cx, n| {
            l2.borrow_mut()
                .push(format!("op2 children={}", cx.engine().tree().children(n).count()));
        });
    let _ = build_seq(v);
    assert_eq!(*log.borrow(), ["op1 children=0", "op2 children=0", "child"]);
}

#[test]
fn on_delete_runs_when_node_deleted() {
    let mut h = EngineHarness::new(100, 100);
    let screen = h.screen();
    let scope = twine_reactive::create_root();
    let ran = Rc::new(RefCell::new(0));
    let node = {
        let mut cx = BuildCx::new(h.engine_mut(), screen, scope);
        let n = cx.build(label("bye"));
        let r = ran.clone();
        cx.on_delete(n, move || {
            *r.borrow_mut() += 1;
            // The engine is lent to the callback.
            assert!(EngineAccess::available());
        });
        n
    };
    h.engine_mut().delete(node).unwrap();
    assert_eq!(*ran.borrow(), 1);
    scope.dispose();
}
