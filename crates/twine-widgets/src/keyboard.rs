//! [`Keyboard`]: an on-screen keyboard built on the button matrix (LVGL `lv_keyboard`).

use alloc::boxed::Box;

use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, GroupDef,
    NodeId, OBJ_FLAGS, ObjFlags, State, Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_style::{Align, BaseDir, Length, Part, Selector, StyleProp};
use twine_text::symbols;

use crate::buttonmatrix::{BtnCtrl, ButtonMatrix, MapSrc};
use crate::log_set;
use crate::textarea::{self, with_textarea};

/// The mode key switching to lower case (LVGL `LV_KEYBOARD_CTRL_BUTTON_MODE_TEXT_LOWER`).
pub const MODE_TEXT_LOWER: &str = "abc";
/// The mode key switching to upper case (LVGL `LV_KEYBOARD_CTRL_BUTTON_MODE_TEXT_UPPER`).
pub const MODE_TEXT_UPPER: &str = "ABC";
/// The mode key switching to the special characters (LVGL
/// `LV_KEYBOARD_CTRL_BUTTON_MODE_SPECIAL`).
pub const MODE_SPECIAL: &str = "1#";

/// The control bits of the keyboard's control keys (LVGL `LV_KEYBOARD_CTRL_BUTTON_FLAGS`:
/// no repeat, triggered on release, checked — the theme draws checked keys darker).
pub const KEYBOARD_CTRL_BUTTON_FLAGS: BtnCtrl = BtnCtrl::from_bits_retain(
    BtnCtrl::NO_REPEAT.bits() | BtnCtrl::CLICK_TRIG.bits() | BtnCtrl::CHECKED.bits(),
);

/// A control key of width `w` (`LV_KEYBOARD_CTRL_BUTTON_FLAGS | w`).
const fn ctl(w: u16) -> BtnCtrl {
    BtnCtrl::from_bits_retain(KEYBOARD_CTRL_BUTTON_FLAGS.bits() | w)
}

/// A character key with a popover (`LV_KB_BTN(w)` = `LV_BUTTONMATRIX_CTRL_POPOVER | w`).
const fn kb(w: u16) -> BtnCtrl {
    BtnCtrl::from_bits_retain(BtnCtrl::POPOVER.bits() | w)
}

/// A checked key (`LV_BUTTONMATRIX_CTRL_CHECKED | w`).
const fn chk(w: u16) -> BtnCtrl {
    BtnCtrl::from_bits_retain(BtnCtrl::CHECKED.bits() | w)
}

/// A checked character key with a popover.
const fn chk_kb(w: u16) -> BtnCtrl {
    BtnCtrl::from_bits_retain(BtnCtrl::CHECKED.bits() | BtnCtrl::POPOVER.bits() | w)
}

/// A plain key of width `w`.
const fn w(w: u16) -> BtnCtrl {
    BtnCtrl::from_bits_retain(w)
}

/// LVGL `default_kb_map_lc`.
pub static MAP_LOWER: [&str; 43] = [
    MODE_SPECIAL,
    "q",
    "w",
    "e",
    "r",
    "t",
    "y",
    "u",
    "i",
    "o",
    "p",
    symbols::BACKSPACE,
    "\n",
    MODE_TEXT_UPPER,
    "a",
    "s",
    "d",
    "f",
    "g",
    "h",
    "j",
    "k",
    "l",
    symbols::NEW_LINE,
    "\n",
    "_",
    "-",
    "z",
    "x",
    "c",
    "v",
    "b",
    "n",
    "m",
    ".",
    ",",
    ":",
    "\n",
    symbols::KEYBOARD,
    symbols::LEFT,
    " ",
    symbols::RIGHT,
    symbols::OK,
];

/// LVGL `default_kb_ctrl_lc_map` (`default_kb_ctrl_uc_map` is identical).
const CTRL_LC: [BtnCtrl; 40] = [
    ctl(5),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    kb(4),
    chk(7),
    ctl(6),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    kb(3),
    chk(7),
    chk_kb(1),
    chk_kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    chk_kb(1),
    chk_kb(1),
    chk_kb(1),
    ctl(2),
    chk(2),
    w(6),
    chk(2),
    ctl(2),
];

/// LVGL `default_kb_ctrl_lc_map`.
pub static CTRL_LOWER: [BtnCtrl; 40] = CTRL_LC;

