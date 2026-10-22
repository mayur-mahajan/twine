//! [`Keyboard`]: an on-screen keyboard built on the button matrix (LVGL `lv_keyboard`).

use alloc::boxed::Box;

use twine_engine::{
    DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult, GroupDef,
    NodeId, OBJ_FLAGS, ObjFlags, State, Widget, WidgetClass, WidgetCx, fmt_node_id,
};
use twine_style::{Align, BaseDir, Length, Part, Selector, StyleProp};
use twine_text::Symbol;

use crate::buttonmatrix::{BtnCtrl, BtnMap, ButtonMatrix, MapBtn};
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

/// A key of a map: `text` with control bits `ctrl`.
const fn k(text: &'static str, ctrl: BtnCtrl) -> MapBtn {
    MapBtn::new(text, ctrl)
}

/// LVGL `default_kb_map_lc` with `default_kb_ctrl_lc_map`.
pub static MAP_LOWER: [&[MapBtn]; 4] = [
    &[
        k(MODE_SPECIAL, ctl(5)),
        k("q", kb(4)),
        k("w", kb(4)),
        k("e", kb(4)),
        k("r", kb(4)),
        k("t", kb(4)),
        k("y", kb(4)),
        k("u", kb(4)),
        k("i", kb(4)),
        k("o", kb(4)),
        k("p", kb(4)),
        k(Symbol::Backspace.as_str(), chk(7)),
    ],
    &[
        k(MODE_TEXT_UPPER, ctl(6)),
        k("a", kb(3)),
        k("s", kb(3)),
        k("d", kb(3)),
        k("f", kb(3)),
        k("g", kb(3)),
        k("h", kb(3)),
        k("j", kb(3)),
        k("k", kb(3)),
        k("l", kb(3)),
        k(Symbol::NewLine.as_str(), chk(7)),
    ],
    &[
        k("_", chk_kb(1)),
        k("-", chk_kb(1)),
        k("z", kb(1)),
        k("x", kb(1)),
        k("c", kb(1)),
        k("v", kb(1)),
        k("b", kb(1)),
        k("n", kb(1)),
        k("m", kb(1)),
        k(".", chk_kb(1)),
        k(",", chk_kb(1)),
        k(":", chk_kb(1)),
    ],
    &[
        k(Symbol::Keyboard.as_str(), ctl(2)),
        k(Symbol::Left.as_str(), chk(2)),
        k(" ", w(6)),
        k(Symbol::Right.as_str(), chk(2)),
        k(Symbol::Ok.as_str(), ctl(2)),
    ],
];

/// LVGL `default_kb_map_uc` with `default_kb_ctrl_uc_map` (the lower case control bits).
pub static MAP_UPPER: [&[MapBtn]; 4] = [
    &[
        k(MODE_SPECIAL, ctl(5)),
        k("Q", kb(4)),
        k("W", kb(4)),
        k("E", kb(4)),
        k("R", kb(4)),
        k("T", kb(4)),
        k("Y", kb(4)),
        k("U", kb(4)),
        k("I", kb(4)),
        k("O", kb(4)),
        k("P", kb(4)),
        k(Symbol::Backspace.as_str(), chk(7)),
    ],
    &[
        k(MODE_TEXT_LOWER, ctl(6)),
        k("A", kb(3)),
        k("S", kb(3)),
        k("D", kb(3)),
        k("F", kb(3)),
        k("G", kb(3)),
        k("H", kb(3)),
        k("J", kb(3)),
        k("K", kb(3)),
        k("L", kb(3)),
        k(Symbol::NewLine.as_str(), chk(7)),
    ],
    &[
        k("_", chk_kb(1)),
        k("-", chk_kb(1)),
        k("Z", kb(1)),
        k("X", kb(1)),
        k("C", kb(1)),
        k("V", kb(1)),
        k("B", kb(1)),
        k("N", kb(1)),
        k("M", kb(1)),
        k(".", chk_kb(1)),
        k(",", chk_kb(1)),
        k(":", chk_kb(1)),
    ],
    &[
        k(Symbol::Close.as_str(), ctl(2)),
        k(Symbol::Left.as_str(), chk(2)),
        k(" ", w(6)),
        k(Symbol::Right.as_str(), chk(2)),
        k(Symbol::Ok.as_str(), ctl(2)),
    ],
];

