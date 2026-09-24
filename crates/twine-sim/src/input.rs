//! Simulated input devices: mouse → [`SimPointer`], wheel + middle button → [`SimEncoder`],
//! keyboard → [`SimKeypad`].
//!
//! Window events (or headless script commands) update a shared [`SimInputState`]; the devices are
//! cheap handles over it. All devices report [`PollHint::Interrupt`]: the simulator wakes the
//! app on every input event, so no periodic polling is needed.
//!
//! The mapping functions ([`mouse_to_panel`], [`WheelAccum`], [`map_key`]) are pure and tested
//! without a window.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use twine_core::Point;
use twine_hal::{EncoderData, InputData, InputDevice, InputKind, Key, KeypadData, PointerData, PollHint};
use winit::keyboard::{Key as WKey, NamedKey};

/// Keypad events kept at most; older ones are dropped (with a warning) when nobody reads them.
pub const KEY_QUEUE_CAPACITY: usize = 64;

/// The state shared by the simulated devices.
#[derive(Debug)]
pub struct SimInputState {
    pointer: PointerData,
    keys: VecDeque<(Key, bool)>,
    last_key: Key,
    last_key_pressed: bool,
    enc_diff: i32,
    enc_pressed: bool,
    click: Option<Point>,
    /// Keys pressed since the last [`take_raw_keys`](Self::take_raw_keys) (bounded).
    raw_keys: VecDeque<Key>,
    /// Devices whose state changed since the last [`take_changes`](Self::take_changes).
    changes: InputChanges,
}

/// Which simulated devices changed state (the engine is notified for these, like an
/// interrupt line).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputChanges {
    /// Pointer moved, pressed or released.
    pub pointer: bool,
    /// Keys queued.
    pub keypad: bool,
    /// Encoder rotated, pressed or released.
    pub encoder: bool,
}

impl Default for SimInputState {
    fn default() -> Self {
        Self {
            pointer: PointerData::default(),
            keys: VecDeque::new(),
            last_key: Key::Enter,
            last_key_pressed: false,
            enc_diff: 0,
            enc_pressed: false,
            click: None,
            raw_keys: VecDeque::new(),
            changes: InputChanges::default(),
        }
    }
}

impl SimInputState {
    /// Presses the pointer at `p`.
    pub fn pointer_press(&mut self, p: Point) {
        self.pointer = PointerData {
            point: p,
            pressed: true,
        };
        self.changes.pointer = true;
        log::debug!(target: "twine::sim", "pointer pressed at {p}");
    }

    /// Moves the pointer to `p` (logged at debug while pressed, trace otherwise).
    pub fn pointer_move(&mut self, p: Point) {
        if self.pointer.point == p {
            return;
        }
        self.pointer.point = p;
        self.changes.pointer = true;
        if self.pointer.pressed {
            log::debug!(target: "twine::sim", "pointer moved to {p}");
        } else {
            log::trace!(target: "twine::sim", "pointer hover at {p}");
        }
    }

    /// Releases the pointer (no-op when not pressed).
    pub fn pointer_release(&mut self) {
        if self.pointer.pressed {
            self.pointer.pressed = false;
            self.changes.pointer = true;
            self.click = Some(self.pointer.point);
            log::debug!(target: "twine::sim", "pointer released at {}", self.pointer.point);
        }
    }

    /// The keys pressed since the previous call (for [`SimConfig::on_raw_key`](crate::SimConfig::on_raw_key);
    /// the keypad queue itself stays for the keypad device).
    pub fn take_raw_keys(&mut self) -> Vec<Key> {
        self.raw_keys.drain(..).collect()
    }

    /// The devices whose state changed since the previous call.
    pub fn take_changes(&mut self) -> InputChanges {
        core::mem::take(&mut self.changes)
    }

    /// The position of the last pointer release since the previous call (a "click").
    pub fn take_click(&mut self) -> Option<Point> {
        self.click.take()
    }

    /// Drains the keypad queue, returning the keys pressed since the previous call (for
    /// framebuffer runners, where no keypad device reads the queue).
    pub fn take_pressed_keys(&mut self) -> Vec<Key> {
        self.keys
            .drain(..)
            .filter(|(_, pressed)| *pressed)
            .map(|(k, _)| k)
            .collect()
    }