/// LVGL `default_kb_map_uc`.
pub static MAP_UPPER: [&str; 43] = [
    MODE_SPECIAL,
    "Q",
    "W",
    "E",
    "R",
    "T",
    "Y",
    "U",
    "I",
    "O",
    "P",
    symbols::BACKSPACE,
    "\n",
    MODE_TEXT_LOWER,
    "A",
    "S",
    "D",
    "F",
    "G",
    "H",
    "J",
    "K",
    "L",
    symbols::NEW_LINE,
    "\n",
    "_",
    "-",
    "Z",
    "X",
    "C",
    "V",
    "B",
    "N",
    "M",
    ".",
    ",",
    ":",
    "\n",
    symbols::CLOSE,
    symbols::LEFT,
    " ",
    symbols::RIGHT,
    symbols::OK,
];

/// LVGL `default_kb_ctrl_uc_map` (the same as the lower case one).
pub static CTRL_UPPER: [BtnCtrl; 40] = CTRL_LC;

/// LVGL `default_kb_map_spec`.
pub static MAP_SPECIAL: [&str; 43] = [
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "0",
    symbols::BACKSPACE,
    "\n",
    MODE_TEXT_LOWER,
    "+",
    "&",
    "/",
    "*",
    "=",
    "%",
    "!",
    "?",
    "#",
    "<",
    ">",
    "\n",
    "\\",
    "@",
    "$",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    "\"",
    "'",
    "\n",
    symbols::KEYBOARD,
    symbols::LEFT,
    " ",
    symbols::RIGHT,
    symbols::OK,
];

/// LVGL `default_kb_ctrl_spec_map`.
pub static CTRL_SPECIAL: [BtnCtrl; 40] = [
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    chk(2),
    ctl(2),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    kb(1),
    ctl(2),
    chk(2),
    w(6),
    chk(2),
    ctl(2),
];

/// LVGL `default_kb_map_num`.
pub static MAP_NUMBER: [&str; 20] = [
    "1",
    "2",
    "3",
    symbols::KEYBOARD,
    "\n",
    "4",
    "5",
    "6",
    symbols::OK,
    "\n",
    "7",
    "8",
    "9",
    symbols::BACKSPACE,
    "\n",
    "+/-",
    "0",
    ".",
    symbols::LEFT,
    symbols::RIGHT,
];

/// LVGL `default_kb_ctrl_num_map`.
pub static CTRL_NUMBER: [BtnCtrl; 17] = [
    w(1),
    w(1),
    w(1),
    ctl(2),
    w(1),
    w(1),
    w(1),
    ctl(2),
    w(1),
    w(1),
    w(1),
    w(2),
    w(1),
    w(1),
    w(1),
    w(1),
    w(1),
];

/// The mode of a [`Keyboard`] (LVGL `lv_keyboard_mode_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardMode {
    /// Lower case letters.
    #[default]
    TextLower,
    /// Upper case letters.
    TextUpper,
    /// Digits and special characters.
    Special,
    /// A number pad.
    Number,
    /// A user map (lower case until [`Keyboard::set_map`] sets one).
    User1,
    /// A user map.
    User2,
    /// A user map.
    User3,
    /// A user map.
    User4,
}

impl KeyboardMode {
    const ALL: [KeyboardMode; 8] = [
        KeyboardMode::TextLower,
        KeyboardMode::TextUpper,
        KeyboardMode::Special,
        KeyboardMode::Number,
        KeyboardMode::User1,
        KeyboardMode::User2,
        KeyboardMode::User3,
        KeyboardMode::User4,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }
}

/// A key map with its control bits.
type KbMap = (&'static [&'static str], &'static [BtnCtrl]);

/// LVGL's `kb_map` / `kb_ctrl` tables (the user modes start with the lower case map).
const DEFAULT_MAPS: [KbMap; 8] = [
    (&MAP_LOWER, &CTRL_LOWER),
    (&MAP_UPPER, &CTRL_UPPER),
    (&MAP_SPECIAL, &CTRL_SPECIAL),
    (&MAP_NUMBER, &CTRL_NUMBER),
    (&MAP_LOWER, &CTRL_LOWER),
    (&MAP_LOWER, &CTRL_LOWER),
    (&MAP_LOWER, &CTRL_LOWER),
    (&MAP_LOWER, &CTRL_LOWER),
];

