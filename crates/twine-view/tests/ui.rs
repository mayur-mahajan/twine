//! Compile-fail tests (`cargo test -p twine-view --test ui`): wrong value types are rejected
//! with the property traits' own messages; `#[component]` misuse (a missing required prop, a
//! wrong prop type, an unsupported signature, modifiers on a `View`-only component) is a
//! compile-time error naming the problem (`component_*.rs`).

// The expected messages list the implementors of `IntoText` / `IntoProp`; the
// `display-integrity` (and so `safety`) feature adds `Validity` texts to that list, so the
// references are those of the default features.
#[cfg(not(feature = "display-integrity"))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