    /// The current pointer state.
    #[must_use]
    pub fn pointer(&self) -> PointerData {
        self.pointer
    }

    /// Queues a key press or release.
    pub fn key(&mut self, key: Key, pressed: bool) {
        if self.keys.len() >= KEY_QUEUE_CAPACITY {
            let dropped = self.keys.pop_front();
            log::warn!(target: "twine::sim", "keypad queue full, dropping {dropped:?}");
        }
        self.keys.push_back((key, pressed));
        self.changes.keypad = true;
        if pressed {
            if self.raw_keys.len() >= KEY_QUEUE_CAPACITY {
                self.raw_keys.pop_front();
            }
            self.raw_keys.push_back(key);
        }
        log::debug!(
            target: "twine::sim",
            "key {} {}",
            key_label(key),
            if pressed { "pressed" } else { "released" }
        );
    }

    /// Adds `diff` encoder steps.
    pub fn encoder_rotate(&mut self, diff: i16) {
        if diff != 0 {
            self.enc_diff = self.enc_diff.saturating_add(i32::from(diff));
            self.changes.encoder = true;
            log::debug!(target: "twine::sim", "encoder diff {diff}");
        }
    }

    /// Presses or releases the encoder button.
    pub fn encoder_button(&mut self, pressed: bool) {
        if self.enc_pressed != pressed {
            self.enc_pressed = pressed;
            self.changes.encoder = true;
            log::debug!(
                target: "twine::sim",
                "encoder {}",
                if pressed { "pressed" } else { "released" }
            );
        }
    }

    /// Releases the pointer and the encoder button (window focus lost: no stuck presses).
    pub fn release_all(&mut self) {
        self.pointer_release();
        self.encoder_button(false);
        let held = self
            .keys
            .back()
            .map_or(self.last_key_pressed.then_some(self.last_key), |(k, p)| {
                p.then_some(*k)
            });
        if let Some(k) = held {
            self.key(k, false);
        }
    }

    /// Number of queued keypad events.
    #[must_use]
    pub fn queued_keys(&self) -> usize {
        self.keys.len()
    }

    fn read_key(&mut self) -> KeypadData {
        match self.keys.pop_front() {
            Some((key, pressed)) => {
                self.last_key = key;
                self.last_key_pressed = pressed;
                KeypadData {
                    key,
                    pressed,
                    more: !self.keys.is_empty(),
                }
            }
            // A key stays held until its release arrives.
            None => KeypadData {
                key: self.last_key,
                pressed: self.last_key_pressed,
                more: false,
            },
        }
    }

    fn read_encoder(&mut self) -> EncoderData {
        let diff = self.enc_diff.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        self.enc_diff -= diff;
        EncoderData {
            diff: diff as i16,
            pressed: self.enc_pressed,
        }
    }
}

fn key_label(key: Key) -> String {
    match key {
        Key::Char(c) => format!("Char({c:?})"),
        k => k.name().to_string(),
    }
}

/// Shared handle to the [`SimInputState`].
pub type SharedInput = Rc<RefCell<SimInputState>>;

