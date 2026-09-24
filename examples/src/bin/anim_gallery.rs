//! `cargo xtask sim anim_gallery`: animations, style transitions and screen load animations.
//!
//! Screen 1: one row per easing curve (linear, ease-in, ease-out, ease-in-out, overshoot,
//! bounce, step, a custom cubic bezier and a custom function), each with a 20 × 20 box going
//! back and forth (1.5 s each way, forever); the colored label of each row is its legend (also
//! printed to the terminal). Below: a box pulsing its opacity, and a box whose pressed state
//! changes color and size through a style transition (press it).
//!
//! Keys: `1`–`9` and `a`–`f` switch between screen 1 and screen 2 with each of the 15 screen
//! load animations (printed when used); `space` pauses / resumes every animation; F5 slow
//! motion, F6 pause time, F7 single step while paused.

use std::cell::RefCell;
use std::rc::Rc;

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, Duration, Opa, Rect, Scale};
use twine_engine::{Anim, AnimId, AnimProp, Easing, Engine, NodeId, ObjFlags, Repeat, ScreenAnim, State};
use twine_examples::scenes::child_box;
use twine_examples::tile::tile;
use twine_hal::Key;
use twine_sim::{SimConfig, run_engine};
use twine_style::{Length, PropId, Selector, StyleProp, TransitionDsc};

/// Display size.
const W: i32 = 360;
const H: i32 = 300;
/// Row geometry: label column, track start, box size, row pitch.
const TRACK_X: i32 = 112;
const BOX: i32 = 20;
const ROW: i32 = 24;
/// One way of the back-and-forth movement.
const LEG: Duration = Duration::ms(1500);

/// Smoothstep, `t² (3 − 2t)`, as a custom easing function (progress in 1024ths).
fn smoothstep(t: u16) -> i32 {
    let t = i64::from(t);
    (t * t * (3 * 1024 - 2 * t) / (1024 * 1024)) as i32
}

/// The rows: name, easing, color.
fn rows() -> [(&'static str, Easing, u32); 9] {
    [
        ("linear", Easing::Linear, 0xE5_39_35),
        ("ease-in", Easing::EaseIn, 0xFB_8C_00),
        ("ease-out", Easing::EaseOut, 0xC0_CA_33),
        ("ease-in-out", Easing::EaseInOut, 0x43_A0_47),
        ("overshoot", Easing::Overshoot, 0x00_89_7B),
        ("bounce", Easing::Bounce, 0x1E_88_E5),
        ("step", Easing::Step, 0x3F_51_B5),
        ("bezier", Easing::CubicBezier(256, -400, 768, 1400), 0x8E_24_AA),
        ("smoothstep", Easing::Custom(smoothstep), 0x6D_4C_41),
    ]
}

/// The screen load animation of key `k` (`1`–`9`, `a`–`f`).
fn screen_anim_for(k: Key) -> Option<ScreenAnim> {
    let all = ScreenAnim::all(Duration::ms(500));
    let i = match k {
        Key::Char(c @ '1'..='9') => c as usize - '1' as usize,
        Key::Char(c @ 'a'..='f') => 9 + c as usize - 'a' as usize,
        _ => return None,
    };
    all.get(i).copied()
}

static PRESS_PROPS: [PropId; 3] = [PropId::BgColor, PropId::TransformScaleX, PropId::TransformScaleY];
static PRESS_TRANSITION: TransitionDsc =
    TransitionDsc::new(&PRESS_PROPS, Duration::ms(250), Easing::EaseInOut);

/// What the key handler needs.
struct Gallery {
    screens: [NodeId; 2],
    anims: Vec<AnimId>,
    paused: bool,
}

fn screen_bg(e: &mut Engine, s: NodeId, c: u32) {
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgColor(Color::hex(c)));
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
}

