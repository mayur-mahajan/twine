//! Input devices (LVGL `lv_indev`): registration, power-aware polling and the per-kind
//! processors that turn device samples into events.
//!
//! Devices are read by [`Engine::read_inputs`] (called first by [`Engine::step`]):
//!
//! - [`PollHint::Periodic`] devices are read every [`EngineConfig::read_period`], so the engine
//!   asks to be woken that often.
//! - [`PollHint::Interrupt`] devices are read only after [`Engine::notify_input`] (call it from
//!   the main loop when the device's interrupt fired) and then every `read_period` while they
//!   are active (pointer or button pressed, key or encoder button held). An idle touch UI with
//!   an interrupt line therefore never wakes the CPU.
//!
//! [`EngineConfig::read_period`]: crate::EngineConfig::read_period

mod button;
mod encoder;
mod gesture;
mod keypad;
mod pointer;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use twine_core::{Duration, Instant, Point};
use twine_hal::{InputData, InputDevice, InputKind, PollHint};
use twine_style::Dir;

pub(crate) use button::ButtonProc;
pub(crate) use encoder::{EncoderProc, is_editable};
pub(crate) use keypad::KeypadProc;
pub(crate) use pointer::PointerProc;

use crate::{DisplayId, Engine, EngineError, EventCode, EventParam, GroupId, NodeId, fmt_node_id};

/// Most input devices registered at once.
pub const MAX_INPUTS: usize = 8;

/// Keypad events read in one [`Engine::read_inputs`] call while the device reports `more`;
/// the rest stays queued in the driver and is read at the next update, which is requested
/// at once (nothing is dropped).
const MAX_KEYPAD_READS: usize = 16;

/// Handle of an input device registered with [`Engine::add_input`]. Printed as `i0`, `i1`, ….
///
/// Slots of removed devices are reused by later devices.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InputId(u8);

impl InputId {
    /// The device's slot index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for InputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i{}", self.0)
    }
}

impl fmt::Display for InputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i{}", self.0)
    }
}

/// The processor of one device kind.
pub(crate) enum Proc {
    Pointer(PointerProc),
    Keypad(KeypadProc),
    Encoder(EncoderProc),
    Button(ButtonProc),
}

impl Proc {
    fn new(kind: InputKind) -> Self {
        match kind {
            InputKind::Pointer => Proc::Pointer(PointerProc::new()),
            InputKind::Keypad => Proc::Keypad(KeypadProc::new()),
            InputKind::Encoder => Proc::Encoder(EncoderProc::new()),
            InputKind::Button => Proc::Button(ButtonProc::new()),
        }
    }

    /// Whether the device is held (pressed pointer / button / key / encoder button).
    fn is_active(&self) -> bool {
        match self {
            // A throw keeps the pointer being read after the release.
            Proc::Pointer(p) => p.pressed || p.scroll_obj.is_some(),
            Proc::Button(b) => b.ptr.pressed || b.ptr.scroll_obj.is_some(),
            Proc::Keypad(k) => k.pressed,
            Proc::Encoder(e) => e.pressed,
        }
    }

    /// The next instant a timer (long press, long press repeat) fires.
    fn deadline(&self, t: &Timing) -> Option<Instant> {
        match self {
            Proc::Pointer(p) => p.deadline(t),
            Proc::Button(b) => b.ptr.deadline(t),
            Proc::Keypad(k) => k.deadline(t),
            Proc::Encoder(e) => e.deadline(t),
        }
    }

    /// LVGL `indev_proc_reset_query_handler`.
    fn reset(&mut self, forget: Forget) {
        match self {
            Proc::Pointer(p) => p.reset(forget),
            Proc::Button(b) => b.ptr.reset(forget),
            Proc::Keypad(k) => k.reset(),
            Proc::Encoder(e) => e.reset(),
        }
    }

    /// The node a pointer (or button) device scrolls.
    fn scroll_obj(&self) -> Option<NodeId> {
        match self {
            Proc::Pointer(p) => p.scroll_obj,
            Proc::Button(b) => b.ptr.scroll_obj,
            _ => None,
        }
    }

    fn wait_release(&mut self) {
        match self {
            Proc::Pointer(p) => p.wait_until_release = true,
            Proc::Button(b) => b.ptr.wait_until_release = true,
            Proc::Keypad(k) => k.wait_until_release = true,
            Proc::Encoder(e) => e.wait_until_release = true,
        }
    }
}