macro_rules! device {
    ($(#[$m:meta])* $name:ident, $kind:expr, |$s:ident| $read:expr) => {
        $(#[$m])*
        #[derive(Clone, Debug)]
        pub struct $name(SharedInput);

        impl $name {
            /// A device over `state`.
            #[must_use]
            pub fn new(state: SharedInput) -> Self {
                Self(state)
            }
        }

        impl InputDevice for $name {
            fn kind(&self) -> InputKind {
                $kind
            }

            fn read(&mut self) -> InputData {
                let $s = &mut *self.0.borrow_mut();
                $read
            }

            fn poll_hint(&self) -> PollHint {
                PollHint::Interrupt
            }
        }
    };
}

device!(
    /// The mouse as a touch pointer (left button = pressed).
    SimPointer,
    InputKind::Pointer,
    |s| InputData::Pointer(s.pointer)
);
device!(
    /// The keyboard as a keypad (one queued event per read, `more` while more are queued).
    SimKeypad,
    InputKind::Keypad,
    |s| InputData::Keypad(s.read_key())
);
device!(
    /// The mouse wheel (diff) and middle button (pressed) as an encoder.
    SimEncoder,
    InputKind::Encoder,
    |s| InputData::Encoder(s.read_encoder())
);

/// The simulator's input devices, all over one shared state.
#[derive(Clone, Debug)]
pub struct SimDevices {
    /// The shared state (updated by the window or the headless script).
    pub state: SharedInput,
    /// Pointer device.
    pub pointer: SimPointer,
    /// Keypad device.
    pub keypad: SimKeypad,
    /// Encoder device.
    pub encoder: SimEncoder,
}

impl SimDevices {
    /// Fresh devices over a new state.
    #[must_use]
    pub fn new() -> Self {
        let state: SharedInput = Rc::default();
        Self {
            pointer: SimPointer::new(state.clone()),
            keypad: SimKeypad::new(state.clone()),
            encoder: SimEncoder::new(state.clone()),
            state,
        }
    }
}

impl Default for SimDevices {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps a window position (physical pixels) to a panel point: divides by the window scale and
/// clamps to the panel.
///
/// `scale` is window pixels per panel pixel (normally the configured integer scale).
#[must_use]
pub fn mouse_to_panel(pos: (f64, f64), scale: f64, panel: (u16, u16)) -> Point {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let clamp = |v: f64, max: u16| -> i32 {
        let v = (v / scale).floor();
        if v.is_nan() {
            0
        } else {
            v.clamp(0.0, f64::from(max.saturating_sub(1))) as i32
        }
    };
    Point::new(clamp(pos.0, panel.0), clamp(pos.1, panel.1))
}

/// Pixels of a pixel-precise scroll (touchpad) per encoder step.
pub const WHEEL_PIXELS_PER_STEP: f64 = 40.0;

/// Accumulates wheel motion into whole encoder steps (+1 per line scrolled **down**).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelAccum {
    acc: f64,
}

impl WheelAccum {
    /// Adds a line delta in winit's convention (positive `y` = content moves down, i.e. the
    /// wheel is rolled up) and returns the whole steps now available.
    pub fn lines(&mut self, y: f64) -> i16 {
        self.push(-y)
    }

    /// Adds a pixel delta in winit's convention (see [`lines`](Self::lines)).
    pub fn pixels(&mut self, y: f64) -> i16 {
        self.push(-y / WHEEL_PIXELS_PER_STEP)
    }

    fn push(&mut self, steps: f64) -> i16 {
        if !steps.is_finite() {
            return 0;
        }
        self.acc += steps;
        let whole = self.acc.trunc().clamp(f64::from(i16::MIN), f64::from(i16::MAX));
        self.acc -= whole;
        whole as i16
    }
}

/// A keypad character from typed text: exactly one non-control character.
#[must_use]
pub fn char_key(text: &str) -> Option<Key> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(Key::Char(c)),
        _ => None,
    }
}

