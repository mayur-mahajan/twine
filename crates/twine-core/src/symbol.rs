//! [`Symbol`]: the built-in icon glyphs (LVGL `LV_SYMBOL_*`), with the exact code points of
//! LVGL v9 `src/font/lv_symbol_def.h` (MIT licensed, see `THIRD_PARTY.md`).
//!
//! The Font Awesome symbols are merged into the built-in Montserrat fonts (the font generator
//! takes its symbol list from [`Symbol::ALL`]), so a symbol is text: it is an image source
//! (`ImageSource::from(Symbol::Ok)` in `twine-image`) and it formats into strings without
//! allocating (`format!("{} Save", Symbol::Save)`, or `text!` in `twine-view`).
//!
//! `Symbol` is one byte; [`Symbol::as_str`] is a `const fn` lookup in a static table, so
//! `Symbol::Ok.as_str()` also works in `const`/`static` tables (keyboard maps, static image
//! sources).
//!
//! ```
//! use twine_core::Symbol;
//!
//! assert_eq!(Symbol::Ok.as_str(), "\u{F00C}");
//! assert_eq!(Symbol::Ok.as_char(), '\u{F00C}');
//! assert_eq!(Symbol::from_char('\u{F00C}'), Some(Symbol::Ok));
//! assert_eq!(format!("{} Wi-Fi", Symbol::Wifi), "\u{F1EB} Wi-Fi");
//! assert_eq!(core::mem::size_of::<Symbol>(), 1);
//! ```

use core::fmt;

macro_rules! symbols {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $cp:literal, $name:literal, $alias:literal;
    )+) => {
        /// A built-in symbol (icon glyph of the symbol font).
        ///
        /// Every variant except [`Symbol::Bullet`] (from the text font itself) and
        /// [`Symbol::Dummy`] (invisible) is a Font Awesome glyph merged into the built-in
        /// fonts; see the [module docs](self).
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum Symbol {
            $(
                $(#[doc = $doc])*
                #[doc(alias = $alias)]
                $variant,
            )+
        }

        /// The glyph of every variant (indexed by discriminant).
        const GLYPHS: &[&str] = &[$($cp),+];
        /// The name of every variant (indexed by discriminant; only linked when used).
        const NAMES: &[&str] = &[$($name),+];

        impl Symbol {
            /// Every symbol, in declaration order (ascending code point, except
            /// [`Symbol::Bullet`] first).
            pub const ALL: &'static [Symbol] = &[$(Symbol::$variant),+];

            /// The symbol whose glyph is `c` (`None` for any other character).
            #[must_use]
            pub const fn from_char(c: char) -> Option<Symbol> {
                // `GLYPHS` literals are one `char` each (checked by a unit test).
                $(
                    if c == const { first_char($cp) } {
                        return Some(Symbol::$variant);
                    }
                )+
                None
            }
        }
    };
}

/// The first `char` of a one-character string literal (const context).
const fn first_char(s: &str) -> char {
    let b = s.as_bytes();
    // Symbols are BMP characters (1 to 3 UTF-8 bytes).
    let cp = match b.len() {
        1 => b[0] as u32,
        2 => ((b[0] as u32 & 0x1F) << 6) | (b[1] as u32 & 0x3F),
        _ => ((b[0] as u32 & 0x0F) << 12) | ((b[1] as u32 & 0x3F) << 6) | (b[2] as u32 & 0x3F),
    };
    match char::from_u32(cp) {
        Some(c) => c,
        None => '\0',
    }
}

