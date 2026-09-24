//! `cargo xtask sim engine_boxes`: the engine's refresh pipeline on an imperative scene.
//!
//! A gradient header, a card with shadow, radius and border holding six styled boxes, a 50 %
//! opacity group, a rotated box and an orange "player" ball that the arrow keys move by 4 px.
//! Only the areas that change are redrawn: press F2 and move the ball to see exactly the old
//! and new ball areas flash (the ball is the focused node of the simulator's default group
//! and moves in its `Key` event handler); F4 shows FPS / CPU / render / flush times, F3 outlines every node,
//! F8 dumps the tree.
//!
//! Animation: a small purple box pulses its opacity forever (playback, infinite repeat); with
//! F2 only its area refreshes. F5 slows time down ×0.25, F6 pauses it, F7 steps 16 ms while
//! paused. `x` deletes the pulsing box: nothing animates any more and the CPU goes idle. `n`
//! slides to a second screen (`ScreenAnim::MoveLeft`, 300 ms), `b` slides back (`MoveRight`).
//!
//! Options (after `--`):
//!
//! - `--bus-hz <bits/s>`: emulate the SPI bus speed (e.g. `40000000`); with two draw buffers
//!   the next chunk renders while the previous one is transferred
//!   (`RUST_LOG=twine::refresh=trace` logs `wait=…us` per chunk).
//! - `--buffers single|double|full|direct`: the buffer mode (partial single / double buffers,
//!   double framebuffer with area sync, single framebuffer rendered in place).
//! - `--rotation 0|90|180|270`: software rotation (partial modes; the window shows the physical
//!   panel).
//! - `--format rgb565|rgb565swapped|rgb888|xrgb8888|l8|i1`: the emulated panel format.

use std::cell::Cell;
use std::rc::Rc;
use twine_assets::fonts::MONTSERRAT_14;

use twine_core::{Color, ColorFormat, Duration, Opa, Rect, Rotation};
use twine_engine::{Anim, AnimProp, Engine, EventCode, EventFilter, EventResult, NodeId, Repeat, ScreenAnim};
use twine_examples::scenes::{child_box, engine_boxes};
use twine_hal::{BufferSpec, Key};
use twine_sim::{SimConfig, run_engine};
use twine_style::{Part, PropId, StyleProp};

/// Command line options.
#[derive(Debug, Clone, Copy)]
struct Options {
    bus_hz: Option<u32>,
    buffers: BufferSpec,
    rotation: Rotation,
    format: Option<ColorFormat>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut o = Options {
        bus_hz: None,
        buffers: BufferSpec::PartialDouble { rows: 40 },
        rotation: Rotation::Deg0,
        format: None,
    };
    let mut args = args;
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--bus-hz" => o.bus_hz = Some(value()?.parse().map_err(|e| format!("--bus-hz: {e}"))?),
            "--buffers" => {
                o.buffers = match value()?.as_str() {
                    "single" => BufferSpec::PartialSingle { rows: 40 },
                    "double" => BufferSpec::PartialDouble { rows: 40 },
                    "full" => BufferSpec::Full,
                    "direct" => BufferSpec::Direct,
                    v => {
                        return Err(format!(
                            "--buffers: unknown mode `{v}` (single|double|full|direct)"
                        ));
                    }
                };
            }
            "--rotation" => {
                o.rotation = match value()?.as_str() {
                    "0" => Rotation::Deg0,
                    "90" => Rotation::Deg90,
                    "180" => Rotation::Deg180,
                    "270" => Rotation::Deg270,
                    v => return Err(format!("--rotation: `{v}` is not 0, 90, 180 or 270")),
                };
            }
            "--format" => {
                let v = value()?;
                o.format = Some(
                    twine_sim::config::parse_format(&v).ok_or_else(|| format!("--format: unknown `{v}`"))?,
                );
            }
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(o)
}

/// Moves the player by 4 px per arrow key (its `X`/`Y` position; the layout pass moves it) and
/// logs the key with the last frame's statistics.
fn on_key(e: &mut Engine, player: NodeId, key: Key) {
    let (dx, dy) = match key {
        Key::Left => (-4, 0),
        Key::Right => (4, 0),
        Key::Up => (0, -4),
        Key::Down => (0, 4),
        _ => return,
    };
    let x = e.style_i32(player, Part::Main, PropId::X) + dx;
    let y = e.style_i32(player, Part::Main, PropId::Y) + dy;
    e.set_pos(player, x, y);
    let d = e.default_display().expect("display");
    twine_core::info!(target: "twine::sim", "{}: player at ({}, {}); previous frame {:?}", key, x, y, e.last_stats(d));
}

