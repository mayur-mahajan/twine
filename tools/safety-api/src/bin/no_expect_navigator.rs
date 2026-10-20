//! `expect_navigator` does not exist in the `safety` profile.
use twine::prelude::*;

#[allow(dead_code)]
fn page(cx: Scope) -> Navigator<u8> {
    twine::view::expect_navigator::<u8>(cx)
}

fn main() {}