symbols! {
    /// Bullet (U+2022, from the text font itself).
    Bullet = "\u{2022}", "Bullet", "LV_SYMBOL_BULLET";
    /// Audio (Font Awesome U+F001).
    Audio = "\u{F001}", "Audio", "LV_SYMBOL_AUDIO";
    /// Video (Font Awesome U+F008).
    Video = "\u{F008}", "Video", "LV_SYMBOL_VIDEO";
    /// List (Font Awesome U+F00B).
    List = "\u{F00B}", "List", "LV_SYMBOL_LIST";
    /// OK / check mark (Font Awesome U+F00C).
    Ok = "\u{F00C}", "Ok", "LV_SYMBOL_OK";
    /// Close / cross (Font Awesome U+F00D).
    Close = "\u{F00D}", "Close", "LV_SYMBOL_CLOSE";
    /// Power (Font Awesome U+F011).
    Power = "\u{F011}", "Power", "LV_SYMBOL_POWER";
    /// Settings / gear (Font Awesome U+F013).
    Settings = "\u{F013}", "Settings", "LV_SYMBOL_SETTINGS";
    /// Home (Font Awesome U+F015).
    Home = "\u{F015}", "Home", "LV_SYMBOL_HOME";
    /// Download (Font Awesome U+F019).
    Download = "\u{F019}", "Download", "LV_SYMBOL_DOWNLOAD";
    /// Drive (Font Awesome U+F01C).
    Drive = "\u{F01C}", "Drive", "LV_SYMBOL_DRIVE";
    /// Refresh (Font Awesome U+F021).
    Refresh = "\u{F021}", "Refresh", "LV_SYMBOL_REFRESH";
    /// Mute (Font Awesome U+F026).
    Mute = "\u{F026}", "Mute", "LV_SYMBOL_MUTE";
    /// Volume, medium (Font Awesome U+F027).
    VolumeMid = "\u{F027}", "VolumeMid", "LV_SYMBOL_VOLUME_MID";
    /// Volume, maximum (Font Awesome U+F028).
    VolumeMax = "\u{F028}", "VolumeMax", "LV_SYMBOL_VOLUME_MAX";
    /// Image (Font Awesome U+F03E).
    Image = "\u{F03E}", "Image", "LV_SYMBOL_IMAGE";
    /// Tint / drop (Font Awesome U+F043).
    Tint = "\u{F043}", "Tint", "LV_SYMBOL_TINT";
    /// Previous (Font Awesome U+F048).
    Prev = "\u{F048}", "Prev", "LV_SYMBOL_PREV";
    /// Play (Font Awesome U+F04B).
    Play = "\u{F04B}", "Play", "LV_SYMBOL_PLAY";
    /// Pause (Font Awesome U+F04C).
    Pause = "\u{F04C}", "Pause", "LV_SYMBOL_PAUSE";
    /// Stop (Font Awesome U+F04D).
    Stop = "\u{F04D}", "Stop", "LV_SYMBOL_STOP";
    /// Next (Font Awesome U+F051).
    Next = "\u{F051}", "Next", "LV_SYMBOL_NEXT";
    /// Eject (Font Awesome U+F052).
    Eject = "\u{F052}", "Eject", "LV_SYMBOL_EJECT";
    /// Left chevron (Font Awesome U+F053).
    Left = "\u{F053}", "Left", "LV_SYMBOL_LEFT";
    /// Right chevron (Font Awesome U+F054).
    Right = "\u{F054}", "Right", "LV_SYMBOL_RIGHT";
    /// Plus (Font Awesome U+F067).
    Plus = "\u{F067}", "Plus", "LV_SYMBOL_PLUS";
    /// Minus (Font Awesome U+F068).
    Minus = "\u{F068}", "Minus", "LV_SYMBOL_MINUS";
    /// Eye, open (Font Awesome U+F06E).
    EyeOpen = "\u{F06E}", "EyeOpen", "LV_SYMBOL_EYE_OPEN";
    /// Eye, closed (Font Awesome U+F070).
    EyeClose = "\u{F070}", "EyeClose", "LV_SYMBOL_EYE_CLOSE";
    /// Warning triangle (Font Awesome U+F071).
    Warning = "\u{F071}", "Warning", "LV_SYMBOL_WARNING";
    /// Shuffle (Font Awesome U+F074).
    Shuffle = "\u{F074}", "Shuffle", "LV_SYMBOL_SHUFFLE";
    /// Up chevron (Font Awesome U+F077).
    Up = "\u{F077}", "Up", "LV_SYMBOL_UP";
    /// Down chevron (Font Awesome U+F078).
    Down = "\u{F078}", "Down", "LV_SYMBOL_DOWN";
    /// Loop (Font Awesome U+F079).
    Loop = "\u{F079}", "Loop", "LV_SYMBOL_LOOP";
    /// Directory / folder (Font Awesome U+F07B).
    Directory = "\u{F07B}", "Directory", "LV_SYMBOL_DIRECTORY";
    /// Upload (Font Awesome U+F093).
    Upload = "\u{F093}", "Upload", "LV_SYMBOL_UPLOAD";
    /// Call / phone (Font Awesome U+F095).
    Call = "\u{F095}", "Call", "LV_SYMBOL_CALL";
    /// Cut (Font Awesome U+F0C4).
    Cut = "\u{F0C4}", "Cut", "LV_SYMBOL_CUT";
    /// Copy (Font Awesome U+F0C5).
    Copy = "\u{F0C5}", "Copy", "LV_SYMBOL_COPY";
    /// Save (Font Awesome U+F0C7).
    Save = "\u{F0C7}", "Save", "LV_SYMBOL_SAVE";
    /// Bars / menu (Font Awesome U+F0C9).
    Bars = "\u{F0C9}", "Bars", "LV_SYMBOL_BARS";
    /// Envelope (Font Awesome U+F0E0).
    Envelope = "\u{F0E0}", "Envelope", "LV_SYMBOL_ENVELOPE";
    /// Charge / lightning (Font Awesome U+F0E7).
    Charge = "\u{F0E7}", "Charge", "LV_SYMBOL_CHARGE";
    /// Paste (Font Awesome U+F0EA).
    Paste = "\u{F0EA}", "Paste", "LV_SYMBOL_PASTE";
    /// Bell (Font Awesome U+F0F3).
    Bell = "\u{F0F3}", "Bell", "LV_SYMBOL_BELL";
    /// Keyboard (Font Awesome U+F11C).
    Keyboard = "\u{F11C}", "Keyboard", "LV_SYMBOL_KEYBOARD";
    /// GPS (Font Awesome U+F124).
    Gps = "\u{F124}", "Gps", "LV_SYMBOL_GPS";
    /// File (Font Awesome U+F15B).
    File = "\u{F15B}", "File", "LV_SYMBOL_FILE";
    /// Wi-Fi (Font Awesome U+F1EB).
    Wifi = "\u{F1EB}", "Wifi", "LV_SYMBOL_WIFI";
    /// Battery, full (Font Awesome U+F240).
    BatteryFull = "\u{F240}", "BatteryFull", "LV_SYMBOL_BATTERY_FULL";
    /// Battery, three quarters (Font Awesome U+F241).
    Battery3 = "\u{F241}", "Battery3", "LV_SYMBOL_BATTERY_3";
    /// Battery, half (Font Awesome U+F242).
    Battery2 = "\u{F242}", "Battery2", "LV_SYMBOL_BATTERY_2";
    /// Battery, one quarter (Font Awesome U+F243).
    Battery1 = "\u{F243}", "Battery1", "LV_SYMBOL_BATTERY_1";
    /// Battery, empty (Font Awesome U+F244).
    BatteryEmpty = "\u{F244}", "BatteryEmpty", "LV_SYMBOL_BATTERY_EMPTY";
    /// USB (Font Awesome U+F287).
    Usb = "\u{F287}", "Usb", "LV_SYMBOL_USB";
    /// Bluetooth (Font Awesome U+F293).
    Bluetooth = "\u{F293}", "Bluetooth", "LV_SYMBOL_BLUETOOTH";
    /// Trash (Font Awesome U+F2ED).
    Trash = "\u{F2ED}", "Trash", "LV_SYMBOL_TRASH";
    /// Edit / pen (Font Awesome U+F304).
    Edit = "\u{F304}", "Edit", "LV_SYMBOL_EDIT";
    /// Backspace (Font Awesome U+F55A).
    Backspace = "\u{F55A}", "Backspace", "LV_SYMBOL_BACKSPACE";
    /// SD card (Font Awesome U+F7C2).
    SdCard = "\u{F7C2}", "SdCard", "LV_SYMBOL_SD_CARD";
    /// New line / return arrow (U+F8A2).
    NewLine = "\u{F8A2}", "NewLine", "LV_SYMBOL_NEW_LINE";
    /// Invisible dummy symbol (U+F8FF): marks a label text as a symbol without drawing
    /// anything.
    Dummy = "\u{F8FF}", "Dummy", "LV_SYMBOL_DUMMY";
}

