//! Selectors: which part of a widget, in which state, a style applies to.

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

/// A part of a widget (LVGL `lv_part_t`; the values here are LVGL's shifted right by 16).
///
/// The built-in parts cover what LVGL's widgets draw. A widget that draws more (a gauge's
/// track, needle and readout, a text area's placeholder) takes **custom parts**:
/// [`Part::custom::<N>()`](Part::custom), `N` in `0..`[`Part::CUSTOM_COUNT`] (7, LVGL's
/// `LV_PART_CUSTOM_FIRST` and the six values after it). A custom part means whatever its
/// widget class says: name them with constants next to the class and list them in
/// the widget class's `parts` (`twine_engine::WidgetClass::parts`), e.g.
///
/// ```
/// use twine_style::Part;
///
/// /// The gauge's background ring.
/// pub const TRACK: Part = Part::custom::<0>();
/// /// The gauge's needle.
/// pub const NEEDLE: Part = Part::custom::<1>();
/// assert_ne!(TRACK, NEEDLE);
/// assert_eq!(NEEDLE.custom_index(), Some(1));
/// assert_eq!(Part::Main.custom_index(), None);
/// ```
///
/// The same custom part number means different things in different classes (the text
/// area's placeholder is `custom::<0>()` like the gauge's track), so custom parts have no
/// global names (unlike [`State::custom`] states); `Debug` prints `Custom0` … `Custom6`.
/// The variants `Custom0` … `Custom6` are the representation (usable in `match` like the
/// constants above); [`Part::custom`] is the spelling to use, mirroring [`State::custom`].
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Part {
    /// `LV_PART_MAIN`: the background rectangle.
    #[default]
    Main = 0x00,
    /// `LV_PART_SCROLLBAR`
    Scrollbar = 0x01,
    /// `LV_PART_INDICATOR`: e.g. the bar/slider/switch indicator, the checkbox tick box.
    Indicator = 0x02,
    /// `LV_PART_KNOB`: a handle to grab.
    Knob = 0x03,
    /// `LV_PART_SELECTED`: the selected option or section.
    Selected = 0x04,
    /// `LV_PART_ITEMS`: repeated elements (table cells, buttons of a button matrix).
    Items = 0x05,
    /// `LV_PART_CURSOR`: a marker, e.g. the text area cursor.
    Cursor = 0x06,
    /// Custom part 0 ([`Part::custom::<0>()`](Part::custom), LVGL `LV_PART_CUSTOM_FIRST`).
    Custom0 = 0x08,
    /// Custom part 1 ([`Part::custom::<1>()`](Part::custom)).
    Custom1 = 0x09,
    /// Custom part 2 ([`Part::custom::<2>()`](Part::custom)).
    Custom2 = 0x0A,
    /// Custom part 3 ([`Part::custom::<3>()`](Part::custom)).
    Custom3 = 0x0B,
    /// Custom part 4 ([`Part::custom::<4>()`](Part::custom)).
    Custom4 = 0x0C,
    /// Custom part 5 ([`Part::custom::<5>()`](Part::custom)).
    Custom5 = 0x0D,
    /// Custom part 6 ([`Part::custom::<6>()`](Part::custom)).
    Custom6 = 0x0E,
    /// `LV_PART_ANY`: in a selector, matches every part (a Twine extension for style lookup;
    /// LVGL uses it only to address all parts when removing styles).
    Any = 0x0F,
}

impl Part {
    /// How many custom parts exist: [`Part::custom::<0>()`](Part::custom) to `custom::<6>()`
    /// (LVGL's part values `LV_PART_CUSTOM_FIRST` … `LV_PART_ANY - 1`).
    pub const CUSTOM_COUNT: usize = 7;

    /// Custom part number `N` (`0..`[`CUSTOM_COUNT`](Self::CUSTOM_COUNT)), for a part of a
    /// custom widget the built-in parts do not describe (see [`Part`]). An `N` out of range is
    /// a compile-time error, so this never panics at run time.
    ///
    /// ```
    /// use twine_style::Part;
    ///
    /// const LABEL: Part = Part::custom::<3>();
    /// assert_eq!(LABEL, Part::Custom3);
    /// ```
    ///
    /// ```compile_fail
    /// const TOO_MANY: twine_style::Part = twine_style::Part::custom::<7>();
    /// # let _ = TOO_MANY;
    /// ```
    #[doc(alias = "LV_PART_CUSTOM_FIRST")]
    #[must_use]
    pub const fn custom<const N: usize>() -> Part {
        const {
            assert!(
                N < Part::CUSTOM_COUNT,
                "Part::custom::<N>: N must be below Part::CUSTOM_COUNT (7)"
            );
        }
        match N {
            0 => Part::Custom0,
            1 => Part::Custom1,
            2 => Part::Custom2,
            3 => Part::Custom3,
            4 => Part::Custom4,
            5 => Part::Custom5,
            _ => Part::Custom6,
        }
    }

