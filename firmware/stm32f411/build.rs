//! Adds the cortex-m-rt and defmt linker scripts (`memory.x` comes from embassy-stm32's
//! `memory-x` feature).

fn main() {
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
