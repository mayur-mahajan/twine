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
//! | widgets | [`Widget`], [`WidgetClass`] (identity, [`WidgetClass::base`], [`WidgetClass::is_a`], [`WidgetClass::lineage`], [`Lineage`], [`MAX_CLASS_DEPTH`]), [`Obj`], [`MeasureCx`], [`WidgetCx`], [`DrawCx`] |
//! | styles | [`Engine::add_style`], [`Engine::set_local_prop`], [`Engine::set_local_transition`], [`Engine::style_prop`], [`Engine::finish_style_value`], [`MainStyle`] |
//! | themes | [`ThemeHook`], [`IntoTheme`], [`Engine::set_theme`], [`Engine::set_theme_mode`], [`Engine::theme_mode`], [`Engine::theme_modes`], [`Engine::design_value`], [`Engine::resolve_design_value`], [`Engine::design_table`], [`Engine::design_epoch`] ([design elements](twine_style::design)) |
//! | displays | [`Engine::add_display`], [`Engine::add_framebuffer_display`], [`Engine::add_chunked_display`], [`BufferMode`], [`BufferSpec`], [`DisplayId`] |
//! | display control | [`Engine::set_rotation`], [`Engine::set_display_brightness`], [`Engine::set_display_sleep`], [`Engine::display_command`], [`DisplayCmd`], [`Engine::reserve_rotation`], [`DisplayControlFault`] |
//! | refresh | [`Engine::step`], [`Engine::step_budgeted`], [`StepBudget`] ([`EngineConfig::step_budget`], [`EngineConfig::on_chunk`]), [`Wake`] ([`Wake::IdleFor`]), [`RefreshStats`], [`PerfMonitor`], [`InvalidateReason`] |
//! | timing | [`Engine::timing`], [`Engine::reset_worst`], [`TimingReport`] (worst update / chunk render / chunk flush; `FrameOverrun` faults) |
//! | events | [`Engine::send_event`], [`Engine::add_event_handler`], [`Event`], [`EventCode`], [`EventCx`] |
//! | input | [`Engine::add_input`], [`Engine::notify_input`], [`Engine::read_inputs`], [`Engine::inactive_for`], [`Engine::trigger_activity`], [`InputId`] |
//! | layout | [`Engine::set_size`], [`Engine::set_pos`], [`Engine::align`], [`Engine::align_to`], [`Engine::set_flex_flow`], [`Engine::set_grid_tracks`], [`Engine::set_local_grid_tracks`] (grid tracks: [`GridTracks`](twine_style::GridTracks)), [`Engine::set_grid_cell`], [`Engine::update_layout`], [`Engine::layout_passes`], [`LayoutStats`]; docking (keyboard avoidance): [`ObjFlags::DOCK_BOTTOM`] |
//! | animation | [`Engine::anim_start`], [`Engine::anim_start_fn`], [`Engine::timer_add`], [`Engine::load_screen_anim`], [`Engine::finish_screen_anim`], [`Engine::set_motion`], [`Engine::motion`], [`Motion`], [`AnimProp`], [`AnimSpec`], [`ScreenAnim`], [`Deferred`] |
//! | focus | [`Engine::create_group`], [`Engine::group_add`], [`Engine::focus_next`], [`gridnav`] |
//! | faults | [`Engine::raise_fault`], [`Engine::take_faults`], [`Engine::set_fault_hook`], [`Engine::raise_alloc_after_seal`], [`FaultRecord`], [`FaultKind`](twine_core::fault::FaultKind), [`CapacityFault`](twine_core::fault::CapacityFault) |
//! | scrolling | [`Engine::scroll_by`], [`Engine::scroll_to`], [`Engine::scroll_to_view`], [`Engine::scroll_to_view_after_layout`], [`Engine::scroll_top`], [`Engine::set_scroll_snap_x`], [`Engine::update_snap`], [`Engine::scrollbar_areas`], [`ScrollbarMode`] |
//! | memory | [`Engine::memory_report`], [`EngineMemory`], [`HeapPeak`], [`MemInfo`], [`Engine::with_layer_buf`] |
//! | bounds | [`Engine::set_limits`], [`Engine::limits`] ([`Limits`](twine_core::Limits): nodes, tree depth, text length, handlers, timers, animations; preallocation), [`WidgetCx::fit_text`], [`EngineError::TooDeep`] |
//! | display integrity | [`Engine::heartbeat`], [`Engine::now`]; with the `display-integrity` feature: `FrameObserver`, `FrameChunk`, `FrameReport`, `Engine::set_frame_observer`, `Engine::region`, `Engine::render_region_to`, `Engine::read_back`, `Engine::check_region` (`IntegrityCheck`), `ObjFlags::SAFETY_CRITICAL`, `Engine::critical_nodes`, `Engine::critical_report` (`CriticalReport`, `Widget::value_digest`), `Engine::set_fail_safe_screen`, `Engine::enter_fail_safe`, `Engine::leave_fail_safe`, `Engine::fail_safe` |
//! | audit | `impl Audit for EngineConfig` ([`twine_core::audit`]), [`audit_features`], [`ThemeHook::audit`] |
//! | draw acceleration | [`Engine::set_accel`], [`Engine::clear_accel`] |
//!
//! ## Features
//!
//! - `std`, `log`, `defmt`: as in every Twine crate.
//! - `debug-checks`: tree invariants after every mutation, style cache verification, the
//!   invalidation log and the render hook (tests enable it).
//! - `perf-monitor`: the on-screen performance overlay.
//! - `test-ids`: keep node test ids in release builds.
//! - `color-*`: pixel formats compiled into the renderer.
//! - `safety`: the safety profile (enabled by the `twine` facade's `safety` feature, see
//!   `docs/safety/profile.md` in the repository): the profile's lints; it refuses `perf-monitor`
//!   and `fs`; it implies `display-integrity`.
//! - `display-integrity`: frame observers, region re-render / read-back / checks,
//!   `ObjFlags::SAFETY_CRITICAL`, the stale / invalid marks and the fail-safe screen
//!   (`docs/safety/profile.md` § 11). Off (the default), none of it is compiled: no flag test
//!   per drawn node, no observer branch per chunk.
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
//! engine.set_local_prop(boxed, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
//! engine.set_local_prop(boxed, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
//! assert!(matches!(engine.step(Instant::from_millis(0)), Wake::Idle)); // rendered, nothing left
//! assert_eq!(engine.step(Instant::from_millis(100)), Wake::Idle);      // idle: no work at all
//! ```
#![no_std]
#![forbid(unsafe_code)]
// The safety profile's lints (`docs/safety/profile.md` § Lints): denied in library code under
// the `safety` feature (`cargo xtask ci` stage `clippy-safety`).
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable,)
)]
// NOTE(R5.S12): sites of these lints remain (counts: `docs/safety/profile.md` § Lint status,
// `cargo xtask safety-lints`); a lint moves to `deny` above when its last site is fixed.
#![cfg_attr(
    all(feature = "safety", not(test)),
    allow(clippy::indexing_slicing, clippy::arithmetic_side_effects,)
)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]

