//! Typed navigation, screens and modals.

use std::cell::Cell;

use twine_hal::Key;
use twine_reactive::Runtime;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

thread_local! {
    static SETTINGS_SIGNAL: Cell<Option<Signal<u32>>> = const { Cell::new(None) };
    static HOME_CLICKS: Cell<u32> = const { Cell::new(0) };
    static BUILT: Cell<u32> = const { Cell::new(0) };
}

const D: Duration = Duration::ms(300);

#[derive(Clone, PartialEq, Debug)]
enum Route {
    Home,
    Settings,
    Detail(u32, String),
    Editor,
    Redirect,
}

fn app(cx: Scope) -> impl View {
    navigator(cx, Route::Home, page)
}

fn page(cx: Scope, route: Route) -> AnyView {
    BUILT.with(|b| b.set(b.get() + 1));
    match route {
        Route::Home => home(cx).into_any(),
        Route::Settings => settings(cx).into_any(),
        Route::Detail(id, name) => detail(cx, id, name).into_any(),
        Route::Editor => editor(cx).into_any(),
        Route::Redirect => {
            // A page that sends the user elsewhere while it is built.
            use_navigator::<Route>(cx)
                .unwrap()
                .replace(Route::Settings, ScreenAnim::None);
            label("redirecting").into_any()
        }
    }
}

fn home(cx: Scope) -> impl View {
    let nav = use_navigator::<Route>(cx).unwrap();
    column((
        label("Home").test_id("title"),
        button(label("Settings")).on_click(move || nav.push(Route::Settings, ScreenAnim::MoveLeft(D))),
        button(label("Count")).on_click(|| HOME_CLICKS.with(|c| c.set(c.get() + 1))),
    ))
    .gap(8)
    .padding(8)
}

fn settings(cx: Scope) -> impl View {
    let nav = use_navigator::<Route>(cx).unwrap();
    let s = cx.signal(1u32);
    SETTINGS_SIGNAL.with(|c| c.set(Some(s)));
    column((
        label(text!("Settings {}", s.get())).test_id("title"),
        button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(D))),
    ))
    .gap(8)
    .padding(8)
}

/// A page taking data from its route.
fn detail(cx: Scope, id: u32, name: String) -> impl View {
    let nav = use_navigator::<Route>(cx).unwrap();
    column((
        label(text!("Item {id}: {name}")).test_id("title"),
        button(label("Back")).on_click(move || nav.pop(ScreenAnim::None)),
    ))
}

thread_local! {
    static EDITOR: Cell<Option<(Signal<bool>, Signal<u32>)>> = const { Cell::new(None) };
    static GUARD_DEST: std::cell::RefCell<Vec<Route>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A page with unsaved changes: its guard denies leaving while `dirty`.
fn editor(cx: Scope) -> impl View {
    let nav = use_navigator::<Route>(cx).unwrap();
    let dirty = cx.signal(false);
    let asked = cx.signal(0u32);
    EDITOR.with(|c| c.set(Some((dirty, asked))));
    nav.on_leave(cx, move |to: &Route| {
        GUARD_DEST.with(|d| d.borrow_mut().push(to.clone()));
        // A guard may not navigate.
        assert_eq!(nav.push(Route::Settings, ScreenAnim::None), NavOutcome::Busy);
        if dirty.get() {
            asked.update(|a| *a += 1);
            Leave::Deny
        } else {
            Leave::Allow
        }
    });
    label("Editor").test_id("title")
}

/// Compile-time check: every view handle is `Copy`, like `Signal`.
const _: fn() = || {
    fn is_copy<T: Copy>() {}
    is_copy::<Navigator<Route>>();
    is_copy::<ModalHandle>();
    is_copy::<ThemeHandle>();
    is_copy::<AnimController>();
    is_copy::<NodeRef<twine_widgets::label::Label>>();
    is_copy::<MenuPageRef>();
};

fn nav_of(t: &TestUi) -> Navigator<Route> {
    use_navigator::<Route>(t.root_scope()).unwrap()
}

fn screens(t: &TestUi) -> usize {
    let e = t.engine();
    e.screens(e.default_display().unwrap()).len()
}

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

fn detail_route(id: u32, name: &str) -> Route {
    Route::Detail(id, name.to_string())
}

#[test]
fn push_pop_disposes_popped_scope() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    assert_eq!(nav_of(&t).stack(), [Route::Home]);
    t.find(by_text("Settings")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
    assert_eq!(nav_of(&t).stack(), [Route::Home, Route::Settings]);
    assert_eq!(nav_of(&t).current(), Some(Route::Settings));
    let s = SETTINGS_SIGNAL.with(Cell::get).unwrap();
    assert!(s.is_alive());
    t.find(by_text("Back")).click();
    t.run_until_idle();
    assert_eq!(nav_of(&t).depth(), 1);
    assert!(!s.is_alive(), "the popped page's scope was disposed");
    assert_eq!(t.find(by_id("title")).text(), "Home");
    assert_eq!(screens(&t), 2, "the Ui screen and home");
}

#[test]
fn pop_at_root_is_refused() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::AtRoot);
    assert_eq!(nav.pop_to_root(ScreenAnim::None), NavOutcome::AtRoot);
    assert_eq!(nav.push(Route::Settings, ScreenAnim::None), NavOutcome::Accepted);
    assert_eq!(nav.depth(), 2, "requests change the stack at once");
    assert!(nav.pop(ScreenAnim::None).is_accepted());
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::AtRoot);
    t.run_until_idle();
    assert_eq!(nav.depth(), 1);
    assert_eq!(t.find(by_id("title")).text(), "Home");
}

