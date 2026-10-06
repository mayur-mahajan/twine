//! A todo list built on the **store pattern**: the application state is one `Copy` struct of
//! signals and memos, [`Todos`], provided by the root scope as context; components read it
//! with `cx.expect_context::<Todos>()` and change it only through its action methods
//! ([`Todos::push`], [`toggle`](Todos::toggle), [`move_up`](Todos::move_up),
//! [`remove`](Todos::remove), [`clear_done`](Todos::clear_done)). Commands from outside the
//! UI (a companion app, a remote) arrive through a [`Ports`] channel and call the same
//! actions.
//!
//! The list is a keyed [`for_each`]: adding an item builds one row and redraws the counter;
//! toggling an item updates that row's label in place; moving an item up moves one row
//! (`RUST_LOG=twine::view=debug` logs `for_each reconcile: … moved=1`).
//!
//! See the facade's guide `twine::guide::app_state` for the pattern.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use twine::prelude::*;
use twine::reactive::StoredValue;

/// One entry of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Todo {
    /// Stable identity (the `for_each` key), never reused.
    pub id: u32,
    /// What to do.
    pub text: String,
    /// Whether it is done.
    pub done: bool,
}

/// A command from outside the UI, sent through [`Ports::remote`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    /// Add an item with this text.
    Add(&'static str),
    /// Remove the items that are done.
    ClearDone,
}

/// The demo's connections to the rest of the system (the ports pattern): passed to [`app`]
/// by whoever owns them — `main`, the simulator, a test (`TestUi::channel()`).
#[derive(Clone, Copy, Debug)]
pub struct Ports {
    /// Commands from another task, thread or interrupt (`REMOTE.try_send(..)`; wakes the UI).
    pub remote: &'static Channel<Remote, 4>,
}

/// The application state: a `Copy` struct of reactive handles, provided as context by the
/// root scope ([`Todos::provide`]) and read by any component with
/// `cx.expect_context::<Todos>()`.
///
/// The fields are private: views read through the read-only accessors ([`items`](Self::items),
/// [`left`](Self::left), [`total`](Self::total)) and change the state only through the action
/// methods, so every rule of the list (unique ids, what "clear done" means) lives here.
///
/// ```
/// use twine_demos::todos::Todos;
///
/// // A store is plain reactive state: testable without a UI.
/// let cx = twine::reactive::Runtime::take().unwrap().create_root();
/// let todos = Todos::new(cx);
/// let milk = todos.push("Buy milk");
/// todos.push("Water plants");
/// todos.toggle(milk);
/// assert_eq!((todos.left().get(), todos.total()), (1, 2));
/// todos.clear_done();
/// assert_eq!(todos.items().with(|v| v[0].text.clone()), "Water plants");
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Todos {
    items: Signal<Vec<Todo>>,
    /// The next id: plain state (no view depends on it), so a stored value, not a signal.
    next_id: StoredValue<u32>,
    left: Memo<usize>,
}

impl Todos {
    /// An empty list owned by `cx` (its signals and memo are disposed with `cx`).
    ///
    /// # Panics
    ///
    /// If `cx` was disposed.
    #[must_use]
    pub fn new(cx: Scope) -> Self {
        let items = cx.signal(Vec::<Todo>::new());
        Todos {
            items,
            next_id: cx.stored_value(1),
            left: cx.memo(move || items.with(|v| v.iter().filter(|t| !t.done).count())),
        }
    }

    /// A new, empty list owned by `cx` and provided as context to `cx` and its descendants.
    ///
    /// # Panics
    ///
    /// If `cx` was disposed.
    pub fn provide(cx: Scope) -> Self {
        let todos = Todos::new(cx);
        cx.provide(todos);
        todos
    }

    /// Applies the commands arriving on `ports.remote` (for as long as `cx` lives).
    pub fn connect(self, cx: Scope, ports: Ports) {
        cx.on_message(ports.remote, move |cmd| match cmd {
            Remote::Add(text) => {
                self.push(text);
            }
            Remote::ClearDone => self.clear_done(),
        });
    }

    /// The items, read-only. Tracked when read with `with`/`get`.
    #[must_use]
    pub fn items(self) -> ReadSignal<Vec<Todo>> {
        self.items.read_only()
    }

    /// How many items are not done (a memo: dependents re-run only when the count changes).
    #[must_use]
    pub fn left(self) -> Memo<usize> {
        self.left
    }

