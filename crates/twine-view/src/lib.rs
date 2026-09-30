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
//! | [`IntoProp`], [`Prop`], [`PropValue`] ([`prop_value!`]), [`IntoText`], [`text!`], [`IntoModel`], [`ModelValue`] | constant, signal, memo or closure property values (your own types with one line); zero-allocation text; two-way bindings |
//! | [`ViewExt`] | every style, flag, event and identity modifier |
//! | [`column()`], [`row`], [`grid`], [`container`], [`stack`], [`spacer`], [`scroll_view`] | layout containers |
//! | [`label`], [`button`], [`image`], [`image_button`], [`animimg`] | core widget views |
//! | [`bar`], [`slider`], [`switch`], [`checkbox`], [`arc`], [`led`], [`line()`], [`spinner`] | basic controls (value widgets take an [`IntoModel`]: two-way with a signal) |
//! | [`textarea`], [`keyboard`], [`spinbox`], [`buttonmatrix`], [`spangroup`] + [`span`] | text and number entry, rich text |
//! | [`dropdown`], [`roller`] (options: [`IntoOptions`]) | selection widgets (`twine-widgets-ext`) |
//! | [`list`], [`menu`], [`tabview`], [`tileview`], [`window`], [`msgbox`] | containers (`twine-widgets-ext`) |
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
//! **The rule:** the engine is available inside event handlers, effects, timers (and
//! animation and channel-message callbacks) and while building; elsewhere use the [`Ui`]
//! methods or post a message. Hooks that create something (timers, tweens, animations,
//! modals) and navigation defer their engine work to the next update, which is always
//! correct. Calls that need the engine *now* — [`NodeRef::with_mut`], the [`AnimController`]
//! methods, [`ThemeHandle::set`] — return `None` / do nothing without it and, in debug builds
//! (`debug_assertions`), log `warn!` once per call site (compiled out in release builds; see
//! [`EngineAccess`] § Diagnostics).
//!
//! ```
//! use twine_view::prelude::*;
//! use twine_widgets::label::Label;
//!
//! static STATUS: Channel<&'static str, 4> = Channel::new();
//!
//! fn app(cx: Scope) -> impl View {
//!     let r: NodeRef<Label> = cx.node_ref();
//!     // A message handler runs inside `Ui::update`: the engine is lent.
//!     cx.on_message(&STATUS, move |s| {
//!         r.with_mut(|l: &mut Label, cx| l.set_text(cx, s));
//!     });
//!     label("idle").node_ref(r)
//! }
//! # let _ = app;
//! // From another task or an ISR: post a message rather than calling `r.with_mut` there.
//! let _ = STATUS.try_send("busy");
//! ```
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
#[cfg(feature = "async")]
mod async_ui;
mod bind;
mod build;
mod containers;
mod error;
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
#[cfg(feature = "async")]
pub use async_ui::{AsyncUi, AsyncUiBuilder, NoInputWait};
pub use build::{BuildCx, BuildOp, WidgetView, widget_view};
pub use containers::{Container, Flex, Grid, column, container, flex, grid, row, scroll_view, spacer, stack};
pub use error::{BuildError, BuildFailure, UiError};
pub use flow::{Dynamic, ForEach, VirtualList, When, WhenElse, dynamic, for_each, virtual_list, when};
pub use hooks::{AnimController, ScopeExt, ThemeHandle, use_theme};
pub use model::{IntoModel, Model, ModelValue, bind_model, event_value};
pub use modifiers::ViewExt;
pub use nav::{ModalHandle, Navigator, navigator, use_navigator};
pub use node_ref::NodeRef;
pub use prop::{IntoGridSpan, IntoIcon, IntoProp, Prop, PropValue};
pub use text::{IntoOptions, IntoText, TextFn, TextProp};
pub use ui::{DisplaySetup, Framebuffer, Partial, Ui, UiBuilder, UiCore};
pub use view::{AnyView, IntoAnyView, View, ViewSeq};
pub use widgets::{
    Btn, MenuPageRef, MenuPageView, SpanView, TabView, TilePos, TileView, animimg, arc, bar, btn, button,
    buttonmatrix, checkbox, dropdown, image, image_button, keyboard, label, led, line, line_static, list,
    list_button, list_text, menu, menu_cont, menu_page, menu_section, menu_separator, msgbox, roller, slider,
    span, spangroup, spinbox, spinner, switch, tab, tabview, textarea, tile, tileview, window, window_button,
};
#[cfg(feature = "vector")]
pub use widgets::{VectorCanvas, vector_canvas};

/// When `Ui::update` must be called again (defined by the engine, re-exported here).
pub use twine_engine::Wake;

/// Screen load animations (defined by the engine, re-exported here).
pub use twine_engine::{ScreenAnim, ScreenLoad};

#[doc(hidden)]
pub mod __private {
    //! Items used by the exported macros.
    pub use alloc::boxed::Box;
}