/// LVGL `default_kb_map_spec` with `default_kb_ctrl_spec_map`.
pub static MAP_SPECIAL: [&[MapBtn]; 4] = [
    &[
        k("1", kb(1)),
        k("2", kb(1)),
        k("3", kb(1)),
        k("4", kb(1)),
        k("5", kb(1)),
        k("6", kb(1)),
        k("7", kb(1)),
        k("8", kb(1)),
        k("9", kb(1)),
        k("0", kb(1)),
        k(Symbol::Backspace.as_str(), chk(2)),
    ],
    &[
        k(MODE_TEXT_LOWER, ctl(2)),
        k("+", kb(1)),
        k("&", kb(1)),
        k("/", kb(1)),
        k("*", kb(1)),
        k("=", kb(1)),
        k("%", kb(1)),
        k("!", kb(1)),
        k("?", kb(1)),
        k("#", kb(1)),
        k("<", kb(1)),
        k(">", kb(1)),
    ],
    &[
        k("\\", kb(1)),
        k("@", kb(1)),
        k("$", kb(1)),
        k("(", kb(1)),
        k(")", kb(1)),
        k("{", kb(1)),
        k("}", kb(1)),
        k("[", kb(1)),
        k("]", kb(1)),
        k(";", kb(1)),
        k("\"", kb(1)),
        k("'", kb(1)),
    ],
    &[
        k(Symbol::Keyboard.as_str(), ctl(2)),
        k(Symbol::Left.as_str(), chk(2)),
        k(" ", w(6)),
        k(Symbol::Right.as_str(), chk(2)),
        k(Symbol::Ok.as_str(), ctl(2)),
    ],
];

/// LVGL `default_kb_map_num` with `default_kb_ctrl_num_map`.
pub static MAP_NUMBER: [&[MapBtn]; 4] = [
    &[
        k("1", w(1)),
        k("2", w(1)),
        k("3", w(1)),
        k(Symbol::Keyboard.as_str(), ctl(2)),
    ],
    &[
        k("4", w(1)),
        k("5", w(1)),
        k("6", w(1)),
        k(Symbol::Ok.as_str(), ctl(2)),
    ],
    &[
        k("7", w(1)),
        k("8", w(1)),
        k("9", w(1)),
        k(Symbol::Backspace.as_str(), w(2)),
    ],
    &[
        k("+/-", w(1)),
        k("0", w(1)),
        k(".", w(1)),
        k(Symbol::Left.as_str(), w(1)),
        k(Symbol::Right.as_str(), w(1)),
    ],
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

/// The maps of the eight modes, reached by the mode (no index: one map per variant of
/// [`KeyboardMode`], in its order).
#[derive(Clone, Copy, Debug)]
struct KbMaps([KbMap; 8]);

impl KbMaps {
    fn get(&self, mode: KeyboardMode) -> KbMap {
        use KeyboardMode as M;
        let [lower, upper, special, number, u1, u2, u3, u4] = self.0;
        match mode {
            M::TextLower => lower,
            M::TextUpper => upper,
            M::Special => special,
            M::Number => number,
            M::User1 => u1,
            M::User2 => u2,
            M::User3 => u3,
            M::User4 => u4,
        }
    }

    fn get_mut(&mut self, mode: KeyboardMode) -> &mut KbMap {
        use KeyboardMode as M;
        let [lower, upper, special, number, u1, u2, u3, u4] = &mut self.0;
        match mode {
            M::TextLower => lower,
            M::TextUpper => upper,
            M::Special => special,
            M::Number => number,
            M::User1 => u1,
            M::User2 => u2,
            M::User3 => u3,
            M::User4 => u4,
        }
    }
}

/// A key map: rows of keys with their control bits.
type KbMap = &'static [&'static [MapBtn]];

