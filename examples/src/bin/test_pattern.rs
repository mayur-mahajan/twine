//! `cargo xtask sim test_pattern`: animated color bars and a bouncing square.
//!
//! Eight vertical color bars (white, yellow, cyan, green, magenta, red, blue, black) scroll
//! 1 px per frame; a 32×32 white square bounces diagonally. Useful to check the simulator
//! window, the emulated panel formats (`TWINE_SIM_FORMAT=rgb565swapped|rgb888|l8|i1`), bus speed
//! emulation (`TWINE_SIM_BUS_HZ=10000000` ≈ 8 fps) and input logging
//! (`RUST_LOG=twine::sim=debug`). F1 lists the hotkeys, F9 saves a screenshot.

use twine_core::Color;
use twine_examples::framebuffer::Frame;
use twine_sim::{SimConfig, show_framebuffer};

const W: u32 = 320;
const H: u32 = 240;
const SQUARE: u32 = 32;
const BARS: [Color; 8] = [
    Color::WHITE,
    Color::YELLOW,
    Color::CYAN,
    Color::GREEN,
    Color::MAGENTA,
    Color::RED,
    Color::BLUE,
    Color::BLACK,
];

/// Position on a 0..=`max` triangle wave moving 2 px per frame.
fn bounce(frame: u32, max: u32) -> u32 {
    let period = 2 * max;
    let t = (frame * 2) % period;
    if t <= max { t } else { period - t }
}

fn main() {
    let cfg = SimConfig::new(W as u16, H as u16)
        .title("twine test pattern")
        .scale(2);
    show_framebuffer(cfg, |fb, format, frame| {
        let mut f = Frame::new(fb, format, W);
        let bar_w = W / BARS.len() as u32;
        for x in 0..W {
            let c = BARS[(((x + frame) % W) / bar_w) as usize % BARS.len()];
            for y in 0..H {
                f.put(x, y, c);
            }
        }
        let (sx, sy) = (bounce(frame, W - SQUARE), bounce(frame + 37, H - SQUARE));
        f.fill_rect(sx, sy, sx + SQUARE, sy + SQUARE, Color::WHITE);
    });
}