/// Which node references a reset drops (besides the pressed node).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Forget {
    /// Keep the last pressed and hovered nodes.
    #[default]
    Nothing,
    /// Drop references to this node.
    Node(NodeId),
    /// Drop every reference.
    All,
}

/// The timing configuration the processors use.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    pub(crate) long_press: Duration,
    pub(crate) long_press_repeat: Duration,
    pub(crate) multi_click: Duration,
    pub(crate) multi_click_distance: i32,
    pub(crate) scroll_limit: i32,
    pub(crate) scroll_throw: u8,
    pub(crate) gesture_limit: i32,
    pub(crate) gesture_min_velocity: i32,
}

/// A registered input device.
pub(crate) struct InputState {
    pub(crate) id: InputId,
    /// Distinguishes devices that reuse a slot.
    serial: u32,
    pub(crate) kind: InputKind,
    driver: Box<dyn InputDevice>,
    pub(crate) display: DisplayId,
    pub(crate) enabled: bool,
    notified: bool,
    next_read: Option<Instant>,
    pub(crate) group: Option<GroupId>,
    /// `None` while the processor runs (it borrows the engine).
    proc: Option<Proc>,
    last_data: Option<InputData>,
    /// A reset was requested ([`Engine::input_reset`]); applied at the next read.
    reset_query: bool,
    forget: Forget,
    /// Ignore the device until it is released.
    wait_release: bool,
}

impl InputState {
    fn apply_pending(&mut self, proc: &mut Proc) {
        if self.reset_query {
            proc.reset(core::mem::take(&mut self.forget));
            self.reset_query = false;
        }
        if self.wait_release {
            proc.wait_release();
            self.wait_release = false;
        }
    }
}

/// Whether a sample shows the device held.
fn data_active(d: &InputData) -> bool {
    match d {
        InputData::Pointer(p) => p.pressed,
        InputData::Keypad(k) => k.pressed,
        InputData::Encoder(e) => e.pressed,
        InputData::Button(b) => b.pressed,
    }
}

/// Counts short clicks into single / double / triple clicks (LVGL `indev_proc_short_click`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClickCounter {
    count: u8,
    last_time: Option<Instant>,
    last_point: Point,
}

impl ClickCounter {
    /// Registers a short click at `now` (at `point` for pointers) and returns which multi-click
    /// event it is.
    pub(crate) fn click(&mut self, now: Instant, point: Option<Point>, t: &Timing) -> EventCode {
        self.count = self.count.wrapping_add(1);
        let in_time = self
            .last_time
            .is_some_and(|l| now.saturating_duration_since(l) <= t.multi_click);
        if !in_time {
            self.count = 1;
        } else if let Some(p) = point {
            let (dx, dy) = (
                i64::from(self.last_point.x) - i64::from(p.x),
                i64::from(self.last_point.y) - i64::from(p.y),
            );
            let d = i64::from(t.multi_click_distance);
            if dx * dx + dy * dy > d * d {
                self.count = 1;
            }
        }
        self.last_time = Some(now);
        if let Some(p) = point {
            self.last_point = p;
        }
        match (self.count - 1) % 3 {
            0 => EventCode::SingleClicked,
            1 => EventCode::DoubleClicked,
            _ => EventCode::TripleClicked,
        }
    }
}

impl Engine {
    pub(crate) fn input_timing(&self) -> Timing {
        let c = &self.config;
        Timing {
            long_press: c.long_press_time,
            long_press_repeat: c.long_press_repeat,
            multi_click: c.multi_click_time,
            multi_click_distance: c.multi_click_distance,
            scroll_limit: c.scroll_limit,
            scroll_throw: c.scroll_throw,
            gesture_limit: c.gesture_limit,
            gesture_min_velocity: c.gesture_min_velocity,
        }
    }

    pub(crate) fn input_state(&self, id: InputId) -> Option<&InputState> {
        self.inputs.get(id.index()).and_then(Option::as_ref)
    }

    pub(crate) fn input_state_mut(&mut self, id: InputId) -> Option<&mut InputState> {
        self.inputs.get_mut(id.index()).and_then(Option::as_mut)
    }