#[test]
fn screen_anim_move_left_positions() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    t.find(by_text("Settings")).click();
    t.advance(Duration::ms(150));
    let d = t.engine().default_display().unwrap();
    let new = t.engine().active_screen(d).unwrap();
    let x = t.engine().coords(new).x0;
    assert!(
        (x - 120).abs() <= 20,
        "new screen at x = {x} after half the animation"
    );
    t.assert_panel_snapshot("nav_mid_anim_move_left");
    t.run_until_idle();
    assert_eq!(t.engine().coords(new).x0, 0);
}

#[test]
fn pop_animation_deletes_the_page_when_it_ends() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    let s = SETTINGS_SIGNAL.with(Cell::get).unwrap();
    let d = t.engine().default_display().unwrap();
    let settings_screen = t.engine().active_screen(d).unwrap();
    nav.pop(ScreenAnim::MoveRight(D));
    t.advance(Duration::ms(150));
    assert!(
        t.engine().screen_anim_running(d),
        "the engine's screen-load animation runs"
    );
    assert!(
        t.engine().tree().contains(settings_screen),
        "still shown while it moves out"
    );
    assert!(s.is_alive());
    let x = t.engine().coords(settings_screen).x0;
    assert!(
        x > 0 && x < 240,
        "the old page at x = {x}, moving out to the right"
    );
    t.run_until_idle();
    assert!(!t.engine().tree().contains(settings_screen));
    assert!(!s.is_alive());
    assert_eq!(screens(&t), 2);
}

#[test]
fn only_active_screen_receives_input() {
    HOME_CLICKS.with(|c| c.set(0));
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let count_btn = t.find(by_text("Count")).coords().center();
    t.tap(count_btn);
    t.run_until_idle();
    assert_eq!(HOME_CLICKS.with(Cell::get), 1);
    t.find(by_text("Settings")).click();
    t.run_until_idle();
    t.tap(count_btn);
    t.run_until_idle();
    assert_eq!(HOME_CLICKS.with(Cell::get), 1, "home is not active");
}

#[test]
fn push_route_with_data() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(detail_route(42, "answer"), ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.depth(), 2);
    assert_eq!(t.find(by_id("title")).text(), "Item 42: answer");
    t.find(by_text("Back")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Home");
}

#[test]
fn replace_keeps_depth_and_disposes_the_old_page() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    let s = SETTINGS_SIGNAL.with(Cell::get).unwrap();
    assert!(
        nav.replace(detail_route(7, "replaced"), ScreenAnim::FadeIn(Duration::ms(100)))
            .is_accepted()
    );
    assert_eq!(nav.stack(), [Route::Home, detail_route(7, "replaced")]);
    t.run_until_idle();
    assert!(!s.is_alive());
    assert_eq!(t.find(by_id("title")).text(), "Item 7: replaced");
    assert_eq!(screens(&t), 3, "the Ui screen, home and the detail");
    assert!(nav.pop(ScreenAnim::None).is_accepted());
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Home");
}

#[test]
fn replace_root() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.replace(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.stack(), [Route::Settings]);
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
    assert!(t.find_all(by_text("Home")).is_empty());
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::AtRoot);
    assert_eq!(screens(&t), 2);
}

