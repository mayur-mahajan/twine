//! Navigation, screens and modals.

use std::cell::Cell;
use std::rc::Rc;

use twine_hal::Key;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

thread_local! {
    static SETTINGS_SIGNAL: Cell<Option<Signal<u32>>> = const { Cell::new(None) };
    static HOME_CLICKS: Cell<u32> = const { Cell::new(0) };
}

const D: Duration = Duration::ms(300);

fn app(cx: Scope) -> impl View {
    navigator(cx, home)
}

fn home(cx: Scope) -> impl View {
    let nav = use_navigator(cx);
    column((
        label("Home").test_id("title"),
        button(label("Settings")).on_click(move || nav.push(settings, ScreenAnim::MoveLeft(D))),
        button(label("Count")).on_click(|| HOME_CLICKS.with(|c| c.set(c.get() + 1))),
    ))
    .gap(8)
    .padding(8)
}

fn settings(cx: Scope) -> impl View {
    let nav = use_navigator(cx);
    let s = cx.signal(1u32);
    SETTINGS_SIGNAL.with(|c| c.set(Some(s)));
    column((
        label(text!("Settings {}", s.get())).test_id("title"),
        button(label("Back")).on_click(move || {
            nav.pop(ScreenAnim::MoveRight(D));
        }),
    ))
    .gap(8)
    .padding(8)
}

/// Compile-time check: every view handle is `Copy`, like `Signal`.
const _: fn() = || {
    fn is_copy<T: Copy>() {}
    is_copy::<Navigator>();
    is_copy::<ModalHandle>();
    is_copy::<ThemeHandle>();
    is_copy::<AnimController>();
    is_copy::<NodeRef<twine_widgets::label::Label>>();
    is_copy::<MenuPageRef>();
};

fn nav_of(t: &TestUi) -> Navigator {
    t.root_scope().expect_context::<Navigator>()
}

#[test]
fn push_pop_disposes_popped_scope() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    t.find(by_text("Settings")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Settings 1");
    assert_eq!(nav_of(&t).depth(), 2);
    let s = SETTINGS_SIGNAL.with(Cell::get).unwrap();
    assert!(s.is_alive());
    t.find(by_text("Back")).click();
    t.run_until_idle();
    assert_eq!(nav_of(&t).depth(), 1);
    assert!(!s.is_alive(), "the popped screen's scope was disposed");
    assert_eq!(t.find(by_id("title")).text(), "Home");
    assert_eq!(
        t.engine().screens(t.harness_mut_display()).len(),
        2,
        "the Ui screen and home"
    );
}

trait Display {
    fn harness_mut_display(&self) -> twine_engine::DisplayId;
}

impl Display for TestUi {
    fn harness_mut_display(&self) -> twine_engine::DisplayId {
        self.engine().default_display().unwrap()
    }
}

#[test]
fn pop_at_root_returns_false() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    assert!(!nav.pop(ScreenAnim::None));
    nav.push(settings, ScreenAnim::None);
    assert_eq!(nav.depth(), 2, "queued operations count");
    assert!(nav.pop(ScreenAnim::None));
    assert!(!nav.pop(ScreenAnim::None));
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
        .expect_context::<Signal<Option<ModalHandle>>>()
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
    let modal = t.root_scope().expect_context::<ModalHandle>();
    assert!(modal.is_open());
    assert!(!t.find_all(by_text("M")).is_empty());
    modal.close();
    t.run_until_idle();
    assert!(t.find_all(by_text("M")).is_empty());
    t.root_scope().expect_context::<Scope>().dispose();
    // The handle's state went with its scope: every method is a no-op, none panics.
    assert!(!modal.is_open());
    assert_eq!(modal.node(), None);
    modal.close();
    t.run_until_idle();
}

#[test]
fn navigator_used_from_a_loop_without_clone() {
    fn home(cx: Scope) -> impl View {
        let nav = use_navigator(cx);
        // One `Copy` handle moved into many handlers: no `clone()`.
        let buttons: Vec<_> = (0..4u32)
            .map(|i| {
                button(label(format!("go {i}")))
                    .on_click(move || nav.push(move |_| label(format!("screen {i}")), ScreenAnim::None))
            })
            .collect();
        column(buttons)
    }
    let mut t = TestUi::new(240, 240).mount(|cx| navigator(cx, home));
    t.run_until_idle();
    let nav = nav_of(&t);
    for i in 0..4u32 {
        t.find(by_text(&format!("go {i}"))).click();
        t.run_until_idle();
        assert_eq!(nav.depth(), 2);
        assert!(!t.find_all(by_text(&format!("screen {i}"))).is_empty());
        nav.pop(ScreenAnim::None);
        t.run_until_idle();
        assert_eq!(nav.depth(), 1);
    }
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
fn navigation_back_and_forth_leaks_nothing() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let cycle = |t: &mut TestUi| {
        let nav = nav_of(t);
        nav.push(settings, ScreenAnim::MoveLeft(D));
        t.run_until_idle();
        nav.pop(ScreenAnim::MoveRight(D));
        t.run_until_idle();
    };
    // Warm up caches and pools.
    for _ in 0..3 {
        cycle(&mut t);
    }
    let nodes = t.engine().tree().len();
    let reactive = rt().stats();
    let ((), stats) = count_allocs(|| {
        for _ in 0..100 {
            cycle(&mut t);
        }
    });
    assert_eq!(t.engine().tree().len(), nodes);
    assert_eq!(rt().stats().nodes, reactive.nodes);
    assert_eq!(rt().stats().scopes, reactive.scopes);
    assert!(stats.live.abs() <= 256, "heap changed by {} bytes", stats.live);
}

