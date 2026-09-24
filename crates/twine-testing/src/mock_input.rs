//! Mock input devices: [`MockPointer`], [`MockKeypad`], [`MockEncoder`], [`MockButton`].
//!
//! Each mock is a cheap clonable handle over shared state (`Rc<RefCell<…>>`): the test keeps
//! one handle to drive the device while the engine owns another as its [`InputDevice`].
//! Every mock counts its [`read`](InputDevice::read) and [`rearm`](InputDevice::rearm) calls
//! and has a configurable [`PollHint`] (default [`PollHint::Periodic`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use twine_core::Point;
use twine_hal::{
    ButtonData, EncoderData, InputData, InputDevice, InputKind, Key, KeypadData, PointerData, PollHint,
};

#[derive(Debug, Default)]
struct Common {
    hint: PollHint,
    reads: u64,
    rearms: u64,
}

macro_rules! common_methods {
    () => {
        /// Sets the poll hint reported to the engine.
        pub fn set_poll_hint(&self, hint: PollHint) {
            self.0.borrow_mut().common.hint = hint;
        }

        /// Number of `read` calls so far.
        #[must_use]
        pub fn reads(&self) -> u64 {
            self.0.borrow().common.reads
        }

        /// Number of `rearm` calls so far.
        #[must_use]
        pub fn rearms(&self) -> u64 {
            self.0.borrow().common.rearms
        }
    };
}

macro_rules! common_trait_methods {
    () => {
        fn poll_hint(&self) -> PollHint {
            self.0.borrow().common.hint
        }

        fn rearm(&mut self) {
            self.0.borrow_mut().common.rearms += 1;
        }
    };
}

#[derive(Debug, Default)]
struct PointerState {
    common: Common,
    data: PointerData,
}

/// A mock touch / mouse pointer.
///
/// ```
/// use twine_core::Point;
/// use twine_hal::{InputData, InputDevice, PointerData};
/// use twine_testing::MockPointer;
///
/// let p = MockPointer::new();
/// let mut dev = p.clone();
/// p.press(Point::new(10, 20));
/// assert_eq!(dev.read(), InputData::Pointer(PointerData { point: Point::new(10, 20), pressed: true }));
/// p.release();
/// assert_eq!(dev.read(), InputData::Pointer(PointerData { point: Point::new(10, 20), pressed: false }));
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockPointer(Rc<RefCell<PointerState>>);

impl MockPointer {
    /// A released pointer at the origin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Presses at `p`.
    pub fn press(&self, p: Point) {
        self.0.borrow_mut().data = PointerData {
            point: p,
            pressed: true,
        };
    }

    /// Moves to `p`, keeping the pressed state.
    pub fn move_to(&self, p: Point) {
        self.0.borrow_mut().data.point = p;
    }

    /// Releases at the current position.
    pub fn release(&self) {
        self.0.borrow_mut().data.pressed = false;
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> PointerData {
        self.0.borrow().data
    }

    common_methods!();
}

impl InputDevice for MockPointer {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        let mut s = self.0.borrow_mut();
        s.common.reads += 1;
        InputData::Pointer(s.data)
    }

    common_trait_methods!();
}

#[derive(Debug)]
struct KeypadState {
    common: Common,
    queue: VecDeque<(Key, bool)>,
    last: Key,
    last_pressed: bool,
}

impl Default for KeypadState {
    fn default() -> Self {
        Self {
            common: Common::default(),
            queue: VecDeque::new(),
            last: Key::Enter,
            last_pressed: false,
        }
    }
}

/// A mock keypad with an event queue.
///
/// Each `read` pops one event and sets `more` while further events are queued. With an empty
/// queue it reports the last key in its last state (a key pushed as pressed stays held until
/// its release is pushed), like a keypad driver reporting the current key state.
///
/// ```
/// use twine_hal::{InputData, InputDevice, Key, KeypadData};
/// use twine_testing::MockKeypad;
///
/// let k = MockKeypad::new();
/// let mut dev = k.clone();
/// k.push(Key::Char('a'), true);
/// k.push(Key::Char('a'), false);
/// assert_eq!(dev.read(), InputData::Keypad(KeypadData { key: Key::Char('a'), pressed: true, more: true }));
/// assert_eq!(dev.read(), InputData::Keypad(KeypadData { key: Key::Char('a'), pressed: false, more: false }));
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockKeypad(Rc<RefCell<KeypadState>>);

