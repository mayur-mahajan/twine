//! Events: [`Event`], [`EventCode`], [`EventParam`], [`EventResult`] and the context handlers
//! receive, [`EventCx`].
//!
//! An event is sent to a *target* node with [`Engine::send_event`]. On each node it reaches,
//! the built-in object behaviour runs first (pressed / checked / focused states, like LVGL's
//! `lv_obj` class), then the widget's [`Widget::event`](crate::Widget::event), then the user
//! handlers registered with [`Engine::add_event_handler`] in registration order. If the node has
//! [`ObjFlags::EVENT_BUBBLE`](crate::ObjFlags::EVENT_BUBBLE) and nobody stopped it, the event
//! continues on the parent.

use twine_core::Point;
use twine_hal::Key;
use twine_style::{Dir, State};

use crate::{Engine, InputId, NodeId, WidgetCx};

/// What happened (LVGL `lv_event_code_t`).
///
/// ```
/// use twine_engine::EventCode;
/// assert!(EventCode::Clicked.is_input());
/// assert!(EventCode::DrawMain.is_draw());
/// assert!(!EventCode::ValueChanged.is_input());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventCode {
    // Input.
    /// The node was pressed.
    Pressed,
    /// The node is being pressed (sent on every read while pressed).
    Pressing,
    /// The pointer slid off the node while pressed (and the node lost the press).
    PressLost,
    /// Released before the long press time, without scrolling.
    ShortClicked,
    /// First short click of a series.
    SingleClicked,
    /// Second short click within the multi-click time and distance.
    DoubleClicked,
    /// Third short click within the multi-click time and distance.
    TripleClicked,
    /// Pressed for the long press time.
    LongPressed,
    /// Sent every long press repeat period after `LongPressed`.
    LongPressedRepeat,
    /// Released without scrolling (also after a long press).
    Clicked,
    /// Released (always sent on release).
    Released,
    /// Scrolling started.
    ScrollBegin,
    /// A scroll throw (momentum) started.
    ScrollThrowBegin,
    /// Scrolling ended.
    ScrollEnd,
    /// The node scrolled.
    Scroll,
    /// A swipe gesture ([`EventParam::Dir`]).
    Gesture,
    /// A key for the focused node ([`EventParam::Key`]).
    Key,
    /// A rotation by `n` steps ([`EventParam::Rotary`]), e.g. a mouse wheel over the node,
    /// sent by the application. Encoders do not send it: in edit mode their rotation arrives
    /// as `Key(Right)` / `Key(Left)` per step, like in LVGL, so widgets handle one input path.
    Rotary,
    /// The node got the focus.
    Focused,
    /// The node lost the focus.
    Defocused,
    /// The node lost the focus to a node of another group (pointer click focus).
    Leave,
    /// Advanced hit test ([`EventParam::Point`]); a handler returning `Stop` vetoes the hit.
    HitTest,
    /// A mouse pointer moved over the node.
    HoverOver,
    /// A mouse pointer left the node.
    HoverLeave,
    // Drawing.
    /// Cover check (custom draw hooks).
    CoverCheck,
    /// Extra draw size request (custom draw hooks).
    RefreshExtDrawSize,
    /// Before the main drawing.
    DrawMainBegin,
    /// Main drawing.
    DrawMain,
    /// After the main drawing.
    DrawMainEnd,
    /// Before the post drawing.
    DrawPostBegin,
    /// Post drawing (after the children).
    DrawPost,
    /// After the post drawing.
    DrawPostEnd,
    // Special.
    /// The widget's value changed (e.g. `CHECKED` toggled).
    ValueChanged,
    /// Text is being inserted.
    Insert,
    /// Request to refresh the widget.
    Refresh,
    /// A process finished.
    Ready,
    /// A process was cancelled (e.g. `Esc`).
    Cancel,
    // Lifecycle and layout.
    /// The node was created (sent after `Widget::init`).
    Create,
    /// The node is being deleted (sent bottom-up, before it is freed).
    Delete,
    /// A child was added, removed or moved.
    ChildChanged,
    /// A child was created (sent to the parent; [`Event::target`] is the child).
    ChildCreated,
    /// A child was deleted (sent to the parent; [`Event::target`] is the deleted child).
    ChildDeleted,
    /// The screen starts unloading.
    ScreenUnloadStart,
    /// The screen starts loading.
    ScreenLoadStart,
    /// The screen was loaded.
    ScreenLoaded,
    /// The screen was unloaded.
    ScreenUnloaded,
    /// The size changed ([`EventParam::Area`] holds the old coordinates).
    SizeChanged,
    /// A style of the node changed.
    StyleChanged,
    /// The node's state changed ([`EventParam::State`] holds the previous state; LVGL
    /// `LV_EVENT_STATE_CHANGED`). Sent after the new state is set, including for
    /// [`Engine::add_state`] / [`Engine::clear_state`] called directly; when the node's
    /// widget is busy (its own `event` or setter is running) it arrives right after that
    /// returns (see [`Engine::post_event`]).
    StateChanged,
    /// The layout of the node changed.
    LayoutChanged,
    /// Content size request.
    GetSelfSize,
    // User.
    /// An application-defined event (see [`Engine::register_event_code`]).
    Custom(u16),
}

