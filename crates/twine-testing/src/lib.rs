//! # twine-testing
//!
//! Test utilities of the Twine GUI library. This is a std-only crate, at the top of the crate
//! layering, used as a **dev-dependency** from integration tests (`tests/*.rs`)
//! of the other crates — never from `#[cfg(test)]` unit tests, where a second copy of the crate
//! under test would be compiled and types would not match.
//!
//! Feature-independent parts (always available):
//!
//! | Item | Purpose |
//! |------|---------|
//! | [`MemoryDisplay`], [`FlushRecord`], [`leak_buffer`] | in-memory [`DisplayDriver`](twine_hal::DisplayDriver) recording flushes, with DMA-like latency |
//! | [`MockClock`] | manually advanced [`Clock`](twine_hal::Clock) |
//! | [`MockPlatform`] | [`Platform`](twine_hal::Platform) and [`AsyncPlatform`](twine_hal::AsyncPlatform) on simulated time (records waits, counts notifications, simulates interrupt context) |
//! | [`MockPointer`], [`MockKeypad`], [`MockEncoder`], [`MockButton`] | scriptable [`InputDevice`](twine_hal::InputDevice)s |
//! | [`snapshot`] | pixel-exact PNG snapshots ([`assert_rgb_snapshot`], [`snapshot_config!`]) |
//! | [`convert`] | any supported pixel format → RGB888 |
//! | [`alloc`] | [`CountingAllocator`](alloc::CountingAllocator), [`count_allocs`](alloc::count_allocs) |
//! | [`init_test_logging`], [`capture_logs`] | `env_logger` for tests, capturing log records |
//!
//! Harness tiers selected by features (so lower crates can test before higher crates exist):
//!
//! | Feature | Items |
//! |---------|-------|
//! | `render` | [`RenderHarness`] over the renderer; [`gallery`]: the renderer's reference drawings (pages of the `render_gallery` example and the renderer snapshot tests) |
//! | `engine` | [`EngineHarness`] over a bare engine, [`MockDmaDisplay`], [`MockFramebufferDisplay`]; [`scenes`]: reference engine scenes (`engine_boxes`, `scroll_list`, …) shared by tests, benchmarks and examples |
//! | `ui` (default) | [`TestUi`] over the declarative UI; per-test ports ([`TestUi::channel`], [`TestUi::latest`], [`TestUi::outbox`]: fresh `'static` queues handed to the application instead of global `static`s shared by parallel tests), the shipped configuration ([`TestUi::app_config`]) and [`TestUi::memory_report`] |
//!
//! ```
//! use twine_core::{Color, ColorFormat, Rect};
//! use twine_hal::{DisplayDriver, DisplayInfo};
//! use twine_testing::{MemoryDisplay, leak_buffer};
//!
//! let mut display = MemoryDisplay::new(DisplayInfo::new(8, 8, ColorFormat::L8)).with_latency(2);
//! let mut buf = leak_buffer(4);
//! buf.as_mut_slice().fill(255);
//! display.begin_flush(Rect::from_xywh(0, 0, 2, 2), buf).unwrap();
//! assert!(display.poll_flush().is_none()); // "DMA" still running
//! assert!(display.poll_flush().is_some());
//! assert_eq!(display.pixel(1, 1), Color::WHITE);
//! ```
#![deny(unsafe_code)]

pub mod alloc;
pub mod clock;
pub mod convert;
#[cfg(feature = "engine")]
pub mod engine_harness;
#[cfg(feature = "render")]
pub mod gallery;
pub mod logging;
pub mod memory_display;
#[cfg(feature = "engine")]
pub mod mock_display;
pub mod mock_input;
pub mod platform;
pub mod png_io;
#[cfg(feature = "render")]
pub mod render_harness;
#[cfg(feature = "engine")]
pub mod scenes;
pub mod snapshot;
#[cfg(feature = "ui")]
pub mod ui;

/// Log capture for tests: `logs::capture(|| …)` runs a closure and returns the records it
/// logged (on this thread).
pub mod logs {
    pub use crate::logging::{CapturedLog as LogRecord, capture_logs};

    /// Runs `f` and returns the log records it produced (with `f`'s result dropped).
    pub fn capture(f: impl FnOnce()) -> Vec<LogRecord> {
        capture_logs(f).1
    }
}

pub use clock::MockClock;
#[cfg(feature = "engine")]
pub use engine_harness::{EngineHarness, FbMode, Query, StepFn, TeardownFn, by_class, by_id, by_text};
pub use logging::{CapturedLog, capture_logs, init_test_logging};
pub use memory_display::{FlushRecord, MemoryDisplay, MemoryDisplayError, leak_buffer};
#[cfg(feature = "engine")]
pub use mock_display::{
    DmaEvent, MockDmaDisplay, MockFramebufferDisplay, MockPresentError, clear_dma_log, dma_log,
    record_render_start,
};
pub use mock_input::{MockButton, MockEncoder, MockKeypad, MockPointer};
pub use platform::MockPlatform;
#[cfg(feature = "render")]
pub use render_harness::RenderHarness;
pub use snapshot::{SnapshotConfig, Tolerance, assert_rgb_snapshot};
#[cfg(feature = "ui")]
pub use ui::{NodeHandle, TestUi};