impl MockKeypad {
    /// An empty keypad.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a press (`pressed = true`) or release of `key`.
    pub fn push(&self, key: Key, pressed: bool) {
        self.0.borrow_mut().queue.push_back((key, pressed));
    }

    /// Queues a press and a release of `key`.
    pub fn tap(&self, key: Key) {
        self.push(key, true);
        self.push(key, false);
    }

    /// Number of queued events.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.0.borrow().queue.len()
    }

    common_methods!();
}

impl InputDevice for MockKeypad {
    fn kind(&self) -> InputKind {
        InputKind::Keypad
    }

    fn read(&mut self) -> InputData {
        let mut s = self.0.borrow_mut();
        s.common.reads += 1;
        let data = match s.queue.pop_front() {
            Some((key, pressed)) => {
                s.last = key;
                s.last_pressed = pressed;
                KeypadData {
                    key,
                    pressed,
                    more: !s.queue.is_empty(),
                }
            }
            None => KeypadData {
                key: s.last,
                pressed: s.last_pressed,
                more: false,
            },
        };
        InputData::Keypad(data)
    }

    common_trait_methods!();
}

#[derive(Debug, Default)]
struct EncoderState {
    common: Common,
    diff: i32,
    pressed: bool,
}

/// A mock rotary encoder; the accumulated rotation is consumed by `read`.
///
/// ```
/// use twine_hal::{EncoderData, InputData, InputDevice};
/// use twine_testing::MockEncoder;
///
/// let e = MockEncoder::new();
/// let mut dev = e.clone();
/// e.rotate(2);
/// e.rotate(1);
/// assert_eq!(dev.read(), InputData::Encoder(EncoderData { diff: 3, pressed: false }));
/// assert_eq!(dev.read(), InputData::Encoder(EncoderData { diff: 0, pressed: false }));
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockEncoder(Rc<RefCell<EncoderState>>);

impl MockEncoder {
    /// A released encoder with no pending rotation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `diff` steps (positive = clockwise).
    pub fn rotate(&self, diff: i16) {
        self.0.borrow_mut().diff += i32::from(diff);
    }

    /// Presses the button.
    pub fn press(&self) {
        self.0.borrow_mut().pressed = true;
    }

    /// Releases the button.
    pub fn release(&self) {
        self.0.borrow_mut().pressed = false;
    }

    common_methods!();
}

impl InputDevice for MockEncoder {
    fn kind(&self) -> InputKind {
        InputKind::Encoder
    }

    fn read(&mut self) -> InputData {
        let mut s = self.0.borrow_mut();
        s.common.reads += 1;
        let diff = s.diff.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        s.diff -= diff;
        InputData::Encoder(EncoderData {
            diff: diff as i16,
            pressed: s.pressed,
        })
    }

    common_trait_methods!();
}

#[derive(Debug, Default)]
struct ButtonState {
    common: Common,
    data: ButtonData,
}

/// A mock physical button (reports one button id at a time).
///
/// ```
/// use twine_hal::{ButtonData, InputData, InputDevice};
/// use twine_testing::MockButton;
///
/// let b = MockButton::new();
/// let mut dev = b.clone();
/// b.press(3);
/// assert_eq!(dev.read(), InputData::Button(ButtonData { id: 3, pressed: true }));
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockButton(Rc<RefCell<ButtonState>>);

impl MockButton {
    /// Button 0, released.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Presses button `id`.
    pub fn press(&self, id: u8) {
        self.0.borrow_mut().data = ButtonData { id, pressed: true };
    }

    /// Releases the current button.
    pub fn release(&self) {
        self.0.borrow_mut().data.pressed = false;
    }

    common_methods!();
}

impl InputDevice for MockButton {
    fn kind(&self) -> InputKind {
        InputKind::Button
    }

    fn read(&mut self) -> InputData {
        let mut s = self.0.borrow_mut();
        s.common.reads += 1;
        InputData::Button(s.data)
    }

    common_trait_methods!();
}