/// Default flags of [`KEYBOARD_CLASS`] (LVGL `lv_keyboard_constructor`:
/// `lv_obj_set_click_focusable(obj, false)`).
const KEYBOARD_FLAGS: ObjFlags = OBJ_FLAGS.difference(ObjFlags::CLICK_FOCUSABLE);

/// The class of [`Keyboard`]: `"keyboard"`, parts `Main` and `Items`, editable, in the
/// default focus group (LVGL `lv_keyboard_class`, a button matrix subclass).
pub static KEYBOARD_CLASS: WidgetClass = WidgetClass::new("keyboard")
    .parts(&[Part::Main, Part::Items])
    .item_parts(&[Part::Items])
    .default_flags(KEYBOARD_FLAGS)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// An on-screen keyboard (LVGL `lv_keyboard`): a [`ButtonMatrix`] with LVGL's four layouts
/// (lower case, upper case, special characters, number pad) and four user modes, typing
/// into an attached [`Textarea`](crate::textarea::Textarea).
///
/// - The mode keys ("abc", "ABC", "1#") switch the layout; `BACKSPACE` deletes, `LEFT` and
///   `RIGHT` move the cursor, `NEW_LINE` adds a line break (or sends `Ready` to a one-line
///   textarea), `OK` sends `Ready` and `CLOSE` / `KEYBOARD` send `Cancel` to the keyboard
///   and the textarea, "+/-" toggles the sign at the start, any other key is typed.
/// - Attaching a textarea gives it the `FOCUSED` state, so its cursor shows and blinks.
/// - With [popovers](Self::set_popovers), pressed character keys show an enlarged copy on
///   the top layer.
/// - The default size is the parent's full width and half its height, at the bottom
///   (LVGL). Keypad and encoder navigate like a button matrix.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::keyboard::{self, Keyboard, KeyboardMode};
/// use twine_widgets::textarea::{self, Textarea};
///
/// let mut h = EngineHarness::new(320, 240);
/// let screen = h.screen();
/// let ta = textarea::create(h.engine_mut(), screen).unwrap();
/// let kb = keyboard::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(kb, |k: &mut Keyboard, cx| {
///     k.set_textarea(cx, Some(ta));
///     k.set_mode(cx, KeyboardMode::Number);
/// });
/// assert_eq!(h.engine().widget::<Keyboard>(kb).unwrap().textarea(), Some(ta));
/// ```
#[derive(Debug)]
pub struct Keyboard {
    btnm: ButtonMatrix,
    ta: Option<NodeId>,
    mode: KeyboardMode,
    popovers: bool,
    maps: [KbMap; 8],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    /// A lower case keyboard without popovers and without a textarea.
    #[must_use]
    pub fn new() -> Self {
        Self {
            btnm: ButtonMatrix::new(MapSrc::Static(&MAP_LOWER)),
            ta: None,
            mode: KeyboardMode::TextLower,
            popovers: false,
            maps: DEFAULT_MAPS,
        }
    }

    /// The button matrix (map, control bits, button areas).
    #[must_use]
    pub fn buttonmatrix(&self) -> &ButtonMatrix {
        &self.btnm
    }

