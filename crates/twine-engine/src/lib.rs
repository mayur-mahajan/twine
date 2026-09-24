//! # twine-engine
//!
//! The core of the Twine GUI library: the retained widget tree, the [`Widget`] trait, styles
//! attached to nodes, precise invalidation, displays with screens and layers, and the refresh
//! pipeline that turns dirty areas into pixels on a panel with as little work as possible.
//!
//! `twine-engine` sits above the renderer (`twine-render`), text, images and styles, and below
//! themes, widgets and the declarative view layer. It is fully usable on its own, imperatively
//! (LVGL-style); it does not depend on the reactive runtime.
//!
//! ## How a frame happens
//!
//! 1. Changes invalidate exactly the affected area: a node's coordinates plus its extra draw
//!    size (shadow, outline, transform), clipped by its ancestors (P2). Setters that do not
//!    change anything invalidate nothing (P3).
//! 2. Layout-affecting changes (sizes, positions, flex/grid properties, children, flags) mark
//!    the node dirty; [`Engine::update_layout`] (run by `step`) lays out only the dirty
//!    subtrees with `twine-layout`, moves unchanged subtrees by delta and invalidates the old
//!    and new areas of the nodes that actually moved.
//! 3. [`Engine::step`] refreshes each display at most once per
//!    [`EngineConfig::refr_period`]: dirty areas are merged, rounded to the display's alignment
//!    and split into chunks that fit the draw buffer.
//! 4. Each chunk is drawn starting from the topmost node that covers it opaquely (nodes hidden
//!    below are skipped), back to front, with opacity groups, transforms and blend modes
//!    rendered through layers.
//! 5. With two draw buffers the next chunk is rendered while the driver still transfers the
//!    previous one (DMA pipelining); framebuffer displays render into a back buffer and swap.
//! 6. Input devices are read first in each step: devices with an interrupt only after
//!    [`Engine::notify_input`] and while held, so an idle touch UI never wakes the CPU.
//! 7. Timers and animations run right after the inputs, on wall-clock time: a late frame
//!    jumps to the right value. Animations write node properties through the same setters as
//!    user code; while one plays `step` asks for the next frame, and once they end (or are
//!    paused) it asks for nothing ([`Engine::anim_start`], [`Engine::timer_add`], style
//!    transitions on state changes, [`Engine::load_screen_anim`]).
//! 8. Scrolling moves a container's children by its scroll offset without a layout pass and
//!    redraws only the container ([`Engine::scroll_by`], drag / throw / snap by pointers,
//!    scrollbars in `Part::Scrollbar`).
//! 9. When nothing is dirty, `step` returns [`Wake::Idle`] and does nothing at all (P1). A
//!    steady-state frame performs no heap allocation (P4).
//!
//! | Module concept | Items |
//! |----------------|-------|
//! | tree | [`Tree`], [`Node`], [`NodeId`], iterators, [`Tree::check_invariants`] |
//! | widgets | [`Widget`], [`WidgetClass`], [`Obj`], [`MeasureCx`], [`WidgetCx`], [`DrawCx`] |
//! | styles | [`Engine::add_style`], [`Engine::set_local_prop`], [`Engine::style_prop`], [`MainStyle`] |
//! | displays | [`Engine::add_display`], [`Engine::add_framebuffer_display`], [`BufferMode`], [`DisplayId`] |
//! | refresh | [`Engine::step`], [`Wake`], [`RefreshStats`], [`PerfMonitor`], [`InvalidateReason`] |
//! | events | [`Engine::send_event`], [`Engine::add_event_handler`], [`Event`], [`EventCode`], [`EventCx`] |
//! | input | [`Engine::add_input`], [`Engine::notify_input`], [`Engine::read_inputs`], [`InputId`] |
//! | layout | [`Engine::set_size`], [`Engine::set_pos`], [`Engine::align`], [`Engine::align_to`], [`Engine::set_flex_flow`], [`Engine::set_grid_cell`], [`Engine::update_layout`], [`LayoutStats`] |
//! | animation | [`Engine::anim_start`], [`Engine::anim_start_fn`], [`Engine::timer_add`], [`Engine::load_screen_anim`], [`ScreenAnim`], [`Deferred`] |
//! | focus | [`Engine::create_group`], [`Engine::group_add`], [`Engine::focus_next`], [`gridnav`] |
//! | scrolling | [`Engine::scroll_by`], [`Engine::scroll_to`], [`Engine::scroll_to_view`], [`Engine::scroll_top`], [`Engine::set_scroll_snap_x`], [`Engine::update_snap`], [`Engine::scrollbar_areas`], [`ScrollbarMode`] |
//!
//! ## Features
//!
//! - `std`, `log`, `defmt`: as in every Twine crate.
//! - `debug-checks`: tree invariants after every mutation, style cache verification, the
//!   invalidation log and the render hook (tests enable it).
//! - `perf-monitor`: the on-screen performance overlay.
//! - `test-ids`: keep node test ids in release builds.
//! - `color-*`: pixel formats compiled into the renderer.
//!
//! ## Example
//!
//! ```
//! use twine_core::{Color, ColorFormat, Instant, Opa, Rect};
//! use twine_engine::{BufferMode, Engine, EngineConfig, Obj, Wake};
//! use twine_hal::DisplayInfo;
//! use twine_style::{Selector, StyleProp};
//! # use twine_hal::{DisplayDriver, DrawBufferMem};
//! # struct Panel(Option<DrawBufferMem>);
//! # impl DisplayDriver for Panel {
//! #     type Error = ();
//! #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
//! #     fn begin_flush(&mut self, _: Rect, b: DrawBufferMem) -> Result<(), ()> { self.0 = Some(b); Ok(()) }
//! #     fn poll_flush(&mut self) -> Option<DrawBufferMem> { self.0.take() }
//! # }
//!
//! let mut engine = Engine::new(EngineConfig::default()).unwrap();
//! let buf: &'static mut [u8] = Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice());
//! let display = engine.add_display(Panel(None), BufferMode::partial_single(buf)).unwrap();
//! let screen = engine.active_screen(display).unwrap();
//! let boxed = engine.create(screen, Box::new(Obj)).unwrap();
//! engine.set_pos(boxed, 8, 8); // laid out by the next `step`
//! engine.set_size(boxed, 20, 10);
//! engine.set_local_prop(boxed, Selector::MAIN, StyleProp::BgColor(Color::RED));
//! engine.set_local_prop(boxed, Selector::MAIN, StyleProp::BgOpa(Opa::COVER));
//! assert!(matches!(engine.step(Instant::from_millis(0)), Wake::Idle)); // rendered, nothing left
//! assert_eq!(engine.step(Instant::from_millis(100)), Wake::Idle);      // idle: no work at all
//! ```
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

