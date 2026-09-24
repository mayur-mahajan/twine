//! User event handlers ([`HandlerId`], [`EventFilter`]) and the event dispatcher
//! ([`Engine::send_event`]).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::event::{Event, EventCode, EventCx, EventParam, EventResult, EventText};
use crate::{Engine, NodeId, ObjFlags, Widget, fmt_node_id};

/// Handle of a user event handler, returned by [`Engine::add_event_handler`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HandlerId(u32);

/// Which events a handler receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventFilter {
    /// Every event.
    All,
    /// Only events with this code.
    Code(EventCode),
}

impl EventFilter {
    fn matches(self, code: EventCode) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::Code(c) => c == code,
        }
    }
}

/// A user event handler.
pub type Handler = Box<dyn FnMut(&mut EventCx<'_>, &Event) -> EventResult>;

/// Events waiting for a busy widget and the texts of events being dispatched. Every buffer
/// keeps its capacity: posting events and sending texts allocate nothing in steady state.
#[derive(Debug, Default)]
pub(crate) struct EventQueues {
    /// Events posted to nodes whose widget was busy, in posting order.
    posted: Vec<(NodeId, EventCode, EventParam)>,
    /// The texts of the events being dispatched (nested dispatch stacks them).
    text: String,
    /// The handles of those texts, innermost last.
    text_frames: Vec<EventText>,
    next_text_serial: u32,
    /// A `flush_posted` is running.
    flushing: bool,
}

/// The user handlers of one node. A handler's slot is `None` while it runs.
#[derive(Default)]
pub(crate) struct Handlers {
    list: Vec<(HandlerId, EventFilter, Option<Handler>)>,
}

impl core::fmt::Debug for Handlers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.list.iter().map(|(id, filter, _)| (id, filter)))
            .finish()
    }
}

impl Handlers {
    /// Takes the first handler after `after` (exclusive) and before `limit` (exclusive) that
    /// matches `code` and is not running.
    fn take_next(
        &mut self,
        after: Option<HandlerId>,
        limit: HandlerId,
        code: EventCode,
    ) -> Option<(HandlerId, Handler)> {
        self.list
            .iter_mut()
            .filter(|(id, filter, h)| {
                after.is_none_or(|a| *id > a) && *id < limit && filter.matches(code) && h.is_some()
            })
            .find_map(|(id, _, h)| h.take().map(|h| (*id, h)))
    }

    /// Puts a handler back into its slot. Returns `false` if it was removed meanwhile.
    fn put_back(&mut self, id: HandlerId, h: Handler) -> bool {
        match self.list.iter_mut().find(|(i, ..)| *i == id) {
            Some((.., slot)) if slot.is_none() => {
                *slot = Some(h);
                true
            }
            _ => false,
        }
    }
}

