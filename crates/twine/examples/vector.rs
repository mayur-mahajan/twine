//! `cargo xtask sim vector`: vector graphics in four tabs — shapes and fill rules, gradients
//! (linear, radial, focal, spreads), strokes (joins, caps, dashes) and SVG icons with a
//! rotating, pulsing star. F2 shows that the animation redraws only the star's bounds.

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(480, 320).title("vector").scale(2),
        twine_demos::vector::app,
    );
}
