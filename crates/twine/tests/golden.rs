//! Golden images (R5.S08): every widget × state × theme mode of the default theme, and a sheet
//! of widgets × rotation × colour format × theme mode, compared with the hash manifest
//! `tests/snapshots.toml` (`twine_testing::snapshot`).
//!
//! - **Widgets:** the built-in widgets of `twine-widgets` and `twine-widgets-ext` whose look
//!   comes from the theme. Image-based widgets (`image`, `image_button`, `animimg`) draw their
//!   images and have their own snapshots; `vector_view` needs the application's drawing.
//! - **States:** default, pressed, focused, disabled, and the R5.S06 validity looks `STALE`
//!   (sparse hatch) and `INVALID` (cross-hatch); `CHECKED` for the checkable widgets.
//! - **Theme modes:** `Light`, `Dark`, `Night`, `HighContrast` of `DefaultTheme`.
//! - **Rotation × colour format:** a sheet (button, checked checkbox, slider, switch, bar, a stale
//!   and an invalid label) in all four rotations and every colour format the renderer offers.
//!
//! Every image is pixel-exact (`Tolerance::EXACT`): rendering is integer-only, so the manifest's
//! hashes hold on every host (P6).
//!
//! req: SAF-017, SAF-023

use twine::core::Duration;
use twine::core::{ColorFormat, Rotation};
use twine::engine::{Engine, NodeId, ObjFlags};
use twine::style::{Align, State, ThemeMode};
use twine::theme::DefaultTheme;
use twine::widgets::{
    arc, bar, button, buttonmatrix, checkbox, container, keyboard, label, led, slider, spinbox, spinner,
    switch, textarea,
};
use twine::widgets_ext::{dropdown, list, menu, msgbox, roller, tabview, tileview, window};
use twine_testing::EngineHarness;

/// Renders a fixed 400 ms of updates (from a full redraw) and compares the panel with `name`.
/// Not `run_until_idle`: a focused textarea's cursor blinks and a spinner turns for ever; the
/// fixed schedule keeps their phase, and so the image, deterministic.
fn snapshot(h: &mut EngineHarness, name: &str) {
    let display = h.display();
    let area = twine::core::Rect::new(0, 0, 400, 400);
    h.engine_mut()
        .invalidate_area(display, area, twine::engine::InvalidateReason::Explicit);
    for _ in 0..20 {
        h.advance(Duration::from_millis(20));
        let _ = h.update();
    }
    h.assert_panel_snapshot(name);
}

/// A theme mode and its snapshot-name suffix.
const MODES: [(ThemeMode, &str); 4] = [
    (ThemeMode::Light, "light"),
    (ThemeMode::Dark, "dark"),
    (ThemeMode::Night, "night"),
    (ThemeMode::HighContrast, "hc"),
];

/// A state applied to the widget, and its name.
const STATES: [(State, &str); 6] = [
    (State::DEFAULT, "default"),
    (State::PRESSED, "pressed"),
    // Keyboard focus: the focus ring is drawn only for focus from a keypad or encoder.
    (State::FOCUSED.union(State::FOCUS_KEY), "focused"),
    (State::DISABLED, "disabled"),
    (State::STALE, "stale"),
    (State::INVALID, "invalid"),
];

/// Creates a widget under `parent` and returns it.
type Make = fn(&mut Engine, NodeId) -> NodeId;

fn with_label(e: &mut Engine, parent: NodeId, text: &'static str) {
    let l = label::create_with(e, parent, text).unwrap();
    e.align(l, Align::Center, 0, 0);
}

