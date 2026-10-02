//! Helpers shared by the complex widget tests.
#![allow(dead_code)] // each test binary uses a subset

use std::rc::Rc;

use twine_engine::{NodeId, Widget, WidgetCx};
use twine_testing::EngineHarness;
use twine_theme::DefaultTheme;

/// Light or dark default theme, for snapshots of both variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Light, Mode::Dark];

    pub fn suffix(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }
}

/// A `w × h` harness with the default theme in `mode`.
pub fn harness(w: u16, h: u16, mode: Mode) -> EngineHarness {
    let t = match mode {
        Mode::Light => DefaultTheme::light(),
        Mode::Dark => DefaultTheme::dark(),
    };
    EngineHarness::new(w, h).theme(t)
}

/// A harness with a default focus group (keypad and encoder tests).
pub fn harness_with_group(w: u16, h: u16, mode: Mode) -> EngineHarness {
    let mut h = harness(w, h, mode);
    let g = h.engine_mut().create_group().unwrap();
    h.engine_mut().set_default_group(Some(g));
    h
}

/// Calls a widget setter.
pub fn with<W: Widget, R>(
    h: &mut EngineHarness,
    id: NodeId,
    f: impl FnOnce(&mut W, &mut WidgetCx<'_>) -> R,
) -> R {
    h.engine_mut()
        .with_widget_mut(id, f)
        .expect("widget of that type")
}

/// The widget of `id`.
pub fn get<W: Widget>(h: &EngineHarness, id: NodeId) -> &W {
    h.engine().widget::<W>(id).expect("widget of that type")
}

/// The class name of `id`.
pub fn class(h: &EngineHarness, id: NodeId) -> &'static str {
    h.engine().tree().node(id).expect("node").class().name
}

/// Whether `id` has `state`.
pub fn has_state(h: &EngineHarness, id: NodeId, state: twine_engine::State) -> bool {
    h.engine()
        .tree()
        .node(id)
        .is_some_and(|n| n.state().contains(state))
}

/// The center of `id`.
pub fn center(h: &EngineHarness, id: NodeId) -> twine_core::Point {
    let c = h.engine().coords(id);
    twine_core::Point::new(c.x0 + c.width() / 2, c.y0 + c.height() / 2)
}

/// Records the `EventParam::Value` of every `ValueChanged` sent to `id`.
pub fn values(h: &mut EngineHarness, id: NodeId) -> Rc<std::cell::RefCell<Vec<i32>>> {
    use twine_engine::{EventCode, EventFilter, EventParam, EventResult};
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    h.engine_mut()
        .add_event_handler(id, EventFilter::Code(EventCode::ValueChanged), move |_, ev| {
            if ev.target == ev.current_target {
                if let EventParam::Value(v) = ev.param {
                    l.borrow_mut().push(v);
                } else {
                    l.borrow_mut().push(i32::MIN);
                }
            }
            EventResult::Continue
        });
    log
}

/// Counts the events of `code` sent to `id` itself.
pub fn count_events(
    h: &mut EngineHarness,
    id: NodeId,
    code: twine_engine::EventCode,
) -> Rc<std::cell::Cell<u32>> {
    use twine_engine::{EventFilter, EventResult};
    let n = Rc::new(std::cell::Cell::new(0));
    let c = n.clone();
    h.engine_mut()
        .add_event_handler(id, EventFilter::Code(code), move |_, ev| {
            if ev.target == ev.current_target {
                c.set(c.get() + 1);
            }
            EventResult::Continue
        });
    n
}
