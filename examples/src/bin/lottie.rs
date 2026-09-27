//! `cargo xtask sim lottie`: three Lottie animations (loader, check mark, heart) with
//! play/pause buttons, loop switches and frame scrubbers. Pause all three and the UI idles;
//! F2 shows that a playing animation redraws only its own area, once per frame change.

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(480, 320).title("lottie").scale(2),
        twine_demos::lottie::app,
    );
}
