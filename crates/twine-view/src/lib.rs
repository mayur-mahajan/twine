//! # twine-view
//!
//! The declarative layer of the Twine GUI library: an application is a function
//! `fn app(cx: Scope) -> impl View` that runs **once** and describes a tree of widgets. [`Ui`]
//! builds that tree into the engine, and every dynamic value becomes a fine-grained binding —
//! a reactive effect that calls one idempotent widget setter — so a signal change runs exactly
//! the bindings that read it, invalidates exactly the pixels that change, and an idle UI does
//! no work at all ([`Wake::Idle`]).
//!
//! ```
//! use twine_view::prelude::*;
//!
//! fn counter(cx: Scope) -> impl View {
//!     let count = cx.signal(0u32);
//!     column((
//!         label(text!("Clicked {} times", count.get())).test_id("count"),
//!         button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
//!     ))
//!     .gap(12)
//!     .padding(16)
//! }
//! # let _ = counter;
//! ```
//!
//! ## Building blocks
//!
//! | Item | Role |
//! |------|------|
//! | [`View`], [`ViewSeq`], [`AnyView`] | descriptions consumed once by [`View::build`] |
//! | [`BuildCx`], [`WidgetView`], [`widget_view`] | building nodes; the generic widget builder every widget view wraps |
//! | [`IntoProp`], [`Prop`], [`IntoText`], [`text!`], [`IntoModel`] | constant, signal, memo or closure property values; zero-allocation text; two-way bindings |
//! | [`ViewExt`] | every style, flag, event and identity modifier |
//! | [`column()`], [`row`], [`grid`], [`container`], [`stack`], [`spacer`], [`scroll_view`] | layout containers |
//! | [`label`], [`button`], [`image`] | widget views |
//! | [`when`], [`dynamic`], [`for_each`], [`virtual_list`] | structural reactivity limited to one region |
//! | [`NodeRef`], [`ScopeExt`] | the imperative escape hatch, tweens, animations, timers, modals |
//! | [`Ui`], [`UiBuilder`], [`UiCore`] | the runtime: the update cycle and [`Wake`] |
//! | [`Navigator`], [`navigator`], [`ScreenAnim`] | a stack of screens with LVGL screen-load animations |
//!
//! ## How bindings reach the engine
//!
//! Views are built with the engine borrowed by the [`BuildCx`]. Later, handlers, effects and
//! timer callbacks reach it through [`EngineAccess`]: `Ui` lends the engine to the code it
//! runs (event handlers, effect flushes, timers, animations) and any code below borrows it
//! exclusively for a moment. A binding that runs without an engine (a signal written outside
//! `Ui::update`) re-queues itself for the next update.
//!
//! ## Allocation
//!
//! Views allocate while they are built (boxed closures, one reactive effect per dynamic
//! property) — once. Steady-state updates allocate nothing: text bindings format into the
//! label's own buffers ([`text!`]), setters that do not change a value do nothing.
//!
//! ## `no_std`
//!
//! Without the `std` feature the reactive runtime lives in a `static`; bind it to the UI
//! context with the `unsafe` [`UiBuilder::bind_to_current_context`] before building the
//! `Ui` (see its safety contract).
//!
//! ## Features
//!
//! `std`, `log`, `defmt`: as in every Twine crate.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod access;
mod bind;
mod build;
mod containers;
pub mod flow;
mod hooks;
mod model;
mod modifiers;
mod nav;
mod node_ref;
pub mod prelude;
mod prop;
pub mod text;
mod ui;
mod view;
pub mod widgets;

pub use access::{EffectCx, EngineAccess};
pub use build::{BuildCx, BuildOp, WidgetView, widget_view};
pub use containers::{Container, Flex, Grid, column, container, flex, grid, row, scroll_view, spacer, stack};
pub use flow::{Dynamic, ForEach, VirtualList, When, WhenElse, dynamic, for_each, virtual_list, when};
pub use hooks::{AnimController, ScopeExt, ThemeHandle, use_theme};
pub use model::{IntoModel, Model};
pub use modifiers::ViewExt;
pub use nav::{ModalHandle, Navigator, navigator, use_navigator};
pub use node_ref::NodeRef;
pub use prop::{IntoProp, Prop};
pub use text::{IntoText, TextFn, TextProp};
pub use ui::{DisplaySetup, Framebuffer, Partial, Ui, UiBuilder, UiCore};
pub use view::{AnyView, IntoAnyView, View, ViewSeq};
pub use widgets::{button, image, label};

/// When `Ui::update` must be called again (defined by the engine, re-exported here).
pub use twine_engine::Wake;

/// Screen load animations (defined by the engine, re-exported here).
pub use twine_engine::{ScreenAnim, ScreenLoad};

#[doc(hidden)]
pub mod __private {
    //! Items used by the exported macros.
    pub use alloc::boxed::Box;
}
