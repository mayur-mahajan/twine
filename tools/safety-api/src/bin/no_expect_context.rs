//! `Scope::expect_context` does not exist in the `safety` profile.
use twine::prelude::*;

#[allow(dead_code)]
fn lookup(cx: Scope) -> u8 {
    cx.expect_context::<u8>()
}

fn main() {}
