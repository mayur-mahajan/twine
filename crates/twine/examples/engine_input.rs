//! `cargo xtask sim engine_input`: the engine's input system — pointer, keypad, encoder,
//! focus groups, gestures and grid navigation — on an imperative scene.
//!
//! Left: a 3×3 grid of boxes in a container with grid navigation; right: a column of three
//! boxes (the middle one is "editable", like a slider). Boxes change color when pressed and
//! checked (the checkable ones have a thin dark border), and show an outline when focused
//! (thicker from the keyboard / wheel, orange while edited). Colors, outlines and the slight
//! shrink of a pressed box are animated by a style transition (150 ms, ease-out). Every event
//! is logged:
//!
//! - Mouse: click, double / triple click, long press (hold), swipe (drag fast) for gestures.
//! - Keyboard: Tab / Shift+Tab move the focus between the grid and the right column, arrows
//!   move inside the grid, Enter clicks, Esc cancels, letters arrive as `Key(Char)`.
//! - Wheel: moves the focus like Tab; middle click on the editable box toggles edit mode, in
//!   which the wheel sends `Key(Left/Right)` and `Rotary`.
//!
//! - `+` / `-`: make the grid boxes wider / narrower; the grid is a wrapping flex row, so the
//!   boxes flow onto more or fewer rows (gridnav follows the new geometry).
//!
//! Everything is positioned by the layout: the grid is a `RowWrap` flex container, the right
//! column a `Column` flex container; no coordinates are set by hand.
//!
//! `RUST_LOG=twine::input=debug` also logs every device read and every event sent by the
//! input processors.

use twine::core::{Color, Duration, Opa, Scale};
use twine::engine::{
    Editable, Engine, EventCode, EventFilter, EventParam, EventResult, GridnavCtrl, NodeId, Obj, ObjFlags,
    State, Widget, WidgetClass, gridnav,
};
use twine::hal::Key;
use twine::style::{
    CrossAlign, FlexFlow, LayoutKind, Length, MainAlign, Part, PropId, Props, Radius, Selector, StyleProp,
    Transition, TransitionRef,
};
use twine_sim::{SimConfig, run_engine};

/// A box with an encoder edit mode (stands in for a slider until widgets exist).
struct EditableBox;

static EDITABLE_BOX: WidgetClass = WidgetClass::new("editable_box")
    .default_flags(ObjFlags::CLICKABLE.union(ObjFlags::CLICK_FOCUSABLE))
    .editable(Editable::True);

impl Widget for EditableBox {
    fn class(&self) -> &'static WidgetClass {
        &EDITABLE_BOX
    }
}

/// What changes smoothly when a box changes state: its background, outline and transform
/// (property groups).
static TRANSITION: Transition = Transition::of(
    Props::BG.union(Props::OUTLINE).union(Props::TRANSFORM),
    Duration::ms(150),
)
.ease_out();

/// Box size and the width step of `+` / `-`.
const BOX_W: i32 = 80;
const BOX_H: i32 = 50;
const STEP: i32 = 8;

/// Paddings on all four sides.
fn set_padding(e: &mut Engine, id: NodeId, p: i32) {
    for prop in [
        StyleProp::PaddingLeft(Length::Px(p).into()),
        StyleProp::PaddingTop(Length::Px(p).into()),
        StyleProp::PaddingRight(Length::Px(p).into()),
        StyleProp::PaddingBottom(Length::Px(p).into()),
    ] {
        e.set_local_prop(id, Selector::MAIN, prop);
    }
}

/// Local styles for every state the demo shows.
fn style_box(e: &mut Engine, b: NodeId, checkable: bool) {
    e.set_size(b, BOX_W, BOX_H);
    let main = Selector::MAIN;
    let props = [
        (main, StyleProp::BgColor(Color::hex(0x5C_6B_C0).into())),
        (main, StyleProp::BgOpacity(Opa::COVER.into())),
        (main, StyleProp::Radius(Radius::Px(6).into())),
        (main, StyleProp::Transition(TransitionRef::Static(&TRANSITION))),
        (main, StyleProp::TransformPivotX(Length::Pct(50).into())),
        (main, StyleProp::TransformPivotY(Length::Pct(50).into())),
        (
            main.with_state(State::PRESSED),
            StyleProp::TransformScaleX(Scale::from_raw_256(236)),
        ),
        (
            main.with_state(State::PRESSED),
            StyleProp::TransformScaleY(Scale::from_raw_256(236)),
        ),
        (
            main.with_state(State::PRESSED),
            StyleProp::BgColor(Color::hex(0x28_35_93).into()),
        ),
        (
            main.with_state(State::CHECKED),
            StyleProp::BgColor(Color::hex(0x43_A0_47).into()),
        ),
        (main.with_state(State::FOCUSED), StyleProp::OutlineWidth(2)),
        (
            main.with_state(State::FOCUSED),
            StyleProp::OutlineColor(Color::hex(0x21_21_21).into()),
        ),
        (main.with_state(State::FOCUSED), StyleProp::OutlineOffset(2)),
        (main.with_state(State::FOCUS_KEY), StyleProp::OutlineWidth(4)),
        (
            main.with_state(State::FOCUS_KEY),
            StyleProp::OutlineColor(Color::hex(0x29_79_FF).into()),
        ),
        (
            main.with_state(State::EDITED),
            StyleProp::OutlineColor(Color::hex(0xFF_6D_00).into()),
        ),
    ];
    for (sel, p) in props {
        e.set_local_prop(b, sel, p);
    }
    if checkable {
        e.set_flag(b, ObjFlags::CHECKABLE, true);
        e.set_local_prop(b, main, StyleProp::BorderWidth(Length::Px(2).into()));
        e.set_local_prop(b, main, StyleProp::BorderColor(Color::hex(0x1A_23_7E).into()));
    }
}

