//! Input devices: [`InputDevice`], [`InputData`], [`Key`], [`PollHint`] and the per-kind data.
//!
//! Reading is always **non-blocking**: a driver caches its last sample (updated from an IRQ or
//! when read) and [`InputDevice::read`] returns it. Drivers whose reading depends on time
//! (debounced GPIO encoders, the monkey tester) take a [`Clock`](crate::Clock) at
//! construction; the trait itself is time-free (`docs/design/09-platforms.md` §1).

use core::fmt;

use twine_core::Point;

/// The kind of an input device, which decides how the engine processes its data
/// (`docs/design/04-engine.md` §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InputKind {
    /// Touch panel or mouse: a point plus pressed state.
    Pointer,
    /// Keyboard or keypad: [`Key`] presses.
    Keypad,
    /// Rotary encoder with a push button.
    Encoder,
    /// Physical buttons mapped to screen points.
    Button,
}

/// A key of a keypad (LVGL `LV_KEY_*` plus text characters).
///
/// `Next`/`Prev` move focus within a group; `Enter` presses/clicks the focused widget.
///
/// ```
/// use twine_hal::Key;
/// assert_eq!(Key::from_name("Enter"), Some(Key::Enter));
/// assert_eq!(Key::from_name("a"), Some(Key::Char('a')));
/// assert_eq!(Key::from_name("enter"), None);
/// assert_eq!(Key::Home.name(), "Home");
/// assert_eq!(Key::Char('x').to_string(), "x");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Key {
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Escape / cancel.
    Esc,
    /// Delete the character after the cursor.
    Del,
    /// Delete the character before the cursor.
    Backspace,
    /// Enter / confirm.
    Enter,
    /// Focus the next widget.
    Next,
    /// Focus the previous widget.
    Prev,
    /// Home.
    Home,
    /// End.
    End,
    /// A text character.
    Char(char),
}

impl Key {
    /// Every named (non-`Char`) key.
    pub const NAMED: [Key; 12] = [
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Esc,
        Key::Del,
        Key::Backspace,
        Key::Enter,
        Key::Next,
        Key::Prev,
        Key::Home,
        Key::End,
    ];

    /// Parses a key name: a variant name (`"Up"`, `"Enter"`, … — case-sensitive) or a string of
    /// exactly one character (→ [`Key::Char`]).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Key> {
        if let Some(k) = Key::NAMED.iter().find(|k| k.name() == name) {
            return Some(*k);
        }
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(Key::Char(c)),
            _ => None,
        }
    }

    /// The variant name (`"Char"` for every [`Key::Char`]; use `Display` to get the character).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Key::Up => "Up",
            Key::Down => "Down",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Esc => "Esc",
            Key::Del => "Del",
            Key::Backspace => "Backspace",
            Key::Enter => "Enter",
            Key::Next => "Next",
            Key::Prev => "Prev",
            Key::Home => "Home",
            Key::End => "End",
            Key::Char(_) => "Char",
        }
    }
}

/// Writes the name of a named key, or the character itself; the output parses back with
/// [`Key::from_name`].
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            k => f.write_str(k.name()),
        }
    }
}

/// Sample of a pointer device (touch panel, mouse).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PointerData {
    /// Position in logical screen coordinates (the last position when released).
    pub point: Point,
    /// Whether the pointer is pressed (touched / left button down).
    pub pressed: bool,
}

/// One key event of a keypad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeypadData {
    /// The key.
    pub key: Key,
    /// Pressed (`true`) or released (`false`).
    pub pressed: bool,
    /// More events are queued: the engine reads again before processing further.
    pub more: bool,
}

/// Sample of a rotary encoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EncoderData {
    /// Steps rotated since the last read (positive = clockwise / down).
    pub diff: i16,
    /// Whether the encoder's button is pressed.
    pub pressed: bool,
}

/// Sample of a physical button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ButtonData {
    /// Button index (into the points the engine maps buttons to).
    pub id: u8,
    /// Whether the button is pressed.
    pub pressed: bool,
}