// The safety profile refuses the parts it excludes (`docs/safety/profile.md` § Exclusions).
#[cfg(all(feature = "safety", feature = "perf-monitor"))]
compile_error!(
    "the `safety` profile excludes the performance overlay (`perf-monitor`, a default feature \
     of `twine`: depend on `twine` with `default-features = false`); see docs/safety/profile.md"
);
#[cfg(all(feature = "safety", feature = "fs"))]
compile_error!(
    "the `safety` profile excludes the file system (`fs`: run-time file images); embed images \
     at build time instead; see docs/safety/profile.md"
);

extern crate alloc;
#[cfg(any(test, feature = "std"))]
extern crate std;

mod accel;
mod anim;
mod config;
mod display;
mod display_control;
mod draw_cx;
mod draw_dsc;
mod engine;
mod error;
mod event;
mod fault;
mod files;
mod flags;
pub mod gridnav;
mod group;
mod handlers;
mod health;
mod id;
mod input;
#[cfg(feature = "display-integrity")]
mod integrity;
mod invalidate;
mod layout;
mod memory;
mod obj;
mod outside;
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
#[cfg(feature = "svg")]
mod svg;
mod theme_hook;
mod transition;
mod tree;
mod wake;
mod widget;

pub use anim::{AnimProp, Deferred, defer};
pub use config::EngineConfig;
pub use display::{BufferMode, BufferSpec, MAX_DISPLAYS};
pub use display_control::{DisplayCmd, DisplayControlFault, DisplayRequests, DisplayResponses};
pub use draw_cx::DrawCx;
pub use draw_dsc::RectStyle;
pub use engine::Engine;
pub use error::{DriverErrorCode, EngineError, InvariantError};
pub use event::{Event, EventCode, EventCx, EventParam, EventResult, EventText};
pub use fault::{FaultHook, FaultRecord};
#[cfg(feature = "fs")]
pub use files::VfsFileSource;
pub use flags::{LayoutDirty, ObjFlags, flag_names};
pub use gridnav::GridnavCtrl;
pub use group::{EdgeCb, FocusCb, GroupId, MAX_GROUPS, RefocusPolicy};
pub use handlers::{EventFilter, Handler, HandlerId};
pub use health::{DisplayHealth, DisplayState, FlushPolicy};
pub use id::{DEAD_NODE, DisplayId, NodeId, NodeIdFmt, fmt_node_id};
pub use input::{InputId, MAX_INPUTS};
#[cfg(feature = "display-integrity")]
pub use integrity::{CriticalReport, FrameChunk, FrameObserver, FrameReport, IntegrityCheck, Region};
pub use invalidate::InvalidateReason;
pub use layout::{LayoutStats, MAX_LAYOUT_ITERATIONS};
pub use memory::EngineMemory;
pub use obj::{OBJ_CLASS, OBJ_FLAGS, Obj};
#[cfg(feature = "perf-monitor")]
pub use perf_overlay::{PERF_OVERLAY_CLASS, PerfOverlay};
pub use refresh::{FrameInfo, RefreshOutcome};
pub use screen_anim::{ScreenAnim, ScreenLoad};
pub use scroll::{SCROLL_ANIM_TIME_MAX, SCROLL_ANIM_TIME_MIN, SCROLL_ELASTIC_FACTOR};
pub use stats::{HeapPeak, MemInfo, PerfMonitor, RefreshStats, TimingReport};
pub use style_cache::MainStyle;
pub use style_list::StyleList;
pub use theme_hook::{IntoTheme, ThemeCx, ThemeHook};
pub use tree::{Ancestors, Children, ChildrenRev, Descendants, Node, Tree};
pub use twine_anim::{Anim, AnimId, AnimSpec, Easing, Motion, Repeat, TimerId};
pub use twine_hal::{InputKind, Key};
pub use twine_style::{Axis, ScrollSnap, ScrollbarMode, Side, Sides};
pub use twine_style::{State, ThemeMode};
pub use wake::{StepBudget, Wake};
pub use widget::{
    AsAny, Editable, GroupDef, Lineage, MAX_CLASS_DEPTH, MeasureCx, Widget, WidgetClass, WidgetCx,
    default_covers, default_hit_test,
};