// ---- Text style of screens and modals ------------------------------------------------------

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_20};

use twine_reactive::Runtime;

/// The calling thread's reactive runtime.
fn rt() -> Runtime {
    Runtime::current_thread()
}

/// Whether the node found by `id` is drawn with `font`.
fn font_is(t: &TestUi, id: &'static str, font: &'static twine_text::Font) -> bool {
    let n = t.find(by_id(id)).id();
    core::ptr::eq(t.engine().style_font(n, twine_style::Part::Main), font)
}

fn styled_nav_app(cx: Scope) -> impl View {
    column((navigator(cx, styled_home),)).font(&MONTSERRAT_20)
}

fn styled_home(cx: Scope) -> impl View {
    let nav = use_navigator(cx);
    column((
        label("Home").test_id("home_title"),
        button(label("Next")).on_click(move || nav.push(styled_next, ScreenAnim::None)),
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

// ---- Screens as closures ---------------------------------------------------------------------

/// A screen taking data from its caller (captured by the closure passed to `push`).
fn detail(cx: Scope, id: u32, name: String) -> impl View {
    let nav = use_navigator(cx);
    column((
        label(text!("Item {id}: {name}")).test_id("title"),
        button(label("Back")).on_click(move || {
            nav.pop(ScreenAnim::None);
        }),
    ))
}

#[test]
fn push_screen_capturing_an_id() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    let id = 42u32;
    // A non-`Clone` capture: the screen closure is `FnOnce`.
    let name = String::from("answer");
    nav.push(move |cx| detail(cx, id, name), ScreenAnim::None);
    t.run_until_idle();
    assert_eq!(nav.depth(), 2);
    assert_eq!(t.find(by_id("title")).text(), "Item 42: answer");
    t.find(by_text("Back")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Home");
}

#[test]
fn replace_with_capturing_closure() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    let built = Rc::new(Cell::new(0u32));
    let b = built.clone();
    let name = String::from("replaced");
    nav.replace(
        move |cx| {
            b.set(b.get() + 1);
            detail(cx, 7, name)
        },
        ScreenAnim::None,
    );
    assert_eq!(
        built.get(),
        0,
        "built when the replace is applied, not when queued"
    );
    t.run_until_idle();
    assert_eq!(built.get(), 1, "called exactly once");
    assert_eq!(nav.depth(), 1);
    assert_eq!(t.find(by_id("title")).text(), "Item 7: replaced");
    t.run_until_idle();
    assert_eq!(built.get(), 1);
}

#[test]
fn navigator_initial_capturing_closure() {
    let mut t = TestUi::new(240, 160).mount(|cx| {
        let name = String::from("first");
        navigator(cx, move |cx| detail(cx, 1, name))
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Item 1: first");
}

#[test]
fn push_boxes_screen_closure_once() {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    let nav = nav_of(&t);
    // Warm up the queue's capacity and the runtime's pending lists.
    for _ in 0..3 {
        nav.push(settings, ScreenAnim::None);
        t.run_until_idle();
        nav.pop(ScreenAnim::None);
        t.run_until_idle();
    }
    // A `fn` item is zero-sized: queuing it allocates nothing.
    let ((), fn_item) = count_allocs(|| nav.push(settings, ScreenAnim::None));
    assert_eq!(fn_item.allocs, 0, "fn item: {fn_item:?}");
    t.run_until_idle();
    nav.pop(ScreenAnim::None);
    t.run_until_idle();
    // A capturing closure is boxed exactly once.
    let (id, name) = (9u32, String::from("nine"));
    let ((), closure) = count_allocs(move || nav.push(move |cx| detail(cx, id, name), ScreenAnim::None));
    assert_eq!(closure.allocs, 1, "capturing closure: {closure:?}");
    t.run_until_idle();
    assert_eq!(t.find(by_id("title")).text(), "Item 9: nine");
}
