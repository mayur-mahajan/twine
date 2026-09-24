//! `NodeRef`, tweens, animations, timers and channel messages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake as TaskWake, Waker};

use twine_reactive::debug_stats;
use twine_testing::{TestUi, by_id, by_text, capture_logs};
use twine_view::prelude::*;
use twine_widgets::label::Label;

#[test]
fn node_ref_with_mut_in_handler() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let r: NodeRef<Label> = cx.node_ref();
        column((
            label("before").node_ref(r).test_id("l"),
            button(label("poke")).on_click(move || {
                let n = r.with_mut(|l: &mut Label, wcx| {
                    l.set_text(wcx, "after");
                    7
                });
                assert_eq!(n, Some(7));
            }),
        ))
    });
    t.run_until_idle();
    t.find(by_text("poke")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("l")).text(), "after");
}

#[test]
fn node_ref_outside_ui_warns() {
    let refs: Rc<Cell<Option<NodeRef<Label>>>> = Rc::default();
    let r2 = refs.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let r = cx.node_ref::<Label>();
        r2.set(Some(r));
        label("x").node_ref(r)
    });
    t.run_until_idle();
    let r = refs.get().unwrap();
    let (res, logs) = capture_logs(|| r.with_mut(|l: &mut Label, wcx| l.set_text(wcx, "no")));
    assert!(res.is_none());
    assert!(
        logs.iter()
            .any(|l| l.level == log::Level::Warn && l.message.contains("outside"))
    );
    // A wrong widget type is refused too (inside the Ui).
    let wrong: NodeRef<twine_widgets::button::Button> = NodeRef::new(t.root_scope());
    assert!(wrong.with_mut(|_, _| ()).is_none());
}

#[test]
fn tween_reaches_target_and_idles() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let open = cx.signal(false);
        cx.provide(open);
        let h = cx.tween(
            move || if open.get() { 80 } else { 20 },
            Duration::ms(250),
            Easing::EaseInOut,
        );
        cx.provide(h);
        container(()).height(h).width(50).test_id("c")
    });
    t.run_until_idle();
    assert_eq!(t.find(by_id("c")).coords().height(), 20);
    let open = t.root_scope().expect_context::<Signal<bool>>();
    open.set(true);
    t.advance(Duration::ms(100));
    let mid = t.find(by_id("c")).coords().height();
    assert!(mid > 20 && mid < 80, "{mid}");
    let took = t.run_until_idle();
    assert!(took <= Duration::ms(250), "{took:?}");
    assert_eq!(t.find(by_id("c")).coords().height(), 80);
    t.assert_idle();
}

#[test]
fn tween_retarget_midflight_is_continuous() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let target = cx.signal(0);
        cx.provide(target);
        let v = cx.tween(move || target.get(), Duration::ms(300), Easing::Linear);
        cx.provide(v);
        label(text!("{}", v.get()))
    });
    t.run_until_idle();
    let target = t.root_scope().expect_context::<Signal<i32>>();
    let v = t.root_scope().expect_context::<ReadSignal<i32>>();
    target.set(1000);
    let mut last = v.get_untracked();
    let mut max_step = 0;
    let step = t.engine().config().refr_period;
    for i in 0..30 {
        if i == 5 {
            target.set(-500); // retarget mid-flight
        }
        t.advance(step);
        let now = v.get_untracked();
        max_step = max_step.max((now - last).abs());
        last = now;
    }
    t.run_until_idle();
    assert_eq!(v.get_untracked(), -500);
    // Linear over 300 ms: 1500 units at most ~5 per ms plus one frame of slack.
    let per_frame = 1500 * step.as_micros() as i32 / 300_000;
    assert!(
        max_step <= per_frame + per_frame / 2,
        "jump of {max_step} (frame step {per_frame})"
    );
}

