//! [`EngineConfig`]: every tunable of the engine, with LVGL-like defaults.

use twine_core::{Duration, Instant};
use twine_render::RenderConfig;
use twine_text::Font;

use crate::{EngineError, MemInfo};

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
    /// When both draw buffers are being flushed, return [`Wake::Now`](crate::Wake::Now) from
    /// `step` instead of spinning until the driver hands one back (the caller can sleep until
    /// the DMA interrupt). Default `false`.
    pub cooperative_flush: bool,
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
    /// `layer_buf_bytes ≥ 4096`.
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
    }
}