extern crate alloc;
#[cfg(any(test, feature = "std"))]
extern crate std;

mod anim;
mod config;
mod display;
mod draw_cx;
mod draw_dsc;
mod engine;
mod error;
mod event;
mod flags;
pub mod gridnav;
mod group;
mod handlers;
mod id;
mod input;
mod invalidate;
mod layout;
mod obj;
#[cfg(feature = "perf-monitor")]
mod perf_overlay;
mod refresh;
mod reparent;
mod screen_anim;
mod scroll;
mod scroll_drag;
mod scrollbar;
mod stats;
mod style_cache;
mod style_list;
mod theme_hook;
mod transition;
mod tree;
mod wake;
mod widget;

pub use anim::{Deferred, defer};
pub use config::EngineConfig;
pub use display::{BufferMode, MAX_DISPLAYS};
pub use draw_cx::DrawCx;
pub use draw_dsc::RectStyle;
pub use engine::Engine;
pub use error::{EngineError, InvariantError};
pub use event::{Event, EventCode, EventCx, EventParam, EventResult};
pub use flags::{LayoutDirty, ObjFlags, flag_names};
pub use gridnav::GridnavCtrl;
pub use group::{EdgeCb, FocusCb, GroupId, MAX_GROUPS, RefocusPolicy};
pub use handlers::{EventFilter, Handler, HandlerId};
pub use id::{DisplayId, NodeId, NodeIdFmt, fmt_node_id};
pub use input::{InputId, MAX_INPUTS};
pub use invalidate::InvalidateReason;
pub use layout::{LayoutStats, MAX_LAYOUT_ITERATIONS};
pub use obj::{OBJ_CLASS, OBJ_FLAGS, Obj};
#[cfg(feature = "perf-monitor")]
pub use perf_overlay::{PERF_OVERLAY_CLASS, PerfOverlay};
pub use refresh::RefreshOutcome;
pub use screen_anim::{ScreenAnim, ScreenLoad};
pub use scroll::{SCROLL_ANIM_TIME_MAX, SCROLL_ANIM_TIME_MIN, SCROLL_ELASTIC_FACTOR};
pub use stats::{MemInfo, PerfMonitor, RefreshStats};
pub use style_cache::MainStyle;
pub use style_list::StyleList;
pub use theme_hook::{ThemeCx, ThemeHook};
pub use tree::{Ancestors, Children, ChildrenRev, Descendants, Node, Tree};
pub use twine_anim::{Anim, AnimId, AnimProp, Easing, Repeat, TimerId};
pub use twine_hal::{InputKind, Key};
pub use twine_style::State;
pub use twine_style::{Dir, ScrollSnap, ScrollbarMode};
pub use wake::Wake;
pub use widget::{
    AsAny, Editable, GroupDef, MeasureCx, Widget, WidgetClass, WidgetCx, default_covers, default_hit_test,
};