/// Writes the engine's cargo features of this build as `build.engine_<feature>=<bool>`
/// entries of an audit report (`AppConfig::audit_report` calls it): `debug-checks`,
/// `perf-monitor`, `test-ids`, `fs`, `svg` and every `color-*` renderer. Never allocates.
///
/// ```
/// use twine_core::audit::{Audit, AuditSink, KeyValues};
///
/// struct Features;
/// impl Audit for Features {
///     fn audit(&self, out: &mut dyn AuditSink) { twine_engine::audit_features(out) }
/// }
/// assert!(KeyValues(&Features).to_string().contains("build.engine_color_rgb565="));
/// ```
pub fn audit_features(out: &mut dyn twine_core::audit::AuditSink) {
    use twine_core::audit::AuditValue::Bool;
    let features = [
        ("engine_debug_checks", cfg!(feature = "debug-checks")),
        ("engine_perf_monitor", cfg!(feature = "perf-monitor")),
        ("engine_test_ids", cfg!(feature = "test-ids")),
        ("engine_fs", cfg!(feature = "fs")),
        ("engine_svg", cfg!(feature = "svg")),
        ("engine_color_rgb565", cfg!(feature = "color-rgb565")),
        (
            "engine_color_rgb565_swapped",
            cfg!(feature = "color-rgb565-swapped"),
        ),
        ("engine_color_rgb888", cfg!(feature = "color-rgb888")),
        ("engine_color_xrgb8888", cfg!(feature = "color-xrgb8888")),
        ("engine_color_argb8888", cfg!(feature = "color-argb8888")),
        ("engine_color_l8", cfg!(feature = "color-l8")),
        ("engine_color_i1", cfg!(feature = "color-i1")),
    ];
    for (key, on) in features {
        out.entry("build", key, Bool(on));
    }
}
