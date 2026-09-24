//! [`Obj`]: the base object widget (LVGL `lv_obj`), and [`OBJ_CLASS`].

use twine_hal::Key;
use twine_style::{Dir, Part, ScrollSnap, ScrollbarMode, State};

use crate::{Engine, Event, EventCode, EventParam, InputKind, NodeId, ObjFlags, Widget, WidgetClass};

/// Default flags of [`OBJ_CLASS`] (LVGL `lv_obj_constructor`); widget classes derived from
/// the base object start from these.
pub const OBJ_FLAGS: ObjFlags = ObjFlags::CLICKABLE
    .union(ObjFlags::CLICK_FOCUSABLE)
    .union(ObjFlags::SCROLLABLE)
    .union(ObjFlags::SCROLL_ELASTIC)
    .union(ObjFlags::SCROLL_MOMENTUM)
    .union(ObjFlags::SCROLL_WITH_ARROW)
    .union(ObjFlags::SCROLL_CHAIN)
    .union(ObjFlags::SCROLL_ON_FOCUS)
    .union(ObjFlags::SNAPPABLE)
    .union(ObjFlags::PRESS_LOCK)
    .union(ObjFlags::GESTURE_BUBBLE);

/// The class of [`Obj`]: parts `Main` and `Scrollbar`.
pub static OBJ_CLASS: WidgetClass = WidgetClass::new("obj")
    .parts(&[Part::Main, Part::Scrollbar])
    .default_flags(OBJ_FLAGS);

/// The base object: a styled rectangle that can hold children. Screens, layers and plain
/// containers are `Obj`s.
///
/// ```
/// use twine_engine::{OBJ_CLASS, Obj, Widget};
/// assert_eq!(Obj.class().name, "obj");
/// assert!(core::ptr::eq(Obj.class(), &OBJ_CLASS));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Obj;

impl Widget for Obj {
    fn class(&self) -> &'static WidgetClass {
        &OBJ_CLASS
    }
}

/// The behaviour every node has (LVGL `lv_obj_event`), run for each node an event reaches
/// before the widget's own `event`:
///
/// - `Pressed` adds `PRESSED`; `Released` and `PressLost` remove it.
/// - `Released` toggles `CHECKED` on `CHECKABLE` nodes and sends `ValueChanged` (not when the
///   press turned into a scroll).
/// - `Key` `Right`/`Up` checks and `Left`/`Down` unchecks a `CHECKABLE` node (`ValueChanged`
///   when it changed); otherwise the arrows scroll a `SCROLLABLE | SCROLL_WITH_ARROW` node
///   that is not editable by a quarter of its size (bounded, without animation).
/// - `Focused` scrolls the node into view through all its ancestors when it has
///   `SCROLL_ON_FOCUS` (animated), then adds `FOCUSED` (+ `FOCUS_KEY` when a keypad or encoder
///   moved the focus, + `EDITED` when the group is in edit mode); `Defocused` removes all three.
/// - `ScrollBegin` / `ScrollEnd` add / remove `SCROLLED` (an `Active` scrollbar disappears).
/// - `SizeChanged` snaps the children again when the node has scroll snapping.
/// - `HoverOver` / `HoverLeave` add / remove `HOVERED`.
pub(crate) fn obj_event(e: &mut Engine, ev: &Event) {
    let obj = ev.current_target;
    match ev.code {
        EventCode::Pressed => e.add_state(obj, State::PRESSED),
        EventCode::PressLost => e.clear_state(obj, State::PRESSED),
        EventCode::Released => {
            e.clear_state(obj, State::PRESSED);
            let scrolled = e.active_input().and_then(|i| e.indev_scroll_of(i)).is_some();
            if !scrolled && e.has_flag(obj, ObjFlags::CHECKABLE) {
                let checked = e.tree.node(obj).is_some_and(|n| n.state.contains(State::CHECKED));
                e.set_state(obj, State::CHECKED, !checked);
                e.send_event(obj, EventCode::ValueChanged, EventParam::None);
            }
        }
        EventCode::Key => {
            let EventParam::Key(k) = ev.param else {
                return;
            };
            if e.has_flag(obj, ObjFlags::CHECKABLE) {
                let was = e.tree.node(obj).is_some_and(|n| n.state.contains(State::CHECKED));
                match k {
                    Key::Right | Key::Up => e.add_state(obj, State::CHECKED),
                    Key::Left | Key::Down => e.clear_state(obj, State::CHECKED),
                    _ => {}
                }
                let now = e.tree.node(obj).is_some_and(|n| n.state.contains(State::CHECKED));
                // With Enter, `Released` sends `ValueChanged`.
                if k != Key::Enter && was != now {
                    e.send_event(obj, EventCode::ValueChanged, EventParam::None);
                }
            } else if e.has_flag(obj, ObjFlags::SCROLLABLE | ObjFlags::SCROLL_WITH_ARROW)
                && !crate::input::is_editable(e, obj)
            {
                arrow_scroll(e, obj, k);
            }
        }
        EventCode::Focused => {
            if e.has_flag(obj, ObjFlags::SCROLL_ON_FOCUS) {
                e.scroll_to_view_recursive(obj, true);
            }
            let editing = e
                .tree
                .node(obj)
                .and_then(|n| n.group)
                .is_some_and(|g| e.group_editing(g));
            let mut s = State::FOCUSED;
            if matches!(
                e.active_input_kind(),
                Some(InputKind::Keypad | InputKind::Encoder)
            ) {
                s |= State::FOCUS_KEY;
            }
            if editing {
                e.add_state(obj, s | State::EDITED);
            } else {
                e.add_state(obj, s);
                e.clear_state(obj, State::EDITED);
            }
        }
        EventCode::Defocused => e.clear_state(obj, State::FOCUSED | State::EDITED | State::FOCUS_KEY),
        EventCode::ScrollBegin => e.add_state(obj, State::SCROLLED),
        EventCode::ScrollEnd => {
            e.clear_state(obj, State::SCROLLED);
            if e.scrollbar_mode(obj) == ScrollbarMode::Active {
                e.scrollbar_invalidate(obj);
            }
        }
        EventCode::SizeChanged => {
            if e.scroll_snap_x(obj) != ScrollSnap::None || e.scroll_snap_y(obj) != ScrollSnap::None {
                e.update_snap(obj, false);
            }
        }
        EventCode::HoverOver => e.add_state(obj, State::HOVERED),
        EventCode::HoverLeave => e.clear_state(obj, State::HOVERED),
        _ => {}
    }
}