#[test]
fn regression_replacing_the_root_keeps_the_ui_focus_group() {
    // The first page uses the focus group the `Ui` had; replacing it must not delete that
    // group (the navigator only deletes the groups it created).
    let mut t = TestUi::new(240, 160)
        .mount_engine(|e| {
            let g = e.create_group().unwrap();
            e.set_default_group(Some(g));
        })
        .mount(|cx| column((button(label("outside")), navigator(cx, Route::Home, page))));
    t.run_until_idle();
    let ui_group = t.engine().default_group();
    assert!(ui_group.is_some());
    let d = t.engine().default_display().unwrap();
    let ui_screen = t.engine().screens(d)[0];
    let outside = t.engine().tree().children(ui_screen).next().unwrap();
    let outside_btn = t.engine().tree().children(outside).next().unwrap();
    assert_eq!(t.engine().group_of(outside_btn), ui_group);
    let nav = nav_of(&t);
    nav.replace(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(
        t.engine().group_of(outside_btn),
        ui_group,
        "the Ui's group still exists"
    );
    assert_ne!(
        t.engine().default_group(),
        ui_group,
        "the new root has a group of its own"
    );
}

#[test]
fn pop_to_and_pop_to_root() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Settings, ScreenAnim::None);
    nav.push(detail_route(1, "a"), ScreenAnim::None);
    nav.push(detail_route(2, "b"), ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.depth(), 4);
    assert_eq!(nav.pop_to(&Route::Editor, ScreenAnim::None), NavOutcome::NotFound);
    assert_eq!(
        nav.pop_to(&detail_route(2, "b"), ScreenAnim::None),
        NavOutcome::Accepted,
        "already there"
    );
    assert_eq!(nav.depth(), 4);
    assert!(
        nav.pop_to(&Route::Settings, ScreenAnim::MoveRight(D))
            .is_accepted()
    );
    assert_eq!(nav.stack(), [Route::Home, Route::Settings]);
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
    assert_eq!(screens(&t), 3);
    assert!(nav.pop_to_root(ScreenAnim::None).is_accepted());
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Home");
    assert_eq!(screens(&t), 2);
}

#[test]
fn several_requests_in_one_handler_build_only_the_top_page() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    BUILT.with(|b| b.set(0));
    nav.push(Route::Settings, ScreenAnim::None);
    nav.push(detail_route(3, "top"), ScreenAnim::MoveLeft(D));
    t.run_until_idle();
    assert_eq!(BUILT.with(Cell::get), 1, "only the top page was built");
    assert_eq!(screens(&t), 3, "the Ui screen, home and the top page");
    nav.pop(ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(BUILT.with(Cell::get), 2, "settings is built when it is shown");
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
}

#[test]
fn reset_replaces_the_stack() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    let s = SETTINGS_SIGNAL.with(Cell::get).unwrap();
    assert_eq!(
        nav.reset(Vec::<Route>::new(), ScreenAnim::None),
        NavOutcome::Empty
    );
    BUILT.with(|b| b.set(0));
    assert!(
        nav.reset(
            [Route::Settings, detail_route(9, "nine")],
            ScreenAnim::FadeIn(Duration::ms(100))
        )
        .is_accepted()
    );
    t.run_until_idle();
    assert!(!s.is_alive(), "every old page was removed");
    assert_eq!(BUILT.with(Cell::get), 1);
    assert_eq!(nav.stack(), [Route::Settings, detail_route(9, "nine")]);
    assert_eq!(t.find(by_id("title")).text(), "Item 9: nine");
    assert_eq!(screens(&t), 2, "the Ui screen and the new top page");
    nav.pop(ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
}