/// The pulsing box: opacity 255 → 40 and back, forever.
fn pulse(e: &mut Engine, screen: NodeId) -> NodeId {
    let b = child_box(
        e,
        screen,
        Rect::from_xywh(272, 204, 28, 24),
        &[
            StyleProp::BgColor(Color::hex(0x8E_24_AA)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(6),
        ],
    );
    e.anim_start(
        b,
        AnimProp::Opa,
        Anim::new(255, 40)
            .duration(Duration::ms(700))
            .playback(Duration::ms(700))
            .repeat(Repeat::Infinite),
    );
    b
}

/// The second screen (`n` / `b`): a dark background with three boxes.
fn second_screen(e: &mut Engine) -> NodeId {
    let d = e.default_display().expect("display");
    let s = e.create_screen(d).expect("screen");
    e.set_local_prop(
        s,
        twine_style::Selector::MAIN,
        StyleProp::BgColor(Color::hex(0x26_32_38)),
    );
    e.set_local_prop(s, twine_style::Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
    for (i, c) in [0x4F_C3_F7, 0xAE_D5_81, 0xFF_B7_4D].into_iter().enumerate() {
        child_box(
            e,
            s,
            Rect::from_xywh(40 + i as i32 * 90, 80, 60, 80),
            &[
                StyleProp::BgColor(Color::hex(c)),
                StyleProp::BgOpa(Opa::COVER),
                StyleProp::Radius(10),
            ],
        );
    }
    s
}

fn main() {
    twine_sim::init_logging();
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("engine_boxes: {e}");
            std::process::exit(2);
        }
    };
    let mut cfg = SimConfig::new(320, 240)
        .title("engine_boxes")
        .scale(2)
        .buffers(opts.buffers)
        .bus_hz(opts.bus_hz)
        .rotation(opts.rotation)
        .hw_rotation(opts.rotation == Rotation::Deg0);
    if let Some(f) = opts.format {
        cfg = cfg.format(f);
    }
    // Filled by the setup: the pulsing box and the two screens.
    let ids: Rc<Cell<Option<(NodeId, NodeId, NodeId)>>> = Rc::default();
    let keys = ids.clone();
    let cfg = cfg.on_raw_key(move |e, k| {
        let Some((pulse_box, first, second)) = keys.get() else { return };
        match k {
            Key::Char('x') if e.tree().contains(pulse_box) => {
                e.delete(pulse_box).expect("pulse box exists");
                twine_core::info!(target: "twine::sim", "pulsing box deleted: {} animations left", e.anim_count());
            }
            Key::Char('n') => e.load_screen_anim(second, ScreenAnim::MoveLeft(Duration::ms(300))),
            Key::Char('b') => e.load_screen_anim(first, ScreenAnim::MoveRight(Duration::ms(300))),
            _ => {}
        }
    });
    run_engine(cfg, move |e| {
        e.config_mut().default_font = Some(&MONTSERRAT_14);
        let scene = engine_boxes(e);
        let player = scene.player;
        let pulse_box = pulse(e, scene.screen);
        let second = second_screen(e);
        ids.set(Some((pulse_box, scene.screen, second)));
        // The simulator's keypad sends keys to the focused node of the default group.
        if let Some(g) = e.default_group() {
            e.group_add(g, player);
        }
        e.add_event_handler(player, EventFilter::Code(EventCode::Key), |cx, ev| {
            if let Some(k) = ev.key() {
                let id = cx.node();
                on_key(cx.engine_mut(), id, k);
            }
            EventResult::Continue
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> impl Iterator<Item = String> {
        s.split_whitespace()
            .map(String::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_options() {
        let o = parse_args(args("--bus-hz 40000000 --buffers full --rotation 90 --format i1")).unwrap();
        assert_eq!(o.bus_hz, Some(40_000_000));
        assert_eq!(o.buffers, BufferSpec::Full);
        assert_eq!(o.rotation, Rotation::Deg90);
        assert_eq!(o.format, Some(ColorFormat::I1));
        assert!(parse_args(args("--buffers triple")).is_err());
        assert!(parse_args(args("--rotation")).is_err());
        assert!(parse_args(args("--nope")).is_err());
    }
}
