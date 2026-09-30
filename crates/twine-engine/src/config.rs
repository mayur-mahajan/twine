//! [`EngineConfig`]: every tunable of the engine, with LVGL-like defaults.

use twine_core::{Duration, Instant};
use twine_render::RenderConfig;
use twine_text::Font;

use crate::{EngineError, FlushPolicy, MemInfo};

/// Engine configuration. Construct with `EngineConfig::default()` and change fields:
///
/// ```
/// use twine_core::Duration;
/// use twine_engine::EngineConfig;
/// let cfg = EngineConfig { refr_period: Duration::ms(33), ..EngineConfig::default() };
/// assert!(cfg.validate().is_ok());
/// assert_eq!(cfg.max_dirty_areas, 32);
/// ```
///
/// Cache sizes are read once by [`Engine::new`](crate::Engine::new); changing them later
/// through [`Engine::config_mut`](crate::Engine::config_mut) has no effect.
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Minimum time between two frames of a display (default 16 ms). Invalidations in between
    /// accumulate.
    pub refr_period: Duration,
    /// Dirty areas kept per display before they collapse into their bounding box (1..=32,
    /// default 32).
    pub max_dirty_areas: usize,
    /// When no draw buffer is free because the driver still flushes them all (or a framebuffer
    /// swap is pending), return [`Wake::Now`](crate::Wake::Now) from `step` instead of waiting
    /// inside it. Default `false`.
    ///
    /// - `false` suits a dedicated UI loop (bare metal, a UI task of its own): the step waits
    ///   for the DMA itself, which is the shortest path to the next chunk. The wait is bounded
    ///   by [`flush_timeout`](Self::flush_timeout).
    /// - `true` suits a loop that has something better to do while the DMA runs: other work
    ///   in a super-loop, other tasks of an RTOS or executor, or sleeping until the DMA
    ///   interrupt. The step returns at once; call it again when a buffer can be free.
    ///
    /// Displays added with [`add_chunked_display`](crate::Engine::add_chunked_display) (the
    /// async runtime `AsyncUi`) are not affected: their caller flushes, and an async flush
    /// is awaited, which is cooperative by construction.
    pub cooperative_flush: bool,
    /// Longest wait for a display driver to finish a flush (to hand a draw buffer back through
    /// `poll_flush`) or a framebuffer swap (`present_done`), measured from the start of the
    /// flush. `None`: wait forever (a hung DMA transfer freezes the UI). Default 500 ms — far
    /// above any chunk transfer on a working bus; raise it for very slow panels (e.g. e-paper
    /// drivers whose flush includes the panel refresh).
    ///
    /// When it expires the engine raises
    /// [`FaultKind::FlushTimeout`](twine_core::fault::FaultKind::FlushTimeout) (the record's
    /// `code` is the time waited in ms), marks the display
    /// [`DisplayState::Failed`](crate::DisplayState::Failed) (halted under
    /// [`FlushPolicy::Halt`]), redraws the chunks that were in flight later (unless
    /// [`FlushPolicy::Ignore`]) and returns from the step. While the driver keeps the buffer,
    /// no frame starts: the display retries once per `refr_period`
    /// ([`Wake::At`](crate::Wake::At)), and it recovers on its own once the buffer comes back
    /// and a flush succeeds.
    ///
    /// **Time source.** The engine knows the time only through the `now` of each step and the
    /// optional [`hires_timer`](Self::hires_timer). With a `hires_timer`, a step waiting for a
    /// buffer spins and checks the timer. Without one, time stands still inside a step, so a
    /// bounded wait never spins: the step returns [`Wake::Now`](crate::Wake::Now) (as with
    /// [`cooperative_flush`](Self::cooperative_flush)) and the timeout is measured with the
    /// `now` of the following steps. Only `None` without a `hires_timer` spins inside a step.
    pub flush_timeout: Option<Duration>,
    /// What happens when a display driver reports a failed flush or present (default
    /// [`FlushPolicy::Reinvalidate`]: the area is redrawn on the next frame). Every failure
    /// raises [`FaultKind::FlushError`](twine_core::fault::FaultKind::FlushError) whatever the
    /// policy.
    pub flush_policy: FlushPolicy,
    /// Consecutive failed flushes or presents of a display (each chunk counts) after which
    /// it is [`DisplayState::Failed`](crate::DisplayState::Failed) — and, with
    /// [`FlushPolicy::Halt`], stops refreshing (≥ 1, default 8). See
    /// [`Engine::display_health`](crate::Engine::display_health).
    pub max_consecutive_flush_errors: u16,
    /// Tint every refreshed area with a color that changes per frame (LVGL
    /// `LV_USE_REFR_DEBUG`). Default `false`.
    pub debug_refresh: bool,
    /// Bytes of the ARGB8888 layer buffer used for opacity groups, transforms and blend modes
    /// (≥ 4 KiB, default 24 KiB).
    pub layer_buf_bytes: usize,
    /// Glyph cache budget in bytes (default 8 KiB).
    pub glyph_cache_bytes: usize,
    /// Decoded image cache budget in bytes (default 0: only uncompressed static images).
    pub image_cache_bytes: usize,
    /// Quarter-circle coverage cache entries (default 4).
    pub circle_cache_entries: u8,
    /// Gradient color map cache entries (default 4).
    pub gradient_cache_entries: u8,
    /// Blurred shadow corner cache entries (default 1).
    pub shadow_cache_entries: u8,
    /// The default `TextFont` when no style sets one (the engine cannot depend on the built-in
    /// fonts; `None` = `twine_text::EMPTY_FONT`, which draws nothing).
    pub default_font: Option<&'static Font>,
    /// Period of the `twine::perf` log line (default 5 s).
    pub perf_log_period: Duration,
    /// Memory statistics provider for [`RefreshStats`](crate::RefreshStats) (e.g. the heap
    /// allocator's counters). Default `None`.
    pub mem_info: Option<fn() -> MemInfo>,
    /// High-resolution time source for intra-frame timing (render / flush durations). `None`:
    /// the timing fields of [`RefreshStats`](crate::RefreshStats) stay 0.
    pub hires_timer: Option<fn() -> Instant>,
    /// Maximum number of live nodes, screens and layers included (1..=65 535, default 65 535:
    /// the tree's capacity). Creating a node beyond it fails with
    /// [`EngineError::TooManyNodes`] and raises
    /// [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity).
    // NOTE(R5.S03): moves into `Limits` with the other capacity bounds.
    pub max_nodes: u16,
    /// Press duration that triggers a long press (default 400 ms).
    pub long_press_time: Duration,
    /// Repeat period of long-press-repeat events (default 100 ms).
    pub long_press_repeat: Duration,
    /// Pointer movement in pixels that starts scrolling (default 10).
    pub scroll_limit: i32,
    /// Momentum decay of a scroll throw in percent per frame (default 10).
    pub scroll_throw: u8,
    /// Movement in pixels that makes a gesture (default 50).
    pub gesture_limit: i32,
    /// Minimum velocity of a gesture in pixels per read period (default 3).
    pub gesture_min_velocity: i32,
    /// Maximum gap between clicks of a double / triple click (default 300 ms).
    pub multi_click_time: Duration,
    /// Maximum distance in pixels between clicks of a multi-click (default 5).
    pub multi_click_distance: i32,
    /// Input device read period while pressed or for polled devices (default 30 ms).
    pub read_period: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            refr_period: Duration::ms(16),
            max_dirty_areas: 32,
            cooperative_flush: false,
            flush_timeout: Some(Duration::ms(500)),
            flush_policy: FlushPolicy::Reinvalidate,
            max_consecutive_flush_errors: 8,
            debug_refresh: false,
            layer_buf_bytes: 24 * 1024,
            glyph_cache_bytes: 8 * 1024,
            image_cache_bytes: 0,
            circle_cache_entries: 4,
            gradient_cache_entries: 4,
            shadow_cache_entries: 1,
            default_font: None,
            perf_log_period: Duration::secs(5),
            mem_info: None,
            hires_timer: None,
            max_nodes: u16::MAX,
            long_press_time: Duration::ms(400),
            long_press_repeat: Duration::ms(100),
            scroll_limit: 10,
            scroll_throw: 10,
            gesture_limit: 50,
            gesture_min_velocity: 3,
            multi_click_time: Duration::ms(300),
            multi_click_distance: 5,
            read_period: Duration::ms(30),
        }
    }
}