#[test]
fn restore_starts_with_a_saved_stack() {
    BUILT.with(|b| b.set(0));
    let mut t = TestUi::new(240, 160).mount(|cx| {
        navigator(cx, Route::Home, page).restore([Route::Home, Route::Settings, detail_route(5, "saved")])
    });
    t.run_until_idle();
    let nav = nav_of(&t);
    assert_eq!(nav.depth(), 3);
    assert_eq!(BUILT.with(Cell::get), 1, "only the top page is built");
    assert_eq!(t.find(by_id("title")).text(), "Item 5: saved");
    nav.pop_to_root(ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Home");
    // An empty saved stack keeps the initial route.
    let mut t = TestUi::new(240, 160).mount(|cx| navigator(cx, Route::Settings, page).restore([]));
    t.run_until_idle();
    assert_eq!(nav_of(&t).stack(), [Route::Settings]);
}

#[test]
fn guard_denies_and_allows() {
    GUARD_DEST.with(|d| d.borrow_mut().clear());
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Editor, ScreenAnim::None);
    t.run_until_idle();
    let (dirty, asked) = EDITOR.with(Cell::get).unwrap();
    // A push keeps the page: no guard.
    nav.push(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    assert!(GUARD_DEST.with(|d| d.borrow().is_empty()));
    nav.pop(ScreenAnim::None);
    t.run_until_idle();
    assert!(
        GUARD_DEST.with(|d| d.borrow().is_empty()),
        "the settings page has no guard"
    );
    dirty.set(true);
    for (outcome, what) in [
        (nav.pop(ScreenAnim::None), "pop"),
        (nav.replace(Route::Settings, ScreenAnim::None), "replace"),
        (nav.pop_to_root(ScreenAnim::None), "pop_to_root"),
        (nav.reset([Route::Home], ScreenAnim::None), "reset"),
    ] {
        assert_eq!(outcome, NavOutcome::Denied, "{what}");
    }
    assert_eq!(asked.get_untracked(), 4);
    assert_eq!(
        GUARD_DEST.with(|d| d.borrow().clone()),
        [Route::Home, Route::Settings, Route::Home, Route::Home],
        "the guard sees the route that would be shown"
    );
    t.run_until_idle();
    assert_eq!(
        nav.stack(),
        [Route::Home, Route::Editor],
        "denied: nothing changed"
    );
    assert_eq!(t.find(by_id("title")).text(), "Editor");
    dirty.set(false);
    assert!(nav.pop(ScreenAnim::None).is_accepted());
    t.run_until_idle();
    assert!(!dirty.is_alive(), "the editor was disposed (its guard with it)");
    assert_eq!(t.find(by_id("title")).text(), "Home");
}

thread_local! {
    static INNER: Cell<Option<Scope>> = const { Cell::new(None) };
}

#[test]
fn guard_registered_in_an_inner_scope_lives_with_it() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        navigator(cx, Route::Home, |cx, route| {
            let nav = use_navigator::<Route>(cx).unwrap();
            if route == Route::Editor {
                let inner = cx.child();
                INNER.with(|i| i.set(Some(inner)));
                nav.on_leave(inner, |_| Leave::Deny);
            }
            label("page")
        })
    });
    t.run_until_idle();
    let nav = nav_of(&t);
    // A scope outside the pages cannot register a guard (logged, ignored).
    nav.on_leave(t.root_scope(), |_| Leave::Deny);
    nav.push(Route::Editor, ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::Denied);
    // The guard goes with the scope that registered it.
    INNER.with(Cell::get).unwrap().dispose();
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::Accepted);
    t.run_until_idle();
    assert_eq!(nav.depth(), 1);
}

#[test]
fn navigator_used_from_a_loop_without_clone() {
    fn home(cx: Scope) -> impl View {
        let nav = use_navigator::<Route>(cx).unwrap();
        // One `Copy` handle moved into many handlers: no `clone()`.
        let buttons: Vec<_> = (0..4u32)
            .map(|i| {
                button(label(format!("go {i}")))
                    .on_click(move || nav.push(Route::Detail(i, format!("screen {i}")), ScreenAnim::None))
            })
            .collect();
        column(buttons)
    }
    let mut t = TestUi::new(240, 240).mount(|cx| {
        navigator(cx, Route::Home, |cx, route| match route {
            Route::Detail(id, name) => detail(cx, id, name).into_any(),
            _ => home(cx).into_any(),
        })
    });
    t.run_until_idle();
    let nav = nav_of(&t);
    for i in 0..4u32 {
        t.find(by_text(&format!("go {i}"))).click();
        t.run_until_idle();
        assert_eq!(nav.depth(), 2);
        assert_eq!(t.find(by_id("title")).text(), format!("Item {i}: screen {i}"));
        nav.pop(ScreenAnim::None);
        t.run_until_idle();
        assert_eq!(nav.depth(), 1);
    }
}