#[test]
fn animation_controller_pause_resume() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let (a, ctl) = cx.animation(Anim::new(0, 1000).duration(Duration::ms(1000)));
        cx.provide((a, ctl.clone()));
        let c2 = ctl.clone();
        let c3 = ctl.clone();
        column((
            button(label("pause")).on_click(move || c2.pause()),
            button(label("resume")).on_click(move || c3.resume()),
            button(label("restart")).on_click(move || ctl.restart()),
        ))
    });
    t.advance(Duration::ms(200));
    let (a, _ctl) = t
        .root_scope()
        .expect_context::<(ReadSignal<i32>, AnimController)>();
    t.find(by_text("pause")).click();
    let paused = a.get_untracked();
    assert!(paused > 0 && paused < 1000);
    t.advance(Duration::ms(300));
    assert_eq!(a.get_untracked(), paused, "paused");
    t.find(by_text("resume")).click();
    t.advance(Duration::ms(100));
    assert!(a.get_untracked() > paused);
    t.run_until_idle();
    assert_eq!(a.get_untracked(), 1000);
    t.find(by_text("restart")).click();
    t.advance(Duration::ms(100));
    assert!(a.get_untracked() < 1000);
    t.run_until_idle();
}

#[test]
fn interval_stops_on_dispose() {
    let ticks = Rc::new(Cell::new(0));
    let tk = ticks.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let on = cx.signal(true);
        cx.provide(on);
        let tk = tk.clone();
        when(
            move || on.get(),
            move |cx| {
                let tk = tk.clone();
                cx.interval(Duration::ms(100), move || tk.set(tk.get() + 1));
                label("ticking")
            },
        )
    });
    t.advance(Duration::ms(1000));
    let n = ticks.get();
    // LVGL timers restart from the time they ran: with frames every `refr_period` they drift.
    assert!((7..=11).contains(&n), "{n}");
    let on = t.root_scope().expect_context::<Signal<bool>>();
    on.set(false);
    t.run_until_idle();
    t.advance(Duration::ms(1000));
    assert_eq!(ticks.get(), n);
    assert_eq!(t.engine().timer_count(), 0);
    t.assert_idle();
}

#[test]
fn timeout_runs_once() {
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        let r = r.clone();
        cx.timeout(Duration::ms(300), move || r.set(r.get() + 1));
        label("x")
    });
    t.advance(Duration::ms(200));
    assert_eq!(runs.get(), 0);
    t.advance(Duration::ms(200));
    assert_eq!(runs.get(), 1);
    t.advance(Duration::ms(1000));
    assert_eq!(runs.get(), 1);
    t.run_until_idle();
    assert_eq!(t.engine().timer_count(), 0);
}

static BATCH_CH: Channel<i32, 8> = Channel::new();

#[test]
fn on_message_drains_in_batch() {
    let mut t = TestUi::new(200, 100).mount(|cx| {
        let last = cx.signal(0);
        cx.on_message(&BATCH_CH, move |v| last.set(v));
        label(text!("{}", last.get())).test_id("l")
    });
    t.run_until_idle();
    for v in [1, 2, 3] {
        BATCH_CH.try_send(v).unwrap();
    }
    let runs = debug_stats().effect_runs;
    t.update();
    assert_eq!(
        debug_stats().effect_runs - runs,
        1,
        "one binding run for three messages"
    );
    assert_eq!(t.find(by_id("l")).text(), "3");
    t.run_until_idle();
}

static THREAD_CH: Channel<u32, 4> = Channel::new();

struct Count(AtomicUsize);

impl TaskWake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn channel_send_from_thread_wakes_ui() {
    let got = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    let mut t = TestUi::new(200, 100).mount(move |cx| {
        cx.on_message(&THREAD_CH, move |v| g.borrow_mut().push(v));
        label("x")
    });
    assert_eq!(t.run_until_idle(), Duration::ZERO);
    let woken = Arc::new(Count(AtomicUsize::new(0)));
    t.waker().register(&Waker::from(woken.clone()));
    std::thread::spawn(|| THREAD_CH.try_send(42).unwrap())
        .join()
        .unwrap();
    assert!(woken.0.load(Ordering::SeqCst) >= 1, "the send woke the UI task");
    assert!(t.waker().is_set());
    t.update();
    assert_eq!(*got.borrow(), [42]);
    t.run_until_idle();
}
