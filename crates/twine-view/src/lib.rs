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
//! | [`WidgetView::bind_after_children`], [`bind_model`], [`on_value_changed`], [`event_value`] | the pieces of a value widget's view: a value applied after its other settings, two-way binding, `.on_change` (the built-in views use them; so can custom widget views outside this crate) |
//! | [`IntoProp`], [`Prop`], [`Icon`], [`IntoText`], [`text!`], [`IntoModel`] | constant, signal, memo or closure property values (any type, converted with `From`); zero-allocation text; two-way bindings |
//! | [`ViewExt`] | flag, per-part / per-state style ([`ViewExt::part`], [`ViewExt::on_state`]), style, event and identity modifiers |
//! | [`StyleExt`], [`StyleScope`] | one modifier per style property and shorthand (generated from the property table), on every view and in part/state scopes; colors, lengths, radii, opacities and fonts also take [design elements](twine_style::design) (`design::SURFACE`) |
//! | [`use_theme`], [`ThemeHandle`], [`IntoTheme`](twine_engine::IntoTheme) | switch the theme (a value or a shared `Rc`, the same parameter everywhere) or its mode (`set_mode`: light ↔ dark without rebuilding), read the mode and design element values (tracked) |
//! | [`use_motion`], [`MotionHandle`], [`UiBuilder::motion`] | the global motion preference ([`Motion`](twine_anim::Motion): full, reduced, none), honoured by tweens, animations, style transitions, screen loads and scroll animations |
//! | [`column()`], [`row`], [`grid`], [`container`], [`card`], [`stack`], [`spacer`], [`scroll_view`], [`Layout`] | layout containers |
//! | [`label`], [`button`], [`image`], [`image_button`], [`animimg`] | core widget views |
//! | [`bar`], [`slider`], [`switch`], [`checkbox`], [`arc`], [`led`], [`line()`], [`spinner`] | basic controls (value widgets take an [`IntoModel`]: two-way with a signal) |
//! | [`textarea`], [`keyboard`], [`spinbox`], [`buttonmatrix`], [`spangroup`] + [`span`] | text and number entry, rich text |
//! | [`dropdown`], [`roller`] (options: [`IntoOptions`]) | selection widgets (`twine-widgets-ext`) |
//! | [`list`], [`menu`], [`tabview`], [`tileview`], [`window`], [`msgbox`] | containers (`twine-widgets-ext`) |
//! | [`when`], [`dynamic`], [`for_each`], [`virtual_list`] | structural reactivity limited to one region |
//! | [`NodeRef`], [`ScopeExt`] | the imperative escape hatch, tweens and animations of any [`Interpolate`](twine_anim::Interpolate) value (timed by an [`AnimSpec`](twine_anim::AnimSpec)), timers, modals |
//! | [`Ui`], [`UiBuilder`], [`UiCore`] | the runtime: the update cycle, [`Wake`], [`StepBudget`] |
//! | [`UiBuilder::display`], [`Ui::mount_on`], [`DisplayBuilder`], [`Ui::display_mut`], [`DisplayMut`] | several displays in one `Ui`: each with its own buffers, inputs, theme and application, updated by the same loop |
//! | [`Ui::set_rotation`], [`Ui::set_brightness`], [`Ui::set_display_sleep`], [`UiBuilder::display_commands`], [`DisplayCmd`](twine_engine::DisplayCmd) | display control at run time (rotation, brightness, sleep / wake), also from other tasks and interrupts through a command channel |
//! | [`BuildReport`], [`BuildFault`], [`BuildError`] | widgets that could not be created while mounting: the per-mount report and the error that fails the mount |
//! | [`AppConfig`], [`UiBuilder::app_config`], [`UiCore::mount_configured`] | the application's configuration (engine, theme, motion, rotation, budgets, fault hook), one value for firmware, simulator and tests |
//! | [`typestate`] | the builder's required parts (runtime, clock / platform, draw buffers), checked at compile time |
//! | [`run::blocking`], [`run::blocking_with`], [`run::LoopEvent`] | the run loop over a [`Platform`](twine_hal::Platform) (bare metal, an RTOS task, a host thread) |
//! | [`draw_buffers!`], [`DrawBuffers`] | `static` draw buffers sized at compile time and taken once ([`UiBuilder::buffers`]) |
//! | [`LayerBuffer`] | a `static` layer buffer taken once ([`UiBuilder::layer_buf`]), so the engine's layer buffer stays out of the heap |
//! | [`Ui::memory_report`], [`MemoryReport`] | what the memory is used for: engine (tree, caches, layer and draw buffers), reactive runtime, command queues, waker pool |
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
//! modals) and navigation defer their engine work to the next update. Engine side effects —
//! the cleanups of a scope disposed outside an update (stopping its animations, removing its
//! timers, closing its modals), the [`AnimController`] methods, [`ThemeHandle::set`],
//! [`ThemeHandle::set_mode`] and [`MotionHandle::set`] — are
//! queued on the `Ui` that owns the scope and applied at the start of its next update (a
//! bounded, allocation-free queue: see [`UiCore`] § Engine commands). The queue is bounded:
//! when it is full a command is dropped and the next update raises a
//! [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity) record with code
//! [`CapacityFault::EngineQueue`] (see [`Ui::take_faults`]); it never panics. Calls
//! that must return what the engine holds — [`NodeRef::with_mut`],
//! [`AnimController::is_playing`] — return `None` / `false` without it and, in debug builds
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
//! ## The runtime token
//!
//! The reactive runtime belongs to the execution context (UI thread or task) that took its
//! [`Runtime`](twine_reactive::Runtime) token: `Runtime::take()` once, early in `main`, and
//! hand it to the builder ([`UiBuilder::runtime`]); the `Ui` keeps it. The token is `!Send`,
//! so the compiler keeps the UI in its context — no `unsafe`, also without `std` (where the
//! runtime lives in a `static`). With `std` every thread has its own runtime, and
//! `Runtime::current_thread()` gives a test or host tool its thread's token at any time.
//!
//! ## Features
//!
//! `std`, `log`, `defmt`: as in every Twine crate.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod access;
#[cfg(feature = "async")]
mod async_ui;
mod bind;
mod build;
mod config;
mod containers;
mod display;
mod draw_buffers;
mod engine_queue;
mod error;
pub mod flow;
mod hooks;
mod memory;
mod model;
mod modifiers;
mod nav;
mod node_ref;
pub mod prelude;
mod prop;
pub mod run;
mod style_ext;
pub mod text;
pub mod typestate;
mod ui;
mod view;
pub mod widgets;