/// LVGL's `kb_map` / `kb_ctrl` tables (the user modes start with the lower case map).
const DEFAULT_MAPS: [KbMap; 8] = [
    &MAP_LOWER,
    &MAP_UPPER,
    &MAP_SPECIAL,
    &MAP_NUMBER,
    &MAP_LOWER,
    &MAP_LOWER,
    &MAP_LOWER,
    &MAP_LOWER,
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
///   (LVGL), drawn over the parent's other children. Keypad and encoder navigate like a
///   button matrix.
/// - [Docked](Self::set_docked), it reserves its height instead: the parent's other
///   children are laid out in the area above it, and the attached textarea is scrolled into
///   view whenever the keyboard appears or changes size (a rotation) and whenever another
///   textarea is attached — keyboard avoidance without application code.
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
    maps: KbMaps,
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
            btnm: ButtonMatrix::new(BtnMap::Static(&MAP_LOWER)),
            ta: None,
            mode: KeyboardMode::TextLower,
            popovers: false,
            maps: KbMaps(DEFAULT_MAPS),
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
        let node = cx.node();
        log_set(KEYBOARD_CLASS.name, node, "textarea");
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
                if e.has_flag(node, ObjFlags::DOCK_BOTTOM) {
                    e.scroll_to_view_after_layout(new, false);
                }
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

    /// Docks the keyboard to the bottom of its parent (`on`), or lets it float over the
    /// parent's other children as in LVGL (`off`, the default). Idempotent; never panics.
    ///
    /// Docked, the keyboard sets [`ObjFlags::DOCK_BOTTOM`]: it gets the full width of the
    /// parent's content area and its own height at the bottom, and the parent's other
    /// children are laid out in the area above it (a `Pct(100)` page shrinks, so everything
    /// stays reachable by scrolling). The attached textarea is then scrolled into view — through
    /// every scrollable ancestor, without animation — after the layout pass that places the
    /// keyboard ([`Engine::scroll_to_view_after_layout`]): when the keyboard appears, when its
    /// size changes (a rotation of the display) and when another textarea is attached. Moving
    /// the keyboard to another textarea causes no layout. Removing (or hiding) the keyboard
    /// gives the parent's area back in the next layout pass.
    ///
    /// Place a docked keyboard in the node whose area it should share, typically the page's
    /// root container (or the screen), whose height does not depend on its content; the
    /// page's scrolling content then fills the rest.
    ///
    /// ```
    /// use twine_engine::ObjFlags;
    /// use twine_style::Length;
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::keyboard::{self, Keyboard};
    /// use twine_widgets::textarea;
    ///
    /// let mut h = EngineHarness::new(320, 240);
    /// let screen = h.screen();
    /// let page = h.engine_mut().create(screen, Box::new(twine_engine::Obj)).unwrap();
    /// h.engine_mut().set_size(page, Length::pct(100), Length::pct(100));
    /// let ta = textarea::create(h.engine_mut(), page).unwrap();
    /// let kb = keyboard::create(h.engine_mut(), screen).unwrap();
    /// h.engine_mut().with_widget_mut(kb, |k: &mut Keyboard, cx| {
    ///     k.set_docked(cx, true);
    ///     k.set_textarea(cx, Some(ta));
    /// });
    /// h.update();
    /// let e = h.engine();
    /// assert!(e.has_flag(kb, ObjFlags::DOCK_BOTTOM));
    /// // Half the screen at the bottom; the page takes the half above it.
    /// assert_eq!((e.coords(kb).y0, e.coords(kb).y1), (120, 240));
    /// assert_eq!(e.coords(page).y1, 120);
    /// ```
    pub fn set_docked(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        let node = cx.node();
        let e = cx.engine_mut();
        if e.has_flag(node, ObjFlags::DOCK_BOTTOM) == on {
            return;
        }
        log_set(KEYBOARD_CLASS.name, node, "docked");
        e.set_flag(node, ObjFlags::DOCK_BOTTOM, on);
        if on {
            self.reveal_textarea(e, node);
        }
    }

    /// Scrolls the attached textarea into view after the layout pass, if the keyboard is
    /// docked.
    fn reveal_textarea(&self, e: &mut Engine, node: NodeId) {
        if let Some(ta) = self.ta.filter(|t| e.tree().contains(*t)) {
            if e.has_flag(node, ObjFlags::DOCK_BOTTOM) {
                e.scroll_to_view_after_layout(ta, false);
            }
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

    /// Sets the map of `mode` for this keyboard: rows of keys, each with its text and
    /// control bits (LVGL `lv_keyboard_set_map`, which changes a global table and takes the
    /// texts and the control bits as two parallel arrays; here per keyboard, one table).
    /// Idempotent (the same table changes nothing). Never panics.
    ///
    /// ```
    /// use twine_testing::EngineHarness;
    /// use twine_widgets::buttonmatrix::{BtnCtrl, MapBtn};
    /// use twine_widgets::keyboard::{self, Keyboard, KeyboardMode};
    ///
    /// static HEX: [&[MapBtn]; 2] = [
    ///     &[MapBtn::text("A"), MapBtn::text("B"), MapBtn::text("C")],
    ///     &[MapBtn::new("abc", keyboard::KEYBOARD_CTRL_BUTTON_FLAGS.union(BtnCtrl::width(3)))],
    /// ];
    /// let mut h = EngineHarness::new(320, 240);
    /// let screen = h.screen();
    /// let kb = keyboard::create(h.engine_mut(), screen).unwrap();
    /// h.engine_mut().with_widget_mut(kb, |k: &mut Keyboard, cx| {
    ///     k.set_map(cx, KeyboardMode::User1, &HEX);
    ///     k.set_mode(cx, KeyboardMode::User1);
    /// });
    /// let m = h.engine().widget::<Keyboard>(kb).unwrap().buttonmatrix();
    /// assert_eq!((m.btn_count(), m.btn_text(3)), (4, Some("abc")));
    /// ```
    pub fn set_map(&mut self, cx: &mut WidgetCx<'_>, mode: KeyboardMode, map: &'static [&'static [MapBtn]]) {
        if core::ptr::eq(self.maps.get(mode), map) {
            return;
        }
        log_set(KEYBOARD_CLASS.name, cx.node(), "map");
        *self.maps.get_mut(mode) = map;
        if self.mode == mode {
            self.update_map(cx);
        }
    }

    /// LVGL `lv_keyboard_update_map`.
    fn update_map(&mut self, cx: &mut WidgetCx<'_>) {
        let map = self.maps.get(self.mode);
        self.btnm.set_map(cx, BtnMap::Static(map));
        self.update_ctrl_map(cx);
    }

    /// LVGL `lv_keyboard_update_ctrl_map`: the mode's control bits, without `POPOVER` when
    /// popovers are off.
    fn update_ctrl_map(&mut self, cx: &mut WidgetCx<'_>) {
        let map = self.maps.get(self.mode);
        let mask = if self.popovers {
            BtnCtrl::all()
        } else {
            !BtnCtrl::POPOVER
        };
        self.btnm.set_ctrl_map_with(cx, |i| {
            nth_key(map, i).map_or(BtnCtrl::empty(), |b| b.ctrl) & mask
        });
    }

    /// The text of key `idx` in the current (static) map.
    fn key_text(&self, idx: u16) -> Option<&'static str> {
        nth_key(self.maps.get(self.mode), usize::from(idx)).map(|b| b.text)
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
        let sym = Symbol::from_text(txt);
        if matches!(sym, Some(Symbol::Close | Symbol::Keyboard)) {
            if cx.send(node, EventCode::Cancel, EventParam::None) == EventResult::Consumed {
                return;
            }
            if let Some(ta) = ta {
                cx.send(ta, EventCode::Cancel, EventParam::None);
            }
            return;
        }
        if sym == Some(Symbol::Ok) {
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
        let typed = match (sym, txt) {
            (Some(Symbol::NewLine), _) | (None, "Enter") => with_textarea(e, ta, |t, tcx| {
                t.add_char(tcx, '\n');
                t.one_line()
            })
            .map(|one_line| {
                if one_line {
                    e.send_event(ta, EventCode::Ready, EventParam::None);
                }
            }),
            (Some(Symbol::Left), _) => with_textarea(e, ta, super::textarea::Textarea::cursor_left),
            (Some(Symbol::Right), _) => with_textarea(e, ta, super::textarea::Textarea::cursor_right),
            (Some(Symbol::Backspace), _) => with_textarea(e, ta, super::textarea::Textarea::delete_char),
            (None, "+/-") => {
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
                        t.set_cursor_pos(tcx, cur.saturating_add(1));
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

/// Key `i` of `map`, in reading order.
fn nth_key(map: KbMap, mut i: usize) -> Option<&'static MapBtn> {
    for row in map {
        if let Some(b) = row.get(i) {
            return Some(b);
        }
        // `i ≥ row.len()` (no key `i` in this row): exact.
        i = i.saturating_sub(row.len());
    }
    None
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
        if ev.target == cx.node() {
            match ev.code {
                EventCode::Delete => self.release_textarea(cx.engine_mut()),
                // Placed or resized (shown, rotated): keep the textarea visible above it.
                EventCode::SizeChanged => self.reveal_textarea(cx.engine_mut(), ev.target),
                _ => {}
            }
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

    fn counts(map: &[&[MapBtn]]) -> (usize, usize) {
        (map.len(), map.iter().map(|r| r.len()).sum())
    }

    #[test]
    fn default_maps_match_lvgl_tables() {
        // Rows and keys of LVGL's `default_kb_map_*` / `default_kb_ctrl_*_map` tables.
        assert_eq!(counts(&MAP_LOWER), (4, 40));
        assert_eq!(counts(&MAP_UPPER), (4, 40));
        assert_eq!(counts(&MAP_SPECIAL), (4, 40));
        assert_eq!(counts(&MAP_NUMBER), (4, 17));
        assert_eq!(MAP_LOWER[0][0].ctrl.width_units(), 5);
        assert!(MAP_LOWER[0][1].ctrl.contains(BtnCtrl::POPOVER));
        assert!(MAP_LOWER[0][0].ctrl.contains(KEYBOARD_CTRL_BUTTON_FLAGS));
        assert_eq!(MAP_NUMBER[2][3].ctrl.width_units(), 2);
        assert_eq!(MAP_NUMBER[3][0].text, "+/-");
    }
}