impl Symbol {
    /// The glyph as a one-character string (a `const fn`: usable in `const`/`static` tables).
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        GLYPHS[self as usize]
    }

    /// The glyph's code point.
    #[must_use]
    #[inline]
    pub const fn as_char(self) -> char {
        first_char(self.as_str())
    }

    /// The variant name (`"Ok"`, `"VolumeMid"`…), for galleries and tools.
    #[must_use]
    pub const fn name(self) -> &'static str {
        NAMES[self as usize]
    }

    /// `true` for the Font Awesome glyphs merged into the built-in fonts from the symbol font
    /// (every symbol except [`Symbol::Bullet`] and [`Symbol::Dummy`]).
    #[must_use]
    pub const fn is_font_awesome(self) -> bool {
        !matches!(self, Symbol::Bullet | Symbol::Dummy)
    }

    /// The symbol whose glyph is exactly `s` (one character), e.g. a key text of a keyboard.
    ///
    /// ```
    /// use twine_core::Symbol;
    /// assert_eq!(Symbol::from_text("\u{F053}"), Some(Symbol::Left));
    /// assert_eq!(Symbol::from_text("\u{F053}x"), None);
    /// assert_eq!(Symbol::from_text("a"), None);
    /// ```
    #[must_use]
    pub fn from_text(s: &str) -> Option<Symbol> {
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Symbol::from_char(c),
            _ => None,
        }
    }
}