/// The widgets of the matrix: name, constructor, checkable.
const WIDGETS: &[(&str, Make, bool)] = &[
    (
        "button",
        |e, p| {
            let b = button::create(e, p).unwrap();
            with_label(e, b, "OK");
            b
        },
        true,
    ),
    (
        "label",
        |e, p| label::create_with(e, p, "Label 42").unwrap(),
        false,
    ),
    (
        "checkbox",
        |e, p| checkbox::create_with(e, p, "Check").unwrap(),
        true,
    ),
    ("switch", |e, p| switch::create(e, p).unwrap(), true),
    (
        "slider",
        |e, p| {
            let s = slider::create(e, p).unwrap();
            e.with_widget_mut(s, |w: &mut slider::Slider, cx| w.set_value(cx, 40, false));
            s
        },
        false,
    ),
    (
        "bar",
        |e, p| {
            let b = bar::create(e, p).unwrap();
            e.with_widget_mut(b, |w: &mut bar::Bar, cx| w.set_value(cx, 60, false));
            b
        },
        false,
    ),
    (
        "arc",
        |e, p| {
            let a = arc::create(e, p).unwrap();
            e.with_widget_mut(a, |w: &mut arc::Arc, cx| w.set_value(cx, 50));
            a
        },
        false,
    ),
    ("spinbox", |e, p| spinbox::create(e, p).unwrap(), false),
    (
        "textarea",
        |e, p| {
            let t = textarea::create(e, p).unwrap();
            e.with_widget_mut(t, |w: &mut textarea::Textarea, cx| w.set_text(cx, "Text"));
            e.set_size(t, 150, 60);
            t
        },
        false,
    ),
    ("led", |e, p| led::create(e, p).unwrap(), false),
    ("spinner", |e, p| spinner::create(e, p).unwrap(), false),
    (
        "buttonmatrix",
        |e, p| {
            let m = buttonmatrix::create(e, p).unwrap();
            e.set_size(m, 180, 100);
            m
        },
        false,
    ),
    (
        "card",
        |e, p| {
            let c = container::create_card(e, p).unwrap();
            e.set_size(c, 120, 70);
            c
        },
        false,
    ),
    (
        "keyboard",
        |e, p| {
            let k = keyboard::create(e, p).unwrap();
            e.set_size(k, 196, 110);
            k
        },
        false,
    ),
    (
        "dropdown",
        |e, p| {
            let d = dropdown::create(e, p).unwrap();
            e.with_widget_mut(d, |w: &mut dropdown::Dropdown, cx| {
                w.set_options(cx, "Alpha\nBeta\nGamma");
            });
            d
        },
        false,
    ),
    (
        "roller",
        |e, p| {
            let r = roller::create(e, p).unwrap();
            e.with_widget_mut(r, |w: &mut roller::Roller, cx| {
                w.set_options(cx, "One\nTwo\nThree\nFour", roller::RollerMode::Normal);
            });
            r
        },
        false,
    ),
    (
        "list",
        |e, p| {
            let l = list::create(e, p).unwrap();
            e.set_size(l, 150, 100);
            list::add_text(e, l, "Section").unwrap();
            l
        },
        false,
    ),
    (
        "tabview",
        |e, p| {
            let t = tabview::create(e, p).unwrap();
            e.set_size(t, 180, 100);
            e.with_widget_mut(t, |w: &mut tabview::Tabview, cx| {
                w.add_tab(cx, "Tab 1");
                w.add_tab(cx, "Tab 2");
            });
            t
        },
        false,
    ),
    (
        "tileview",
        |e, p| {
            let t = tileview::create(e, p).unwrap();
            e.set_size(t, 180, 100);
            t
        },
        false,
    ),
    (
        "menu",
        |e, p| {
            let m = menu::create(e, p).unwrap();
            e.set_size(m, 180, 100);
            m
        },
        false,
    ),
    (
        "window",
        |e, p| {
            let w = window::create(e, p).unwrap();
            e.set_size(w, 180, 100);
            e.with_widget_mut(w, |win: &mut window::Window, cx| {
                win.add_title(cx, "Window");
            });
            w
        },
        false,
    ),
    (
        "msgbox",
        |e, p| {
            let m = msgbox::create(e, Some(p)).unwrap();
            e.with_widget_mut(m, |b: &mut msgbox::Msgbox, cx| {
                b.add_title(cx, "Title");
                b.add_text(cx, "Message");
            });
            m
        },
        false,
    ),
];

