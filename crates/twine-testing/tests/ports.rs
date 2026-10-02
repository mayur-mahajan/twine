//! Per-test cross-context objects (`TestUi::channel`, `latest`, `outbox`; R3.S05): tests that
//! the harness runs in parallel each get their own, so one test's messages never reach
//! another test's UI (the flakiness a shared `static` channel caused).

use twine_reactive::{Channel, Latest, Outbox};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;

#[derive(Clone, Copy)]
struct Ports {
    input: &'static Channel<u32, 4>,
    level: &'static Latest<u32>,
    out: &'static Outbox<u32, 4>,
}

fn app(cx: Scope, p: Ports) -> impl View {
    let sum = cx.signal(0u32);
    let level = cx.watch(p.level);
    cx.on_message(p.input, move |v| sum.update(|s| *s += v));
    column((
        label(text!("{}", sum.get())).test_id("sum"),
        label(text!("{}", level.get())).test_id("level"),
        button(label("send")).on_click(move || {
            let _ = p.out.try_send(sum.get_untracked());
        }),
    ))
}

fn ports() -> Ports {
    Ports {
        input: TestUi::channel(),
        level: TestUi::latest(0),
        out: TestUi::outbox(),
    }
}

/// One test's worth of traffic, identified by `id`: sums `id` 50 times.
fn exercise(id: u32) {
    let p = ports();
    let mut t = TestUi::new(160, 120).mount(move |cx| app(cx, p));
    for round in 1..=50 {
        p.input.try_send(id).unwrap();
        p.level.set(id * 1000 + round);
        t.run_until_idle();
        assert_eq!(t.find(by_id("sum")).text(), (id * round).to_string());
        assert_eq!(t.find(by_id("level")).text(), (id * 1000 + round).to_string());
    }
    t.find(by_text("send")).click();
    assert_eq!(p.out.try_recv(), Some(id * 50));
    assert_eq!(p.out.try_recv(), None);
}

#[test]
fn per_test_ports_a() {
    exercise(1);
}

#[test]
fn per_test_ports_b() {
    exercise(2);
}

#[test]
fn per_test_ports_c() {
    exercise(3);
}

#[test]
fn per_test_ports_on_many_threads_at_once() {
    let threads: Vec<_> = (10..18)
        .map(|id| std::thread::spawn(move || exercise(id)))
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

#[test]
fn each_call_is_a_new_object() {
    let (a, b) = (TestUi::channel::<u8, 2>(), TestUi::channel::<u8, 2>());
    assert!(!core::ptr::eq(a, b));
    a.try_send(1).unwrap();
    assert!(b.is_empty());
    assert_eq!(TestUi::latest(5u8).get(), 5);
    assert!(TestUi::outbox::<u8, 1>().is_empty());
}
