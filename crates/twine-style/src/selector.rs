//! Selectors: which part of a widget, in which state, a style applies to.

/// A part of a widget (LVGL `lv_part_t`; the values here are LVGL's shifted right by 16).
///
/// A closed enum: a widget that needs one more part than these uses [`Part::CustomFirst`] and
/// documents what it means (LVGL widgets never need more than one custom part).
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
    /// `LV_PART_CUSTOM_FIRST`: the extension part of custom widgets.
    CustomFirst = 0x08,
    /// `LV_PART_ANY`: in a selector, matches every part (a Twine extension for style lookup;
    /// LVGL uses it only to address all parts when removing styles).
    Any = 0x0F,
}

bitflags::bitflags! {
    /// Widget states (LVGL v9.6 `lv_state_t`, same bit values). A widget can be in several at
    /// once; [`State::DEFAULT`] is the empty set.
    ///
    /// Higher bit values have precedence in style resolution (see [`Selector::weight`]), so
    /// `DISABLED` beats `PRESSED`, which beats `CHECKED`, and the user states beat all of them.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
        /// `LV_STATE_USER_1`: custom state.
        const USER_1 = 1 << 12;
        /// `LV_STATE_USER_2`: custom state.
        const USER_2 = 1 << 13;
        /// `LV_STATE_USER_3`: custom state.
        const USER_3 = 1 << 14;
        /// `LV_STATE_USER_4`: custom state.
        const USER_4 = 1 << 15;
        /// `LV_STATE_ANY`: in a selector, applies in every state with weight 0, like `DEFAULT`
        /// (a Twine extension; in LVGL such a style never matches in lookups).
        const ANY = 0xFFFF;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for State {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "State({=u16:#x})", self.bits());
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
/// assert_eq!(sel.weight(), State::PRESSED.bits());
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
    /// matches and every selector state is active (LVGL: `(state_act & ~node_state) == 0`).
    /// [`State::ANY`] matches every state.
    #[inline]
    #[must_use]
    pub fn matches(&self, part: Part, node_state: State) -> bool {
        self.part_matches(part) && (self.state == State::ANY || self.state.bits() & !node_state.bits() == 0)
    }

    /// Precedence among matching entries: the numeric value of the state bits (LVGL compares
    /// `state_act` numerically); higher wins. [`State::ANY`] weighs 0 like
    /// [`State::DEFAULT`], so an entry whose states equal the node's state always has the
    /// highest achievable weight (the resolver's early exit relies on it).
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
            (Selector::state(State::USER_1),                  Part::Main,      State::USER_1 | p, true),
        ];
        for (i, (sel, part, st, want)) in table.iter().enumerate() {
            assert_eq!(
                sel.matches(*part, *st),
                *want,
                "row {i}: {sel:?} vs {part:?} {st:?}"
            );
        }
    }

    #[test]
    fn weight_is_state_bits() {
        for s in [
            State::DEFAULT,
            State::CHECKED,
            State::PRESSED | State::CHECKED,
            State::DISABLED,
            State::USER_4,
            State::USER_4 | State::USER_1,
        ] {
            assert_eq!(Selector::state(s).weight(), s.bits());
        }
        // LVGL v9.6 precedence order of single states.
        let order = [
            State::ALT,
            State::CHECKED,
            State::FOCUSED,
            State::FOCUS_KEY,
            State::EDITED,
            State::HOVERED,
            State::PRESSED,
            State::SCROLLED,
            State::DISABLED,
            State::USER_1,
            State::USER_2,
            State::USER_3,
            State::USER_4,
        ];
        for w in order.windows(2) {
            assert!(Selector::state(w[0]).weight() < Selector::state(w[1]).weight());
        }
        assert!(
            Selector::state(State::DISABLED).weight()
                > Selector::state(State::PRESSED | State::CHECKED).weight()
        );
        assert_eq!(State::ANY.bits(), 0xFFFF);
        assert_eq!(Selector::state(State::ANY).weight(), 0);
        assert_eq!(Selector::default(), Selector::MAIN);
        assert_eq!(Selector::from(State::PRESSED), Selector::state(State::PRESSED));
        assert_eq!(Selector::from(Part::Knob), Selector::part(Part::Knob));
    }
}