impl fmt::Display for Symbol {
    /// Writes the glyph (no allocation): `format!("{} Save", Symbol::Save)`.
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol::{}", self.name())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Symbol {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Symbol::{=str}", self.name());
    }
}

impl From<Symbol> for char {
    fn from(s: Symbol) -> char {
        s.as_char()
    }
}

impl From<Symbol> for &'static str {
    fn from(s: Symbol) -> &'static str {
        s.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn table_is_consistent() {
        assert_eq!(Symbol::ALL.len(), 62);
        assert_eq!(GLYPHS.len(), Symbol::ALL.len());
        assert_eq!(NAMES.len(), Symbol::ALL.len());
        for (i, &s) in Symbol::ALL.iter().enumerate() {
            assert_eq!(s as usize, i);
            // One character per glyph; `as_char` agrees with `as_str`.
            let mut it = s.as_str().chars();
            assert_eq!(it.next(), Some(s.as_char()));
            assert_eq!(it.next(), None);
            assert_eq!(Symbol::from_char(s.as_char()), Some(s));
            assert_eq!(Symbol::from_text(s.as_str()), Some(s));
        }
        // Ascending code points after the bullet.
        assert!(
            Symbol::ALL[1..]
                .windows(2)
                .all(|w| w[0].as_char() < w[1].as_char())
        );
        assert_eq!(Symbol::ALL.iter().filter(|s| s.is_font_awesome()).count(), 60);
        assert_eq!(Symbol::from_char('a'), None);
        assert_eq!(Symbol::from_text(""), None);
    }

    #[test]
    fn lvgl_code_points() {
        assert_eq!(Symbol::Ok.as_str(), "\u{F00C}");
        assert_eq!(Symbol::Bullet.as_char(), '\u{2022}');
        assert_eq!(Symbol::NewLine.as_char(), '\u{F8A2}');
        assert_eq!(Symbol::Dummy.as_char(), '\u{F8FF}');
        assert_eq!(Symbol::VolumeMid.name(), "VolumeMid");
    }

    #[test]
    fn display_and_debug() {
        assert_eq!(format!("{} Save", Symbol::Save), "\u{F0C7} Save");
        assert_eq!(format!("{:?}", Symbol::Ok), "Symbol::Ok");
    }

    #[test]
    fn const_usable() {
        const MAP: [&str; 2] = [Symbol::Left.as_str(), Symbol::Right.as_str()];
        assert_eq!(MAP, ["\u{F053}", "\u{F054}"]);
        assert_eq!(core::mem::size_of::<Symbol>(), 1);
        assert_eq!(core::mem::size_of::<Option<Symbol>>(), 1);
    }
}