#[test]
fn navigation_back_and_forth_leaks_nothing() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let cycle = |t: &mut TestUi| {
        GUARD_DEST.with(|d| d.borrow_mut().clear()); // the editor's guard records here
        let nav = nav_of(t);
        nav.push(Route::Settings, ScreenAnim::MoveLeft(D));
        t.run_until_idle();
        nav.replace(Route::Editor, ScreenAnim::FadeIn(D));
        t.run_until_idle();
        nav.pop(ScreenAnim::MoveRight(D));
        t.run_until_idle();
        nav.reset([Route::Home], ScreenAnim::None);
        t.run_until_idle();
    };
    // Warm up caches and pools.
    for _ in 0..3 {
        cycle(&mut t);
    }
    let nodes = t.engine().tree().len();
    let reactive = rt().stats();
    let screens_before = screens(&t);
    let ((), stats) = count_allocs(|| {
        for _ in 0..50 {
            cycle(&mut t);
        }
    });
    assert_eq!(t.engine().tree().len(), nodes);
    assert_eq!(screens(&t), screens_before);
    assert_eq!(rt().stats().nodes, reactive.nodes);
    assert_eq!(rt().stats().scopes, reactive.scopes);
    assert!(stats.live.abs() <= 256, "heap changed by {} bytes", stats.live);
}

#[test]
fn navigator_removed_with_its_scope_deletes_its_pages() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let show = cx.signal(true);
        cx.provide(show);
        column((
            label("outside").test_id("outside"),
            when(move || show.get(), |cx| navigator(cx, Route::Home, page)),
        ))
    });
    t.run_until_idle();
    let show = t.root_scope().use_context::<Signal<bool>>().unwrap();
    let d = t.engine().default_display().unwrap();
    let ui_screen = t.engine().screens(d)[0];
    show.set(false);
    t.run_until_idle();
    show.set(true);
    t.run_until_idle();
    let nodes = t.engine().tree().len();
    let reactive = rt().stats();
    for _ in 0..3 {
        // The navigator's handle is in the `when` branch's scope: find it through a page.
        let e = t.engine();
        let active = e.active_screen(d).unwrap();
        drop(e);
        assert_ne!(active, ui_screen, "a page is shown");
        show.set(false);
        t.run_until_idle();
        assert_eq!(
            t.engine().active_screen(d),
            Some(ui_screen),
            "the Ui's screen is shown again"
        );
        assert_eq!(screens(&t), 1, "the pages were deleted");
        show.set(true);
        t.run_until_idle();
    }
    assert_eq!(t.engine().tree().len(), nodes);
    assert_eq!(rt().stats().nodes, reactive.nodes);
    assert_eq!(rt().stats().scopes, reactive.scopes);
}

#[test]
fn push_allocates_nothing_after_warm_up() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    for _ in 0..3 {
        nav.push(Route::Settings, ScreenAnim::None);
        t.run_until_idle();
        nav.pop(ScreenAnim::None);
        t.run_until_idle();
    }
    let ((), push) = count_allocs(|| {
        nav.push(Route::Settings, ScreenAnim::None);
    });
    assert_eq!(push.allocs, 0, "push: {push:?}");
    let ((), pop) = count_allocs(|| {
        nav.pop(ScreenAnim::None);
    });
    assert_eq!(pop.allocs, 0, "pop: {pop:?}");
    t.run_until_idle();
}

thread_local! {
    static DEPTHS: std::cell::RefCell<Vec<(usize, Option<Route>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn reads_are_tracked() {
    DEPTHS.with(|d| d.borrow_mut().clear());
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let view = navigator(cx, Route::Home, page);
        let nav = use_navigator::<Route>(cx).unwrap();
        cx.effect(move || {
            let seen = (nav.depth(), nav.current());
            DEPTHS.with(|d| d.borrow_mut().push(seen));
        });
        view
    });
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Settings, ScreenAnim::None);
    t.run_until_idle();
    nav.pop(ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(
        DEPTHS.with(|d| d.borrow().clone()),
        [
            (1, Some(Route::Home)),
            (2, Some(Route::Settings)),
            (1, Some(Route::Home))
        ]
    );
}

#[test]
fn page_redirecting_while_built() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    nav.push(Route::Redirect, ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.stack(), [Route::Home, Route::Settings]);
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
    assert!(t.find_all(by_text("redirecting")).is_empty());
    assert_eq!(screens(&t), 3);
}

