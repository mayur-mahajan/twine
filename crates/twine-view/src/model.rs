//! Two-way bindings: [`Model`] and [`IntoModel`].

use alloc::string::String;

use twine_engine::{Engine, Event, EventCode, EventFilter, EventResult, NodeId};
use twine_reactive::Signal;

use crate::access::EngineAccess;
use crate::bind::bind_node;
use crate::build::BuildCx;
use crate::prop::Prop;

/// The value of an editable widget: owned by the widget (changes are reported through
/// `.on_change`) or bound to a signal (the widget shows it and writes user changes back).
pub enum Model<T: 'static> {
    /// The widget owns the state, starting at this value.
    Owned(T),
    /// The widget displays the signal and writes changes back to it.
    Bound(Signal<T>),
}

impl<T: core::fmt::Debug + 'static> core::fmt::Debug for Model<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Model::Owned(v) => f.debug_tuple("Owned").field(v).finish(),
            Model::Bound(s) => f.debug_tuple("Bound").field(s).finish(),
        }
    }
}

/// Anything usable as the value of an editable widget: a plain value or a [`Signal`].
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let on = cx.signal(false);
/// let _bound = button(label("Wi-Fi")).checkable(true).checked(on); // two-way
/// let _owned = button(label("Bluetooth")).checkable(true).checked(true)
///     .on_change(|checked: bool| { let _ = checked; });
/// cx.dispose();
/// ```
pub trait IntoModel<T: 'static> {
    /// The model.
    fn into_model(self) -> Model<T>;
}

impl<T: 'static> IntoModel<T> for Signal<T> {
    fn into_model(self) -> Model<T> {
        Model::Bound(self)
    }
}

impl<T: 'static> IntoModel<T> for Model<T> {
    fn into_model(self) -> Model<T> {
        self
    }
}

/// Implements [`IntoModel<T>`] for plain values of each listed type.
#[macro_export]
macro_rules! impl_into_model {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::IntoModel<$t> for $t {
                fn into_model(self) -> $crate::Model<$t> {
                    $crate::Model::Owned(self)
                }
            }
        )*
    };
}

impl_into_model!(bool, i32, u8, u16, u32, usize, String, (u8, u8));

impl<T: 'static> IntoModel<Option<T>> for Option<T> {
    fn into_model(self) -> Model<Option<T>> {
        Model::Owned(self)
    }
}

/// Wires a model to a widget: `show` displays the value (once for owned values, as a binding
/// for bound signals); for bound signals, every `ValueChanged` of the node reads the widget's
/// value with `read` (which also gets the event: widgets that send `ValueChanged` from their own
/// event handler carry the new value in its parameter) and writes it back with
/// `set_if_changed`. The widget setter's
/// idempotency ends the round trip (the binding re-runs once and changes nothing).
pub(crate) fn bind_model<T: PartialEq + Clone + 'static>(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    model: Model<T>,
    show: impl Fn(&mut Engine, NodeId, T) + 'static,
    read: impl Fn(&Engine, NodeId, &Event) -> T + 'static,
) {
    match model {
        Model::Owned(v) => show(cx.engine(), node, v),
        Model::Bound(sig) => {
            bind_node(
                cx,
                node,
                Prop::Dynamic(alloc::boxed::Box::new(move || sig.get())),
                show,
            );
            cx.engine().add_event_handler(
                node,
                EventFilter::Code(EventCode::ValueChanged),
                move |ecx, ev| {
                    if ev.target == ecx.node() && sig.is_alive() {
                        let v = read(ecx.engine(), ev.target, ev);
                        EngineAccess::provide(ecx.engine_mut(), || sig.set_if_changed(v));
                    }
                    EventResult::Continue
                },
            );
        }
    }
}

/// Like [`bind_model`], for widgets whose value cannot be read from the `ValueChanged` event
/// (e.g. a range slider: the event carries the value of whichever knob moved): every
/// `ValueChanged` of the node schedules a read, which runs in the effect flush of the same
/// update (the widget is back in its node then) and writes the value with `set_if_changed`.
pub(crate) fn bind_model_synced<T: PartialEq + Clone + 'static>(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    model: Model<T>,
    show: impl Fn(&mut Engine, NodeId, T) + 'static,
    read: impl Fn(&Engine, NodeId) -> Option<T> + 'static,
) {
    match model {
        Model::Owned(v) => show(cx.engine(), node, v),
        Model::Bound(sig) => {
            bind_node(
                cx,
                node,
                Prop::Dynamic(alloc::boxed::Box::new(move || sig.get())),
                show,
            );
            let scope = cx.scope();
            let tick = scope.signal(0u32);
            cx.engine().add_event_handler(
                node,
                EventFilter::Code(EventCode::ValueChanged),
                move |ecx, ev| {
                    if ev.target == ecx.node() && tick.is_alive() {
                        EngineAccess::provide(ecx.engine_mut(), || tick.update(|t| *t = t.wrapping_add(1)));
                    }
                    EventResult::Continue
                },
            );
            let mut first = true;
            scope.effect(move || {
                let _ = tick.get();
                if core::mem::take(&mut first) {
                    return;
                }
                if !EngineAccess::available() {
                    return twine_reactive::defer_current_effect();
                }
                let v = EngineAccess::with(|e| {
                    if e.tree().contains(node) {
                        read(e, node)
                    } else {
                        None
                    }
                })
                .flatten();
                if let Some(v) = v {
                    if sig.is_alive() {
                        sig.set_if_changed(v);
                    }
                }
            });
        }
    }
}

/// The value an event carries as [`EventParam::Value`](twine_engine::EventParam::Value).
pub(crate) fn event_value(ev: &Event) -> Option<i32> {
    match ev.param {
        twine_engine::EventParam::Value(v) => Some(v),
        _ => None,
    }
}

/// Registers `f` for every `ValueChanged` sent to `node` itself, with `map(engine, event)`
/// as argument (skipped when `map` returns `None`); the engine is lent to `f` through
/// [`EngineAccess`].
pub(crate) fn on_value_changed<T: 'static>(
    cx: &mut BuildCx<'_>,
    node: NodeId,
    map: impl Fn(&Engine, NodeId, &Event) -> Option<T> + 'static,
    mut f: impl FnMut(T) + 'static,
) {
    cx.engine().add_event_handler(
        node,
        EventFilter::Code(EventCode::ValueChanged),
        move |ecx, ev| {
            if ev.target == ecx.node() {
                if let Some(v) = map(ecx.engine(), ev.target, ev) {
                    EngineAccess::provide(ecx.engine_mut(), || f(v));
                }
            }
            EventResult::Continue
        },
    );
}
