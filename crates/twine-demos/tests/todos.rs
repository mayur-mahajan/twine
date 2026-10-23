//! The `todos` demo (the store pattern): components and tests reach the same `Todos` store
//! through the context; clicks call its actions, actions update exactly the affected rows,
//! commands from the ports call the same actions, and the store goes with its scope.

use twine::prelude::*;
use twine_demos::todos::{Ports, Remote, Todos, app};
use twine_testing::{TestUi, by_id, by_text};

fn ui() -> (TestUi, Todos, Ports) {
    let ports = Ports {
        remote: TestUi::channel(),
    };
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(move |cx| app(cx, ports));
    t.run_until_idle();
    let todos = t.root_scope().expect_context::<Todos>();
    (t, todos, ports)
}

fn texts(todos: Todos) -> Vec<String> {
    todos
        .items()
        .with_untracked(|v| v.iter().map(|t| t.text.clone()).collect())
}

fn count(t: &TestUi) -> String {
    t.find(by_id("count")).text()
}

#[test]
fn clicks_call_the_store_actions() {
    let (mut t, todos, _) = ui();
    assert_eq!(count(&t), "0 of 0 left");
    t.find(by_id("add")).click();
    t.find(by_id("add")).click();
    t.run_until_idle();
    assert_eq!(texts(todos), ["Item 1", "Item 2"]);
    assert_eq!(count(&t), "2 of 2 left");

    // The second row's buttons: done, up.
    t.find_all(by_text(Symbol::Ok.as_str()))[1].click();
    t.run_until_idle();
    assert_eq!(count(&t), "1 of 2 left");
    t.find_all(by_text(Symbol::Up.as_str()))[1].click();
    t.run_until_idle();
    assert_eq!(texts(todos), ["Item 2", "Item 1"]);

    t.find(by_id("clear")).click();
    t.run_until_idle();
    assert_eq!(texts(todos), ["Item 1"]);
    assert_eq!(count(&t), "1 of 1 left");
    t.find(by_text(Symbol::Close.as_str())).click();
    t.run_until_idle();
    assert_eq!(texts(todos), Vec::<String>::new());
    t.assert_idle();
}

#[test]
fn store_actions_update_the_view() {
    let (mut t, todos, _) = ui();
    let a = todos.push("Buy milk");
    todos.push("Water plants");
    t.run_until_idle();
    assert!(t.find(by_text("Buy milk")).is_visible());
    assert!(
        t.find(by_id("clear")).state().contains(State::DISABLED),
        "nothing done yet"
    );
    todos.toggle(a);
    t.run_until_idle();
    assert_eq!(count(&t), "1 of 2 left");
    assert!(!t.find(by_id("clear")).state().contains(State::DISABLED));
    todos.remove(a);
    t.run_until_idle();
    assert!(t.find_all(by_text("Buy milk")).is_empty());
    assert_eq!(count(&t), "1 of 1 left");
}

#[test]
fn a_toggle_updates_its_row_in_place() {
    let (mut t, todos, _) = ui();
    let a = todos.push("A");
    todos.push("B");
    t.run_until_idle();
    let row_a = t.find(by_text("A")).id();
    let row_b = t.find(by_text("B")).id();
    todos.toggle(a);
    t.run_until_idle();
    assert_eq!(t.find(by_text("A")).id(), row_a, "the row is kept, not rebuilt");
    assert_eq!(t.find(by_text("B")).id(), row_b);
}

#[test]
fn remote_commands_go_through_the_same_actions() {
    let (mut t, todos, ports) = ui();
    ports.remote.try_send(Remote::Add("From the phone")).unwrap();
    t.run_until_idle();
    assert_eq!(texts(todos), ["From the phone"]);
    todos.toggle(todos.items().with_untracked(|v| v[0].id));
    ports.remote.try_send(Remote::ClearDone).unwrap();
    t.run_until_idle();
    assert_eq!(texts(todos), Vec::<String>::new());
    assert_eq!(count(&t), "0 of 0 left");
}

#[test]
fn the_store_lives_as_long_as_its_scope() {
    let cx = Runtime::current_thread().create_root();
    let page = cx.child();
    let todos = Todos::provide(page);
    todos.push("x");
    assert!(page.child().use_context::<Todos>().is_ok());
    assert!(cx.use_context::<Todos>().is_err(), "provided below the root");
    page.dispose();
    assert!(
        !todos.items().is_alive(),
        "the store's signals went with its scope"
    );
    cx.dispose();
}

#[test]
fn a_nested_store_shadows_the_root_store() {
    let cx = Runtime::current_thread().create_root();
    let root_todos = Todos::provide(cx);
    let dialog = cx.child();
    let draft = Todos::provide(dialog); // e.g. an editor's scratch list
    draft.push("draft");
    let seen = dialog.child().expect_context::<Todos>();
    assert_eq!(texts(seen), ["draft"]);
    assert_eq!(texts(root_todos), Vec::<String>::new());
    dialog.dispose();
    assert_eq!(texts(cx.child().expect_context::<Todos>()), Vec::<String>::new());
    cx.dispose();
}