/// Scrolls `obj` by a quarter of its size for an arrow key (LVGL `lv_obj_event` `KEY`):
/// `Up`/`Down` vertically; `Left`/`Right` horizontally when the node can scroll horizontally,
/// else vertically.
fn arrow_scroll(e: &mut Engine, obj: NodeId, k: Key) {
    let c = e.coords(obj);
    let off = e.scroll_offset(obj);
    let (dy, dx) = (c.height() / 4, c.width() / 4);
    let hor = e.scroll_dir(obj).intersects(Dir::HOR) && (e.scroll_left(obj) > 0 || e.scroll_right(obj) > 0);
    match k {
        Key::Right if hor => e.scroll_to_x(obj, off.x + dx, false),
        Key::Left if hor => e.scroll_to_x(obj, off.x - dx, false),
        Key::Down | Key::Right => e.scroll_to_y(obj, off.y + dy, false),
        Key::Up | Key::Left => e.scroll_to_y(obj, off.y - dy, false),
        _ => {}
    }
}

/// Stand-in stored in a node while its widget is temporarily taken out (e.g. during
/// `Widget::init`). A zero-sized type, so boxing it does not allocate.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Detached;

static DETACHED_CLASS: WidgetClass = WidgetClass::new("detached");

impl Widget for Detached {
    fn class(&self) -> &'static WidgetClass {
        &DETACHED_CLASS
    }
    fn draw(&self, _cx: &mut crate::DrawCx<'_, '_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_class_default_flags_match_lvgl() {
        let expected = ObjFlags::CLICKABLE
            | ObjFlags::CLICK_FOCUSABLE
            | ObjFlags::SCROLLABLE
            | ObjFlags::SCROLL_ELASTIC
            | ObjFlags::SCROLL_MOMENTUM
            | ObjFlags::SCROLL_WITH_ARROW
            | ObjFlags::SCROLL_CHAIN_HOR
            | ObjFlags::SCROLL_CHAIN_VER
            | ObjFlags::SCROLL_ON_FOCUS
            | ObjFlags::SNAPPABLE
            | ObjFlags::PRESS_LOCK
            | ObjFlags::GESTURE_BUBBLE;
        assert_eq!(OBJ_CLASS.default_flags, expected);
        assert!(!OBJ_CLASS.default_flags.contains(ObjFlags::HIDDEN));
        assert_eq!(OBJ_CLASS.parts, &[Part::Main, Part::Scrollbar]);
    }
}
