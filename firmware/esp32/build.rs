//! Links with esp-hal's `linkall.x` (last, as esp-hal requires).

fn main() {
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}