fn theme(mode: ThemeMode) -> DefaultTheme {
    DefaultTheme::builder().mode(mode).build()
}

/// A 200×120 RGB565 scene with `make`'s widget centred, in `state`.
fn widget_scene(mode: ThemeMode, make: Make, state: State) -> EngineHarness {
    let mut h = EngineHarness::new(200, 120).theme(theme(mode));
    let screen = h.screen();
    let e = h.engine_mut();
    let w = make(e, screen);
    e.align(w, Align::Center, 0, 0);
    if state != State::DEFAULT {
        e.add_state(w, state);
    }
    h
}

#[test]
fn golden_widgets_states_modes() {
    for &(name, make, checkable) in WIDGETS {
        for (mode, m) in MODES {
            let states = STATES
                .iter()
                .copied()
                .chain(checkable.then_some((State::CHECKED, "checked")));
            for (state, s) in states {
                let mut h = widget_scene(mode, make, state);
                snapshot(&mut h, &format!("golden_{name}_{s}_{m}"));
            }
        }
    }
}

/// The sheet: a column of widgets, two of them with the validity looks.
fn sheet(e: &mut Engine, screen: NodeId) {
    let b = button::create(e, screen).unwrap();
    with_label(e, b, "OK");
    e.set_pos(b, 6, 6);
    let c = checkbox::create_with(e, screen, "On").unwrap();
    e.add_state(c, State::CHECKED);
    e.set_pos(c, 70, 10);
    let s = slider::create(e, screen).unwrap();
    e.with_widget_mut(s, |w: &mut slider::Slider, cx| w.set_value(cx, 30, false));
    e.set_pos(s, 10, 50);
    e.set_width(s, 100);
    let sw = switch::create(e, screen).unwrap();
    e.add_state(sw, State::CHECKED);
    e.set_pos(sw, 120, 40);
    let br = bar::create(e, screen).unwrap();
    e.with_widget_mut(br, |w: &mut bar::Bar, cx| w.set_value(cx, 70, false));
    e.set_pos(br, 10, 76);
    e.set_width(br, 100);
    let stale = label::create_with(e, screen, "12.5").unwrap();
    e.add_state(stale, State::STALE);
    e.set_pos(stale, 10, 94);
    let invalid = label::create_with(e, screen, "---").unwrap();
    e.add_state(invalid, State::INVALID);
    e.set_pos(invalid, 80, 94);
    e.set_flag(invalid, ObjFlags::CLICKABLE, false);
}

/// Every colour format the renderer offers (the facade's `full` enables them all).
const FORMATS: [(ColorFormat, &str); 7] = [
    (ColorFormat::Rgb565, "rgb565"),
    (ColorFormat::Rgb565Swapped, "rgb565sw"),
    (ColorFormat::Rgb888, "rgb888"),
    (ColorFormat::Xrgb8888, "xrgb8888"),
    (ColorFormat::Argb8888, "argb8888"),
    (ColorFormat::L8, "l8"),
    (ColorFormat::I1, "i1"),
];

const ROTATIONS: [(Rotation, &str); 4] = [
    (Rotation::Deg0, "r0"),
    (Rotation::Deg90, "r90"),
    (Rotation::Deg180, "r180"),
    (Rotation::Deg270, "r270"),
];

#[test]
fn golden_sheet_rotations_formats_modes() {
    for (mode, m) in MODES {
        for (rotation, r) in ROTATIONS {
            for (format, f) in FORMATS {
                let mut h = EngineHarness::new(160, 120)
                    .theme(theme(mode))
                    .format(format)
                    .rotation(rotation);
                let screen = h.screen();
                sheet(h.engine_mut(), screen);
                snapshot(&mut h, &format!("golden_sheet_{m}_{r}_{f}"));
            }
        }
    }
}