    /// The number of a custom part (`Some(N)` for [`Part::custom::<N>()`](Part::custom)),
    /// `None` for the built-in parts and [`Part::Any`].
    #[must_use]
    pub const fn custom_index(self) -> Option<usize> {
        let v = self as u8;
        if v >= Part::Custom0 as u8 && v <= Part::Custom6 as u8 {
            Some((v - Part::Custom0 as u8) as usize)
        } else {
            None
        }
    }
}

bitflags::bitflags! {
    /// Widget states. A widget can be in several at once; [`State::DEFAULT`] is the empty set.
    ///
    /// The built-in states have LVGL v9.6's names and bit values (`lv_state_t`); the
    /// application's own states are made with [`State::custom`] (up to
    /// [`State::CUSTOM_COUNT`], LVGL's `LV_STATE_USER_1..4`) and can be given readable names
    /// for `Debug`, `Display`, `defmt` and tree dumps with [`State::set_custom_name`].
    ///
    /// # Precedence
    ///
    /// When several styles set a property and all their states are active, the selector with
    /// the higher precedence wins. Precedence is defined by the ranked list
    /// [`State::PRECEDENCE`] (lowest first), not by the bit values:
    ///
    /// | Rank | State | Typical use |
    /// |------|-------|-------------|
    /// | – | `DEFAULT` (no state) | the base look; also [`State::ANY`] in a selector |
    /// | 0 | [`ALT`](State::ALT) | an alternative look |
    /// | 1 | [`CHECKED`](State::CHECKED) | toggled / checked |
    /// | 2 | [`FOCUSED`](State::FOCUSED) | focused |
    /// | 3 | [`FOCUS_KEY`](State::FOCUS_KEY) | focused by keypad / encoder |
    /// | 4 | [`EDITED`](State::EDITED) | edited by an encoder |
    /// | 5 | [`HOVERED`](State::HOVERED) | hovered by a mouse |
    /// | 6 | [`PRESSED`](State::PRESSED) | being pressed |
    /// | 7 | [`SCROLLED`](State::SCROLLED) | being scrolled |
    /// | 8 | [`DISABLED`](State::DISABLED) | disabled |
    /// | 9–12 | [`custom::<0>()`](State::custom) … `custom::<3>()` | the application's states (higher index wins) |
    ///
    /// **The rule:** compare the two selectors' highest-ranked states; the higher rank wins. If
    /// they are the same state, compare the next highest state of each, and so on; a selector
    /// that runs out of states first loses (so `PRESSED | CHECKED` beats `PRESSED`, the more
    /// specific selector wins). In other words, one state outranks *every combination* of
    /// lower-ranked states: `DISABLED` beats `PRESSED | CHECKED | FOCUSED`. Equal state sets
    /// fall back to the entry order (transition, local, latest normal, latest theme style).
    /// This is LVGL's order, so every built-in theme looks exactly as in LVGL; application
    /// states rank above all built-in states, so an app look (an alarm colour) is never
    /// overridden by a theme's pressed or disabled look — combine the states
    /// (`DISABLED | ALARM`) for a look specific to both.
    ///
    /// | Node state | Matching selectors | Winner |
    /// |------------|--------------------|--------|
    /// | `PRESSED \| CHECKED` | `CHECKED`, `PRESSED` | `PRESSED` |
    /// | `PRESSED \| CHECKED` | `PRESSED`, `PRESSED \| CHECKED` | `PRESSED \| CHECKED` |
    /// | `DISABLED \| PRESSED \| CHECKED` | `DISABLED`, `PRESSED \| CHECKED` | `DISABLED` |
    /// | `FOCUSED \| FOCUS_KEY \| PRESSED` | `FOCUS_KEY`, `FOCUSED \| PRESSED` | `FOCUSED \| PRESSED` |
    /// | `DISABLED \| ALARM` | `DISABLED`, `ALARM` (custom) | `ALARM` |
    ///
    /// Comparing two selectors costs one integer comparison of their [`Selector::weight`]s:
    /// the bits of the states are laid out in precedence order (checked at compile time), so
    /// the weight is the state bits themselves and nothing is computed per style lookup.
    ///
    /// ```
    /// use twine_style::{Selector, State};
    ///
    /// const ALARM: State = State::custom::<0>();
    /// let wins = |a: State, b: State| Selector::state(a).weight() > Selector::state(b).weight();
    /// assert!(wins(State::PRESSED, State::CHECKED));
    /// assert!(wins(State::PRESSED | State::CHECKED, State::PRESSED));
    /// assert!(wins(State::DISABLED, State::PRESSED | State::CHECKED | State::FOCUSED));
    /// assert!(wins(ALARM, State::DISABLED));
    /// ```
    #[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct State: u16 {
        /// `LV_STATE_DEFAULT`: no state (every selector state is a superset).
        const DEFAULT = 0;
        /// `LV_STATE_ALT`: alternative look (e.g. a dark variant), lowest precedence.
        const ALT = 1 << 0;
        /// `LV_STATE_CHECKED`: toggled or checked.
        const CHECKED = 1 << 2;
        /// `LV_STATE_FOCUSED`: focused by keypad/encoder or clicked.
        const FOCUSED = 1 << 3;
        /// `LV_STATE_FOCUS_KEY`: focused by keypad/encoder but not by a pointer.
        const FOCUS_KEY = 1 << 4;
        /// `LV_STATE_EDITED`: edited by an encoder.
        const EDITED = 1 << 5;
        /// `LV_STATE_HOVERED`: hovered by a mouse.
        const HOVERED = 1 << 6;
        /// `LV_STATE_PRESSED`: being pressed.
        const PRESSED = 1 << 7;
        /// `LV_STATE_SCROLLED`: being scrolled.
        const SCROLLED = 1 << 8;
        /// `LV_STATE_DISABLED`: disabled.
        const DISABLED = 1 << 9;
        // The application's states (`State::custom`), LVGL's `LV_STATE_USER_1..4`.
        const _ = 0xF000;
    }
}