    /// Registers an input device for `display`. At most [`MAX_INPUTS`] devices; more fail with
    /// [`EngineError::TooManyInputs`].
    ///
    /// ```
    /// use twine_core::Point;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::{InputData, InputDevice, InputKind, PointerData, PollHint};
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_engine::BufferMode;
    /// # use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
    /// # struct Panel(Option<DrawBufferMem>);
    /// # impl DisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
    /// #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
    /// # }
    ///
    /// struct Touch;
    /// impl InputDevice for Touch {
    ///     fn kind(&self) -> InputKind { InputKind::Pointer }
    ///     fn read(&mut self) -> InputData {
    ///         InputData::Pointer(PointerData { point: Point::new(1, 1), pressed: false })
    ///     }
    ///     fn poll_hint(&self) -> PollHint { PollHint::Interrupt }
    /// }
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// # let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
    /// # let display = e.add_display(Panel(None), BufferMode::partial_single(buf)).unwrap();
    /// let touch = e.add_input(Touch, display).unwrap();
    /// // Call from the main loop when the touch interrupt fired:
    /// e.notify_input(touch);
    /// ```
    pub fn add_input(
        &mut self,
        dev: impl InputDevice + 'static,
        display: DisplayId,
    ) -> Result<InputId, EngineError> {
        if display.index() >= self.displays.len() {
            twine_core::warn!(target: "twine::input", "add_input: display {} not found", display);
            return Err(EngineError::DisplayNotFound(display));
        }
        let idx = match self.inputs.iter().position(Option::is_none) {
            Some(i) => i,
            None if self.inputs.len() < MAX_INPUTS => {
                self.inputs.push(None);
                self.inputs.len() - 1
            }
            None => {
                twine_core::warn!(target: "twine::input", "add_input: more than {} input devices", MAX_INPUTS);
                return Err(EngineError::TooManyInputs);
            }
        };
        let kind = dev.kind();
        let id = InputId(idx as u8);
        self.input_serial = self.input_serial.wrapping_add(1);
        twine_core::info!(
            target: "twine::input",
            "input {} added: {:?} ({:?}) on display {}",
            id,
            kind,
            dev.poll_hint(),
            display
        );
        self.inputs[idx] = Some(InputState {
            id,
            serial: self.input_serial,
            kind,
            driver: Box::new(dev),
            display,
            enabled: true,
            notified: false,
            next_read: None,
            group: None,
            proc: Some(Proc::new(kind)),
            last_data: None,
            reset_query: false,
            forget: Forget::Nothing,
            wait_release: false,
        });
        Ok(id)
    }

    /// Removes an input device (it is dropped; a scroll it was doing ends without events).
    pub fn remove_input(&mut self, id: InputId) {
        match self.inputs.get_mut(id.index()) {
            Some(slot @ Some(_)) => {
                *slot = None;
                self.sync_indev_scroll(id, None);
                twine_core::info!(target: "twine::input", "input {} removed", id);
            }
            _ => twine_core::warn!(target: "twine::input", "remove_input: input {} not found", id),
        }
    }

    /// Enables or disables a device. A disabled device is not read at all.
    pub fn set_input_enabled(&mut self, id: InputId, on: bool) {
        match self.input_state_mut(id) {
            Some(st) => st.enabled = on,
            None => twine_core::warn!(target: "twine::input", "set_input_enabled: input {} not found", id),
        }
    }

    /// Whether the device exists and is enabled.
    #[must_use]
    pub fn input_enabled(&self, id: InputId) -> bool {
        self.input_state(id).is_some_and(|s| s.enabled)
    }

    /// The kind of a device.
    #[must_use]
    pub fn input_kind(&self, id: InputId) -> Option<InputKind> {
        self.input_state(id).map(|s| s.kind)
    }

    /// The group a keypad or encoder sends its keys to.
    #[must_use]
    pub fn input_group(&self, id: InputId) -> Option<GroupId> {
        self.input_state(id).and_then(|s| s.group)
    }