#[test]
fn disposed_navigator_is_inert() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let owner = cx.child();
        cx.provide(owner);
        navigator(owner, Route::Home, page)
    });
    t.run_until_idle();
    let nav = use_navigator::<Route>(t.root_scope().use_context::<Scope>().unwrap()).unwrap();
    t.root_scope().use_context::<Scope>().unwrap().dispose();
    assert_eq!(nav.push(Route::Settings, ScreenAnim::None), NavOutcome::Disposed);
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::Disposed);
    assert_eq!(nav.depth(), 0);
    assert_eq!(nav.current(), None);
    assert_eq!(nav.stack(), []);
    t.run_until_idle();
    assert_eq!(screens(&t), 1, "the pages were deleted at the next update");
}

// ---- Text style of screens and modals ------------------------------------------------------

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_20};

/// Whether the node found by `id` is drawn with `font`.
fn font_is(t: &TestUi, id: &'static str, font: &'static twine_text::Font) -> bool {
    let n = t.find(by_id(id)).id();
    core::ptr::eq(t.engine().style_font(n, twine_style::Part::Main), font)
}

fn styled_nav_app(cx: Scope) -> impl View {
    column((navigator(cx, false, |cx, next| {
        if next {
            styled_next(cx).into_any()
        } else {
            styled_home(cx).into_any()
        }
    }),))
    .font(&MONTSERRAT_20)
}

fn styled_home(cx: Scope) -> impl View {
    let nav = use_navigator::<bool>(cx).unwrap();
    column((
        label("Home").test_id("home_title"),
        button(label("Next")).on_click(move || nav.push(true, ScreenAnim::None)),
        button(label("Modal")).on_click(move || {
            let _ = cx.show_modal(|_, _| label("Hello").test_id("modal_label"));
        }),
    ))
}

fn styled_next(_cx: Scope) -> impl View {
    label("Next").test_id("next_title")
}

#[test]
fn navigator_screens_inherit_text_style_around_the_navigator() {
    let mut t = TestUi::new(240, 160).mount(styled_nav_app);
    t.run_until_idle();
    assert!(font_is(&t, "home_title", &MONTSERRAT_20));
    t.find(by_text("Next")).click();
    t.run_until_idle();
    assert!(font_is(&t, "next_title", &MONTSERRAT_20), "a pushed screen too");
    t.engine().tree().check_invariants().unwrap();
}

// ---- Modals ----------------------------------------------------------------------------------

fn modal_app(cx: Scope) -> impl View {
    let clicks = cx.signal(0u32);
    cx.provide(clicks);
    column((
        label(text!("clicks {}", clicks.get())).test_id("clicks"),
        button(label("Below")).on_click(move || clicks.update(|c| *c += 1)),
        button(label("Open")).on_click(move || {
            // The modal closes itself through the handle passed to its view closure.
            let _ = cx.show_modal(|_, modal: ModalHandle| {
                container(
                    column((
                        label("Sure?"),
                        button(label("Close")).on_click(move || modal.close()),
                    ))
                    .gap(6)
                    .align_items(CrossAlign::Center),
                )
            });
        }),
    ))
    .gap(8)
    .padding(8)
}

#[test]
fn modal_blocks_input_below() {
    let mut t = TestUi::new(240, 160).mount(modal_app);
    t.run_until_idle();
    let below = t.find(by_text("Below")).coords().center();
    t.find(by_text("Open")).click();
    t.run_until_idle();
    t.assert_snapshot("modal_open_light");
    t.tap(below);
    t.run_until_idle();
    assert_eq!(t.find(by_id("clicks")).text(), "clicks 0");
    t.find(by_text("Close")).click();
    t.run_until_idle();
    assert!(t.find_all(by_text("Sure?")).is_empty());
    t.tap(below);
    t.run_until_idle();
    assert_eq!(t.find(by_id("clicks")).text(), "clicks 1");
}

#[test]
fn modal_closes_itself_through_the_passed_handle() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let seen: Signal<Option<ModalHandle>> = cx.signal(None);
        cx.provide(seen);
        button(label("Open")).on_click(move || {
            let returned = cx.show_modal(move |_, modal: ModalHandle| {
                seen.set(Some(modal));
                button(label("Done")).on_click(move || modal.close())
            });
            assert!(returned.is_open());
        })
    });
    t.run_until_idle();
    t.find(by_text("Open")).click();
    t.run_until_idle();
    let modal = t
        .root_scope()
        .use_context::<Signal<Option<ModalHandle>>>()
        .unwrap()
        .get_untracked()
        .expect("the view closure received the handle");
    assert!(modal.is_open());
    assert!(modal.node().is_some());
    t.find(by_text("Done")).click();
    t.run_until_idle();
    assert!(!modal.is_open());
    assert!(t.find_all(by_text("Done")).is_empty(), "closed by its own button");
    modal.close(); // closing again does nothing
    t.run_until_idle();
}

