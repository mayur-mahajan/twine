//! Symbol characters (LVGL `LV_SYMBOL_*`), with the exact code points of LVGL v9
//! `src/font/lv_symbol_def.h` (MIT licensed, see `THIRD_PARTY.md`).
//!
//! The Font Awesome symbols are merged into the built-in Montserrat fonts, so they can be used
//! directly in text: `format!("{} Wi-Fi", symbols::WIFI)`.
//!
//! ```
//! use twine_text::symbols;
//! assert_eq!(symbols::OK, "\u{F00C}");
//! assert!(symbols::ALL.contains(&'\u{F00C}'));
//! ```

/// Bullet (U+2022, from the text font itself).
pub const BULLET: &str = "\u{2022}";
/// `LV_SYMBOL_AUDIO` (Font Awesome U+F001).
pub const AUDIO: &str = "\u{F001}";
/// `LV_SYMBOL_VIDEO` (Font Awesome U+F008).
pub const VIDEO: &str = "\u{F008}";
/// `LV_SYMBOL_LIST` (Font Awesome U+F00B).
pub const LIST: &str = "\u{F00B}";
/// `LV_SYMBOL_OK` (Font Awesome U+F00C).
pub const OK: &str = "\u{F00C}";
/// `LV_SYMBOL_CLOSE` (Font Awesome U+F00D).
pub const CLOSE: &str = "\u{F00D}";
/// `LV_SYMBOL_POWER` (Font Awesome U+F011).
pub const POWER: &str = "\u{F011}";
/// `LV_SYMBOL_SETTINGS` (Font Awesome U+F013).
pub const SETTINGS: &str = "\u{F013}";
/// `LV_SYMBOL_HOME` (Font Awesome U+F015).
pub const HOME: &str = "\u{F015}";
/// `LV_SYMBOL_DOWNLOAD` (Font Awesome U+F019).
pub const DOWNLOAD: &str = "\u{F019}";
/// `LV_SYMBOL_DRIVE` (Font Awesome U+F01C).
pub const DRIVE: &str = "\u{F01C}";
/// `LV_SYMBOL_REFRESH` (Font Awesome U+F021).
pub const REFRESH: &str = "\u{F021}";
/// `LV_SYMBOL_MUTE` (Font Awesome U+F026).
pub const MUTE: &str = "\u{F026}";
/// `LV_SYMBOL_VOLUME_MID` (Font Awesome U+F027).
pub const VOLUME_MID: &str = "\u{F027}";
/// `LV_SYMBOL_VOLUME_MAX` (Font Awesome U+F028).
pub const VOLUME_MAX: &str = "\u{F028}";
/// `LV_SYMBOL_IMAGE` (Font Awesome U+F03E).
pub const IMAGE: &str = "\u{F03E}";
/// `LV_SYMBOL_TINT` (Font Awesome U+F043).
pub const TINT: &str = "\u{F043}";
/// `LV_SYMBOL_PREV` (Font Awesome U+F048).
pub const PREV: &str = "\u{F048}";
/// `LV_SYMBOL_PLAY` (Font Awesome U+F04B).
pub const PLAY: &str = "\u{F04B}";
/// `LV_SYMBOL_PAUSE` (Font Awesome U+F04C).
pub const PAUSE: &str = "\u{F04C}";
/// `LV_SYMBOL_STOP` (Font Awesome U+F04D).
pub const STOP: &str = "\u{F04D}";
/// `LV_SYMBOL_NEXT` (Font Awesome U+F051).
pub const NEXT: &str = "\u{F051}";
/// `LV_SYMBOL_EJECT` (Font Awesome U+F052).
pub const EJECT: &str = "\u{F052}";
/// `LV_SYMBOL_LEFT` (Font Awesome U+F053).
pub const LEFT: &str = "\u{F053}";
/// `LV_SYMBOL_RIGHT` (Font Awesome U+F054).
pub const RIGHT: &str = "\u{F054}";
/// `LV_SYMBOL_PLUS` (Font Awesome U+F067).
pub const PLUS: &str = "\u{F067}";
/// `LV_SYMBOL_MINUS` (Font Awesome U+F068).
pub const MINUS: &str = "\u{F068}";
/// `LV_SYMBOL_EYE_OPEN` (Font Awesome U+F06E).
pub const EYE_OPEN: &str = "\u{F06E}";
/// `LV_SYMBOL_EYE_CLOSE` (Font Awesome U+F070).
pub const EYE_CLOSE: &str = "\u{F070}";
/// `LV_SYMBOL_WARNING` (Font Awesome U+F071).
pub const WARNING: &str = "\u{F071}";
/// `LV_SYMBOL_SHUFFLE` (Font Awesome U+F074).
pub const SHUFFLE: &str = "\u{F074}";
/// `LV_SYMBOL_UP` (Font Awesome U+F077).
pub const UP: &str = "\u{F077}";
/// `LV_SYMBOL_DOWN` (Font Awesome U+F078).
pub const DOWN: &str = "\u{F078}";
/// `LV_SYMBOL_LOOP` (Font Awesome U+F079).
pub const LOOP: &str = "\u{F079}";
/// `LV_SYMBOL_DIRECTORY` (Font Awesome U+F07B).
pub const DIRECTORY: &str = "\u{F07B}";
/// `LV_SYMBOL_UPLOAD` (Font Awesome U+F093).
pub const UPLOAD: &str = "\u{F093}";
/// `LV_SYMBOL_CALL` (Font Awesome U+F095).
pub const CALL: &str = "\u{F095}";
/// `LV_SYMBOL_CUT` (Font Awesome U+F0C4).
pub const CUT: &str = "\u{F0C4}";
/// `LV_SYMBOL_COPY` (Font Awesome U+F0C5).
pub const COPY: &str = "\u{F0C5}";
/// `LV_SYMBOL_SAVE` (Font Awesome U+F0C7).
pub const SAVE: &str = "\u{F0C7}";
/// `LV_SYMBOL_BARS` (Font Awesome U+F0C9).
pub const BARS: &str = "\u{F0C9}";
/// `LV_SYMBOL_ENVELOPE` (Font Awesome U+F0E0).
pub const ENVELOPE: &str = "\u{F0E0}";
/// `LV_SYMBOL_CHARGE` (Font Awesome U+F0E7).
pub const CHARGE: &str = "\u{F0E7}";
/// `LV_SYMBOL_PASTE` (Font Awesome U+F0EA).
pub const PASTE: &str = "\u{F0EA}";
/// `LV_SYMBOL_BELL` (Font Awesome U+F0F3).
pub const BELL: &str = "\u{F0F3}";
/// `LV_SYMBOL_KEYBOARD` (Font Awesome U+F11C).
pub const KEYBOARD: &str = "\u{F11C}";
/// `LV_SYMBOL_GPS` (Font Awesome U+F124).
pub const GPS: &str = "\u{F124}";
/// `LV_SYMBOL_FILE` (Font Awesome U+F15B).
pub const FILE: &str = "\u{F15B}";
/// `LV_SYMBOL_WIFI` (Font Awesome U+F1EB).
pub const WIFI: &str = "\u{F1EB}";
/// `LV_SYMBOL_BATTERY_FULL` (Font Awesome U+F240).
pub const BATTERY_FULL: &str = "\u{F240}";
/// `LV_SYMBOL_BATTERY_3` (Font Awesome U+F241).
pub const BATTERY_3: &str = "\u{F241}";
/// `LV_SYMBOL_BATTERY_2` (Font Awesome U+F242).
pub const BATTERY_2: &str = "\u{F242}";
/// `LV_SYMBOL_BATTERY_1` (Font Awesome U+F243).
pub const BATTERY_1: &str = "\u{F243}";
/// `LV_SYMBOL_BATTERY_EMPTY` (Font Awesome U+F244).
pub const BATTERY_EMPTY: &str = "\u{F244}";
/// `LV_SYMBOL_USB` (Font Awesome U+F287).
pub const USB: &str = "\u{F287}";
/// `LV_SYMBOL_BLUETOOTH` (Font Awesome U+F293).
pub const BLUETOOTH: &str = "\u{F293}";
/// `LV_SYMBOL_TRASH` (Font Awesome U+F2ED).
pub const TRASH: &str = "\u{F2ED}";
/// `LV_SYMBOL_EDIT` (Font Awesome U+F304).
pub const EDIT: &str = "\u{F304}";
/// `LV_SYMBOL_BACKSPACE` (Font Awesome U+F55A).
pub const BACKSPACE: &str = "\u{F55A}";
/// `LV_SYMBOL_SD_CARD` (Font Awesome U+F7C2).
pub const SD_CARD: &str = "\u{F7C2}";
/// `LV_SYMBOL_NEW_LINE` (Font Awesome U+F8A2).
pub const NEW_LINE: &str = "\u{F8A2}";
/// Invisible dummy symbol (U+F8FF): marks a label text as a symbol without drawing anything.
pub const DUMMY: &str = "\u{F8FF}";
/// Every symbol constant with its name, in declaration order (for galleries and tools).
pub const NAMED: &[(&str, &str)] = &[
    ("BULLET", BULLET),
    ("AUDIO", AUDIO),
    ("VIDEO", VIDEO),
    ("LIST", LIST),
    ("OK", OK),
    ("CLOSE", CLOSE),
    ("POWER", POWER),
    ("SETTINGS", SETTINGS),
    ("HOME", HOME),
    ("DOWNLOAD", DOWNLOAD),
    ("DRIVE", DRIVE),
    ("REFRESH", REFRESH),
    ("MUTE", MUTE),
    ("VOLUME_MID", VOLUME_MID),
    ("VOLUME_MAX", VOLUME_MAX),
    ("IMAGE", IMAGE),
    ("TINT", TINT),
    ("PREV", PREV),
    ("PLAY", PLAY),
    ("PAUSE", PAUSE),
    ("STOP", STOP),
    ("NEXT", NEXT),
    ("EJECT", EJECT),
    ("LEFT", LEFT),
    ("RIGHT", RIGHT),
    ("PLUS", PLUS),
    ("MINUS", MINUS),
    ("EYE_OPEN", EYE_OPEN),
    ("EYE_CLOSE", EYE_CLOSE),
    ("WARNING", WARNING),
    ("SHUFFLE", SHUFFLE),
    ("UP", UP),
    ("DOWN", DOWN),
    ("LOOP", LOOP),
    ("DIRECTORY", DIRECTORY),
    ("UPLOAD", UPLOAD),
    ("CALL", CALL),
    ("CUT", CUT),
    ("COPY", COPY),
    ("SAVE", SAVE),
    ("BARS", BARS),
    ("ENVELOPE", ENVELOPE),
    ("CHARGE", CHARGE),
    ("PASTE", PASTE),
    ("BELL", BELL),
    ("KEYBOARD", KEYBOARD),
    ("GPS", GPS),
    ("FILE", FILE),
    ("WIFI", WIFI),
    ("BATTERY_FULL", BATTERY_FULL),
    ("BATTERY_3", BATTERY_3),
    ("BATTERY_2", BATTERY_2),
    ("BATTERY_1", BATTERY_1),
    ("BATTERY_EMPTY", BATTERY_EMPTY),
    ("USB", USB),
    ("BLUETOOTH", BLUETOOTH),
    ("TRASH", TRASH),
    ("EDIT", EDIT),
    ("BACKSPACE", BACKSPACE),
    ("SD_CARD", SD_CARD),
    ("NEW_LINE", NEW_LINE),
    ("DUMMY", DUMMY),
];

