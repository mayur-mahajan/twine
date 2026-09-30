//! Input device health (R0.S08): `InputDevice::health` → `FaultKind::InputDevice` on
//! transitions, `Engine::input_health`, and the release of a held press when a device fails.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};

use common::{EvLog, clickable, input_codes, record, white_screen};
use twine_core::fault::FaultKind;
use twine_core::{Duration, Point, Rect};
use twine_engine::{EventCode as C, FaultRecord, Key, NodeId, ObjFlags, State};
use twine_hal::{DeviceHealth, PollHint};
use twine_testing::{EngineHarness, MockPointer};

/// A 100×100 harness with one clickable box at (10, 10, 40, 40) (in the default group) and
/// its event log.
fn setup() -> (EngineHarness, NodeId, EvLog) {
    let mut b = None;
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let g = e.create_group().unwrap();
        e.set_default_group(Some(g));
        let n = clickable(e, s, Rect::from_xywh(10, 10, 40, 40));
        // Like a button: not scrollable (an encoder would edit, i.e. scroll, it).
        e.set_flag(n, ObjFlags::SCROLLABLE, false);
        e.group_add(g, n);
        b = Some(n);
    });
    let b = b.unwrap();
    let log = EvLog::default();
    record(h.engine_mut(), b, &log);
    h.run_until_idle();
    (h, b, log)
}

fn pressed(h: &EngineHarness, n: NodeId) -> bool {
    h.engine()
        .tree()
        .node(n)
        .unwrap()
        .state()
        .contains(State::PRESSED)
}

/// Reads the harness pointer once more (it is interrupt driven).
fn read_pointer(h: &mut EngineHarness) {
    let (id, _) = h.pointer_input();
    h.engine_mut().notify_input(id);
    h.update();
}

fn input_faults(h: &EngineHarness) -> u32 {
    h.engine().fault_counts().get(FaultKind::InputDevice)
}

#[test]
fn fault_raised_on_transition_only() {
    let (mut h, _, _) = setup();
    let (id, m) = h.pointer_input();
    assert_eq!(h.engine().input_health(id), Some(DeviceHealth::Ok));
    read_pointer(&mut h);
    assert_eq!(input_faults(&h), 0);

    m.set_health(DeviceHealth::Degraded { errors: 1 });
    read_pointer(&mut h);
    assert_eq!(input_faults(&h), 1);
    assert!(h.engine_mut().take_faults().contains(FaultKind::InputDevice));
    let r = *h.engine().last_fault(FaultKind::InputDevice).unwrap();
    assert_eq!((r.input, r.code, r.display), (Some(id), 1, None));
    assert_eq!(
        h.engine().input_health(id),
        Some(DeviceHealth::Degraded { errors: 1 })
    );

    // More errors: still degraded, no new fault.
    m.set_health(DeviceHealth::Degraded { errors: 2 });
    read_pointer(&mut h);
    read_pointer(&mut h);
    assert_eq!(input_faults(&h), 1);
    assert!(h.engine_mut().take_faults().is_empty());

    m.set_health(DeviceHealth::Failed);
    for _ in 0..5 {
        read_pointer(&mut h);
    }
    assert_eq!(input_faults(&h), 2);
    let r = *h.engine().last_fault(FaultKind::InputDevice).unwrap();
    assert_eq!((r.input, r.code), (Some(id), 2));

    // Recovery is not a fault; failing again is.
    m.set_health(DeviceHealth::Degraded { errors: 1 });
    read_pointer(&mut h);
    m.set_health(DeviceHealth::Ok);
    read_pointer(&mut h);
    assert_eq!(input_faults(&h), 2);
    assert_eq!(h.engine().input_health(id), Some(DeviceHealth::Ok));
    m.set_health(DeviceHealth::Failed);
    read_pointer(&mut h);
    assert_eq!(input_faults(&h), 3);

    h.engine_mut().remove_input(id);
    assert_eq!(h.engine().input_health(id), None);
}

static HOOK_CALLS: AtomicU32 = AtomicU32::new(0);

