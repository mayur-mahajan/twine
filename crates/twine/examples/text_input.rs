//! `cargo xtask sim text_input`: a form with a name, a password, notes, a quantity and a live
//! rich-text preview. Tap a field to open the on-screen keyboard (OK or the close key hides
//! it), or type on the PC keyboard; the preview follows the name as you type. F2 shows that
//! only the field, its cursor and the preview redraw.

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240).title("Text input").scale(3),
        twine_demos::text_input::app,
    );
}
