//! `ImageButton`: LVGL defaults, image selection per state with LVGL's fallbacks, 3-slice
//! tiling, content size, pressing and checking, and snapshots.

mod common;

use common::{Mode, get, harness, solid_image, with};
use twine_core::{Color, Point, Size};
use twine_engine::{NodeId, ObjFlags, State};
use twine_image::ImageSource;
use twine_style::Align;
use twine_testing::EngineHarness;
use twine_widgets::image_button::{self, IMAGE_BUTTON_CLASS, ImageButton, ImageButtonState as S};

const RED: Color = Color::new(0xF4, 0x43, 0x36);
const GREEN: Color = Color::new(0x4C, 0xAF, 0x50);
const BLUE: Color = Color::new(0x21, 0x96, 0xF3);
const AMBER: Color = Color::new(0xFF, 0xC1, 0x07);
const GREY: Color = Color::new(0x9E, 0x9E, 0x9E);
const PURPLE: Color = Color::new(0x9C, 0x27, 0xB0);

fn scene(mode: Mode) -> (EngineHarness, NodeId) {
    let mut h = harness(120, 60, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let b = image_button::create(e, screen).unwrap();
    e.align(b, Align::Center, 0, 0);
    (h, b)
}

fn set(h: &mut EngineHarness, b: NodeId, st: S, c: Color) {
    let img = solid_image(40, 20, c);
    with(h, b, |w: &mut ImageButton, cx| {
        w.set_src(cx, st, None, Some(img), None);
    });
}

fn center_color(h: &mut EngineHarness, b: NodeId) -> Color {
    h.run_until_idle();
    let p = h.engine().coords(b).center();
    h.pixel(p.x as u32, p.y as u32)
}

/// Equal up to RGB565 rounding.
fn near(a: Color, b: Color) -> bool {
    let d = |x: u8, y: u8| x.abs_diff(y) <= 8;
    d(a.r, b.r) && d(a.g, b.g) && d(a.b, b.b)
}

fn state(h: &EngineHarness, b: NodeId) -> State {
    h.engine().tree().node(b).unwrap().state()
}

#[test]
fn imgbtn_defaults() {
    let (mut h, b) = scene(Mode::Light);
    h.run_until_idle();
    let n = h.engine().tree().node(b).unwrap();
    assert_eq!(n.class().name, "imagebutton");
    assert_eq!(IMAGE_BUTTON_CLASS.default_flags, twine_engine::OBJ_FLAGS);
    assert!(n.flags().contains(ObjFlags::CLICKABLE));
    assert_eq!(h.engine().coords(b).size(), Size::ZERO, "no images: empty");
    assert!(
        get::<ImageButton>(&h, b)
            .src(S::Released)
            .iter()
            .all(Option::is_none)
    );
    h.assert_idle();
}

#[test]
fn imgbtn_set_src_same_value_no_invalidate() {
    let (mut h, b) = scene(Mode::Light);
    let img = solid_image(40, 20, RED);
    let i2 = img.clone();
    with(&mut h, b, |w: &mut ImageButton, cx| {
        w.set_src(cx, S::Released, None, Some(img), None);
    });
    h.run_until_idle();
    with(&mut h, b, |w: &mut ImageButton, cx| {
        w.set_src(cx, S::Released, None, Some(i2), None);
        w.set_state(cx, S::Released);
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn imgbtn_state_image_selection() {
    let (mut h, b) = scene(Mode::Light);
    let colors = [
        (S::Released, RED),
        (S::Pressed, GREEN),
        (S::Disabled, GREY),
        (S::CheckedReleased, BLUE),
        (S::CheckedPressed, AMBER),
        (S::CheckedDisabled, PURPLE),
    ];
    for (st, c) in colors {
        set(&mut h, b, st, c);
    }
    for (st, c) in colors {
        with(&mut h, b, |w: &mut ImageButton, cx| w.set_state(cx, st));
        assert_eq!(S::from_state(state(&h, b)), st);
        assert_eq!(get::<ImageButton>(&h, b).shown_state(state(&h, b)), st);
        let px = center_color(&mut h, b);
        assert!(near(px, c), "{st:?}: {px:?} vs {c:?}");
    }
}

#[test]
fn imgbtn_fallback_to_released() {
    let (mut h, b) = scene(Mode::Light);
    set(&mut h, b, S::Released, RED);
    let w = get::<ImageButton>(&h, b);
    for st in S::ALL {
        assert_eq!(w.shown_state(st.to_state()), S::Released, "{st:?}");
    }
    set(&mut h, b, S::CheckedReleased, BLUE);
    let w = get::<ImageButton>(&h, b);
    assert_eq!(w.shown_state(S::CheckedPressed.to_state()), S::CheckedReleased);
    assert_eq!(w.shown_state(S::CheckedDisabled.to_state()), S::CheckedReleased);
    assert_eq!(w.shown_state(S::Pressed.to_state()), S::Released);
    // Checked-pressed prefers checked, then pressed.
    set(&mut h, b, S::Pressed, GREEN);
    let (mut h2, b2) = scene(Mode::Light);
    set(&mut h2, b2, S::Released, RED);
    set(&mut h2, b2, S::Pressed, GREEN);
    assert_eq!(
        get::<ImageButton>(&h2, b2).shown_state(S::CheckedPressed.to_state()),
        S::Pressed
    );
}

#[test]
fn imgbtn_three_slice_tiles_mid() {
    let (mut h, b) = scene(Mode::Light);
    let (l, m, r) = (
        solid_image(4, 20, RED),
        solid_image(3, 20, GREEN),
        solid_image(5, 20, BLUE),
    );
    with(&mut h, b, |w: &mut ImageButton, cx| {
        w.set_src(cx, S::Released, Some(l), Some(m), Some(r));
    });
    h.run_until_idle();
    assert_eq!(
        h.engine().coords(b).size(),
        Size::new(12, 20),
        "content: the three widths"
    );
    h.engine_mut().set_width(b, 60);
    h.run_until_idle();
    let c = h.engine().coords(b);
    let y = (c.y0 + 10) as u32;
    let at = |h: &EngineHarness, x: i32| h.pixel(x as u32, y);
    assert!(near(at(&h, c.x0), RED));
    assert!(near(at(&h, c.x0 + 3), RED));
    for x in c.x0 + 4..c.x1 - 5 {
        assert!(near(at(&h, x), GREEN), "tiled middle at {x}");
    }
    assert!(near(at(&h, c.x1 - 5), BLUE));
    assert!(near(at(&h, c.x1 - 1), BLUE));
}

#[test]
fn imgbtn_press_and_check() {
    let (mut h, b) = scene(Mode::Light);
    set(&mut h, b, S::Released, RED);
    set(&mut h, b, S::Pressed, GREEN);
    set(&mut h, b, S::CheckedReleased, BLUE);
    h.engine_mut().set_flag(b, ObjFlags::CHECKABLE, true);
    h.run_until_idle();
    let p: Point = h.engine().coords(b).center();
    h.press(p);
    h.advance(twine_core::Duration::ms(40));
    assert!(near(h.pixel(p.x as u32, p.y as u32), GREEN));
    h.release();
    assert!(state(&h, b).contains(State::CHECKED));
    assert!(near(center_color(&mut h, b), BLUE));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn snapshot_imgbtn() {
    static NONE: Option<ImageSource> = None;
    let _ = &NONE;
    for (name, st) in [
        ("released", S::Released),
        ("pressed", S::Pressed),
        ("checked", S::CheckedReleased),
    ] {
        let (mut h, b) = scene(Mode::Light);
        let edge = |c| solid_image(6, 24, c);
        let mid = |c| solid_image(2, 24, c);
        let dark = Color::new(0x19, 0x76, 0xD2);
        with(&mut h, b, |w: &mut ImageButton, cx| {
            w.set_src(
                cx,
                S::Released,
                Some(edge(dark)),
                Some(mid(BLUE)),
                Some(edge(dark)),
            );
            w.set_src(
                cx,
                S::Pressed,
                Some(edge(Color::BLACK)),
                Some(mid(dark)),
                Some(edge(Color::BLACK)),
            );
            w.set_src(
                cx,
                S::CheckedReleased,
                Some(edge(Color::new(0xB7, 0x1C, 0x1C))),
                Some(mid(RED)),
                Some(edge(Color::new(0xB7, 0x1C, 0x1C))),
            );
        });
        h.engine_mut().set_width(b, 80);
        with(&mut h, b, |w: &mut ImageButton, cx| w.set_state(cx, st));
        h.run_until_idle();
        h.assert_snapshot(&format!("imgbtn_{name}"));
    }
}