impl Engine {
    /// Registers a user handler on `id` for the events selected by `filter`. Handlers run in
    /// registration order, after the widget's own [`Widget::event`](crate::Widget::event).
    /// A handler added while an event is being dispatched runs from the next dispatch on.
    /// While a handler runs it is taken out of its node: events sent to the same node from
    /// inside it (nested dispatch) reach the node's other handlers, not the running one.
    ///
    /// Unknown nodes log `warn!`; the returned id then refers to nothing.
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// use twine_engine::{Engine, EngineConfig, EventCode, EventFilter, EventParam, EventResult, Obj};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// let hits = Rc::new(Cell::new(0));
    /// let h = hits.clone();
    /// e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), move |_cx, _ev| {
    ///     h.set(h.get() + 1);
    ///     EventResult::Continue
    /// });
    /// e.send_event(n, EventCode::Clicked, EventParam::None);
    /// e.send_event(n, EventCode::Pressed, EventParam::None); // filtered out
    /// assert_eq!(hits.get(), 1);
    /// ```
    pub fn add_event_handler(
        &mut self,
        id: NodeId,
        filter: EventFilter,
        h: impl FnMut(&mut EventCx<'_>, &Event) -> EventResult + 'static,
    ) -> HandlerId {
        let hid = HandlerId(self.next_handler_id);
        self.next_handler_id = self.next_handler_id.wrapping_add(1);
        match self.tree.node_mut(id) {
            Some(n) => {
                n.handlers
                    .get_or_insert_with(Box::default)
                    .list
                    .push((hid, filter, Some(Box::new(h))));
            }
            None => {
                twine_core::warn!(target: "twine::event", "add_event_handler: node {} not found", fmt_node_id(id));
            }
        }
        hid
    }

    /// Removes the handler `h` of `id` (also allowed from inside the handler itself). Returns
    /// whether it existed.
    pub fn remove_event_handler(&mut self, id: NodeId, h: HandlerId) -> bool {
        let Some(list) = self.tree.node_mut(id).and_then(|n| n.handlers.as_mut()) else {
            return false;
        };
        let before = list.list.len();
        list.list.retain(|(i, ..)| *i != h);
        before != list.list.len()
    }

    /// Removes every user handler of `id`.
    pub fn remove_all_event_handlers(&mut self, id: NodeId) {
        if let Some(n) = self.tree.node_mut(id) {
            n.handlers = None;
        }
    }

    /// Number of user handlers of `id`.
    #[must_use]
    pub fn event_handler_count(&self, id: NodeId) -> usize {
        self.tree
            .node(id)
            .and_then(|n| n.handlers.as_ref())
            .map_or(0, |h| h.list.len())
    }

    /// A new event code for application events (LVGL `lv_event_register_id`): `Custom(0)`,
    /// `Custom(1)`, …
    pub fn register_event_code(&mut self) -> EventCode {
        let c = EventCode::Custom(self.next_event_code);
        self.next_event_code = self.next_event_code.wrapping_add(1);
        c
    }

    /// Sends an event to `target` once its widget can take part (LVGL-style notifications
    /// such as `ValueChanged` sent by the widget itself).
    ///
    /// While a widget's [`Widget::event`], setter ([`with_widget_mut`](Self::with_widget_mut)),
    /// `init` or animation callback runs, the widget is taken out of its node: an event sent
    /// to that node right then would not reach the widget, and the node's handlers could not
    /// read it ([`widget`](Self::widget) gives `None`). A posted event waits until the widget
    /// is back and is dispatched right then, before the running dispatch continues; when the
    /// widget is not busy it is sent immediately. Posted events of a node deleted meanwhile
    /// are dropped. The result of the dispatch is not reported.
    ///
    /// ```
    /// use std::{cell::Cell, rc::Rc};
    /// use twine_engine::{Engine, EngineConfig, EventCode, EventFilter, EventParam, EventResult, Obj};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// let readable = Rc::new(Cell::new(None));
    /// let r = readable.clone();
    /// e.add_event_handler(n, EventFilter::Code(EventCode::ValueChanged), move |cx, ev| {
    ///     r.set(Some(cx.engine().widget::<Obj>(ev.target).is_some()));
    ///     EventResult::Continue
    /// });
    /// // From inside a setter the widget is busy: the event waits for it.
    /// let r = readable.clone();
    /// e.with_widget_mut(n, move |_o: &mut Obj, cx| {
    ///     cx.post_event(EventCode::ValueChanged, EventParam::Value(1));
    ///     assert_eq!(r.get(), None, "not dispatched yet");
    /// });
    /// assert_eq!(readable.get(), Some(true), "dispatched with the widget readable");
    /// ```
    pub fn post_event(&mut self, target: NodeId, code: EventCode, param: EventParam) {
        let busy = self
            .tree
            .node(target)
            .is_some_and(|n| n.widget.is::<crate::obj::Detached>());
        if !busy {
            self.send_event(target, code, param);
            return;
        }
        if matches!(param, EventParam::Text(_)) {
            twine_core::warn!(
                target: "twine::event",
                "post_event: {:?} to busy {} carries a text that ends with the current dispatch",
                code,
                fmt_node_id(target)
            );
        }
        twine_core::trace!(target: "twine::event", "{:?} -> {} posted (widget busy)", code, fmt_node_id(target));
        self.events.posted.push((target, code, param));
    }

    /// Puts `w` back into node `id` after it was taken out, then dispatches the posted events
    /// that became deliverable. Returns `false` (dropping the widget) when the node was
    /// deleted.
    pub(crate) fn restore_widget(&mut self, id: NodeId, w: Box<dyn Widget>) -> bool {
        let Some(n) = self.tree.node_mut(id) else {
            self.events.posted.retain(|p| p.0 != id);
            return false;
        };
        n.widget = w;
        self.flush_posted();
        true
    }

    /// Dispatches posted events in posting order, each once its node's widget is not busy;
    /// events of deleted nodes are dropped. Only the outermost call drains (nested widget
    /// restores during a posted dispatch leave the order to it).
    fn flush_posted(&mut self) {
        if self.events.flushing || self.events.posted.is_empty() {
            return;
        }
        self.events.flushing = true;
        loop {
            let tree = &self.tree;
            self.events.posted.retain(|p| tree.contains(p.0));
            let ready = self.events.posted.iter().position(|p| {
                self.tree
                    .node(p.0)
                    .is_some_and(|n| !n.widget.is::<crate::obj::Detached>())
            });
            let Some(i) = ready else { break };
            let (t, c, p) = self.events.posted.remove(i);
            self.send_event(t, c, p);
        }
        self.events.flushing = false;
    }

    /// Sends an event carrying `text` ([`EventParam::Text`]) and returns the result like
    /// [`send_event`](Self::send_event). The text is copied into a buffer of the engine
    /// (reused: no allocation once it is large enough) and readable by the handlers with
    /// [`EventCx::text`] while the event is dispatched.
    ///
    /// ```
    /// use std::{cell::RefCell, rc::Rc};
    /// use twine_engine::{Engine, EngineConfig, EventCode, EventFilter, EventResult, Obj};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// let seen = Rc::new(RefCell::new(String::new()));
    /// let s = seen.clone();
    /// e.add_event_handler(n, EventFilter::Code(EventCode::Insert), move |cx, ev| {
    ///     s.borrow_mut().push_str(cx.text(ev).unwrap_or(""));
    ///     EventResult::Continue
    /// });
    /// let typed = String::from("héllo");
    /// e.send_event_text(n, EventCode::Insert, &typed);
    /// assert_eq!(*seen.borrow(), "héllo");
    /// ```
    pub fn send_event_text(&mut self, target: NodeId, code: EventCode, text: &str) -> EventResult {
        let q = &mut self.events;
        let start = q.text.len();
        q.text.push_str(text);
        let h = EventText {
            start: start as u32,
            len: text.len() as u32,
            serial: q.next_text_serial,
        };
        q.next_text_serial = q.next_text_serial.wrapping_add(1);
        q.text_frames.push(h);
        let r = self.send_event(target, code, EventParam::Text(h));
        let q = &mut self.events;
        q.text_frames.pop();
        q.text.truncate(start);
        r
    }

    /// The text of an [`EventParam::Text`] while its event is being dispatched (`None` for
    /// other parameters and for handles of finished dispatches).
    #[must_use]
    pub fn event_text(&self, param: &EventParam) -> Option<&str> {
        let EventParam::Text(h) = param else {
            return None;
        };
        if !self.events.text_frames.contains(h) {
            return None;
        }
        let start = h.start as usize;
        self.events.text.get(start..start + h.len as usize)
    }

    /// Sends an event to `target` and returns the most decisive result of its handlers.
    ///
    /// On each node: the built-in object behaviour (states), then
    /// [`Widget::event`](crate::Widget::event), then the matching user handlers in
    /// registration order (skipped when the widget returned [`EventResult::Consumed`] or called
    /// [`EventCx::prevent_default`]). The event then continues on the parent when the node has
    /// [`ObjFlags::EVENT_BUBBLE`], nothing returned `Stop`/`Consumed` and the code bubbles.
    /// If a handler deletes the node, dispatch stops and `Consumed` is returned.
    pub fn send_event(&mut self, target: NodeId, code: EventCode, param: EventParam) -> EventResult {
        self.dispatch(target, target, code, param)
    }

    /// Dispatches starting at `start` with `target` as the original target.
    pub(crate) fn dispatch(
        &mut self,
        target: NodeId,
        start: NodeId,
        code: EventCode,
        param: EventParam,
    ) -> EventResult {
        if !self.tree.contains(start) {
            twine_core::warn!(target: "twine::event", "send_event: node {} not found", fmt_node_id(start));
            return EventResult::Continue;
        }
        let limit = HandlerId(self.next_handler_id);
        let mut result = EventResult::Continue;
        let mut node = start;
        loop {
            twine_core::trace!(
                target: "twine::event",
                "{:?} -> {} (current {})",
                code,
                fmt_node_id(target),
                fmt_node_id(node)
            );
            let ev = Event {
                code,
                target,
                current_target: node,
                param,
            };
            // 1. The base object behaviour (LVGL `lv_obj` class).
            crate::obj::obj_event(self, &ev);
            if !self.tree.contains(node) {
                return EventResult::Consumed;
            }
            // 2. The widget, taken out of the node so it can borrow the engine.
            let Some(n) = self.tree.node_mut(node) else {
                return EventResult::Consumed;
            };
            let mut w = core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached));
            let mut cx = EventCx::new(self, node, target);
            let r = w.event(&mut cx, &ev);
            let (mut stop, prevent) = (cx.stop_bubbling, cx.prevent_default);
            if !self.restore_widget(node, w) || !self.tree.contains(node) {
                return EventResult::Consumed;
            }
            let mut consumed = r == EventResult::Consumed || prevent;
            stop |= r == EventResult::Stop;
            // 3. User handlers.
            if !consumed {
                let mut after = None;
                loop {
                    let Some((hid, mut h)) = self
                        .tree
                        .node_mut(node)
                        .and_then(|n| n.handlers.as_mut())
                        .and_then(|l| l.take_next(after, limit, code))
                    else {
                        break;
                    };
                    after = Some(hid);
                    let mut cx = EventCx::new(self, node, target);
                    let r = h(&mut cx, &ev);
                    let (s, p) = (cx.stop_bubbling, cx.prevent_default);
                    let Some(n) = self.tree.node_mut(node) else {
                        return EventResult::Consumed;
                    };
                    // Not put back: the handler removed itself; it is dropped here.
                    if let Some(list) = n.handlers.as_mut() {
                        let _ = list.put_back(hid, h);
                    }
                    stop |= s || r == EventResult::Stop;
                    if r == EventResult::Consumed || p {
                        consumed = true;
                        break;
                    }
                }
            }
            if consumed {
                return EventResult::Consumed;
            }
            if stop {
                result = EventResult::Stop;
                break;
            }
            let bubble = match code.bubbles() {
                Some(b) => b,
                None => self
                    .tree
                    .node(node)
                    .is_some_and(|n| n.flags.contains(ObjFlags::EVENT_BUBBLE)),
            };
            match self.tree.parent(node) {
                Some(p) if bubble => node = p,
                _ => break,
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineConfig, Obj, Widget, WidgetClass};
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::RefCell;

    type Log = Rc<RefCell<Vec<&'static str>>>;

    struct Loud(Log);
    static LOUD: WidgetClass = WidgetClass::new("loud");
    impl Widget for Loud {
        fn class(&self) -> &'static WidgetClass {
            &LOUD
        }
        fn event(&mut self, _cx: &mut EventCx<'_>, _ev: &Event) -> EventResult {
            self.0.borrow_mut().push("class");
            EventResult::Continue
        }
    }

