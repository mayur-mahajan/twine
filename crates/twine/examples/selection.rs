//! `cargo xtask sim selection`: the selection and container widgets. Pick a city in the
//! dropdown and the roller spins to the same city; swipe between the tabs; open the menu
//! pages in the Lists tab; swipe the tiles; the "i" button opens a modal message box. The PC
//! keyboard (Tab, arrows, Enter, Esc) and the mouse wheel (encoder) work everywhere.

use twine_sim::SimConfig;

fn main() {
    twine_sim::run(
        SimConfig::new(320, 240).title("Selection").scale(3),
        twine_demos::selection::app,
    );
}