/// Bit of the first custom state.
const CUSTOM_SHIFT: u32 = 12;
/// Longest custom state name kept (bytes).
const NAME_MAX: usize = 16;
/// The built-in states with their names, in precedence order (lowest first).
const BUILT_IN: [(State, &str); 9] = [
    (State::ALT, "ALT"),
    (State::CHECKED, "CHECKED"),
    (State::FOCUSED, "FOCUSED"),
    (State::FOCUS_KEY, "FOCUS_KEY"),
    (State::EDITED, "EDITED"),
    (State::HOVERED, "HOVERED"),
    (State::PRESSED, "PRESSED"),
    (State::SCROLLED, "SCROLLED"),
    (State::DISABLED, "DISABLED"),
];

impl State {
    /// How many application states exist: [`State::custom::<0>()`](State::custom) to
    /// `custom::<3>()` (LVGL has the same four, `LV_STATE_USER_1..4`).
    pub const CUSTOM_COUNT: usize = 4;

    /// Every application state ([`State::custom`]) at once, e.g. to clear them all with
    /// `set_state(node, State::CUSTOM_ALL, false)`.
    pub const CUSTOM_ALL: State = State::from_bits_retain(0xF000);

    /// Every state, built-in and custom (LVGL `LV_STATE_ANY`, used the same way: as a
    /// wildcard).
    ///
    /// - **In a style's selector** it means "in every state" and has the lowest precedence,
    ///   exactly like [`State::DEFAULT`]. (In LVGL such a style never applies, since no
    ///   widget is in every state at once; Twine gives it the only useful meaning.)
    /// - **As a filter** (`remove_style`'s selector) it matches the styles of every state.
    /// - **As a state change** (`set_state(node, State::ANY, false)`) it sets or clears every
    ///   state. Its value is the union of the defined states (`0xF3FD`), so a node never gets
    ///   undefined state bits.
    pub const ANY: State = State::all();

