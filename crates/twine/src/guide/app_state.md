Application state: the store pattern for large applications.

Small views keep their state in local signals (`let count = cx.signal(0)`). As an application
grows, state is shared by screens and components far apart in the tree — the current user,
settings, a list of alarms, the data a sensor task delivers — and handing it down as
arguments through every layer becomes noise. This guide shows the pattern Twine recommends for
that: a **store** — a `Copy` struct of signals and memos with **actions as methods** —
**provided once** by the root scope as a context value and **read by any component** with
[`Scope::expect_context`](crate::reactive::Scope::expect_context). It costs nothing per frame
and keeps every rule of the state in one place.

The complete, runnable version is the `todos` demo (`twine_demos::todos`: a keyed list, a
"clear done" action, commands from a companion app through ports) with the `todo_list`
example (`cargo xtask sim todo_list`) and its tests (`crates/twine-demos/tests/todos.rs`).
Every code block below is compiled and run as a doctest.

**Contents:** [1. The store](#1-the-store) ·
[2. Providing and reading it](#2-providing-and-reading-it) ·
[3. Several stores, scoped stores](#3-several-stores-scoped-stores) ·
[4. Testing](#4-testing) · [5. Stores and ports](#5-stores-and-ports) ·
[6. Cost and failure handling](#6-cost-and-failure-handling) · [Checklist](#checklist)

# 1. The store

A store is plain reactive state: a struct of `Copy` handles ([`Signal`](crate::reactive::Signal),
[`Memo`](crate::reactive::Memo), [`StoredValue`](crate::reactive::StoredValue) for values no
view depends on), so the struct is `Copy` itself and moves into any number of closures without
cloning. Keep the fields private: views read through accessors that return read-only handles
(`ReadSignal`, `Memo`) and change the state only through **actions**, methods that apply the
application's rules (here: "louder" unmutes, the volume stays in `0..=10`). Derived values
are memos in the store, computed once for every reader.

```rust
use twine::prelude::*;

/// The player's state: one `Copy` struct of reactive handles.
#[derive(Clone, Copy)]
pub struct Player {
    volume: Signal<u8>, // 0..=10
    muted: Signal<bool>,
    level: Memo<u8>, // what the speaker plays: 0 while muted
}

impl Player {
    /// A player owned by `cx` (its signals and memo are disposed with `cx`).
    pub fn new(cx: Scope) -> Self {
        let (volume, muted) = (cx.signal(5), cx.signal(false));
        let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
        Player { volume, muted, level }
    }

    /// A new player, provided as context to `cx` and its descendants.
    pub fn provide(cx: Scope) -> Self {
        let player = Player::new(cx);
        cx.provide(player);
        player
    }

    // Reads: read-only handles, tracked where they are read.
    pub fn level(self) -> Memo<u8> {
        self.level
    }
    pub fn muted(self) -> ReadSignal<bool> {
        self.muted.read_only()
    }

    // Actions: the only way to change the state, so its rules live here.
    pub fn louder(self) {
        self.muted.set(false);
        self.volume.update(|v| *v = (*v + 1).min(10));
    }
    pub fn quieter(self) {
        self.volume.update(|v| *v = v.saturating_sub(1));
    }
    pub fn toggle_mute(self) {
        self.muted.update(|m| *m = !*m);
    }
}
#
# let cx = twine::reactive::Runtime::take().unwrap().create_root();
# let player = Player::new(cx);
# player.louder();
# assert_eq!(player.level().get(), 6);
# player.toggle_mute();
# assert_eq!(player.level().get(), 0);
```

The store does not know about views, the engine or the `Ui`: it can be created in any scope
and tested without a display (§ 4).

# 2. Providing and reading it

The root of the application creates the store and provides it with
[`Scope::provide`](crate::reactive::Scope::provide); every scope below it — components, pages
of a [`navigator`](crate::view::navigator()), `when` branches, `for_each` rows — finds it with
`cx.expect_context::<Player>()`. A component that needs it takes a
[`Scope`](crate::reactive::Scope) as its first parameter (with
[`#[component]`](macro@crate::component) the scope is passed in, not a prop) and looks the
store up **once**, while it is built; its closures capture the `Copy` handle.

In the `safety` profile `expect_context` does not exist (no panicking lookups): look the store
up with [`Scope::use_context`](crate::reactive::Scope::use_context) and handle the
[`MissingContext`](crate::reactive::MissingContext) error — e.g. build a fallback view, or
make the store a parameter of the components that need it.

```rust
# use twine::prelude::*;
#
# /// The player's state: one `Copy` struct of reactive handles.
# #[derive(Clone, Copy)]
# pub struct Player {
#     volume: Signal<u8>, // 0..=10
#     muted: Signal<bool>,
#     level: Memo<u8>, // what the speaker plays: 0 while muted
# }
#
# impl Player {
#     /// A player owned by `cx` (its signals and memo are disposed with `cx`).
#     pub fn new(cx: Scope) -> Self {
#         let (volume, muted) = (cx.signal(5), cx.signal(false));
#         let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
#         Player { volume, muted, level }
#     }
#
#     /// A new player, provided as context to `cx` and its descendants.
#     pub fn provide(cx: Scope) -> Self {
#         let player = Player::new(cx);
#         cx.provide(player);
#         player
#     }
#
#     // Reads: read-only handles, tracked where they are read.
#     pub fn level(self) -> Memo<u8> {
#         self.level
#     }
#     pub fn muted(self) -> ReadSignal<bool> {
#         self.muted.read_only()
#     }
#
#     // Actions: the only way to change the state, so its rules live here.
#     pub fn louder(self) {
#         self.muted.set(false);
#         self.volume.update(|v| *v = (*v + 1).min(10));
#     }
#     pub fn quieter(self) {
#         self.volume.update(|v| *v = v.saturating_sub(1));
#     }
#     pub fn toggle_mute(self) {
#         self.muted.update(|m| *m = !*m);
#     }
# }
#
/// The application: provides the store at the root, composes the components.
pub fn app(cx: Scope) -> impl View {
    Player::provide(cx);
    column((level_meter(), controls())).gap(8).padding(8)
}

/// The level, read from the store.
#[component]
fn level_meter(cx: Scope) -> impl View {
    let player = cx.expect_context::<Player>(); // once, while building
    row((
        bar(player.level()).range(0..=10).flex_grow(1),
        label(text!("{}", player.level().get())).test_id("level"),
    ))
    .gap(8)
}

/// The buttons call the store's actions.
#[component]
fn controls(cx: Scope) -> impl View {
    let player = cx.expect_context::<Player>();
    row((
        button(label("-")).test_id("quieter").on_click(move || player.quieter()),
        button(label(move || if player.muted().get() { "Unmute" } else { "Mute" }))
            .test_id("mute")
            .on_click(move || player.toggle_mute()),
        button(label("+")).test_id("louder").on_click(move || player.louder()),
    ))
    .gap(8)
}
#
# use twine_testing::{TestUi, by_id};
# let mut t = TestUi::new(240, 120).mount(app);
# t.find(by_id("louder")).click();
# t.run_until_idle();
# assert_eq!(t.find(by_id("level")).text(), "6");
```

Each context value is found by its **type**, so the store type is the key: two stores are
two types, and a plain `Signal<u8>` is a poor context (any other `Signal<u8>` provided closer
would shadow it) — wrap it in a newtype. Components that only need a part of the store can
still take it as a prop (`level_meter(player.level())`): a context is for state that many
components far apart need, not a replacement for every argument.

# 3. Several stores, scoped stores

Split a large application's state by concern — `Settings`, `Alarms`, `Session` — each its own
store type, provided at the root; a component reads only the stores it uses. A store can also
be provided **below** the root, for the part of the tree that needs it: a page of a
navigator, a wizard, an editor. It is disposed with that scope (when the page is popped, its
store's signals and effects go with it), and a store of the same type provided lower
**shadows** the outer one for that subtree — e.g. an editor working on a draft copy:

```rust
# use twine::prelude::*;
#
# /// The player's state: one `Copy` struct of reactive handles.
# #[derive(Clone, Copy)]
# pub struct Player {
#     volume: Signal<u8>, // 0..=10
#     muted: Signal<bool>,
#     level: Memo<u8>, // what the speaker plays: 0 while muted
# }
#
# impl Player {
#     /// A player owned by `cx` (its signals and memo are disposed with `cx`).
#     pub fn new(cx: Scope) -> Self {
#         let (volume, muted) = (cx.signal(5), cx.signal(false));
#         let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
#         Player { volume, muted, level }
#     }
#
#     /// A new player, provided as context to `cx` and its descendants.
#     pub fn provide(cx: Scope) -> Self {
#         let player = Player::new(cx);
#         cx.provide(player);
#         player
#     }
#
#     // Reads: read-only handles, tracked where they are read.
#     pub fn level(self) -> Memo<u8> {
#         self.level
#     }
#     pub fn muted(self) -> ReadSignal<bool> {
#         self.muted.read_only()
#     }
#
#     // Actions: the only way to change the state, so its rules live here.
#     pub fn louder(self) {
#         self.muted.set(false);
#         self.volume.update(|v| *v = (*v + 1).min(10));
#     }
#     pub fn quieter(self) {
#         self.volume.update(|v| *v = v.saturating_sub(1));
#     }
#     pub fn toggle_mute(self) {
#         self.muted.update(|m| *m = !*m);
#     }
# }
#
let root = twine::reactive::Runtime::take().unwrap().create_root();
let live = Player::provide(root);
let editor = root.child(); // e.g. a page's scope
let draft = Player::provide(editor); // shadows `live` below `editor`
draft.louder();
assert_eq!(editor.child().expect_context::<Player>().level().get(), 6);
assert_eq!(root.expect_context::<Player>().level().get(), 5);
editor.dispose(); // the draft store is gone with its scope
assert!(!draft.level().is_alive());
# let _ = live;
```

# 4. Testing

A store needs no UI to be tested: create it in a root scope, call its actions, read its
values (see the first block). To test it **with** the UI, mount the application in a
`TestUi` (crate `twine-testing`) and fetch the store the way the components do —
from the root scope's context — then drive it from both sides: clicks call the actions, and
actions called by the test show up in the view.

```rust
# use twine::prelude::*;
#
# /// The player's state: one `Copy` struct of reactive handles.
# #[derive(Clone, Copy)]
# pub struct Player {
#     volume: Signal<u8>, // 0..=10
#     muted: Signal<bool>,
#     level: Memo<u8>, // what the speaker plays: 0 while muted
# }
#
# impl Player {
#     /// A player owned by `cx` (its signals and memo are disposed with `cx`).
#     pub fn new(cx: Scope) -> Self {
#         let (volume, muted) = (cx.signal(5), cx.signal(false));
#         let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
#         Player { volume, muted, level }
#     }
#
#     /// A new player, provided as context to `cx` and its descendants.
#     pub fn provide(cx: Scope) -> Self {
#         let player = Player::new(cx);
#         cx.provide(player);
#         player
#     }
#
#     // Reads: read-only handles, tracked where they are read.
#     pub fn level(self) -> Memo<u8> {
#         self.level
#     }
#     pub fn muted(self) -> ReadSignal<bool> {
#         self.muted.read_only()
#     }
#
#     // Actions: the only way to change the state, so its rules live here.
#     pub fn louder(self) {
#         self.muted.set(false);
#         self.volume.update(|v| *v = (*v + 1).min(10));
#     }
#     pub fn quieter(self) {
#         self.volume.update(|v| *v = v.saturating_sub(1));
#     }
#     pub fn toggle_mute(self) {
#         self.muted.update(|m| *m = !*m);
#     }
# }
#
# /// The application: provides the store at the root, composes the components.
# pub fn app(cx: Scope) -> impl View {
#     Player::provide(cx);
#     column((level_meter(), controls())).gap(8).padding(8)
# }
#
# /// The level, read from the store.
# #[component]
# fn level_meter(cx: Scope) -> impl View {
#     let player = cx.expect_context::<Player>(); // once, while building
#     row((
#         bar(player.level()).range(0..=10).flex_grow(1),
#         label(text!("{}", player.level().get())).test_id("level"),
#     ))
#     .gap(8)
# }
#
# /// The buttons call the store's actions.
# #[component]
# fn controls(cx: Scope) -> impl View {
#     let player = cx.expect_context::<Player>();
#     row((
#         button(label("-")).test_id("quieter").on_click(move || player.quieter()),
#         button(label(move || if player.muted().get() { "Unmute" } else { "Mute" }))
#             .test_id("mute")
#             .on_click(move || player.toggle_mute()),
#         button(label("+")).test_id("louder").on_click(move || player.louder()),
#     ))
#     .gap(8)
# }
#
use twine_testing::{TestUi, by_id};

let mut t = TestUi::new(240, 120).mount(app);
let player = t.root_scope().expect_context::<Player>();

// The view calls the store...
t.find(by_id("mute")).click();
t.run_until_idle();
assert_eq!(player.level().get(), 0);
assert_eq!(t.find(by_id("mute")).text(), "Unmute");

// ...and the store drives the view.
player.louder();
t.run_until_idle();
assert_eq!(t.find(by_id("level")).text(), "6");
t.assert_idle(); // nothing left to do once the change is drawn
```

To start a test from a particular state, call the actions before the first
`run_until_idle`, or give the application a constructor that takes the initial state
(`app_with(cx, Player::new(cx))`).

# 5. Stores and ports

Data from other tasks, interrupts or threads reaches the UI through the cross-context types
(`Channel`, `Latest`, `Outbox`), passed to the application as **ports** — a `Copy` struct of
`&'static` references the caller owns (see
[`Scope::on_message`](crate::reactive::Scope::on_message)). The store is where they meet: a
`connect` method subscribes it to its ports and turns each message into an **action** — the
same methods the views call — so the rules hold whatever the source. The application gets the
ports from `main`; a test passes fresh ones (`TestUi::channel()`), so parallel tests never
share them.

```rust
# use twine::prelude::*;
#
# /// The player's state: one `Copy` struct of reactive handles.
# #[derive(Clone, Copy)]
# pub struct Player {
#     volume: Signal<u8>, // 0..=10
#     muted: Signal<bool>,
#     level: Memo<u8>, // what the speaker plays: 0 while muted
# }
#
# impl Player {
#     /// A player owned by `cx` (its signals and memo are disposed with `cx`).
#     pub fn new(cx: Scope) -> Self {
#         let (volume, muted) = (cx.signal(5), cx.signal(false));
#         let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
#         Player { volume, muted, level }
#     }
#
#     /// A new player, provided as context to `cx` and its descendants.
#     pub fn provide(cx: Scope) -> Self {
#         let player = Player::new(cx);
#         cx.provide(player);
#         player
#     }
#
#     // Reads: read-only handles, tracked where they are read.
#     pub fn level(self) -> Memo<u8> {
#         self.level
#     }
#     pub fn muted(self) -> ReadSignal<bool> {
#         self.muted.read_only()
#     }
#
#     // Actions: the only way to change the state, so its rules live here.
#     pub fn louder(self) {
#         self.muted.set(false);
#         self.volume.update(|v| *v = (*v + 1).min(10));
#     }
#     pub fn quieter(self) {
#         self.volume.update(|v| *v = v.saturating_sub(1));
#     }
#     pub fn toggle_mute(self) {
#         self.muted.update(|m| *m = !*m);
#     }
# }
#
use twine_testing::{TestUi, by_id};

/// A rotary encoder on the volume knob: +1 / -1 per detent, sent from its interrupt.
#[derive(Clone, Copy)]
pub struct Ports {
    pub knob: &'static Channel<i8, 8>,
}

impl Player {
    /// Applies the knob's turns for as long as `cx` lives.
    pub fn connect(self, cx: Scope, ports: Ports) {
        cx.on_message(ports.knob, move |turn: i8| {
            if turn > 0 { self.louder() } else { self.quieter() }
        });
    }
}

pub fn app(cx: Scope, ports: Ports) -> impl View {
    let player = Player::provide(cx);
    player.connect(cx, ports);
    label(text!("{}", player.level().get())).test_id("level")
}

// In the firmware: `static KNOB: Channel<i8, 8> = Channel::new();`, `Ports { knob: &KNOB }`.
let ports = Ports { knob: TestUi::channel() };
let mut t = TestUi::new(120, 40).mount(move |cx| app(cx, ports));
ports.knob.try_send(1).unwrap(); // e.g. from the encoder interrupt
ports.knob.try_send(1).unwrap();
t.run_until_idle();
assert_eq!(t.find(by_id("level")).text(), "7");
```

Commands **out** of the UI go the other way: an action posts to an `Outbox` port (the
`thermostat` demo sends its set-point that way), so the views still only call actions.

# 6. Cost and failure handling

**Lookup cost.** `use_context` / `expect_context` walk the scope chain from the asking scope
to the root, comparing type ids against the values each scope provides — a few dozen
comparisons in a typical application, **no allocation** (the value is shared, its `Clone`
runs on a `Copy` handle struct). Looking a store up while a component is built costs that
once; avoid it in bindings and frequent handlers by capturing the handle instead. Providing
allocates one `Rc` per value, once. Reading and writing the store's signals is the ordinary
cost of signals: a change re-runs exactly the bindings that read it.

**Failure handling.** Every lookup that can fail exists in two forms:

| Form | Returns | Use |
|------|---------|-----|
| [`use_context`](crate::reactive::Scope::use_context), [`use_navigator`](crate::view::use_navigator), `use_i18n` | `Result<T, MissingContext>`: the error names the missing type | code that must not panic (libraries, the `safety` profile); `.ok()` for an optional context |
| [`expect_context`](crate::reactive::Scope::expect_context), [`expect_navigator`](crate::view::expect_navigator), `expect_i18n` | `T`; panics with `"context {type} not provided"` at the caller's line | application components, where a missing store is a bug |

The panicking forms are thin `#[track_caller]` wrappers over the `Result` forms, so a
library can offer both without duplicating logic — and a component that should work with or
without a store reads it with `.ok()`:

```rust
# use twine::prelude::*;
#
# /// The player's state: one `Copy` struct of reactive handles.
# #[derive(Clone, Copy)]
# pub struct Player {
#     volume: Signal<u8>, // 0..=10
#     muted: Signal<bool>,
#     level: Memo<u8>, // what the speaker plays: 0 while muted
# }
#
# impl Player {
#     /// A player owned by `cx` (its signals and memo are disposed with `cx`).
#     pub fn new(cx: Scope) -> Self {
#         let (volume, muted) = (cx.signal(5), cx.signal(false));
#         let level = cx.memo(move || if muted.get() { 0 } else { volume.get() });
#         Player { volume, muted, level }
#     }
#
#     /// A new player, provided as context to `cx` and its descendants.
#     pub fn provide(cx: Scope) -> Self {
#         let player = Player::new(cx);
#         cx.provide(player);
#         player
#     }
#
#     // Reads: read-only handles, tracked where they are read.
#     pub fn level(self) -> Memo<u8> {
#         self.level
#     }
#     pub fn muted(self) -> ReadSignal<bool> {
#         self.muted.read_only()
#     }
#
#     // Actions: the only way to change the state, so its rules live here.
#     pub fn louder(self) {
#         self.muted.set(false);
#         self.volume.update(|v| *v = (*v + 1).min(10));
#     }
#     pub fn quieter(self) {
#         self.volume.update(|v| *v = v.saturating_sub(1));
#     }
#     pub fn toggle_mute(self) {
#         self.muted.update(|m| *m = !*m);
#     }
# }
#
/// Shows the level when a player is provided, nothing otherwise.
#[component]
fn optional_level(cx: Scope) -> impl View {
    let player = cx.use_context::<Player>().ok();
    label(move || match player {
        Some(p) => p.level().get().to_string(),
        None => String::from("-"),
    })
}

let cx = twine::reactive::Runtime::take().unwrap().create_root();
let missing = cx.use_context::<Player>().err().unwrap();
assert!(missing.to_string().ends_with("Player not provided"));
# let _ = optional_level();
```

# Checklist

- [ ] The store is a `Copy` struct of signals, memos and stored values; fields private.
- [ ] Views read through read-only accessors; every change is an action method.
- [ ] Derived values are memos in the store, not recomputed in each view.
- [ ] Provided once, at the root (or at the root of the subtree that owns it); one type per
  store, no bare `Signal<T>` contexts.
- [ ] Components look the store up once, while building (`cx.expect_context::<Store>()`),
  and capture the handle.
- [ ] Ports feed actions (`Store::connect`); tests pass fresh ports.
- [ ] Tests reach the store through `t.root_scope().expect_context::<Store>()` and drive it
  from both sides.