impl EngineConfig {
    /// Checks the values: `refr_period > 0`, `max_dirty_areas` in `1..=32`,
    /// `layer_buf_bytes ≥ 4096`, `max_nodes ≥ 1`, `max_consecutive_flush_errors ≥ 1`,
    /// `flush_timeout > 0` (when set).
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.refr_period.as_micros() == 0 {
            return Err(EngineError::InvalidConfig("refr_period must be > 0"));
        }
        if !(1..=32).contains(&self.max_dirty_areas) {
            return Err(EngineError::InvalidConfig("max_dirty_areas must be in 1..=32"));
        }
        if self.layer_buf_bytes < 4 * 1024 {
            return Err(EngineError::InvalidConfig("layer_buf_bytes must be >= 4096"));
        }
        if self.max_nodes == 0 {
            return Err(EngineError::InvalidConfig("max_nodes must be >= 1"));
        }
        if self.max_consecutive_flush_errors == 0 {
            return Err(EngineError::InvalidConfig(
                "max_consecutive_flush_errors must be >= 1",
            ));
        }
        if self.flush_timeout.is_some_and(|t| t.as_micros() == 0) {
            return Err(EngineError::InvalidConfig("flush_timeout must be > 0"));
        }
        Ok(())
    }

    /// The renderer's cache budgets derived from this configuration.
    #[must_use]
    pub fn render_config(&self) -> RenderConfig {
        RenderConfig {
            circle_cache_entries: self.circle_cache_entries,
            shadow_cache_entries: self.shadow_cache_entries,
            gradient_cache_entries: self.gradient_cache_entries,
            layer_buf_bytes: u32::try_from(self.layer_buf_bytes).unwrap_or(u32::MAX),
            ..RenderConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_valid() {
        let c = EngineConfig::default();
        assert!(c.validate().is_ok());
        assert_eq!(c.refr_period, Duration::ms(16));
        assert_eq!(c.layer_buf_bytes, 24 * 1024);
        assert_eq!(c.long_press_time, Duration::ms(400));
        assert_eq!(c.flush_timeout, Some(Duration::ms(500)));
        assert_eq!(c.render_config().layer_buf_bytes, 24 * 1024);
    }

    #[test]
    fn config_validate_rejects_zero_period() {
        let c = EngineConfig {
            refr_period: Duration::ms(0),
            ..EngineConfig::default()
        };
        assert!(matches!(c.validate(), Err(EngineError::InvalidConfig(_))));
        let c = EngineConfig {
            max_dirty_areas: 33,
            ..EngineConfig::default()
        };
        assert!(c.validate().is_err());
        let c = EngineConfig {
            max_dirty_areas: 0,
            ..EngineConfig::default()
        };
        assert!(c.validate().is_err());
        let c = EngineConfig {
            layer_buf_bytes: 1024,
            ..EngineConfig::default()
        };
        assert!(c.validate().is_err());
        let c = EngineConfig {
            max_consecutive_flush_errors: 0,
            ..EngineConfig::default()
        };
        assert!(c.validate().is_err());
        let c = EngineConfig {
            flush_timeout: Some(Duration::ms(0)),
            ..EngineConfig::default()
        };
        assert!(c.validate().is_err());
        let c = EngineConfig {
            flush_timeout: None,
            ..EngineConfig::default()
        };
        assert!(c.validate().is_ok());
    }
}