/// Data read from an input device; the variant matches [`InputDevice::kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InputData {
    /// Pointer sample.
    Pointer(PointerData),
    /// Keypad event.
    Keypad(KeypadData),
    /// Encoder sample.
    Encoder(EncoderData),
    /// Button sample.
    Button(ButtonData),
}

impl InputData {
    /// The kind of device this data comes from.
    #[must_use]
    pub const fn kind(&self) -> InputKind {
        match self {
            InputData::Pointer(_) => InputKind::Pointer,
            InputData::Keypad(_) => InputKind::Keypad,
            InputData::Encoder(_) => InputKind::Encoder,
            InputData::Button(_) => InputKind::Button,
        }
    }
}

/// How the engine should schedule reads of a device (power, P1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PollHint {
    /// The device signals input by an interrupt (the app calls `Ui::notify_input()` from the
    /// IRQ); the engine reads it only after a wake, and continuously while pressed.
    Interrupt,
    /// No interrupt: the engine wakes every `read_period` to poll it.
    #[default]
    Periodic,
}

/// An input device (LVGL `lv_indev` driver side).
///
/// ```
/// use twine_core::Point;
/// use twine_hal::{InputData, InputDevice, InputKind, PointerData};
///
/// struct Touch(Point, bool);
/// impl InputDevice for Touch {
///     fn kind(&self) -> InputKind { InputKind::Pointer }
///     fn read(&mut self) -> InputData {
///         InputData::Pointer(PointerData { point: self.0, pressed: self.1 })
///     }
/// }
///
/// let mut t = Touch(Point::new(3, 4), true);
/// assert_eq!(t.read().kind(), InputKind::Pointer);
/// ```
pub trait InputDevice {
    /// The kind of the device; [`read`](Self::read) returns the matching [`InputData`] variant.
    fn kind(&self) -> InputKind;

    /// Returns the current state without blocking (drivers cache the last sample).
    ///
    /// Keypads return one queued event per call and set [`KeypadData::more`] while more are
    /// queued; encoders return the rotation accumulated since the previous read.
    fn read(&mut self) -> InputData;

    /// How the engine should schedule reads. Default: [`PollHint::Periodic`].
    fn poll_hint(&self) -> PollHint {
        PollHint::Periodic
    }

    /// Called by the engine after processing, lets IRQ-based drivers re-arm their interrupt.
    /// Default: no-op.
    fn rearm(&mut self) {}
}

/// Async wait for an input interrupt (used by `twine-embassy`), feature `async`.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AsyncInputWait {
    /// Completes when the device signals new input (e.g. the touch IRQ pin goes active).
    async fn wait_for_interrupt(&mut self);
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::ToString;

    #[test]
    fn key_names_roundtrip() {
        for k in Key::NAMED {
            assert_eq!(Key::from_name(k.name()), Some(k));
            assert_eq!(Key::from_name(&k.to_string()), Some(k));
        }
        for c in ['a', 'Z', '0', ' ', 'é', '€', '"'] {
            let k = Key::Char(c);
            assert_eq!(k.name(), "Char");
            assert_eq!(Key::from_name(&k.to_string()), Some(k));
        }
        assert_eq!(Key::from_name(""), None);
        assert_eq!(Key::from_name("ab"), None);
        assert_eq!(Key::from_name("F13"), None);
    }

    struct Dummy;
    impl InputDevice for Dummy {
        fn kind(&self) -> InputKind {
            InputKind::Button
        }
        fn read(&mut self) -> InputData {
            InputData::Button(ButtonData { id: 2, pressed: true })
        }
    }

    #[test]
    fn default_poll_hint_is_periodic() {
        let mut d = Dummy;
        assert_eq!(d.poll_hint(), PollHint::Periodic);
        assert_eq!(PollHint::default(), PollHint::Periodic);
        d.rearm();
        assert_eq!(d.read().kind(), d.kind());
    }
}
