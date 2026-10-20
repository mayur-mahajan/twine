//! `MissingContext::panic` does not exist in the `safety` profile.
use twine::prelude::*;

#[allow(dead_code)]
fn lookup(cx: Scope) -> u8 {
    match cx.use_context::<u8>() {
        Ok(v) => v,
        Err(e) => e.panic(),
    }
}

fn main() {}
