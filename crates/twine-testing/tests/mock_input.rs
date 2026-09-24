//! Mock input devices.

use twine_core::Point;
use twine_hal::{EncoderData, InputData, InputDevice, InputKind, Key, KeypadData, PointerData, PollHint};
use twine_testing::{MockButton, MockEncoder, MockKeypad, MockPointer};

#[test]
fn keypad_queue_sets_more_flag() {
    let k = MockKeypad::new();
    let mut dev = k.clone();
    assert_eq!(dev.kind(), InputKind::Keypad);
    k.tap(Key::Enter);
    k.push(Key::Char('x'), true);
    assert_eq!(k.queued(), 3);
    let reads: Vec<_> = (0..4).map(|_| dev.read()).collect();
    assert_eq!(
        reads,
        [
            InputData::Keypad(KeypadData {
                key: Key::Enter,
                pressed: true,
                more: true
            }),
            InputData::Keypad(KeypadData {
                key: Key::Enter,
                pressed: false,
                more: true
            }),
            InputData::Keypad(KeypadData {
                key: Key::Char('x'),
                pressed: true,
                more: false
            }),
            // Empty queue: last key in its last state (still held).
            InputData::Keypad(KeypadData {
                key: Key::Char('x'),
                pressed: true,
                more: false
            }),
        ]
    );
    assert_eq!(k.reads(), 4);
}

#[test]
fn encoder_diff_is_consumed_on_read() {
    let e = MockEncoder::new();
    let mut dev = e.clone();
    e.rotate(-2);
    e.rotate(5);
    e.press();
    assert_eq!(
        dev.read(),
        InputData::Encoder(EncoderData {
            diff: 3,
            pressed: true
        })
    );
    e.release();
    assert_eq!(
        dev.read(),
        InputData::Encoder(EncoderData {
            diff: 0,
            pressed: false
        })
    );
    for _ in 0..3 {
        e.rotate(i16::MAX);
    }
    let InputData::Encoder(d) = dev.read() else {
        unreachable!()
    };
    assert_eq!(d.diff, i16::MAX, "saturates per read, keeps the rest");
}

#[test]
fn pointer_and_button_state_and_hints() {
    let p = MockPointer::new();
    let mut dev = p.clone();
    assert_eq!(dev.poll_hint(), PollHint::Periodic);
    p.set_poll_hint(PollHint::Interrupt);
    assert_eq!(dev.poll_hint(), PollHint::Interrupt);
    p.press(Point::new(1, 2));
    p.move_to(Point::new(5, 6));
    assert_eq!(
        dev.read(),
        InputData::Pointer(PointerData {
            point: Point::new(5, 6),
            pressed: true
        })
    );
    dev.rearm();
    assert_eq!(p.rearms(), 1);

    let b = MockButton::new();
    let mut bdev = b.clone();
    b.press(4);
    b.release();
    assert_eq!(bdev.kind(), InputKind::Button);
    let InputData::Button(d) = bdev.read() else {
        unreachable!()
    };
    assert_eq!((d.id, d.pressed), (4, false));
}