    /// How many items there are. **Tracked.**
    #[must_use]
    pub fn total(self) -> usize {
        self.items.with(Vec::len)
    }

    /// The id the next added item gets. Not tracked.
    #[must_use]
    pub fn next_id(self) -> u32 {
        self.next_id.get()
    }

    /// Adds an item at the end; returns its id.
    pub fn push(self, text: impl Into<String>) -> u32 {
        let id = self.next_id.get();
        self.next_id.set(id.wrapping_add(1));
        let text = text.into();
        self.items.update(|v| {
            v.push(Todo {
                id,
                text,
                done: false,
            });
        });
        id
    }

    /// Marks the item `id` done, or not done again. Unknown ids are ignored.
    pub fn toggle(self, id: u32) {
        self.items.update(|v| {
            if let Some(t) = v.iter_mut().find(|t| t.id == id) {
                t.done = !t.done;
            }
        });
    }

    /// Moves the item `id` one place up. The first item and unknown ids stay put.
    pub fn move_up(self, id: u32) {
        self.items.update(|v| {
            if let Some(i) = v.iter().position(|t| t.id == id).filter(|&i| i > 0) {
                v.swap(i, i - 1);
            }
        });
    }

    /// Removes the item `id`. Unknown ids are ignored.
    pub fn remove(self, id: u32) {
        self.items.update(|v| v.retain(|t| t.id != id));
    }

    /// Removes every item that is done.
    pub fn clear_done(self) {
        self.items.update(|v| v.retain(|t| !t.done));
    }
}

/// The todo list over its [`Ports`]: provides the [`Todos`] store, connects it to the ports
/// and shows the header and the list.
///
/// ```
/// use twine_demos::todos::{Ports, Remote, Todos, app};
/// use twine_testing::{TestUi, by_id, by_text};
///
/// let ports = Ports { remote: TestUi::channel() };
/// let mut t = TestUi::new(320, 240).mount(move |cx| app(cx, ports));
/// t.find(by_id("add")).click();
/// ports.remote.try_send(Remote::Add("From the phone")).unwrap();
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("count")).text(), "2 of 2 left");
/// // Tests reach the store like the components do.
/// let todos = t.root_scope().expect_context::<Todos>();
/// assert_eq!(todos.items().with(|v| v[1].text.clone()), "From the phone");
/// assert!(t.find(by_text("From the phone")).is_visible());
/// ```
pub fn app(cx: Scope, ports: Ports) -> impl View {
    let todos = Todos::provide(cx);
    todos.connect(cx, ports);
    column((
        header(),
        scroll_view(
            Axis::Vertical,
            for_each(todos.items(), |t| t.id, |_cx, item| todo_row(item)),
        )
        .gap(6)
        .width(Length::pct(100))
        .flex_grow(1)
        .test_id("list"),
    ))
    .gap(10)
    .padding(10)
    .size(Length::pct(100), Length::pct(100))
}

/// "Add", the counter and "Clear done". Reads the store from the context.
#[component]
fn header(cx: Scope) -> impl View {
    let todos = cx.expect_context::<Todos>(); // once, while building
    row((
        button(label("Add")).test_id("add").on_click(move || {
            todos.push(format!("Item {}", todos.next_id()));
        }),
        label(text!("{} of {} left", todos.left().get(), todos.total()))
            .flex_grow(1)
            .test_id("count"),
        button(label("Clear done"))
            .test_id("clear")
            .disabled(move || todos.left().get() == todos.total())
            .on_click(move || todos.clear_done()),
    ))
    .gap(12)
    .align_items(CrossAlign::Center)
    .width(Length::pct(100))
}

/// One row: done toggle, text (struck through when done), move up, delete. `item` is the
/// row's item signal (updated in place by `for_each`).
#[component]
fn todo_row(cx: Scope, item: ReadSignal<Todo>) -> impl View {
    let todos = cx.expect_context::<Todos>();
    let id = item.with_untracked(|t| t.id);
    row((
        button(label(Symbol::Ok)).on_click(move || todos.toggle(id)),
        label(item.map(|t| t.text.clone()))
            .text_decoration(item.map(|t| {
                if t.done {
                    TextDecor::STRIKETHROUGH
                } else {
                    TextDecor::empty()
                }
            }))
            .flex_grow(1),
        button(label(Symbol::Up)).on_click(move || todos.move_up(id)),
        button(label(Symbol::Close)).on_click(move || todos.remove(id)),
    ))
    .gap(6)
    .align_items(CrossAlign::Center)
    .width(Length::pct(100))
}
