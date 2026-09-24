//! Compile-fail tests of `style!` (`cargo test -p twine-style --test ui`).

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
