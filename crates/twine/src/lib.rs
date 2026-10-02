//! # twine
//!
//! Twine: a declarative, signal-based, low-power GUI library for embedded devices with the
//! feature set of LVGL, `no_std` + `alloc`.
//!
//! This is the facade crate: it re-exports every Twine crate under a short module name and
//! gathers everything an application needs in [`prelude`].
//!
//! ```
//! use twine::prelude::*;
//!
//! pub fn counter(cx: Scope) -> impl View {
//!     let count = cx.signal(0u32);
//!     column((
//!         label(text!("Clicked {} times", count.get())).test_id("count"),
//!         button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
//!     ))
//!     .gap(12)
//!     .padding(16)
//!     .align_items(CrossAlign::Center)
//! }
//! # let _ = counter;
//! ```
//!
//! An application is a function run **once** that describes its widgets; dynamic values are
//! fine-grained bindings, so a change updates exactly the affected widgets and redraws
//! exactly their pixels, and an idle UI uses no CPU at all.
//!
//! ## Guides
//!
//! - [Writing a custom widget](guide::custom_widgets): a gauge with its own class and parts,
//!   drawing, keypad/encoder input, theming with design elements and theme modes, a typed
//!   view, animations within the motion preference, and tests (the `gauge` example).
//!
//! ## Features
//!
//! The facade is the one dependency an application needs: every feature below is forwarded
//! to every Twine crate that has it (`cargo xtask layers` checks the forwarding).
//!
//! - default: `color-rgb565`, `color-rgb565-swapped`, `img-qoi`, `perf-monitor`, `log`.
//! - `std` (host, simulator, tests: the thread-local reactive runtime), `log`, `defmt`
//!   (logging backends), `perf-monitor` (on-screen performance overlay), `debug-checks`
//!   (expensive engine invariant checks), `test-ids` (keep test ids in release builds).
//! - `async`: the async runtime (`AsyncUi`, `Ui::builder_async`), the async HAL traits, async
//!   drivers and `Outbox::recv` (await commands from the UI in any executor).
//! - `color-*`: pixel formats the renderer is compiled for (`color-i1` also compiles `L8`);
//!   `img-*`: image decoders (`qoi`, `png`, `jpeg`, `gif`, `bmp`, `lz4`).
//! - `platform-cortex-m`, `platform-riscv`, `platform-std`, `platform-embassy`: ready-made
//!   [`Platform`](hal::Platform) implementations in [`platform`] (none by default: Twine assumes
//!   no OS, executor or chip).
#![cfg_attr(
    feature = "embassy",
    doc = "- `embassy`: [`embassy`] (the `twine-embassy` crate: the embassy run loop; implies `async`\n  and `platform-embassy`)."
)]
#![cfg_attr(
    not(feature = "embassy"),
    doc = "- `embassy`: the `embassy` module (the `twine-embassy` crate: the embassy run loop; implies\n  `async` and `platform-embassy`)."
)]
#![cfg_attr(
    feature = "drivers",
    doc = "- `drivers`: [`drivers`] (the `twine-drivers` crate); `drivers-<name>` for each driver, bus\n  and input device (`drivers-spi`, `drivers-i2c`, `drivers-xpt2046`, `drivers-ft6x36`, …).\n  A display panel's feature (`drivers-ili9341`, `drivers-st7789`, `drivers-ssd1306`, …)\n  also enables the `color-*` feature its pixel format needs, so a panel can never be\n  refused with `EngineError::FormatDisabled` for a missing format."
)]
#![cfg_attr(
    not(feature = "drivers"),
    doc = "- `drivers`: the `drivers` module (the `twine-drivers` crate); `drivers-<name>` for each\n  driver, bus and input device (`drivers-spi`, `drivers-i2c`, `drivers-xpt2046`,\n  `drivers-ft6x36`, …). A display panel's feature (`drivers-ili9341`, `drivers-st7789`,\n  `drivers-ssd1306`, …) also enables the `color-*` feature its pixel format needs, so a\n  panel can never be refused with `EngineError::FormatDisabled` for a missing format."
)]
//! - Fonts of [`fonts`]: bundles `fonts-latin-small` (Montserrat 12–16 px),
//!   `fonts-latin-medium` (12–24 px), `fonts-latin-all`, `fonts-mono` (unscii), `fonts-rtl`
//!   (Hebrew, Arabic, Persian), `fonts-cjk`, `fonts-all`; and one feature per font
//!   (`montserrat-20`, `unscii-8`, …) for precise flash budgets.
//! - `bidi`, `arabic-shaping`: right-to-left and Arabic/Persian text; `ttf`: runtime TrueType
//!   fonts; `fs`: the file system (`fs` module) and file images; `vector`: vector graphics
//!   (`vector` module, `vector_canvas`); `svg`: SVG images drawn as vectors (implies
//!   `vector`).
//! - `full`: every optional, platform-neutral part above (all pixel formats, decoders, fonts,
//!   `vector`, `svg`, `ttf`, `fs`, `bidi`, `arabic-shaping`, `async`, `perf-monitor`) — for
//!   hosts, simulators, tests and documentation. It leaves out `std`, the logging backends,
//!   platforms, drivers and the debugging aids; firmware enables only what it uses.
//!
//! Firmware that offers alternatives as its own cargo features (one display of several, at
//! most one touch controller) checks their combinations with [`feature_rules!`].
//!
//! ## Bring-up
//!
//! A `Ui` needs a display, a runtime token, a clock (or platform) and draw buffers — checked by
//! the compiler: `build` does not exist until all of them are given, and the error names the
//! missing call ([`view::typestate`]). Firmware declares its buffers as statics; hosts may let
//! the engine allocate them:
//!
//! ```
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock};
//! use twine::prelude::*;
//!
//! twine::draw_buffers!(static BUFS: 2 x 40 rows x 320 px @ Rgb565);
//!
//! # let display = MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565));
//! let ui = Ui::builder(display)
//!     .runtime(Runtime::take().expect("runtime already taken"))
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::partial_double_from(BUFS.take().expect("buffers taken once")))
//!     // or, on the heap: .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 40 }))
//!     .build(|_| label("hi"));
//! # let _ = ui;
//! ```
//!
//! Input devices are passed as constructed: the engine fits each to its display
//! ([`InputDevice::fit_to_display`](hal::InputDevice::fit_to_display)), so touch drivers take
//! their coordinate transform from the display's rotation and size — firmware never derives a
//! native size or a rotation table. Accelerators are handed over the same way and owned by the
//! `Ui` (`Ui::builder_fb(display).accel(Dma2d::new(PacRegs::new(p.DMA2D)?))` with
//! `twine-accel-stm32`):
//!
//! ```
//! # use twine_core::{ColorFormat, Rotation};
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock, MockPointer};
//! use twine::prelude::*;
//!
//! // A 240 × 320 panel turned to landscape.
//! # let display = MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565).with_rotation(Rotation::Deg90));
//! # let touch = MockPointer::new();
//! let ui = Ui::builder(display)
//!     .runtime(Runtime::take().expect("runtime already taken"))
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .input(touch) // e.g. `Ft6x36::new(i2c, Some(irq))`: fitted to the rotated display here
//!     .build(|_| label("hi"));
//! # let _ = ui;
//! ```
//!
//! ## One configuration everywhere
//!
//! What the application decides — engine configuration, theme, motion, rotation, channel and
//! queue budgets, fault hook — is one [`AppConfig`](view::AppConfig), written once as a
//! function and handed to every host, so the simulator and the tests run what ships:
//!
//! ```
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock, TestUi};
//! use twine::prelude::*;
//!
//! fn config() -> AppConfig {
//!     AppConfig::new().theme(DefaultTheme::light()).motion(Motion::Reduced)
//! }
//!
//! // Firmware (plus its board hooks, e.g. `cfg.engine.hires_timer = Some(..)`):
//! # let display = MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565));
//! let ui = Ui::builder(display)
//!     .runtime(Runtime::take().expect("runtime already taken"))
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .app_config(config())
//!     .build(|_| label("hi"));
//! // Simulator: `twine_sim::run(SimConfig::new(320, 240).app_config(config()), app)`.
//! // Tests:
//! let t = TestUi::new(320, 240).app_config(config()).mount(|_| label("hi"));
//! assert_eq!(ui.motion(), t.engine().motion());
//! ```
//!
//! ## Running it
//!
//! [`Ui::update`](view::Ui::update) is one step; it returns when to step again
//! ([`Wake`](view::Wake)). The run loops are thin adapters over it: [`run::blocking`] with a
//! [`Platform`](hal::Platform) (bare metal, an RTOS task, a host thread; see the `rtos_notify`
//! example), or
#![cfg_attr(
    feature = "embassy",
    doc = "[`embassy::run`] on an embassy executor. Schedulers that bound each step"
)]
#![cfg_attr(
    not(feature = "embassy"),
    doc = "`twine::embassy::run` (feature `embassy`) on an embassy executor. Schedulers that bound\n  each step"
)]
//! use [`Ui::update_budgeted`](view::Ui::update_budgeted).
//!
//! ```no_run
//! # fn main_loop(display: impl twine::hal::DisplayDriver + 'static, mut platform: impl twine::hal::Platform + Clone + 'static) -> ! {
//! use twine::prelude::*;
//!
//! let ui = Ui::builder(display)
//!     .runtime(Runtime::take().expect("runtime already taken"))
//!     .platform(&platform) // e.g. `CortexMPlatform::new(timer)`
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .build(|_| label("hi"));
//! twine::run::blocking(ui, &mut platform)
//! # }
//! ```
//!
//! ## Memory
//!
//! [`Ui::memory_report`](view::Ui::memory_report) tells what the memory is used for — the
//! widget tree, the render scratch buffers and caches, the glyph and image caches, the layer
//! and draw buffers (heap or static), the reactive runtime, the command queues and the waker
//! pool ([`MemoryReport`](view::MemoryReport), in bytes, without allocating). Firmware keeps
//! the largest buffers out of the heap with statics: [`draw_buffers!`] for the draw buffers
//! and a [`LayerBuffer`](view::LayerBuffer) for the layer buffer; the allocator's statistics
//! reach the `twine::perf` log through `EngineConfig::mem_info` (see
//! [`MemInfo`](engine::MemInfo) and [`HeapPeak`](engine::HeapPeak)).
//!
//! ```
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock};
//! use twine::prelude::*;
//!
//! twine::draw_buffers!(static BUFS: 2 x 20 rows x 320 px @ Rgb565);
//! static LAYER: LayerBuffer<{ 16 * 1024 }> = LayerBuffer::zeroed();
//!
//! # let display = MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565));
//! let ui = Ui::builder(display)
//!     .runtime(Runtime::take().expect("runtime already taken"))
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::partial_double_from(BUFS.take().expect("buffers taken once")))
//!     .layer_buf(LAYER.take().expect("layer buffer taken once"))
//!     .build(|_| label("hi"));
//! let m = ui.memory_report();
//! assert_eq!(m.static_bytes(), 2 * 320 * 2 * 20 + 16 * 1024);
//! assert!(m.engine.heap_bytes() < m.engine.static_bytes()); // the big buffers are statics
//! ```
//!
//! ## Talking to interrupts and other tasks
//!
//! The UI runs in one context; interrupts, tasks, threads and other cores reach it only
//! through the cross-context types (`Sync`, `critical-section` + `portable-atomic`, no
//! compare-and-swap, no allocation when used):
//!
//! | Type | Direction | Semantics |
//! |------|-----------|-----------|
//! | [`Latest<T>`](reactive::Latest) | into the UI | the newest value wins, nothing queues or drops; [`cx.watch(..)`](reactive::Scope::watch) is a signal of it |
//! | [`Channel<T, N>`](reactive::Channel) | into the UI | FIFO, every value in order ([`cx.on_message(..)`](reactive::Scope::on_message)); a full channel follows its [`Overflow`](reactive::Overflow) policy and raises `ChannelOverflow` |
//! | [`Outbox<T, N>`](reactive::Outbox) | out of the UI | FIFO to a task: `try_recv`, or `recv().await` (feature `async`) |
//!
//! Every send wakes the UI (or the outbox's consumer). Use the **ports** pattern: `main` owns
//! these objects and passes the application a `Copy` struct of references, so the
//! application names no global and every test gets its own (`TestUi::channel`, `latest`,
//! `outbox`):
//!
//! ```
//! use twine::prelude::*;
//!
//! #[derive(Clone, Copy)]
//! struct Ports {
//!     temp: &'static Latest<i16>,     // written by the ADC interrupt
//!     heater: &'static Outbox<bool, 2>, // read by the heater task
//! }
//!
//! fn app(cx: Scope, ports: Ports) -> impl View {
//!     let temp = cx.watch(ports.temp);
//!     let heating = cx.memo(move || temp.get() < 200);
//!     cx.effect(move || {
//!         let _ = ports.heater.try_send(heating.get());
//!     });
//!     label(text!("{} °C", temp.get() / 10))
//! }
//!
//! static TEMP: Latest<i16> = Latest::new(215);
//! static HEATER: Outbox<bool, 2> = Outbox::new().on_full(Overflow::DropOldest);
//! let ports = Ports { temp: &TEMP, heater: &HEATER };
//! // Ui::builder(display)...build(move |cx| app(cx, ports));
//! # let _ = (ports, app);
//! ```
//!
//! The `thermostat` demo and example show the whole pattern.
//!
//! ## Examples
//!
//! The runnable desktop examples (simulator windows) live in this crate's `examples/`
//! directory: `cargo run -p twine --example <name>` (e.g. `counter`, `thermostat`, `gauge`), or
//! `cargo xtask sim <name>` (also headless: `--headless`, `--script <file>`). Board examples
//! are in the repository's `firmware/` directory.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use twine_anim as anim;
pub use twine_assets as assets;
pub use twine_core as core;
/// Display, touch and input drivers (`twine-drivers`, feature `drivers`; each driver, bus and
/// input device with `drivers-<name>`; a panel's feature also enables its pixel format).
#[cfg(feature = "drivers")]
#[cfg_attr(docsrs, doc(cfg(feature = "drivers")))]
pub use twine_drivers as drivers;
/// The embassy run loop (`twine-embassy`, feature `embassy`): `embassy::run` / `run_with` for
/// `Ui` and `AsyncUi`, and the `EmbassyPlatform` builder helpers.
#[cfg(feature = "embassy")]
#[cfg_attr(docsrs, doc(cfg(feature = "embassy")))]
pub use twine_embassy as embassy;
pub use twine_engine as engine;
#[cfg(feature = "fs")]
#[cfg_attr(docsrs, doc(cfg(feature = "fs")))]
pub use twine_fs as fs;
pub use twine_hal as hal;
pub use twine_image as image;
pub use twine_layout as layout;
pub use twine_reactive as reactive;
pub use twine_render as render;
pub use twine_style as style;
pub use twine_text as text;
pub use twine_theme as theme;
#[cfg(feature = "vector")]
#[cfg_attr(docsrs, doc(cfg(feature = "vector")))]
pub use twine_vector as vector;
pub use twine_view as view;
pub use twine_widgets as widgets;
pub use twine_widgets_ext as widgets_ext;