/// Logs every input event of box `name` (`Pressing` only at debug level: it repeats).
fn log_events(e: &mut Engine, b: NodeId, name: &'static str) {
    e.add_event_handler(b, EventFilter::All, move |_cx, ev| {
        let interesting =
            ev.code.is_input() || matches!(ev.code, EventCode::ValueChanged | EventCode::Cancel);
        if !interesting || ev.code == EventCode::HitTest {
            return EventResult::Continue;
        }
        match (ev.code, ev.param) {
            (EventCode::Pressing, _) => twine::core::debug!(target: "twine::sim", "Pressing on box {}", name),
            (_, EventParam::Key(k)) => {
                twine::core::info!(target: "twine::sim", "Key({:?}) on box {}", k, name);
            }
            (_, EventParam::Rotary(d)) => {
                twine::core::info!(target: "twine::sim", "Rotary({}) on box {}", d, name);
            }
            (code, _) => twine::core::info!(target: "twine::sim", "{:?} on box {}", code, name),
        }
        EventResult::Continue
    });
}

/// Names of the grid boxes (row by row) and of the right column.
const NAMES: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
const SIDE: [&str; 3] = ["A", "B (editable)", "C"];

fn scene(e: &mut Engine) {
    let d = e.default_display().expect("display");
    let screen = e.active_screen(d).expect("screen");
    e.set_local_prop(
        screen,
        Selector::MAIN,
        StyleProp::BgColor(Color::hex(0xEC_EF_F1).into()),
    );
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    e.add_event_handler(screen, EventFilter::Code(EventCode::Gesture), |_, ev| {
        if let Some(dir) = ev.dir() {
            twine::core::info!(target: "twine::sim", "Gesture({:?}) on the screen", dir);
        }
        EventResult::Continue
    });

    // The grid: a wrapping flex row with grid navigation, member of the default group.
    let grid = e.create(screen, Box::new(Obj)).expect("grid");
    e.set_pos(grid, 8, 8);
    e.set_size(grid, 272, 186);
    e.set_layout(grid, LayoutKind::Flex);
    e.set_flex_flow(grid, FlexFlow::ROW.wrap(true));
    set_padding(e, grid, 8);
    e.set_local_prop(grid, Selector::MAIN, StyleProp::RowGap(Length::Px(8).into()));
    e.set_local_prop(grid, Selector::MAIN, StyleProp::ColumnGap(Length::Px(8).into()));
    e.set_local_prop(grid, Selector::MAIN, StyleProp::BgColor(Color::WHITE.into()));
    e.set_local_prop(grid, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    e.set_local_prop(grid, Selector::MAIN, StyleProp::Radius(Radius::Px(8).into()));
    // An outline (not a border) as focus ring: it does not shrink the content area.
    e.set_local_prop(
        grid,
        Selector::state(State::FOCUS_KEY),
        StyleProp::OutlineWidth(2),
    );
    e.set_local_prop(
        grid,
        Selector::state(State::FOCUS_KEY),
        StyleProp::OutlineColor(Color::hex(0x29_79_FF).into()),
    );
    for (i, name) in NAMES.iter().enumerate() {
        let b = e.create(grid, Box::new(Obj)).expect("box");
        style_box(e, b, i % 2 == 1);
        log_events(e, b, name);
    }
    gridnav::gridnav_add(e, grid, GridnavCtrl::ROLLOVER);

    // The right column: a flex column of plain group members; the middle one is editable.
    let column = e.create(screen, Box::new(Obj)).expect("column");
    e.set_flag(column, ObjFlags::CLICKABLE, false);
    e.set_pos(column, 296, 8);
    e.set_size(column, Length::Content, 194);
    e.set_layout(column, LayoutKind::Flex);
    e.set_flex_flow(column, FlexFlow::COLUMN);
    e.set_flex_align(
        column,
        MainAlign::SpaceEvenly,
        CrossAlign::Start,
        MainAlign::Start,
    );
    set_padding(e, column, 8);
    let mut side = Vec::new();
    for (i, name) in SIDE.iter().enumerate() {
        let b = if i == 1 {
            e.create(column, Box::new(EditableBox)).expect("editable")
        } else {
            e.create(column, Box::new(Obj)).expect("box")
        };
        style_box(e, b, i == 2);
        log_events(e, b, name);
        side.push(b);
    }

    if let Some(g) = e.default_group() {
        e.group_add(g, grid);
        for b in side {
            e.group_add(g, b);
        }
    }
}

/// `+` / `-`: resizes every grid box by [`STEP`] px (between 24 and 120 px).
fn on_raw_key(e: &mut Engine, key: Key) {
    let step = match key {
        Key::Char('+' | '=') => STEP,
        Key::Char('-') => -STEP,
        _ => return,
    };
    let d = e.default_display().expect("display");
    let screen = e.active_screen(d).expect("screen");
    let Some(grid) = e.tree().children(screen).next() else {
        return;
    };
    let boxes: Vec<NodeId> = e.tree().children(grid).collect();
    let w = (e.style_i32(boxes[0], Part::Main, PropId::Width) + step).clamp(24, 120);
    for b in boxes {
        e.set_width(b, w);
    }
    twine::core::info!(target: "twine::sim", "grid boxes: {} px wide", w);
}

fn main() {
    twine_sim::init_logging();
    let cfg = SimConfig::new(400, 210)
        .title("engine_input")
        .scale(2)
        .on_raw_key(on_raw_key);
    run_engine(cfg, scene);
}