/// Maps a winit logical key (plus the text it produced, if any) to a twine [`Key`].
///
/// Arrows → `Up/Down/Left/Right`, Enter (incl. numpad) → `Enter`, Escape → `Esc`,
/// Backspace, Delete → `Del`, Home, End, Tab → `Next` (Shift+Tab → `Prev`), Space and text
/// (excluding control characters) → `Char`. Function keys and modifiers give `None`.
#[must_use]
pub fn map_key(key: &WKey, text: Option<&str>, shift: bool) -> Option<Key> {
    match key {
        WKey::Named(named) => match named {
            NamedKey::ArrowUp => Some(Key::Up),
            NamedKey::ArrowDown => Some(Key::Down),
            NamedKey::ArrowLeft => Some(Key::Left),
            NamedKey::ArrowRight => Some(Key::Right),
            NamedKey::Enter => Some(Key::Enter),
            NamedKey::Escape => Some(Key::Esc),
            NamedKey::Backspace => Some(Key::Backspace),
            NamedKey::Delete => Some(Key::Del),
            NamedKey::Home => Some(Key::Home),
            NamedKey::End => Some(Key::End),
            NamedKey::Tab => Some(if shift { Key::Prev } else { Key::Next }),
            NamedKey::Space => Some(Key::Char(' ')),
            _ => None,
        },
        WKey::Character(s) => text.and_then(char_key).or_else(|| char_key(s)),
        _ => text.and_then(char_key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_to_panel_divides_by_scale_and_clamps() {
        assert_eq!(
            mouse_to_panel((241.0, 91.5), 2.0, (320, 240)),
            Point::new(120, 45)
        );
        assert_eq!(mouse_to_panel((-5.0, 10.0), 2.0, (320, 240)), Point::new(0, 5));
        assert_eq!(
            mouse_to_panel((5000.0, 5000.0), 2.0, (320, 240)),
            Point::new(319, 239)
        );
        assert_eq!(mouse_to_panel((7.0, 7.0), 0.0, (320, 240)), Point::new(7, 7));
        assert_eq!(mouse_to_panel((f64::NAN, 3.0), 1.0, (320, 240)), Point::new(0, 3));
    }

    #[test]
    fn wheel_pixel_delta_accumulates() {
        let mut w = WheelAccum::default();
        assert_eq!(w.pixels(-15.0), 0);
        assert_eq!(w.pixels(-15.0), 0);
        assert_eq!(w.pixels(-15.0), 1); // 45 px down → 1 step, 5 px kept
        assert_eq!(w.pixels(-35.0), 1);
        assert_eq!(w.pixels(80.0), -2);
        assert_eq!(w.lines(-1.0), 1);
        assert_eq!(w.lines(2.0), -2);
        assert_eq!(w.lines(f64::INFINITY), 0);
    }

    #[test]
    fn shift_tab_maps_to_prev() {
        let tab = WKey::Named(NamedKey::Tab);
        assert_eq!(map_key(&tab, Some("\t"), false), Some(Key::Next));
        assert_eq!(map_key(&tab, Some("\t"), true), Some(Key::Prev));
        assert_eq!(
            map_key(&WKey::Named(NamedKey::ArrowLeft), None, true),
            Some(Key::Left)
        );
        assert_eq!(
            map_key(&WKey::Named(NamedKey::Delete), None, false),
            Some(Key::Del)
        );
        assert_eq!(map_key(&WKey::Named(NamedKey::F9), None, false), None);
    }

    #[test]
    fn control_chars_are_not_chars() {
        assert_eq!(char_key("\u{7f}"), None);
        assert_eq!(char_key("\r"), None);
        assert_eq!(char_key("ab"), None);
        assert_eq!(char_key("é"), Some(Key::Char('é')));
        let c = WKey::Character("\u{1b}".into());
        assert_eq!(map_key(&c, Some("\u{1b}"), false), None);
        let a = WKey::Character("a".into());
        assert_eq!(map_key(&a, Some("A"), true), Some(Key::Char('A')));
        assert_eq!(
            map_key(&a, None, false),
            Some(Key::Char('a')),
            "release has no text"
        );
        assert_eq!(
            map_key(&WKey::Named(NamedKey::Space), None, false),
            Some(Key::Char(' '))
        );
    }

    #[test]
    fn devices_share_state() {
        let d = SimDevices::new();
        let mut kp = d.keypad.clone();
        let mut enc = d.encoder.clone();
        let mut ptr = d.pointer.clone();
        {
            let mut s = d.state.borrow_mut();
            s.pointer_press(Point::new(3, 4));
            s.key(Key::Enter, true);
            s.key(Key::Enter, false);
            s.encoder_rotate(2);
            s.encoder_button(true);
            s.release_all();
        }
        assert_eq!(
            ptr.read(),
            InputData::Pointer(PointerData {
                point: Point::new(3, 4),
                pressed: false
            })
        );
        assert_eq!(
            kp.read(),
            InputData::Keypad(KeypadData {
                key: Key::Enter,
                pressed: true,
                more: true
            })
        );
        assert_eq!(
            enc.read(),
            InputData::Encoder(EncoderData {
                diff: 2,
                pressed: false
            })
        );
        assert_eq!(kp.poll_hint(), PollHint::Interrupt);
    }

    #[test]
    fn key_queue_is_bounded() {
        let mut s = SimInputState::default();
        for _ in 0..KEY_QUEUE_CAPACITY + 5 {
            s.key(Key::Up, true);
        }
        assert_eq!(s.queued_keys(), KEY_QUEUE_CAPACITY);
    }
}
