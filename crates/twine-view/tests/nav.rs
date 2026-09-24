//! Navigation, screens and modals.

use std::cell::{Cell, RefCell};
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
    let modal: Rc<RefCell<Option<ModalHandle>>> = Rc::default();
    cx.provide(clicks);
    let (m1, m2) = (modal.clone(), modal.clone());
    column((
        label(text!("clicks {}", clicks.get())).test_id("clicks"),
        button(label("Below")).on_click(move || clicks.update(|c| *c += 1)),
        button(label("Open")).on_click(move || {
            let m2 = m2.clone();
            let h = cx.show_modal(move |_| {
                container(
                    column((
                        label("Sure?"),
                        button(label("Close")).on_click(move || {
                            if let Some(h) = m2.borrow().as_ref() {
                                h.close();
                            }
                        }),
                    ))
                    .gap(6)
                    .align_items(FlexAlign::Center),
                )
            });
            *m1.borrow_mut() = Some(h);
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
    let reactive = twine_reactive::debug_stats();
    let ((), stats) = count_allocs(|| {
        for _ in 0..100 {
            cycle(&mut t);
        }
    });
    assert_eq!(t.engine().tree().len(), nodes);
    assert_eq!(twine_reactive::debug_stats().nodes, reactive.nodes);
    assert_eq!(twine_reactive::debug_stats().scopes, reactive.scopes);
    assert!(stats.live.abs() <= 256, "heap changed by {} bytes", stats.live);
}
