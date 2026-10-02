//! Compile-fail tests of the property traits (`cargo test -p twine-view --test ui`): wrong
//! value types are rejected with the traits' own messages.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
