//! Guides: walkthroughs of larger tasks, with compiled and tested examples.
//!
//! This module holds documentation only: it has no items, so it adds nothing to any build.
//!
//! | Guide | What it covers |
//! |-------|----------------|
//! | [`app_state`] | application state for large apps: the store pattern (a `Copy` struct of signals and memos with action methods, provided as context), scoped stores and shadowing, testing with `TestUi`, stores fed by ports, lookup cost and the `use_*` / `expect_*` failure convention |
//! | [`custom_widgets`] | writing a widget outside Twine: class and parts, drawing, input, theming with design elements and modes, a typed view, animations and motion, tests; components composing views (`#[component]` or plain functions) |

#[doc = include_str!("custom_widgets.md")]
pub mod custom_widgets {}

#[doc = include_str!("app_state.md")]
pub mod app_state {}
