//! Encoder processing (LVGL `indev_encoder_proc`).

use twine_core::Instant;
use twine_hal::{EncoderData, Key};
use twine_style::State;

use super::keypad::send_key;
use super::{ClickCounter, InputId, Timing};
use crate::{Editable, Engine, EventCode, EventParam, NodeId};

/// The state of one rotary encoder with a push button.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EncoderProc {
    /// Whether the button was pressed at the last read.
    pub(crate) pressed: bool,
    press_time: Instant,
    long_pr_sent: bool,
    longpr_rep_time: Instant,
    clicks: ClickCounter,
    pub(crate) wait_until_release: bool,
}

impl EncoderProc {
    pub(crate) fn new() -> Self {
        Self {
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

    /// One encoder sample (the button acts as `Enter`).
    ///
    /// - Rotation (only counted while the button is up): in navigate mode moves the focus by
    ///   `diff` nodes; in edit mode sends `Key(Right)` / `Key(Left)` `|diff|` times to the
    ///   focused node (LVGL: no `Rotary` event, so widgets have a single encoder path).
    /// - Press: `Pressed` to non-editable nodes, or to any node in edit mode.
    /// - Long press: toggles edit mode on editable nodes (not when the group has a single
    ///   node), else `LongPressed`.
    /// - Release: non-editable nodes get `Released`, `ShortClicked` + multi-click (unless long
    ///   pressed) and `Clicked`; in edit mode the same plus `Key(Enter)`; an editable node in
    ///   navigate mode enters edit mode.
    pub(crate) fn process(&mut self, e: &mut Engine, id: InputId, d: EncoderData, now: Instant, t: &Timing) {
        if d.pressed && self.wait_until_release {
            return;
        }
        if self.wait_until_release {
            self.wait_until_release = false;
            self.long_pr_sent = false;
            self.pressed = false;
        }
        let prev = self.pressed;
        self.pressed = d.pressed;
        let Some(g) = e.input_group(id) else {
            return;
        };
        let Some(f) = e.focused(g) else {
            return;
        };
        // Steps are only valid with the button released.
        let diff = if d.pressed { 0 } else { i32::from(d.diff) };
        let en = e.tree.node(f).is_some_and(|n| !n.state.contains(State::DISABLED));
        // LVGL treats scrollable nodes like editable ones: the rotation scrolls them in edit
        // mode (arrow keys).
        let editable = is_editable(e, f) || e.has_flag(f, crate::ObjFlags::SCROLLABLE);
        match (prev, d.pressed) {
            (false, true) => {
                self.press_time = now;
                self.long_pr_sent = false;
                if (e.group_editing(g) || !editable) && en {
                    e.input_send(id, f, EventCode::Pressed, EventParam::None);
                }
            }
            (true, true) => {
                if !self.long_pr_sent {
                    if now.saturating_duration_since(self.press_time) >= t.long_press {
                        self.long_pr_sent = true;
                        self.longpr_rep_time = now;
                        if editable {
                            // Nowhere to navigate with a single node: stay in edit mode.
                            if e.group_count(g) > 1 {
                                let editing = e.group_editing(g);
                                e.set_editing(g, !editing);
                                e.clear_state(f, State::PRESSED);
                            }
                        } else if en {
                            e.input_send(id, f, EventCode::LongPressed, EventParam::None);
                        }
                    }
                } else if now.saturating_duration_since(self.longpr_rep_time) >= t.long_press_repeat {
                    self.longpr_rep_time = now;
                    if en {
                        e.input_send(id, f, EventCode::LongPressedRepeat, EventParam::None);
                    }
                }
            }
            (true, false) => {
                let long = self.long_pr_sent;
                self.long_pr_sent = false;
                if !editable {
                    if en && !self.click(e, id, f, long, now, t) {
                        return;
                    }
                } else if e.group_editing(g) {
                    // A long press release in edit mode comes from the mode switch.
                    if !long || e.group_count(g) <= 1 {
                        if en && !self.click(e, id, f, long, now, t) {
                            return;
                        }
                        if !send_key(e, id, g, Key::Enter) {
                            return;
                        }
                    } else {
                        e.clear_state(f, State::PRESSED);
                    }
                } else if !long {
                    e.set_editing(g, true);
                }
            }
            (false, false) => {}
        }
        if diff == 0 {
            return;
        }
        if e.group_editing(g) {
            let key = if diff > 0 { Key::Right } else { Key::Left };
            for _ in 0..diff.unsigned_abs() {
                if !send_key(e, id, g, key) {
                    return;
                }
            }
        } else {
            for _ in 0..diff.unsigned_abs() {
                if diff > 0 {
                    e.focus_next(g);
                } else {
                    e.focus_prev(g);
                }
                if e.input_reset_pending(id) {
                    return;
                }
            }
        }
    }

    /// `Released`, `ShortClicked` + multi-click (unless `long`), `Clicked`.
    fn click(
        &mut self,
        e: &mut Engine,
        id: InputId,
        f: NodeId,
        long: bool,
        now: Instant,
        t: &Timing,
    ) -> bool {
        if !e.input_send(id, f, EventCode::Released, EventParam::None) {
            return false;
        }
        if !long {
            let multi = self.clicks.click(now, None, t);
            if !(e.input_send(id, f, EventCode::ShortClicked, EventParam::None)
                && e.input_send(id, f, multi, EventParam::None))
            {
                return false;
            }
        }
        e.input_send(id, f, EventCode::Clicked, EventParam::None)
    }
}

/// Whether the node's class has an encoder edit mode (`Editable::Inherit` counts as not
/// editable: the base object is not).
pub(crate) fn is_editable(e: &Engine, n: NodeId) -> bool {
    e.tree
        .node(n)
        .is_some_and(|x| x.class().editable == Editable::True)
}