/// Platform services: the [`Platform`](hal::Platform) and `AsyncPlatform` traits, and the
/// ready-made implementations behind the `platform-*` features (`CortexMPlatform`,
/// `RiscvPlatform`, `StdPlatform`, `EmbassyPlatform`; none by default).
pub use twine_hal::platform;

/// The built-in fonts (each behind its cargo feature).
pub use twine_assets::fonts;

/// Static draw buffers for firmware: `twine::draw_buffers!(static BUFS: 2 x 40 rows x 320 px @
/// Rgb565Swapped);` declares buffers sized at compile time ([`buffer_bytes`]), aligned by their
/// type and taken once (`BUFS.take()`), for
/// [`BufferMode::partial_double_from`](engine::BufferMode::partial_double_from). See
/// [`view::draw_buffers!`](twine_view::draw_buffers).
pub use twine_hal::buffer_bytes;
#[doc(inline)]
pub use twine_view::draw_buffers;

/// Run loops: [`run::blocking`] / [`run::blocking_with`] step a `Ui` and sleep on a
/// [`Platform`](hal::Platform) (bare metal, an RTOS task, a host thread). The embassy adapter
/// is the `twine-embassy` crate.
#[doc(inline)]
pub use twine_view::run;

mod feature_rules;
pub mod guide;

/// Everything an application needs: `use twine::prelude::*;`.
///
/// Views, modifiers, control flow, the reactive primitives, styles, colors, geometry, time,
/// animation, themes, navigation, the `Ui` runtime and the built-in [`fonts`].
///
/// ```
/// use twine::prelude::*;
///
/// pub static CARD: Style = style! { bg_color: Color::WHITE, bg_opacity: Opa::COVER, radius: 8, padding: 12 };
///
/// fn card(title: &'static str) -> impl View {
///     container(label(title)).style(&CARD)
/// }
/// # let _ = card;
/// ```
pub mod prelude {
    pub use twine_assets::fonts;
    pub use twine_view::prelude::*;
}
