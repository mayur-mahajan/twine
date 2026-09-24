//! Power-aware input polling: interrupt devices are read only when notified or held;
//! periodic devices set the wake-up period.

mod common;

use twine_core::{Duration, Point, Rect};
use twine_engine::{EngineError, Wake};
use twine_hal::{Key, PollHint};
use twine_testing::{EngineHarness, MockButton, MockKeypad, MockPointer};

fn harness() -> EngineHarness {
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        common::clickable(e, s, Rect::from_xywh(10, 10, 40, 40));
    });
    h.run_until_idle();
    h
}

#[test]
fn interrupt_device_not_read_when_idle() {
    let mut h = harness();
    let (_, p) = h.pointer_input();
    for _ in 0..100 {
        h.clock().advance(Duration::ms(10));
        assert_eq!(h.update(), Wake::Idle);
    }
    assert_eq!(p.reads(), 0);
    assert_eq!(h.engine().input_deadline(), None);
}

#[test]
fn interrupt_device_read_continuously_while_pressed() {
    let mut h = harness();
    let (_, p) = h.pointer_input();
    let rp = h.engine().config().read_period;
    let w = h.press(Point::new(20, 20));
    assert_eq!(p.reads(), 1);
    // While pressed the engine asks to be woken to read again.
    let Wake::At(t) = w else { panic!("{w:?}") };
    assert!(t <= h.now() + rp);
    for i in 0..5 {
        h.clock().advance(rp);
        h.update();
        assert_eq!(p.reads(), 2 + i);
    }
    let before = p.reads();
    let rearms = p.rearms();
    h.release();
    assert_eq!(p.reads(), before + 1);
    assert_eq!(p.rearms(), rearms + 1);
    h.run_until_idle();
    for _ in 0..20 {
        h.clock().advance(rp);
        assert_eq!(h.update(), Wake::Idle);
    }
    assert_eq!(p.reads(), before + 1, "no reads after release");
}

#[test]
fn periodic_device_requests_wake_every_read_period() {
    let mut h = harness();
    let p = MockPointer::new();
    let d = h.display();
    h.engine_mut().add_input(p.clone(), d).unwrap();
    let rp = h.engine().config().read_period;
    let start = h.now();
    assert_eq!(h.update(), Wake::At(start + rp));
    assert_eq!(p.reads(), 1);
    // Not due yet: no read.
    h.clock().advance(Duration::ms(10));
    assert_eq!(h.update(), Wake::At(start + rp));
    assert_eq!(p.reads(), 1);
    h.clock().set(start + rp);
    assert_eq!(h.update(), Wake::At(start + rp + rp));
    assert_eq!(p.reads(), 2);
}

#[test]
fn keypad_more_flag_drains_queue() {
    let mut h = harness();
    let k = MockKeypad::new();
    k.set_poll_hint(PollHint::Interrupt);
    let d = h.display();
    let id = h.engine_mut().add_input(k.clone(), d).unwrap();
    for c in "abcdef".chars() {
        k.tap(Key::Char(c));
    }
    h.engine_mut().notify_input(id);
    h.update();
    assert_eq!(k.queued(), 0);
    assert_eq!(k.reads(), 12);
    // At most 16 reads per update.
    for _ in 0..10 {
        k.tap(Key::Char('x'));
    }
    h.engine_mut().notify_input(id);
    h.update();
    assert_eq!(k.queued(), 4);
}

#[test]
fn disabled_device_not_read() {
    let mut h = harness();
    let p = MockPointer::new();
    let d = h.display();
    let id = h.engine_mut().add_input(p.clone(), d).unwrap();
    h.engine_mut().set_input_enabled(id, false);
    assert!(!h.engine().input_enabled(id));
    h.engine_mut().notify_input(id);
    for _ in 0..10 {
        h.clock().advance(Duration::ms(30));
        assert_eq!(h.update(), Wake::Idle);
    }
    assert_eq!(p.reads(), 0);
    h.engine_mut().set_input_enabled(id, true);
    h.update();
    assert_eq!(p.reads(), 1);
}

#[test]
fn too_many_inputs_rejected() {
    let mut h = harness();
    let d = h.display();
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(h.engine_mut().add_input(MockButton::new(), d).unwrap());
    }
    assert_eq!(
        h.engine_mut().add_input(MockButton::new(), d),
        Err(EngineError::TooManyInputs)
    );
    h.engine_mut().remove_input(ids[3]);
    assert_eq!(h.engine().inputs().count(), 7);
    assert_eq!(h.engine_mut().add_input(MockButton::new(), d), Ok(ids[3]));
}

#[test]
fn long_press_deadline_is_exact() {
    let mut h = harness();
    h.press(Point::new(20, 20));
    let press = h.now();
    // Reads every 30 ms plus exactly at the long press time.
    let mut wakes = Vec::new();
    for _ in 0..20 {
        match h.update() {
            Wake::At(t) => {
                wakes.push(t.saturating_duration_since(press).as_millis());
                h.clock().set(t);
            }
            w => panic!("{w:?}"),
        }
        if h.now() >= press + Duration::ms(400) {
            break;
        }
    }
    assert!(wakes.contains(&390) && wakes.contains(&400), "{wakes:?}");
}