    /// The attached textarea.
    #[must_use]
    pub fn textarea(&self) -> Option<NodeId> {
        self.ta
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> KeyboardMode {
        self.mode
    }

    /// Whether character keys show popovers.
    #[must_use]
    pub fn popovers(&self) -> bool {
        self.popovers
    }

    /// Attaches a textarea (or a spinbox) to type into (LVGL `lv_keyboard_set_textarea`):
    /// the old one loses the `FOCUSED` state, the new one gets it (its cursor shows).
    /// Idempotent.
    pub fn set_textarea(&mut self, cx: &mut WidgetCx<'_>, ta: Option<NodeId>) {
        if self.ta == ta {
            return;
        }
        log_set(KEYBOARD_CLASS.name, cx.node(), "textarea");
        let e = cx.engine_mut();
        if let Some(old) = self.ta.filter(|t| e.tree().contains(*t)) {
            // `StateChanged` stops the old cursor's blink.
            e.clear_state(old, State::FOCUSED);
        }
        self.ta = ta;
        if let Some(new) = ta {
            if e.tree().contains(new) {
                // `StateChanged` starts the new cursor's blink.
                e.add_state(new, State::FOCUSED);
            } else {
                twine_core::warn!(target: "twine::engine", "keyboard: textarea {} not found", fmt_node_id(new));
                self.ta = None;
            }
        }
    }

    /// A deleted keyboard gives up its textarea: the textarea loses the `FOCUSED` state the
    /// keyboard gave it (so its cursor stops blinking), unless it holds its focus group's
    /// focus (typing with a keypad continues there).
    fn release_textarea(&mut self, e: &mut Engine) {
        let Some(ta) = self.ta.take().filter(|t| e.tree().contains(*t)) else {
            return;
        };
        let group_focus = e.group_of(ta).is_some_and(|g| e.focused(g) == Some(ta));
        if !group_focus {
            e.clear_state(ta, State::FOCUSED);
        }
    }

    /// Switches the layout (LVGL `lv_keyboard_set_mode`). Idempotent.
    pub fn set_mode(&mut self, cx: &mut WidgetCx<'_>, mode: KeyboardMode) {
        if self.mode == mode {
            return;
        }
        log_set(KEYBOARD_CLASS.name, cx.node(), "mode");
        self.mode = mode;
        self.update_map(cx);
    }

    /// Shows popovers on character keys (LVGL `lv_keyboard_set_popovers`). Idempotent.
    pub fn set_popovers(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.popovers == on {
            return;
        }
        log_set(KEYBOARD_CLASS.name, cx.node(), "popovers");
        self.popovers = on;
        self.update_ctrl_map(cx);
    }

    /// Sets the map and control bits of `mode` for this keyboard (LVGL
    /// `lv_keyboard_set_map`, which changes a global table; here per keyboard). `ctrl` has
    /// one entry per button.
    pub fn set_map(
        &mut self,
        cx: &mut WidgetCx<'_>,
        mode: KeyboardMode,
        map: &'static [&'static str],
        ctrl: &'static [BtnCtrl],
    ) {
        let i = mode.index();
        if core::ptr::eq(self.maps[i].0, map) && core::ptr::eq(self.maps[i].1, ctrl) {
            return;
        }
        log_set(KEYBOARD_CLASS.name, cx.node(), "map");
        self.maps[i] = (map, ctrl);
        if self.mode == mode {
            self.update_map(cx);
        }
    }

    /// LVGL `lv_keyboard_update_map`.
    fn update_map(&mut self, cx: &mut WidgetCx<'_>) {
        let (map, _) = self.maps[self.mode.index()];
        self.btnm.set_map(cx, MapSrc::Static(map));
        self.update_ctrl_map(cx);
    }

    /// LVGL `lv_keyboard_update_ctrl_map`: the mode's control bits, without `POPOVER` when
    /// popovers are off.
    fn update_ctrl_map(&mut self, cx: &mut WidgetCx<'_>) {
        let (_, ctrl) = self.maps[self.mode.index()];
        let mask = if self.popovers {
            BtnCtrl::all()
        } else {
            !BtnCtrl::POPOVER
        };
        self.btnm
            .set_ctrl_map_with(cx, |i| ctrl.get(i).copied().unwrap_or_default() & mask);
    }