fn build(e: &mut Engine) -> Gallery {
    e.config_mut().default_font = Some(&MONTSERRAT_14);
    let d = e.default_display().expect("display");
    let s1 = e.active_screen(d).expect("screen");
    screen_bg(e, s1, 0xEC_EF_F1);
    let mut anims = Vec::new();
    println!("anim_gallery rows:");
    for (i, (name, easing, color)) in rows().into_iter().enumerate() {
        let y = 8 + i as i32 * ROW;
        println!("  row {} ({:06x}): {name}", i + 1, color);
        let label = tile(e, s1, name, Color::hex(color));
        e.set_pos(label, 8, y);
        e.set_size(label, TRACK_X - 16, BOX);
        // The track, then the moving box.
        child_box(
            e,
            s1,
            Rect::from_xywh(TRACK_X, y + BOX / 2 - 1, W - TRACK_X - 8, 2),
            &[
                StyleProp::BgColor(Color::hex(0xB0_BE_C5)),
                StyleProp::BgOpa(Opa::COVER),
            ],
        );
        let b = child_box(
            e,
            s1,
            Rect::from_xywh(TRACK_X, y, BOX, BOX),
            &[
                StyleProp::BgColor(Color::hex(color)),
                StyleProp::BgOpa(Opa::COVER),
                StyleProp::Radius(4),
            ],
        );
        anims.push(
            e.anim_start(
                b,
                AnimProp::X,
                Anim::new(TRACK_X, W - 8 - BOX)
                    .duration(LEG)
                    .easing(easing)
                    .playback(LEG)
                    .repeat(Repeat::Infinite),
            ),
        );
    }
    let bottom = 8 + 9 * ROW + 12;
    let pulse = child_box(
        e,
        s1,
        Rect::from_xywh(TRACK_X, bottom, 44, 44),
        &[
            StyleProp::BgColor(Color::hex(0xD8_1B_60)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(22),
        ],
    );
    anims.push(
        e.anim_start(
            pulse,
            AnimProp::Opa,
            Anim::new(255, 30)
                .duration(Duration::ms(900))
                .easing(Easing::EaseInOut)
                .playback(Duration::ms(900))
                .repeat(Repeat::Infinite),
        ),
    );
    let press = tile(e, s1, "press me", Color::hex(0x00_96_88));
    e.set_pos(press, TRACK_X + 80, bottom);
    e.set_size(press, 110, 44);
    e.set_flag(press, ObjFlags::CLICKABLE, true);
    for (sel, p) in [
        (Selector::MAIN, StyleProp::Transition(&PRESS_TRANSITION)),
        (Selector::MAIN, StyleProp::TransformPivotX(Length::Pct(50))),
        (Selector::MAIN, StyleProp::TransformPivotY(Length::Pct(50))),
        (
            Selector::state(State::PRESSED),
            StyleProp::BgColor(Color::hex(0xFF_6F_00)),
        ),
        (
            Selector::state(State::PRESSED),
            StyleProp::TransformScaleX(Scale(300)),
        ),
        (
            Selector::state(State::PRESSED),
            StyleProp::TransformScaleY(Scale(300)),
        ),
    ] {
        e.set_local_prop(press, sel, p);
    }

    let s2 = e.create_screen(d).expect("screen");
    screen_bg(e, s2, 0x26_32_38);
    let title = tile(e, s2, "screen 2: keys 1-9, a-f", Color::hex(0x5E_35_B1));
    e.set_pos(title, 20, 20);
    e.set_size(title, W - 40, 32);
    for (i, c) in [0x4F_C3_F7, 0xAE_D5_81, 0xFF_B7_4D].into_iter().enumerate() {
        child_box(
            e,
            s2,
            Rect::from_xywh(40 + i as i32 * 100, 100, 80, 140),
            &[
                StyleProp::BgColor(Color::hex(c)),
                StyleProp::BgOpa(Opa::COVER),
                StyleProp::Radius(12),
            ],
        );
    }
    println!("keys: 1-9, a-f screen load animations; space pause/resume; F5 slow motion, F6 pause, F7 step");
    Gallery {
        screens: [s1, s2],
        anims,
        paused: false,
    }
}

fn on_key(e: &mut Engine, g: &mut Gallery, k: Key) {
    if k == Key::Char(' ') {
        g.paused = !g.paused;
        for &a in &g.anims {
            if g.paused {
                e.anim_pause(a);
            } else {
                e.anim_resume(a);
            }
        }
        println!("animations {}", if g.paused { "paused" } else { "running" });
        return;
    }
    let Some(anim) = screen_anim_for(k) else { return };
    let d = e.default_display().expect("display");
    let target = if e.active_screen(d) == Some(g.screens[0]) {
        g.screens[1]
    } else {
        g.screens[0]
    };
    println!(
        "{k}: {} to screen {}",
        anim.name(),
        if target == g.screens[0] { 1 } else { 2 }
    );
    e.load_screen_anim(target, anim);
}

fn main() {
    twine_sim::init_logging();
    let gallery: Rc<RefCell<Option<Gallery>>> = Rc::default();
    let g = gallery.clone();
    let sim = SimConfig::new(W as u16, H as u16)
        .title("anim_gallery")
        .scale(2)
        .on_raw_key(move |e, k| {
            if let Some(g) = g.borrow_mut().as_mut() {
                on_key(e, g, k);
            }
        });
    run_engine(sim, move |e| *gallery.borrow_mut() = Some(build(e)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_all_15_screen_anims() {
        let keys = "123456789abcdef";
        let anims: Vec<ScreenAnim> = keys
            .chars()
            .map(|c| screen_anim_for(Key::Char(c)).unwrap())
            .collect();
        assert_eq!(anims, ScreenAnim::all(Duration::ms(500)));
        assert_eq!(screen_anim_for(Key::Char('g')), None);
    }

    #[test]
    fn smoothstep_endpoints() {
        assert_eq!(smoothstep(0), 0);
        assert_eq!(smoothstep(512), 512);
        assert_eq!(smoothstep(1024), 1024);
    }
}
