//! Scene helpers shared by the engine integration tests.
#![allow(dead_code)]

use twine_core::{Color, Opa, Rect};
use twine_engine::{Engine, NodeId};
use twine_style::{Selector, StyleProp};

/// The active screen of the default display.
pub fn screen(e: &Engine) -> NodeId {
    e.active_screen(e.default_display().expect("display"))
        .expect("screen")
}

/// Sets `props` as local `Main` properties.
pub fn style(e: &mut Engine, id: NodeId, props: &[StyleProp]) {
    for p in props {
        e.set_local_prop(id, Selector::MAIN, *p);
    }
}

/// An opaque colored box under `parent` covering the absolute rectangle `r` (kept by the
/// layout: stored as position and size styles).
pub fn boxed(e: &mut Engine, parent: NodeId, r: Rect, c: Color) -> NodeId {
    twine_testing::scenes::styled_box(
        e,
        parent,
        r,
        &[StyleProp::BgColor(c), StyleProp::BgOpa(Opa::COVER)],
    )
}

/// A white opaque screen background.
pub fn white_screen(e: &mut Engine) -> NodeId {
    let s = screen(e);
    style(
        e,
        s,
        &[StyleProp::BgColor(Color::WHITE), StyleProp::BgOpa(Opa::COVER)],
    );
    s
}

/// A shared log of `(code, current node)` pairs.
pub type EvLog = std::rc::Rc<std::cell::RefCell<Vec<(twine_engine::EventCode, NodeId)>>>;

/// Records every event reaching `id` into `log`.
pub fn record(e: &mut Engine, id: NodeId, log: &EvLog) {
    let l = log.clone();
    e.add_event_handler(id, twine_engine::EventFilter::All, move |cx, ev| {
        l.borrow_mut().push((ev.code, cx.node()));
        twine_engine::EventResult::Continue
    });
}

/// The input-related codes of `log` (input events, `ValueChanged`, `Cancel`), in order.
pub fn input_codes(log: &EvLog) -> Vec<twine_engine::EventCode> {
    use twine_engine::EventCode as C;
    log.borrow()
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| (c.is_input() && *c != C::HitTest) || matches!(c, C::ValueChanged | C::Cancel))
        .collect()
}

/// A clickable box (plain `Obj`) under `parent` at `r`.
pub fn clickable(e: &mut Engine, parent: NodeId, r: Rect) -> NodeId {
    boxed(e, parent, r, Color::BLUE)
}

/// A scroll container under `parent` covering the absolute rectangle `r` (white background)
/// holding `rows` children of `row_h` pixels stacked from the top of its content area, full
/// width, in alternating colors. Returns the container and its rows.
pub fn list(e: &mut Engine, parent: NodeId, r: Rect, rows: usize, row_h: i32) -> (NodeId, Vec<NodeId>) {
    let cont = plain_box(e, parent, r);
    let mut ids = Vec::with_capacity(rows);
    for i in 0..rows {
        let c = if i % 2 == 0 {
            Color::hex(0x0030_60C0)
        } else {
            Color::hex(0x00E0_A040)
        };
        let row = twine_testing::scenes::child_box(
            e,
            cont,
            Rect::from_xywh(0, i as i32 * row_h, r.width(), row_h),
            &[StyleProp::BgColor(c), StyleProp::BgOpa(Opa::COVER)],
        );
        ids.push(row);
    }
    e.update_layout();
    (cont, ids)
}

fn plain_box(e: &mut Engine, parent: NodeId, r: Rect) -> NodeId {
    boxed(e, parent, r, Color::WHITE)
}

/// Every code of `log` for `node`, in order.
pub fn codes_of(log: &EvLog, node: NodeId) -> Vec<twine_engine::EventCode> {
    log.borrow()
        .iter()
        .filter(|(_, n)| *n == node)
        .map(|(c, _)| *c)
        .collect()
}

/// The scroll-related codes of `log` (`ScrollBegin`, `ScrollThrowBegin`, `ScrollEnd`; not the
/// per-move `Scroll`).
pub fn scroll_codes(log: &EvLog) -> Vec<twine_engine::EventCode> {
    use twine_engine::EventCode as C;
    log.borrow()
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| matches!(c, C::ScrollBegin | C::ScrollThrowBegin | C::ScrollEnd))
        .collect()
}