    /// Every registered device.
    pub fn inputs(&self) -> impl Iterator<Item = InputId> + '_ {
        self.inputs.iter().flatten().map(|s| s.id)
    }

    /// Marks an interrupt device as having new input: the next [`read_inputs`](Self::read_inputs)
    /// reads it. Call this from the main loop after the device's interrupt woke it.
    pub fn notify_input(&mut self, id: InputId) {
        match self.input_state_mut(id) {
            Some(st) => st.notified = true,
            None => twine_core::warn!(target: "twine::input", "notify_input: input {} not found", id),
        }
    }

    /// Marks every device as notified (for a shared interrupt line).
    pub fn notify_input_all(&mut self) {
        for st in self.inputs.iter_mut().flatten() {
            st.notified = true;
        }
    }

    /// Maps the buttons of a [`InputKind::Button`] device to screen points: button `n` presses
    /// like a pointer at `points[n]`.
    pub fn set_button_points(&mut self, id: InputId, points: &'static [Point]) {
        match self.input_state_mut(id) {
            Some(InputState {
                proc: Some(Proc::Button(b)),
                ..
            }) => b.points = points,
            _ => {
                twine_core::warn!(target: "twine::input", "set_button_points: {} is not an idle button device", id);
            }
        }
    }

    /// The direction of the last gesture of a pointer (cleared by the next press).
    #[must_use]
    pub fn gesture_dir(&self, id: InputId) -> Option<Dir> {
        match self.input_state(id)?.proc.as_ref()? {
            Proc::Pointer(p) => p.gesture_dir,
            Proc::Button(b) => b.ptr.gesture_dir,
            _ => None,
        }
    }

    /// Changes the gesture thresholds: movement in pixels that makes a gesture and the
    /// minimum movement per read.
    pub fn set_gesture_limits(&mut self, limit: i32, min_velocity: i32) {
        self.config.gesture_limit = limit;
        self.config.gesture_min_velocity = min_velocity;
    }

    /// Resets the processing state of device `id` (`None`: every device), like LVGL
    /// `lv_indev_reset`: the pressed node is forgotten and a device that is held is ignored
    /// until it is released. With `obj`, only references to `obj` among the last pressed and
    /// hovered nodes are dropped; without, all of them.
    pub fn input_reset(&mut self, id: Option<InputId>, obj: Option<NodeId>) {
        for st in self.inputs.iter_mut().flatten() {
            if id.is_some_and(|i| i != st.id) {
                continue;
            }
            st.reset_query = true;
            st.forget = match obj {
                None => Forget::All,
                Some(o) => {
                    if st.forget == Forget::All {
                        Forget::All
                    } else {
                        Forget::Node(o)
                    }
                }
            };
            if st.last_data.as_ref().is_some_and(data_active) {
                st.wait_release = true;
            }
        }
    }

    /// Ignores device `id` until it is released (LVGL `lv_indev_wait_release`).
    pub fn input_wait_release(&mut self, id: InputId) {
        if let Some(st) = self.input_state_mut(id) {
            st.wait_release = true;
        }
    }

    /// Whether a reset of `id` is pending (processors stop at once, LVGL `indev_reset_check`).
    pub(crate) fn input_reset_pending(&self, id: InputId) -> bool {
        self.input_state(id).is_none_or(|s| s.reset_query)
    }

    /// The device being processed.
    #[must_use]
    pub fn active_input(&self) -> Option<InputId> {
        self.input_active.map(|(i, _)| i)
    }

    /// The kind of the device being processed.
    pub(crate) fn active_input_kind(&self) -> Option<InputKind> {
        self.input_active.map(|(_, k)| k)
    }

    /// The point of the pointer being processed.
    pub(crate) fn input_point(&self) -> Option<Point> {
        self.input_point
    }

    /// Reads the devices that need it at `now` and turns their samples into events (design
    /// order: the first thing [`step`](Self::step) does).
    pub fn read_inputs(&mut self, now: Instant) {
        let t = self.input_timing();
        let period = self.config.read_period;
        for idx in 0..self.inputs.len() {
            let Some(st) = self.inputs[idx].as_mut() else {
                continue;
            };
            if !st.enabled {
                continue;
            }
            let hint = st.driver.poll_hint();
            let timer_due = st
                .proc
                .as_ref()
                .and_then(|p| p.deadline(&t))
                .is_some_and(|d| now >= d);
            let read_due = st.next_read.is_some_and(|d| now >= d);
            let due = match hint {
                PollHint::Periodic => st.next_read.is_none() || read_due || timer_due,
                PollHint::Interrupt => st.notified || read_due || timer_due,
            };
            if !due {
                continue;
            }
            st.notified = false;
            let serial = st.serial;
            let mut more = false;
            for _ in 0..MAX_KEYPAD_READS {
                let Some(st) = self.inputs[idx].as_mut().filter(|s| s.serial == serial) else {
                    break;
                };
                let data = st.driver.read();
                if st.last_data != Some(data) {
                    twine_core::debug!(target: "twine::input", "read {:?} -> {:?}", st.id, data);
                    st.last_data = Some(data);
                }
                self.process_input(idx, serial, data, now);
                more = matches!(data, InputData::Keypad(k) if k.more);
                if !more {
                    break;
                }
            }
            let Some(st) = self.inputs[idx].as_mut().filter(|s| s.serial == serial) else {
                continue;
            };
            let active = st.proc.as_ref().is_some_and(Proc::is_active);
            if more {
                // Events left in the driver's queue: read on at the next update, right away.
                twine_core::debug!(target: "twine::input", "{:?}: more than {} keypad events, continuing next update", st.id, MAX_KEYPAD_READS);
                st.next_read = Some(now);
                continue;
            }
            match hint {
                PollHint::Periodic => st.next_read = Some(now + period),
                PollHint::Interrupt if active => st.next_read = Some(now + period),
                PollHint::Interrupt => {
                    st.next_read = None;
                    st.driver.rearm();
                }
            }
        }
    }

    /// Runs the processor of slot `idx` on one sample.
    fn process_input(&mut self, idx: usize, serial: u32, data: InputData, now: Instant) {
        let Some(st) = self.inputs[idx].as_mut() else {
            return;
        };
        let Some(mut proc) = st.proc.take() else {
            return;
        };
        st.apply_pending(&mut proc);
        let (id, kind, display) = (st.id, st.kind, st.display);
        self.sync_indev_scroll(id, proc.scroll_obj());
        if data.kind() == kind {
            let prev = self.input_active.replace((id, kind));
            let t = self.input_timing();
            match (&mut proc, data) {
                (Proc::Pointer(p), InputData::Pointer(d)) => p.process(self, id, display, d, now, &t),
                (Proc::Button(b), InputData::Button(d)) => b.process(self, id, display, d, now, &t),
                (Proc::Keypad(k), InputData::Keypad(d)) => k.process(self, id, d, now, &t),
                (Proc::Encoder(e), InputData::Encoder(d)) => e.process(self, id, d, now, &t),
                _ => {}
            }
            self.input_active = prev;
            self.input_point = None;
        } else {
            twine_core::warn!(target: "twine::input", "input {}: {:?} device returned {:?}", id, kind, data);
        }
        if let Some(st) = self.inputs[idx].as_mut().filter(|s| s.serial == serial) {
            st.apply_pending(&mut proc);
            let scroll = proc.scroll_obj();
            st.proc = Some(proc);
            self.sync_indev_scroll(id, scroll);
        }
    }

    /// The earliest instant an input device needs to be read (polling period, held devices,
    /// long press timers). `None`: no device needs the CPU.
    #[must_use]
    pub fn input_deadline(&self) -> Option<Instant> {
        let t = self.input_timing();
        self.inputs
            .iter()
            .flatten()
            .filter(|s| s.enabled)
            .flat_map(|s| [s.next_read, s.proc.as_ref().and_then(|p| p.deadline(&t))])
            .flatten()
            .min()
    }

    /// Sends an input event to `obj` on behalf of device `id` and logs it. Returns whether
    /// processing may continue: `obj` still exists and no reset of the device is pending.
    pub(crate) fn input_send(
        &mut self,
        id: InputId,
        obj: NodeId,
        code: EventCode,
        param: EventParam,
    ) -> bool {
        twine_core::debug!(
            target: "twine::input",
            "{} {:?} {} at {:?}",
            id,
            code,
            fmt_node_id(obj),
            self.input_point
        );
        self.send_event(obj, code, param);
        self.tree.contains(obj) && !self.input_reset_pending(id)
    }
}

/// `true` while an input processor must stop (the node is gone or a reset is pending).
pub(crate) fn stopped(e: &Engine, id: InputId, obj: NodeId) -> bool {
    !e.tree.contains(obj) || e.input_reset_pending(id)
}

/// Input devices kept in an engine (slot index = [`InputId`]).
pub(crate) type InputSlots = Vec<Option<InputState>>;