/// Every Font Awesome symbol code point (all constants except [`DUMMY`] and [`BULLET`]), ascending.
pub const ALL: &[char] = &[
    '\u{F001}', '\u{F008}', '\u{F00B}', '\u{F00C}', '\u{F00D}', '\u{F011}', '\u{F013}', '\u{F015}',
    '\u{F019}', '\u{F01C}', '\u{F021}', '\u{F026}', '\u{F027}', '\u{F028}', '\u{F03E}', '\u{F043}',
    '\u{F048}', '\u{F04B}', '\u{F04C}', '\u{F04D}', '\u{F051}', '\u{F052}', '\u{F053}', '\u{F054}',
    '\u{F067}', '\u{F068}', '\u{F06E}', '\u{F070}', '\u{F071}', '\u{F074}', '\u{F077}', '\u{F078}',
    '\u{F079}', '\u{F07B}', '\u{F093}', '\u{F095}', '\u{F0C4}', '\u{F0C5}', '\u{F0C7}', '\u{F0C9}',
    '\u{F0E0}', '\u{F0E7}', '\u{F0EA}', '\u{F0F3}', '\u{F11C}', '\u{F124}', '\u{F15B}', '\u{F1EB}',
    '\u{F240}', '\u{F241}', '\u{F242}', '\u{F243}', '\u{F244}', '\u{F287}', '\u{F293}', '\u{F2ED}',
    '\u{F304}', '\u{F55A}', '\u{F7C2}', '\u{F8A2}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_sorted_and_complete() {
        assert_eq!(ALL.len(), 60);
        assert!(ALL.windows(2).all(|w| w[0] < w[1]));
        assert!(ALL.contains(&WIFI.chars().next().unwrap()));
        assert!(!ALL.contains(&BULLET.chars().next().unwrap()));
        assert_eq!(NAMED.len(), ALL.len() + 2);
    }
}