    /// The states in precedence order, lowest first (see [Precedence](State#precedence)): a
    /// state outranks every combination of the states before it. [`Selector::weight`] orders
    /// selectors by this list.
    pub const PRECEDENCE: [State; 13] = [
        State::ALT,
        State::CHECKED,
        State::FOCUSED,
        State::FOCUS_KEY,
        State::EDITED,
        State::HOVERED,
        State::PRESSED,
        State::SCROLLED,
        State::DISABLED,
        State::custom::<0>(),
        State::custom::<1>(),
        State::custom::<2>(),
        State::custom::<3>(),
    ];

    /// The application's state number `N` (`0..`[`CUSTOM_COUNT`](Self::CUSTOM_COUNT)), for a
    /// state the built-in ones do not cover (an alarm, a selected row, a read-only field).
    /// Custom states rank above every built-in state, and a higher `N` above a lower one (see
    /// [Precedence](State#precedence)). An `N` out of range is a compile-time error, so this
    /// never panics at run time.
    ///
    /// Name it for debug output with [`State::set_custom_name`].
    ///
    /// ```
    /// use twine_style::{Selector, State};
    ///
    /// const ALARM: State = State::custom::<0>();
    /// const MUTED: State = State::custom::<1>();
    /// assert_ne!(ALARM, MUTED);
    /// assert!(Selector::state(MUTED).weight() > Selector::state(ALARM).weight());
    /// assert!(Selector::state(ALARM).weight() > Selector::state(State::DISABLED).weight());
    /// ```
    ///
    /// ```compile_fail
    /// const TOO_MANY: twine_style::State = twine_style::State::custom::<4>();
    /// # let _ = TOO_MANY;
    /// ```
    #[doc(alias = "LV_STATE_USER_1")]
    #[doc(alias = "LV_STATE_USER_2")]
    #[doc(alias = "LV_STATE_USER_3")]
    #[doc(alias = "LV_STATE_USER_4")]
    #[must_use]
    pub const fn custom<const N: usize>() -> State {
        const {
            assert!(
                N < State::CUSTOM_COUNT,
                "State::custom::<N>: N must be below State::CUSTOM_COUNT (4)"
            );
        }
        State::from_bits_retain(1 << (CUSTOM_SHIFT + N as u32))
    }

    /// Gives the custom state `state` (one [`State::custom`] state) a name shown by `Debug`,
    /// `Display`, `defmt` and tree dumps instead of `CUSTOM_n`. Call it once at start-up, for
    /// example next to the constants. An empty name restores `CUSTOM_n`.
    ///
    /// The names are global (one set per program: custom states mean the same thing in every
    /// `Ui`) and only used for output, never for matching. A name keeps at most 16 bytes
    /// (longer ones are cut at a character boundary, with a warning); `state` that is not
    /// exactly one custom state is ignored with a warning. Never panics, never allocates;
    /// works from any context (plain atomic loads and stores, no compare-and-swap). Renaming
    /// while another thread formats a state may print `CUSTOM_n` once.
    ///
    /// ```
    /// use twine_style::State;
    ///
    /// const ALARM: State = State::custom::<2>();
    /// State::set_custom_name(ALARM, "ALARM");
    /// assert_eq!(format!("{:?}", ALARM | State::PRESSED), "State(PRESSED | ALARM)");
    /// assert_eq!(format!("{}", ALARM | State::PRESSED), "PRESSED|ALARM");
    /// ```
    pub fn set_custom_name(state: State, name: &str) {
        let Some(i) = state.custom_index() else {
            twine_core::warn!(target: "twine::style", "State::set_custom_name: not exactly one custom state, ignored");
            return;
        };
        let mut len = name.len().min(NAME_MAX);
        while !name.is_char_boundary(len) {
            len -= 1;
        }
        if len < name.len() {
            twine_core::warn!(target: "twine::style", "State::set_custom_name: name longer than 16 bytes, cut");
        }
        let slot = &CUSTOM_NAMES[i];
        slot.len.store(0, Ordering::Release);
        for (dst, b) in slot.bytes.iter().zip(name.as_bytes()[..len].iter()) {
            dst.store(*b, Ordering::Relaxed);
        }
        #[allow(clippy::cast_possible_truncation)] // len <= NAME_MAX = 16
        slot.len.store(len as u8, Ordering::Release);
    }

    /// The index of a single custom state.
    const fn custom_index(self) -> Option<usize> {
        let b = self.bits();
        if b.is_power_of_two() && b & State::CUSTOM_ALL.bits() != 0 {
            Some((b.trailing_zeros() - CUSTOM_SHIFT) as usize)
        } else {
            None
        }
    }