impl EventCode {
    /// Whether this is an input event (pressing, clicking, keys, focus, hover, scrolling).
    #[must_use]
    pub const fn is_input(&self) -> bool {
        matches!(
            self,
            EventCode::Pressed
                | EventCode::Pressing
                | EventCode::PressLost
                | EventCode::ShortClicked
                | EventCode::SingleClicked
                | EventCode::DoubleClicked
                | EventCode::TripleClicked
                | EventCode::LongPressed
                | EventCode::LongPressedRepeat
                | EventCode::Clicked
                | EventCode::Released
                | EventCode::ScrollBegin
                | EventCode::ScrollThrowBegin
                | EventCode::ScrollEnd
                | EventCode::Scroll
                | EventCode::Gesture
                | EventCode::Key
                | EventCode::Rotary
                | EventCode::Focused
                | EventCode::Defocused
                | EventCode::Leave
                | EventCode::HitTest
                | EventCode::HoverOver
                | EventCode::HoverLeave
        )
    }

    /// Whether this is a drawing event.
    #[must_use]
    pub const fn is_draw(&self) -> bool {
        matches!(
            self,
            EventCode::CoverCheck
                | EventCode::RefreshExtDrawSize
                | EventCode::DrawMainBegin
                | EventCode::DrawMain
                | EventCode::DrawMainEnd
                | EventCode::DrawPostBegin
                | EventCode::DrawPost
                | EventCode::DrawPostEnd
        )
    }

    /// Whether the event may continue on the parent (LVGL `event_is_bubbled`): `ChildCreated`
    /// and `ChildDeleted` always do; drawing, hit test, `Refresh`, `Delete`, `Create`,
    /// `ChildChanged`, `SizeChanged`, `StyleChanged` and `GetSelfSize` never do; all others do
    /// when the node has `EVENT_BUBBLE`.
    #[must_use]
    pub(crate) const fn bubbles(self) -> Option<bool> {
        match self {
            EventCode::ChildCreated | EventCode::ChildDeleted => Some(true),
            EventCode::HitTest
            | EventCode::Refresh
            | EventCode::Delete
            | EventCode::Create
            | EventCode::ChildChanged
            | EventCode::SizeChanged
            | EventCode::StyleChanged
            | EventCode::GetSelfSize => Some(false),
            c if c.is_draw() => Some(false),
            _ => None,
        }
    }
}

/// The parameter of an event. Every variant is `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventParam {
    /// No parameter.
    None,
    /// A point (e.g. `HitTest`).
    Point(Point),
    /// A key (`Key`).
    Key(Key),
    /// A direction (`Gesture`: exactly one of `LEFT`, `RIGHT`, `TOP`, `BOTTOM`).
    Dir(Dir),
    /// Encoder steps (`Rotary`).
    Rotary(i32),
    /// A value.
    Value(i32),
    /// A text (e.g. `Insert`), sent with [`Engine::send_event_text`] and read with
    /// [`EventCx::text`] or [`Engine::event_text`] while the event is being dispatched.
    Text(EventText),
    /// A state (`StateChanged`: the previous state).
    State(State),
    /// An area (`SizeChanged`: the old coordinates).
    Area(twine_core::Rect),
}

/// Handle of the text of an event sent with [`Engine::send_event_text`]. The text itself is
/// kept by the engine (in a reused buffer, so sending text allocates nothing in steady state)
/// only while the event is being dispatched; read it with [`EventCx::text`] or
/// [`Engine::event_text`]. A handle kept longer reads as `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EventText {
    pub(crate) start: u32,
    pub(crate) len: u32,
    pub(crate) serial: u32,
}

/// What a handler asks the dispatcher to do next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventResult {
    /// Go on: remaining handlers run and the event bubbles (if enabled).
    #[default]
    Continue,
    /// Remaining handlers of this node run, but the event does not bubble to the parent.
    Stop,
    /// Nothing else runs: no further handlers, no bubbling. From a widget's `event`, the user
    /// handlers are skipped too ("prevent default").
    Consumed,
}