fn hook(r: &FaultRecord) {
    if r.kind == FaultKind::InputDevice {
        HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn hook_called_once_per_transition() {
    let (mut h, _, _) = setup();
    h.engine_mut().set_fault_hook(Some(hook));
    let (_, m) = h.pointer_input();
    m.set_health(DeviceHealth::Failed);
    for _ in 0..4 {
        read_pointer(&mut h);
    }
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn regression_failed_pointer_releases_held_press_without_click() {
    let (mut h, b, log) = setup();
    h.press(Point::new(20, 20));
    assert!(pressed(&h, b));
    let (id, m) = h.pointer_input();
    // The controller latches "pressed" (e.g. ESD) and reports the bus as failed.
    m.set_health(DeviceHealth::Failed);
    read_pointer(&mut h);
    assert!(!pressed(&h, b), "a failed device must not keep the node pressed");
    assert!(!h.engine().input_pressed(id));
    // Long past the long-press time: nothing more reaches the node.
    for _ in 0..20 {
        h.clock().advance(Duration::ms(100));
        read_pointer(&mut h);
    }
    let codes = input_codes(&log);
    assert!(codes.contains(&C::PressLost), "{codes:?}");
    for c in [
        C::Released,
        C::Clicked,
        C::ShortClicked,
        C::LongPressed,
        C::LongPressedRepeat,
    ] {
        assert!(!codes.contains(&c), "{c:?} in {codes:?}");
    }
    assert!(!pressed(&h, b));

    // Recovery while still reported pressed: that press is ignored until released.
    log.borrow_mut().clear();
    m.set_health(DeviceHealth::Ok);
    read_pointer(&mut h);
    assert!(!pressed(&h, b));
    h.release();
    assert!(!input_codes(&log).contains(&C::Clicked));
    // Then the device works normally again.
    h.tap(Point::new(20, 20));
    assert!(input_codes(&log).contains(&C::Clicked));
}

#[test]
fn degraded_pointer_keeps_working() {
    let (mut h, b, log) = setup();
    let (_, m) = h.pointer_input();
    m.set_health(DeviceHealth::Degraded { errors: 1 });
    h.tap(Point::new(20, 20));
    assert!(input_codes(&log).contains(&C::Clicked));
    assert!(!pressed(&h, b));
}

#[test]
fn failed_pointer_press_is_ignored() {
    let (mut h, b, log) = setup();
    let (_, m) = h.pointer_input();
    m.set_health(DeviceHealth::Failed);
    h.tap(Point::new(20, 20));
    h.press(Point::new(20, 20));
    assert!(!pressed(&h, b));
    assert!(input_codes(&log).is_empty(), "{:?}", input_codes(&log));
}

#[test]
fn failed_keypad_releases_held_enter_without_click() {
    let (mut h, b, log) = setup();
    h.key_state(Key::Enter, true);
    assert!(pressed(&h, b));
    let (id, m) = h.keypad_input();
    m.set_health(DeviceHealth::Failed);
    h.engine_mut().notify_input(id);
    h.update();
    assert!(!pressed(&h, b));
    for _ in 0..10 {
        h.clock().advance(Duration::ms(100));
        h.engine_mut().notify_input(id);
        h.update();
    }
    let codes = input_codes(&log);
    assert!(codes.contains(&C::PressLost), "{codes:?}");
    for c in [C::Released, C::Clicked, C::LongPressed] {
        assert!(!codes.contains(&c), "{c:?} in {codes:?}");
    }
    assert_eq!(h.engine().input_health(id), Some(DeviceHealth::Failed));
}

#[test]
fn failed_encoder_releases_held_button_without_click() {
    let (mut h, b, log) = setup();
    h.encoder_button(true);
    assert!(pressed(&h, b));
    let (id, m) = h.encoder_input();
    m.set_health(DeviceHealth::Failed);
    h.engine_mut().notify_input(id);
    h.update();
    assert!(!pressed(&h, b));
    // Rotation of a failed encoder is dropped too.
    h.encoder(3);
    let codes = input_codes(&log);
    assert!(codes.contains(&C::PressLost), "{codes:?}");
    assert!(
        !codes.contains(&C::Clicked) && !codes.contains(&C::Key),
        "{codes:?}"
    );
}

#[test]
fn failed_periodic_device_keeps_being_read_and_recovers() {
    let (mut h, b, log) = setup();
    let m = MockPointer::new();
    m.set_poll_hint(PollHint::Periodic);
    let d = h.display();
    let id = h.engine_mut().add_input(m.clone(), d).unwrap();
    m.set_health(DeviceHealth::Failed);
    h.update();
    let reads = m.reads();
    let period = h.engine().config().read_period;
    for _ in 0..3 {
        h.clock().advance(period);
        h.update();
    }
    assert!(m.reads() >= reads + 3, "{} → {}", reads, m.reads());
    m.set_health(DeviceHealth::Ok);
    h.clock().advance(period);
    h.update();
    m.press(Point::new(20, 20));
    h.clock().advance(period);
    h.update();
    m.release();
    h.clock().advance(period);
    h.update();
    assert_eq!(h.engine().input_health(id), Some(DeviceHealth::Ok));
    assert!(input_codes(&log).contains(&C::Clicked));
    assert!(!pressed(&h, b));
}