    /// Calls `f` with the name of every state in `self`, in precedence order (custom states by
    /// their [`set_custom_name`](Self::set_custom_name) name or `CUSTOM_n`); bits that are no
    /// state come last as one hexadecimal number.
    fn for_each_name(self, mut f: impl FnMut(&str) -> fmt::Result) -> fmt::Result {
        for (s, name) in BUILT_IN {
            if self.contains(s) {
                f(name)?;
            }
        }
        for i in 0..State::CUSTOM_COUNT {
            if self.bits() & (1 << (CUSTOM_SHIFT + i as u32)) != 0 {
                let name = CustomName::load(i);
                f(name.as_str())?;
            }
        }
        let unknown = self.bits() & !State::all().bits();
        if unknown != 0 {
            let mut buf = [0u8; 6];
            f(hex(unknown, &mut buf))?;
        }
        Ok(())
    }
}

/// `0x…` of `v` in `buf`.
fn hex(v: u16, buf: &mut [u8; 6]) -> &str {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..4 {
        buf[2 + i] = DIGITS[usize::from((v >> (12 - 4 * i)) & 0xF)];
    }
    core::str::from_utf8(buf).unwrap_or("0x?")
}

/// The stored name of one custom state.
struct NameSlot {
    len: AtomicU8,
    bytes: [AtomicU8; NAME_MAX],
}

static CUSTOM_NAMES: [NameSlot; State::CUSTOM_COUNT] = [const {
    NameSlot {
        len: AtomicU8::new(0),
        bytes: [const { AtomicU8::new(0) }; NAME_MAX],
    }
}; State::CUSTOM_COUNT];

/// A copy of a custom state's name (or `CUSTOM_n`), so formatting never holds the slot.
struct CustomName {
    buf: [u8; NAME_MAX],
    len: usize,
}

impl CustomName {
    fn load(i: usize) -> Self {
        let mut buf = [0u8; NAME_MAX];
        let slot = &CUSTOM_NAMES[i];
        let len = usize::from(slot.len.load(Ordering::Acquire)).min(NAME_MAX);
        for (d, s) in buf.iter_mut().zip(slot.bytes.iter()).take(len) {
            *d = s.load(Ordering::Relaxed);
        }
        let mut name = Self { buf, len };
        if len == 0 || core::str::from_utf8(&name.buf[..len]).is_err() {
            name.buf[..7].copy_from_slice(b"CUSTOM_");
            #[allow(clippy::cast_possible_truncation)] // i < CUSTOM_COUNT = 4
            {
                name.buf[7] = b'0' + i as u8;
            }
            name.len = 8;
        }
        name
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("CUSTOM")
    }
}

// The precedence list is the contract; the bit layout is chosen so that a selector's
// precedence mask (bit `i` = the state of rank `i`, compared as an integer) is the state bits
// themselves. Reordering `PRECEDENCE` without reassigning the bits fails to compile.
const _: () = {
    let p = State::PRECEDENCE;
    let mut union = 0u16;
    let mut i = 0;
    while i < p.len() {
        assert!(p[i].bits().is_power_of_two(), "one state per rank");
        assert!(
            i == 0 || p[i - 1].bits() < p[i].bits(),
            "bits in precedence order"
        );
        union |= p[i].bits();
        i += 1;
    }
    assert!(union == State::ANY.bits(), "every state is ranked");
};

/// `PRESSED|CHECKED` (precedence order, `|` without spaces), `DEFAULT` for no state; custom
/// states by their [`State::set_custom_name`] name. Used by tree dumps.
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("DEFAULT");
        }
        let mut first = true;
        self.for_each_name(|n| {
            if !first {
                f.write_str("|")?;
            }
            first = false;
            f.write_str(n)
        })
    }
}

/// `State(PRESSED | CHECKED)`, `State(DEFAULT)`; custom states by their
/// [`State::set_custom_name`] name.
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("State(")?;
        if self.is_empty() {
            f.write_str("DEFAULT")?;
        }
        let mut first = true;
        self.for_each_name(|n| {
            if !first {
                f.write_str(" | ")?;
            }
            first = false;
            f.write_str(n)
        })?;
        f.write_str(")")
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for State {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "State(");
        if self.is_empty() {
            defmt::write!(f, "DEFAULT");
        }
        let mut first = true;
        let _ = self.for_each_name(|n| {
            if !first {
                defmt::write!(f, " | ");
            }
            first = false;
            defmt::write!(f, "{=str}", n);
            Ok(())
        });
        defmt::write!(f, ")");
    }
}

