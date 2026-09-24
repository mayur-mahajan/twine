//! `cargo xtask sim hello`: the smallest declarative app — one label. Idle: 0 % CPU.

use twine::prelude::*;
use twine_sim::SimConfig;

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("hello"), |_cx| {
        label("Hello, twine!")
    });
}
