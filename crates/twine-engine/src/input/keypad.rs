//! Keypad processing (LVGL `indev_keypad_proc`).

use twine_core::Instant;
use twine_hal::{Key, KeypadData};
use twine_style::State;

use super::{ClickCounter, InputId, Timing, stopped};
use crate::{Engine, EventCode, EventParam, GroupId, NodeId};

/// The state of one keypad.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeypadProc {
    last_key: Option<Key>,
    /// Whether the last key event was a press.
    pub(crate) pressed: bool,
    press_time: Instant,
    long_pr_sent: bool,
    longpr_rep_time: Instant,
    clicks: ClickCounter,
    pub(crate) wait_until_release: bool,
}

impl KeypadProc {
    pub(crate) fn new() -> Self {
        Self {
            last_key: None,
            pressed: false,
            press_time: Instant::ZERO,
            long_pr_sent: false,
            longpr_rep_time: Instant::ZERO,
            clicks: ClickCounter::default(),
            wait_until_release: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.long_pr_sent = false;
    }

    pub(crate) fn deadline(&self, t: &Timing) -> Option<Instant> {
        if !self.pressed || self.wait_until_release {
            return None;
        }
        Some(if self.long_pr_sent {
            self.longpr_rep_time + t.long_press_repeat
        } else {
            self.press_time + t.long_press
        })
    }

    /// One key event. Without a group (or a focused node) keys go nowhere.
    ///
    /// - Press: `Next`/`Prev` move the focus (leaving edit mode); `Enter` sends `Key(Enter)`
    ///   then `Pressed`; `Esc` sends `Key(Esc)` then `Cancel`; other keys send `Key`.
    /// - Held: `Enter` sends `Pressing`, then `LongPressed` after the long press time and
    ///   `LongPressedRepeat` every repeat period; other keys repeat their action.
    /// - `Enter` release: `Released`, `ShortClicked` + multi-click (unless long pressed),
    ///   `Clicked`.
    pub(crate) fn process(&mut self, e: &mut Engine, id: InputId, d: KeypadData, now: Instant, t: &Timing) {
        if d.pressed && self.wait_until_release {
            return;
        }
        if self.wait_until_release {
            self.wait_until_release = false;
            self.long_pr_sent = false;
            self.pressed = false; // skip the release processing
        }
        let prev_key = self.last_key;
        self.last_key = Some(d.key);
        let prev_pressed = self.pressed;
        self.pressed = d.pressed;

        let Some(g) = e.input_group(id) else {
            return;
        };
        let Some(f) = e.focused(g) else {
            return;
        };
        let en = e.tree.node(f).is_some_and(|n| !n.state.contains(State::DISABLED));
        match (prev_pressed, d.pressed) {
            (false, true) => {
                self.press_time = now;
                self.long_pr_sent = false;
                match d.key {
                    Key::Next => {
                        e.set_editing(g, false);
                        e.focus_next(g);
                    }
                    Key::Prev => {
                        e.set_editing(g, false);
                        e.focus_prev(g);
                    }
                    _ if !en => {}
                    Key::Enter => {
                        if send_key(e, id, g, Key::Enter) && !stopped(e, id, f) {
                            e.input_send(id, f, EventCode::Pressed, EventParam::None);
                        }
                    }
                    Key::Esc => {
                        if send_key(e, id, g, Key::Esc) && !stopped(e, id, f) {
                            e.input_send(id, f, EventCode::Cancel, EventParam::None);
                        }
                    }
                    k => {
                        send_key(e, id, g, k);
                    }
                }
            }
            (true, true) if en => {
                let enter = d.key == Key::Enter;
                if enter && !e.input_send(id, f, EventCode::Pressing, EventParam::None) {
                    return;
                }
                if !self.long_pr_sent {
                    if now.saturating_duration_since(self.press_time) >= t.long_press {
                        self.long_pr_sent = true;
                        self.longpr_rep_time = now;
                        if enter {
                            e.input_send(id, f, EventCode::LongPressed, EventParam::None);
                        }
                    }
                } else if now.saturating_duration_since(self.longpr_rep_time) >= t.long_press_repeat {
                    self.longpr_rep_time = now;
                    match d.key {
                        Key::Enter => {
                            e.input_send(id, f, EventCode::LongPressedRepeat, EventParam::None);
                        }
                        Key::Next => {
                            e.set_editing(g, false);
                            e.focus_next(g);
                        }
                        Key::Prev => {
                            e.set_editing(g, false);
                            e.focus_prev(g);
                        }
                        k => {
                            send_key(e, id, g, k);
                        }
                    }
                }
            }
            (true, false) if en => {
                // The released key is the pressed one, whatever the driver reports.
                let key = prev_key.unwrap_or(d.key);
                self.last_key = Some(key);
                if key == Key::Enter {
                    if !e.input_send(id, f, EventCode::Released, EventParam::None) {
                        self.long_pr_sent = false;
                        return;
                    }
                    if !self.long_pr_sent {
                        let multi = self.clicks.click(now, None, t);
                        if !(e.input_send(id, f, EventCode::ShortClicked, EventParam::None)
                            && e.input_send(id, f, multi, EventParam::None))
                        {
                            self.long_pr_sent = false;
                            return;
                        }
                    }
                    e.input_send(id, f, EventCode::Clicked, EventParam::None);
                }
                self.long_pr_sent = false;
            }
            _ => {}
        }
    }
}

/// LVGL `lv_group_send_data`: sends `Key(k)` to the focused node of `g` unless it is
/// disabled. Returns whether processing may continue.
pub(crate) fn send_key(e: &mut Engine, id: InputId, g: GroupId, k: Key) -> bool {
    let Some(f): Option<NodeId> = e.focused(g) else {
        return true;
    };
    if e.tree.node(f).is_some_and(|n| n.state.contains(State::DISABLED)) {
        return true;
    }
    e.input_send(id, f, EventCode::Key, EventParam::Key(k));
    !e.input_reset_pending(id)
}