/// One event as seen by a handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Event {
    /// What happened.
    pub code: EventCode,
    /// The node the event was sent to.
    pub target: NodeId,
    /// The node whose handlers are running (differs from `target` while bubbling).
    pub current_target: NodeId,
    /// The parameter.
    pub param: EventParam,
}

impl Event {
    /// The key of a `Key` event.
    #[must_use]
    pub fn key(&self) -> Option<Key> {
        match self.param {
            EventParam::Key(k) => Some(k),
            _ => None,
        }
    }

    /// The direction of a `Gesture` event.
    #[must_use]
    pub fn dir(&self) -> Option<Dir> {
        match self.param {
            EventParam::Dir(d) => Some(d),
            _ => None,
        }
    }

    /// The value of an event with [`EventParam::Value`] (e.g. `ValueChanged` of a slider).
    #[must_use]
    pub fn value(&self) -> Option<i32> {
        match self.param {
            EventParam::Value(v) => Some(v),
            _ => None,
        }
    }

    /// The previous state of a `StateChanged` event.
    #[must_use]
    pub fn prev_state(&self) -> Option<State> {
        match self.param {
            EventParam::State(s) => Some(s),
            _ => None,
        }
    }
}

/// The context of an event handler: the engine (mutably) on behalf of the node whose handlers
/// run.
///
/// Handlers may do anything with the engine, including deleting nodes (the dispatcher notices
/// and stops) and sending further events ([`send`](Self::send)).
pub struct EventCx<'a> {
    engine: &'a mut Engine,
    node: NodeId,
    target: NodeId,
    pub(crate) stop_bubbling: bool,
    pub(crate) prevent_default: bool,
}

impl core::fmt::Debug for EventCx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EventCx")
            .field("node", &self.node)
            .field("target", &self.target)
            .field("stop_bubbling", &self.stop_bubbling)
            .field("prevent_default", &self.prevent_default)
            .finish_non_exhaustive()
    }
}

impl<'a> EventCx<'a> {
    pub(crate) fn new(engine: &'a mut Engine, node: NodeId, target: NodeId) -> Self {
        Self {
            engine,
            node,
            target,
            stop_bubbling: false,
            prevent_default: false,
        }
    }

    /// The node whose handlers run (the current target).
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The node the event was sent to.
    #[must_use]
    pub fn target(&self) -> NodeId {
        self.target
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        self.engine
    }

    /// The engine, mutably.
    pub fn engine_mut(&mut self) -> &mut Engine {
        self.engine
    }

    /// A widget context for the current node.
    pub fn widget_cx(&mut self) -> WidgetCx<'_> {
        WidgetCx::new(self.engine, self.node)
    }

    /// Stops bubbling to the parent after this node's handlers.
    pub fn stop_bubbling(&mut self) {
        self.stop_bubbling = true;
    }

    /// Skips the remaining handlers of this node (and bubbling).
    pub fn prevent_default(&mut self) {
        self.prevent_default = true;
        self.stop_bubbling = true;
    }

    /// The last point of the pointer device being processed (`None` outside pointer input).
    #[must_use]
    pub fn point(&self) -> Option<Point> {
        self.engine.input_point()
    }

    /// The input device being processed (`None` for events not caused by input).
    #[must_use]
    pub fn input(&self) -> Option<InputId> {
        self.engine.active_input()
    }

    /// Sends another event (nested dispatch) and returns its result.
    ///
    /// From a widget's own [`Widget::event`](crate::Widget::event), an event sent to the
    /// widget's node does not reach the widget (it is busy) and its handlers cannot read the
    /// widget; use [`post`](Self::post) for notifications such as `ValueChanged`.
    pub fn send(&mut self, target: NodeId, code: EventCode, param: EventParam) -> EventResult {
        self.engine.send_event(target, code, param)
    }

    /// Sends an event once `target`'s widget is back in its node (see
    /// [`Engine::post_event`]): from a widget's `event`, a `ValueChanged` posted to its own
    /// node reaches the handlers right after `event` returns, and they can read the widget.
    pub fn post(&mut self, target: NodeId, code: EventCode, param: EventParam) {
        self.engine.post_event(target, code, param);
    }

    /// The text of `ev` when it carries one ([`EventParam::Text`]), e.g. the text being
    /// inserted into a textarea (`Insert`).
    #[must_use]
    pub fn text(&self, ev: &Event) -> Option<&str> {
        self.engine.event_text(&ev.param)
    }
}