    /// The text of button `idx` in the current (static) map.
    fn key_text(&self, idx: u16) -> Option<&'static str> {
        let (map, _) = self.maps[self.mode.index()];
        map.iter()
            .copied()
            .take_while(|s| !s.is_empty())
            .filter(|s| *s != "\n")
            .nth(usize::from(idx))
    }

    /// The attached textarea if it still exists; a dead one is warned about and cleared.
    fn live_textarea(&mut self, e: &Engine) -> Option<NodeId> {
        let ta = self.ta?;
        if e.tree().contains(ta) {
            return Some(ta);
        }
        twine_core::warn!(target: "twine::engine", "keyboard: textarea {} was deleted", fmt_node_id(ta));
        self.ta = None;
        None
    }

    /// LVGL `lv_keyboard_def_event_cb`: acts on the key `idx`.
    fn def_event(&mut self, cx: &mut EventCx<'_>, idx: u16) {
        let Some(txt) = self.key_text(idx) else {
            return;
        };
        let node = cx.node();
        let mode = match txt {
            MODE_TEXT_LOWER => Some(KeyboardMode::TextLower),
            MODE_TEXT_UPPER => Some(KeyboardMode::TextUpper),
            MODE_SPECIAL => Some(KeyboardMode::Special),
            _ => None,
        };
        if let Some(m) = mode {
            self.mode = m;
            self.update_map(&mut cx.widget_cx());
            return;
        }
        let ta = self.live_textarea(cx.engine());
        if txt == symbols::CLOSE || txt == symbols::KEYBOARD {
            if cx.send(node, EventCode::Cancel, EventParam::None) == EventResult::Consumed {
                return;
            }
            if let Some(ta) = ta {
                cx.send(ta, EventCode::Cancel, EventParam::None);
            }
            return;
        }
        if txt == symbols::OK {
            if cx.send(node, EventCode::Ready, EventParam::None) == EventResult::Consumed {
                return;
            }
            if let Some(ta) = ta {
                cx.send(ta, EventCode::Ready, EventParam::None);
            }
            return;
        }
        let Some(ta) = ta else {
            return;
        };
        let e = cx.engine_mut();
        let typed = match txt {
            "Enter" | symbols::NEW_LINE => with_textarea(e, ta, |t, tcx| {
                t.add_char(tcx, '\n');
                t.one_line()
            })
            .map(|one_line| {
                if one_line {
                    e.send_event(ta, EventCode::Ready, EventParam::None);
                }
            }),
            symbols::LEFT => with_textarea(e, ta, super::textarea::Textarea::cursor_left),
            symbols::RIGHT => with_textarea(e, ta, super::textarea::Textarea::cursor_right),
            symbols::BACKSPACE => with_textarea(e, ta, super::textarea::Textarea::delete_char),
            "+/-" => {
                let first = textarea::text_of(e, ta).and_then(|s| s.chars().next());
                with_textarea(e, ta, |t, tcx| {
                    let cur = i32::try_from(t.cursor_pos()).unwrap_or(0);
                    if let Some('-' | '+') = first {
                        let sign = if first == Some('-') { '+' } else { '-' };
                        t.set_cursor_pos(tcx, 1);
                        t.delete_char(tcx);
                        t.add_char(tcx, sign);
                        t.set_cursor_pos(tcx, cur);
                    } else {
                        t.set_cursor_pos(tcx, 0);
                        t.add_char(tcx, '-');
                        t.set_cursor_pos(tcx, cur + 1);
                    }
                })
            }
            _ => with_textarea(e, ta, |t, tcx| t.add_text(tcx, txt)),
        };
        if typed.is_none() {
            twine_core::warn!(target: "twine::engine", "keyboard: {} is not a textarea", fmt_node_id(ta));
            self.ta = None;
        }
    }
}

/// Creates a keyboard (lower case, full width, half the parent's height, at the bottom) as
/// the last child of `parent` (LVGL `lv_keyboard_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Keyboard::new()))
}

impl Widget for Keyboard {
    fn class(&self) -> &'static WidgetClass {
        &KEYBOARD_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        let e = cx.engine_mut();
        e.set_size(id, Length::Pct(100), Length::Pct(50));
        e.align(id, Align::BottomMid, 0, 0);
        e.set_local_prop(id, Selector::MAIN, StyleProp::BaseDir(BaseDir::Ltr));
        self.update_map(cx);
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        Widget::draw(&self.btnm, cx);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.code == EventCode::Delete && ev.target == cx.node() {
            self.release_textarea(cx.engine_mut());
        }
        if let Some(b) = self.btnm.handle(cx, ev) {
            // LVGL: the keyboard's own handler runs before the application's.
            self.def_event(cx, b);
            let node = cx.node();
            if cx.engine().tree().contains(node) {
                cx.post(node, EventCode::ValueChanged, EventParam::Value(i32::from(b)));
            }
        }
        EventResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(map: &[&str]) -> (usize, usize) {
        let rows = map.iter().filter(|s| **s == "\n").count() + 1;
        (rows, map.len() + 1 - rows)
    }

    #[test]
    fn default_maps_match_lvgl_tables() {
        // Rows and keys of LVGL's `default_kb_map_*` tables.
        assert_eq!(counts(&MAP_LOWER), (4, 40));
        assert_eq!(counts(&MAP_UPPER), (4, 40));
        assert_eq!(counts(&MAP_SPECIAL), (4, 40));
        assert_eq!(counts(&MAP_NUMBER), (4, 17));
        assert_eq!(CTRL_LOWER.len(), 40);
        assert_eq!(CTRL_SPECIAL.len(), 40);
        assert_eq!(CTRL_NUMBER.len(), 17);
        assert_eq!(CTRL_LOWER[0].width_units(), 5);
        assert!(CTRL_LOWER[1].contains(BtnCtrl::POPOVER));
        assert!(CTRL_LOWER[0].contains(KEYBOARD_CTRL_BUTTON_FLAGS));
    }
}