    fn engine() -> Engine {
        Engine::new(EngineConfig::default()).unwrap()
    }

    #[test]
    fn dispatch_order_and_self_removal() {
        let mut e = engine();
        let log: Log = Rc::default();
        let n = e.create_root(Box::new(Loud(log.clone()))).unwrap();
        assert_eq!(*log.borrow(), vec!["class"]); // `Create`
        log.borrow_mut().clear();
        let l = log.clone();
        let first = e.add_event_handler(n, EventFilter::All, move |_, _| {
            l.borrow_mut().push("first");
            EventResult::Continue
        });
        let l = log.clone();
        let hid = Rc::new(RefCell::new(None::<HandlerId>));
        let hid2 = hid.clone();
        let second = e.add_event_handler(n, EventFilter::Code(EventCode::Clicked), move |cx, _| {
            l.borrow_mut().push("second");
            let me = hid2.borrow().expect("id");
            let node = cx.node();
            assert!(cx.engine_mut().remove_event_handler(node, me));
            EventResult::Continue
        });
        *hid.borrow_mut() = Some(second);
        e.send_event(n, EventCode::Clicked, EventParam::None);
        e.send_event(n, EventCode::Clicked, EventParam::None);
        assert_eq!(*log.borrow(), vec!["class", "first", "second", "class", "first"]);
        assert_eq!(e.event_handler_count(n), 1);
        assert!(e.remove_event_handler(n, first));
        assert!(!e.remove_event_handler(n, first));
    }

    #[test]
    fn deleting_target_in_handler_stops() {
        let mut e = engine();
        let root = e.create_root(Box::new(Obj)).unwrap();
        let child = e.create(root, Box::new(Obj)).unwrap();
        let ran = Rc::new(RefCell::new(0));
        e.add_event_handler(child, EventFilter::All, |cx, _| {
            let id = cx.node();
            cx.engine_mut().delete(id).unwrap();
            EventResult::Continue
        });
        let r = ran.clone();
        e.add_event_handler(child, EventFilter::Code(EventCode::Clicked), move |_, _| {
            *r.borrow_mut() += 1;
            EventResult::Continue
        });
        assert_eq!(
            e.send_event(child, EventCode::Clicked, EventParam::None),
            EventResult::Consumed
        );
        assert!(!e.tree().contains(child));
        assert_eq!(*ran.borrow(), 0);
        assert_eq!(
            e.send_event(child, EventCode::Clicked, EventParam::None),
            EventResult::Continue
        );
    }

    #[test]
    fn custom_codes_increment() {
        let mut e = engine();
        assert_eq!(e.register_event_code(), EventCode::Custom(0));
        assert_eq!(e.register_event_code(), EventCode::Custom(1));
    }
}