#[test]
fn modal_handle_after_its_scope_is_disposed_is_inert() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let owner = cx.child();
        cx.provide(owner);
        cx.provide(owner.show_modal(|_, _| label("M")));
        label("root")
    });
    t.run_until_idle();
    let modal = t.root_scope().use_context::<ModalHandle>().unwrap();
    assert!(modal.is_open());
    assert!(!t.find_all(by_text("M")).is_empty());
    modal.close();
    t.run_until_idle();
    assert!(t.find_all(by_text("M")).is_empty());
    t.root_scope().use_context::<Scope>().unwrap().dispose();
    // The handle's state went with its scope: every method is a no-op, none panics.
    assert!(!modal.is_open());
    assert_eq!(modal.node(), None);
    modal.close();
    t.run_until_idle();
}

#[test]
fn modal_close_restores_focus_group() {
    let mut t = TestUi::new(240, 160).mount(modal_app);
    t.run_until_idle();
    t.key(Key::Next); // registers the keypad (default group)
    let e = t.engine();
    let keypad = e.inputs().last().unwrap();
    let main_group = e.input_group(keypad);
    drop(e);
    assert!(main_group.is_some());
    t.find(by_text("Open")).click();
    t.run_until_idle();
    let e = t.engine();
    let modal_group = e.input_group(keypad);
    assert_ne!(modal_group, main_group, "the modal has its own focus group");
    let close = t.find(by_text("Close")).id();
    let close_btn = e.tree().parent(close).unwrap();
    assert_eq!(e.group_of(close_btn), modal_group);
    drop(e);
    t.find(by_text("Close")).click();
    t.run_until_idle();
    assert_eq!(t.engine().input_group(keypad), main_group);
}

#[test]
fn modal_inherits_text_style_of_the_view_that_opened_it() {
    let mut t = TestUi::new(240, 160).mount(styled_nav_app);
    t.run_until_idle();
    t.find(by_text("Modal")).click();
    t.run_until_idle();
    assert!(font_is(&t, "modal_label", &MONTSERRAT_20));
    let n = t.find(by_id("modal_label")).id();
    let e = t.engine();
    let top = e.top_layer(e.default_display().unwrap()).unwrap();
    assert!(
        e.tree().ancestors(n).any(|a| a == top),
        "the modal is on the top layer"
    );
}

#[test]
fn modal_without_font_around_uses_theme_font() {
    fn app(cx: Scope) -> impl View {
        button(label("Modal")).on_click(move || {
            let _ = cx.show_modal(|_, _| label("Hello").test_id("modal_label"));
        })
    }
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    t.find(by_text("Modal")).click();
    t.run_until_idle();
    assert!(font_is(&t, "modal_label", &MONTSERRAT_14));
}

#[test]
fn use_navigator_outside_a_navigator_is_an_error_naming_the_route_type() {
    let t = TestUi::new(240, 160).mount(|_cx| label("no navigator"));
    let Err(err) = use_navigator::<Route>(t.root_scope()) else {
        panic!("no navigator provided")
    };
    assert_eq!(err.type_name(), core::any::type_name::<Navigator<Route>>());
    assert!(err.to_string().contains("Navigator<nav::Route>"), "{err}");
    // A navigator over another route type is not a navigator over `Route`.
    let t = TestUi::new(240, 160).mount(styled_nav_app);
    assert!(use_navigator::<Route>(t.root_scope()).is_err());
    assert!(use_navigator::<bool>(t.root_scope()).is_ok());
}

// `expect_navigator` is removed in the safety profile (`use_navigator` above is its form there).
#[cfg(not(feature = "safety"))]
#[test]
#[should_panic(expected = "Navigator<nav::Route> not provided")]
fn expect_navigator_outside_a_navigator_panics_with_the_route_type() {
    let t = TestUi::new(240, 160).mount(|_cx| label("no navigator"));
    let _ = twine_view::expect_navigator::<Route>(t.root_scope());
}