pub use access::{EffectCx, EngineAccess};
#[cfg(feature = "async")]
pub use async_ui::{AsyncUi, AsyncUiBuilder, NoInputWait};
pub use build::{BuildCx, BuildOp, WidgetView, widget_view};
pub use config::AppConfig;
pub use containers::{
    Container, Flex, Grid, Layout, card, column, container, flex, grid, row, scroll_view, spacer, stack,
};
pub use display::{DisplayBuilder, DisplayMut};
pub use draw_buffers::{DrawBuffers, LayerBuffer};
pub use engine_queue::DEFAULT_ENGINE_QUEUE_CAPACITY;
pub use error::{BuildError, BuildFailure, BuildFault, BuildReport, CapacityFault, UiError};
pub use flow::{Dynamic, ForEach, VirtualList, When, WhenElse, dynamic, for_each, virtual_list, when};
pub use hooks::{AnimController, MotionHandle, ScopeExt, ThemeHandle, use_motion, use_theme};
pub use memory::MemoryReport;
pub use model::{IntoModel, Model, bind_model, event_value, on_value_changed};
pub use modifiers::ViewExt;
pub use nav::{ModalHandle, Navigator, navigator, use_navigator};
pub use node_ref::NodeRef;
pub use prop::{Icon, IntoProp, Prop, marker};
pub use style_ext::{StyleExt, StyleScope};
pub use text::{IntoOptions, IntoText, TextFn, TextProp};
pub use typestate::{HasBuffers, HasClock, HasRuntime, NoBuffers, NoClock, NoRuntime};
#[cfg(feature = "async")]
pub use typestate::{HasPlatform, NoPlatform};
pub use ui::DEFAULT_MESSAGES_PER_CHANNEL;
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

/// How much one budgeted update may render ([`Ui::update_budgeted`]; defined by the engine,
/// re-exported here).
pub use twine_engine::StepBudget;

/// Screen load animations (defined by the engine, re-exported here).
pub use twine_engine::{ScreenAnim, ScreenLoad};

#[doc(hidden)]
pub mod __private {
    //! Items used by the exported macros.
    pub use alloc::boxed::Box;
    pub use twine_core::ColorFormat;
    pub use twine_hal::{DrawBuffer, buffer_bytes};
    pub use twine_reactive::TakeOnce;
}