/// A part plus the states in which a style applies (LVGL `lv_style_selector_t` =
/// `part | state`).
///
/// ```
/// use twine_style::{Part, Selector, State};
///
/// let sel = Selector::part(Part::Indicator).with_state(State::PRESSED);
/// assert!(sel.matches(Part::Indicator, State::PRESSED | State::FOCUSED));
/// assert!(!sel.matches(Part::Indicator, State::FOCUSED)); // PRESSED missing
/// assert!(!sel.matches(Part::Main, State::PRESSED)); // other part
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Selector {
    /// The part.
    pub part: Part,
    /// The states that must all be active (empty = any state).
    pub state: State,
}

impl Selector {
    /// `Part::Main` in every state (LVGL selector `0`).
    pub const MAIN: Selector = Selector {
        part: Part::Main,
        state: State::DEFAULT,
    };

    /// `p` in every state.
    #[must_use]
    pub const fn part(p: Part) -> Self {
        Self {
            part: p,
            state: State::DEFAULT,
        }
    }

    /// `Part::Main` when all of `s` are active.
    #[must_use]
    pub const fn state(s: State) -> Self {
        Self {
            part: Part::Main,
            state: s,
        }
    }

    /// The same part when all of `s` are active.
    #[must_use]
    pub const fn with_state(self, s: State) -> Self {
        Self {
            part: self.part,
            state: s,
        }
    }

    /// Whether the part matches: equal, or the selector's part is [`Part::Any`].
    #[inline]
    #[must_use]
    pub fn part_matches(&self, part: Part) -> bool {
        self.part == part || self.part == Part::Any
    }

    /// Whether a style with this selector applies to `part` of a node in `node_state`: the part
    /// matches ([`part_matches`](Self::part_matches)) and the states do
    /// ([`state_matches`](Self::state_matches)).
    #[inline]
    #[must_use]
    pub fn matches(&self, part: Part, node_state: State) -> bool {
        self.part_matches(part) && self.state_matches(node_state)
    }

    /// Whether the selector's states apply to a node in `node_state`: every selector state is
    /// active (LVGL: `(state_act & ~node_state) == 0`), or the selector's state is
    /// [`State::ANY`] (every state). The one test every part of Twine uses (resolution, state
    /// changes, transitions).
    #[inline]
    #[must_use]
    pub fn state_matches(&self, node_state: State) -> bool {
        self.state == State::ANY || self.state.bits() & !node_state.bits() == 0
    }

    /// Precedence among matching selectors: the higher weight wins (see
    /// [State precedence](State#precedence) for the rule and the table).
    ///
    /// The weight is the selector's *precedence mask*: bit `i` stands for the state of rank
    /// `i` in [`State::PRECEDENCE`], and comparing two masks as integers is exactly "the
    /// highest-ranked differing state wins". The state bits are laid out in precedence order
    /// (a compile-time check next to [`State::PRECEDENCE`]), so the mask is the state bits
    /// themselves: no table, no computation per lookup. [`State::ANY`] weighs 0 like
    /// [`State::DEFAULT`], so an entry whose states equal the node's state always has the
    /// highest achievable weight (the resolver's early exit relies on it).
    ///
    /// ```
    /// use twine_style::{Selector, State};
    /// let w = |s: State| Selector::state(s).weight();
    /// assert!(w(State::DISABLED) > w(State::PRESSED | State::CHECKED));
    /// assert!(w(State::PRESSED | State::CHECKED) > w(State::PRESSED));
    /// assert_eq!(w(State::ANY), w(State::DEFAULT));
    /// ```
    #[inline]
    #[must_use]
    pub fn weight(&self) -> u16 {
        if self.state == State::ANY {
            0
        } else {
            self.state.bits()
        }
    }
}

impl From<Part> for Selector {
    fn from(p: Part) -> Self {
        Selector::part(p)
    }
}

