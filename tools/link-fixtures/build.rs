//! Links the fixtures with `link.x` (a bare memory map: the images are linked, never run).

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    println!("cargo:rustc-link-search={dir}");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=link.x");
}
