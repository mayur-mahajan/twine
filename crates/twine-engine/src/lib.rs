//! Widget tree, events, input, scrolling, invalidation and refresh pipeline.
//!
//! Part of the Twine GUI library. See `docs/design/01-architecture.md`.
#![no_std]

#[allow(unused_extern_crates)] // remove the allow once the crate uses `alloc`
extern crate alloc;