impl From<State> for Selector {
    fn from(s: State) -> Self {
        Selector::state(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_matching_table() {
        let p = State::PRESSED;
        let c = State::CHECKED;
        #[rustfmt::skip]
        let table: &[(Selector, Part, State, bool)] = &[
            // selector                                       query part       node state        matches
            (Selector::MAIN,                                  Part::Main,      State::DEFAULT,   true),
            (Selector::MAIN,                                  Part::Main,      p | c,            true),
            (Selector::MAIN,                                  Part::Knob,      State::DEFAULT,   false),
            (Selector::state(p),                              Part::Main,      State::DEFAULT,   false),
            (Selector::state(p),                              Part::Main,      p,                true),
            (Selector::state(p),                              Part::Main,      p | c,            true),
            (Selector::state(p | c),                          Part::Main,      p,                false),
            (Selector::state(p | c),                          Part::Main,      p | c | State::FOCUSED, true),
            (Selector::part(Part::Knob).with_state(p),        Part::Knob,      p,                true),
            (Selector::part(Part::Knob).with_state(p),        Part::Indicator, p,                false),
            (Selector::part(Part::Any),                       Part::Scrollbar, State::DEFAULT,   true),
            (Selector::part(Part::Any).with_state(c),         Part::Cursor,    State::DEFAULT,   false),
            (Selector::part(Part::Any).with_state(c),         Part::Cursor,    c,                true),
            (Selector::state(State::ANY),                     Part::Main,      State::DEFAULT,   true),
            (Selector::state(State::ANY),                     Part::Main,      State::DISABLED,  true),
            (Selector::part(Part::Items).with_state(State::ANY), Part::Main,   p,                false),
            (Selector::state(State::custom::<0>()),           Part::Main,      State::custom::<0>() | p, true),
            (Selector::state(State::custom::<0>()),           Part::Main,      State::custom::<1>() | p, false),
        ];
        for (i, (sel, part, st, want)) in table.iter().enumerate() {
            assert_eq!(
                sel.matches(*part, *st),
                *want,
                "row {i}: {sel:?} vs {part:?} {st:?}"
            );
        }
    }

    /// The winner among matching selectors, by the documented rule (highest weight, the first
    /// on ties), or `None` when none matches.
    fn winner(node: State, selectors: &[State]) -> Option<State> {
        let mut best: Option<Selector> = None;
        for s in selectors.iter().map(|s| Selector::state(*s)) {
            if s.state_matches(node) && best.is_none_or(|b| s.weight() > b.weight()) {
                best = Some(s);
            }
        }
        best.map(|b| b.state)
    }

    #[test]
    fn precedence_table() {
        const ALARM: State = State::custom::<0>();
        const MUTED: State = State::custom::<3>();
        let (d, p, c, f, k, e, h, s, x) = (
            State::DEFAULT,
            State::PRESSED,
            State::CHECKED,
            State::FOCUSED,
            State::FOCUS_KEY,
            State::EDITED,
            State::HOVERED,
            State::SCROLLED,
            State::DISABLED,
        );
        #[rustfmt::skip]
        let table: &[(State, &[State], State)] = &[
            // node state               matching candidates                 winner
            (p | c,                     &[d, c, p],                         p),
            (p | c,                     &[p, p | c],                        p | c),
            (p | c,                     &[p | c, p],                        p | c),
            (x | p | c,                 &[p | c, x],                        x),
            (x | p | c | f,             &[p | c | f, x],                    x),
            (x | p,                     &[x, x | p],                        x | p),
            (f | k | p,                 &[k, f | p],                        f | p),
            (f | k,                     &[f, k],                            k),
            (f | k | e,                 &[f | k, e],                        e),
            (h | p,                     &[h, p],                            p),
            (p | s,                     &[p, s],                            s),
            (c | State::ALT,            &[State::ALT, c],                   c),
            (x | ALARM,                 &[x, ALARM],                        ALARM),
            (x | ALARM,                 &[ALARM, x | ALARM],                x | ALARM),
            (ALARM | MUTED,             &[MUTED, ALARM],                    MUTED),
            (p,                         &[State::ANY, d],                   State::ANY), // equal weight: first
            (p,                         &[d, State::ANY],                   d),
            (p,                         &[State::ANY, p],                   p),
            (d,                         &[p, c],                            d), // nothing matches
        ];
        for (i, (node, cands, want)) in table.iter().enumerate() {
            let got = winner(*node, cands).unwrap_or(State::DEFAULT);
            assert_eq!(got, *want, "row {i}: node {node:?} among {cands:?}");
        }
    }

    #[test]
    fn precedence_list_orders_weights() {
        for w in State::PRECEDENCE.windows(2) {
            // One state outranks every combination of the lower ones.
            let lower = State::PRECEDENCE
                .iter()
                .take_while(|s| **s != w[1])
                .fold(State::DEFAULT, |a, s| a | *s);
            assert!(Selector::state(w[1]).weight() > Selector::state(lower).weight());
            assert!(Selector::state(w[0]).weight() < Selector::state(w[1]).weight());
        }
        // The weight is the state bits (the layout matches the list; theme outcomes = LVGL).
        for s in [
            State::DEFAULT,
            State::CHECKED,
            State::PRESSED | State::CHECKED,
            State::DISABLED,
        ] {
            assert_eq!(Selector::state(s).weight(), s.bits());
        }
        assert_eq!(
            State::ANY,
            State::PRECEDENCE.iter().fold(State::DEFAULT, |a, s| a | *s)
        );
        assert_eq!(State::ANY.bits(), 0xF3FD);
        assert_eq!(Selector::state(State::ANY).weight(), 0);
        assert_eq!(State::CUSTOM_ALL.bits(), 0xF000);
        assert_eq!(Selector::default(), Selector::MAIN);
        assert_eq!(Selector::from(State::PRESSED), Selector::state(State::PRESSED));
        assert_eq!(Selector::from(Part::Knob), Selector::part(Part::Knob));
    }

    #[test]
    fn custom_parts_fill_lvgls_range() {
        let all = [
            Part::custom::<0>(),
            Part::custom::<1>(),
            Part::custom::<2>(),
            Part::custom::<3>(),
            Part::custom::<4>(),
            Part::custom::<5>(),
            Part::custom::<6>(),
        ];
        assert_eq!(all.len(), Part::CUSTOM_COUNT);
        for (i, p) in all.iter().enumerate() {
            // LVGL `LV_PART_CUSTOM_FIRST` (0x08) up to just below `LV_PART_ANY` (0x0F).
            assert_eq!(*p as usize, 0x08 + i);
            assert_eq!(p.custom_index(), Some(i));
            assert!(Selector::part(*p).matches(*p, State::DEFAULT));
            assert!(Selector::part(Part::Any).matches(*p, State::DEFAULT));
            assert!(!Selector::part(Part::Main).matches(*p, State::DEFAULT));
        }
        for p in [Part::Main, Part::Scrollbar, Part::Cursor, Part::Any] {
            assert_eq!(p.custom_index(), None);
        }
        assert_eq!(alloc::format!("{:?}", Part::custom::<2>()), "Custom2");
    }

    #[test]
    fn custom_states_have_names() {
        use alloc::format;
        // Slot 1 only (other tests of this binary may name other slots).
        const SELECTED: State = State::custom::<1>();
        assert_eq!(format!("{SELECTED:?}"), "State(CUSTOM_1)");
        State::set_custom_name(SELECTED, "SELECTED");
        assert_eq!(
            format!("{:?}", SELECTED | State::CHECKED),
            "State(CHECKED | SELECTED)"
        );
        assert_eq!(format!("{}", SELECTED | State::CHECKED), "CHECKED|SELECTED");
        // Too long: cut at a character boundary (16 bytes max).
        State::set_custom_name(SELECTED, "ÄÄÄÄÄÄÄÄÄ"); // 18 bytes
        assert_eq!(format!("{SELECTED}"), "ÄÄÄÄÄÄÄÄ");
        // Not one custom state: ignored.
        State::set_custom_name(State::PRESSED, "X");
        State::set_custom_name(SELECTED | State::custom::<0>(), "X");
        assert_eq!(format!("{SELECTED}"), "ÄÄÄÄÄÄÄÄ");
        // Empty: back to the default name.
        State::set_custom_name(SELECTED, "");
        assert_eq!(format!("{SELECTED}"), "CUSTOM_1");
        assert_eq!(format!("{:?}", State::DEFAULT), "State(DEFAULT)");
        assert_eq!(format!("{}", State::DEFAULT), "DEFAULT");
        assert_eq!(format!("{}", State::from_bits_retain(0x0402)), "0x0402");
        assert_eq!(
            format!("{:?}", State::PRESSED | State::from_bits_retain(0x0002)),
            "State(PRESSED | 0x0002)"
        );
    }
}
