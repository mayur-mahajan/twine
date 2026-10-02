//! Frame statistics ([`RefreshStats`]), the one-second performance window ([`PerfMonitor`])
//! and memory statistics ([`MemInfo`], [`HeapPeak`]).

use core::sync::atomic::{AtomicU32, Ordering};

use twine_core::{Duration, Instant};

/// Statistics of the most recent frame of a display (see
/// [`Engine::last_stats`](crate::Engine::last_stats)).
///
/// Timing fields need [`EngineConfig::hires_timer`](crate::EngineConfig::hires_timer) and are
/// 0 without it. `fps` and `cpu_percent` come from the one-second [`PerfMonitor`] window.
/// The performance overlay's own redraw is not counted.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RefreshStats {
    /// Frame number (1 = first frame of the display).
    pub frame: u32,
    /// Time from the start of the frame to the last chunk handed to the driver.
    pub frame_time_us: u32,
    /// Time spent drawing.
    pub render_us: u32,
    /// Time spent in the driver's flush calls (plus rotation / format conversion).
    pub flush_us: u32,
    /// Time spent waiting for a draw buffer to come back from the driver.
    pub flush_wait_us: u32,
    /// Dirty areas rendered (after merging).
    pub dirty_areas: u16,
    /// Pixels rendered (sum of the dirty areas after rounding).
    pub dirty_px: u32,
    /// Chunks flushed (partial modes) or areas drawn (framebuffer modes).
    pub chunks: u16,
    /// Nodes drawn (after culling and the top-cover search).
    pub nodes_drawn: u32,
    /// Frames per second over the last complete one-second window.
    pub fps: u16,
    /// Busy time (render + flush wait) in percent of the last window.
    pub cpu_percent: u8,
    /// Memory in use (from [`EngineConfig::mem_info`](crate::EngineConfig::mem_info)).
    pub mem_used: u32,
    /// Peak memory use.
    pub mem_peak: u32,
}

/// Memory statistics reported by the application's allocator, through
/// [`EngineConfig::mem_info`](crate::EngineConfig::mem_info) (a plain `fn() -> MemInfo`: the
/// allocator is the application's choice, so the engine only asks). Values are bytes,
/// saturated to `u32::MAX`.
///
/// Every heap allocator for microcontrollers reports its use; [`HeapPeak`] adds the peak:
///
/// ```
/// use twine_engine::{EngineConfig, HeapPeak, MemInfo};
///
/// # struct Heap; impl Heap { fn used(&self) -> usize { 1000 } fn free(&self) -> usize { 3000 } }
/// # static HEAP: Heap = Heap;
/// // `static HEAP: embedded_alloc::LlffHeap` (or `TlsfHeap`), or `esp_alloc::HEAP`: both
/// // have `used()` and `free()`.
/// fn mem_info() -> MemInfo {
///     static PEAK: HeapPeak = HeapPeak::new();
///     PEAK.sample(HEAP.used(), HEAP.free())
/// }
///
/// let mut config = EngineConfig::default();
/// config.mem_info = Some(mem_info);
/// assert_eq!(mem_info(), MemInfo { used: 1000, peak: 1000, free: 3000 });
/// ```
///
/// On a host (`std`) there are no heap statistics without a counting global allocator; the
/// application installs one if it wants these numbers (test binaries can use
/// `twine_testing::alloc::CountingAllocator`).
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MemInfo {
    /// Bytes in use.
    pub used: u32,
    /// Highest `used` so far.
    pub peak: u32,
    /// Bytes still free.
    pub free: u32,
}

impl MemInfo {
    /// `used` in percent of the heap (`used + free`), 0 for an empty heap. Never panics.
    ///
    /// ```
    /// use twine_engine::MemInfo;
    /// assert_eq!(MemInfo { used: 900, peak: 950, free: 100 }.used_percent(), 90);
    /// assert_eq!(MemInfo::default().used_percent(), 0);
    /// ```
    #[must_use]
    pub fn used_percent(&self) -> u8 {
        let total = u64::from(self.used) + u64::from(self.free);
        if total == 0 {
            return 0;
        }
        (u64::from(self.used) * 100 / total) as u8
    }
}

/// The highest heap use seen, kept in a `static` next to a [`MemInfo`] provider (see the
/// example there): [`sample`](Self::sample) turns an allocator's `used()`/`free()` into a
/// `MemInfo` with a peak.
///
/// The peak is **sampled**: it is the highest `used` passed to `sample`, not every
/// allocation's high-water mark (allocators for microcontrollers do not track one). Call
/// `sample` from one context (the engine calls the provider from its step); it uses only
/// atomic loads and stores (no compare-and-swap), so it works on every target, `thumbv6m`
/// included.
#[derive(Debug, Default)]
pub struct HeapPeak(AtomicU32);

impl HeapPeak {
    /// A tracker that has seen nothing (`peak` 0). `const`, for `static` items. Never panics.
    ///
    /// ```
    /// use twine_engine::HeapPeak;
    /// static PEAK: HeapPeak = HeapPeak::new();
    /// assert_eq!(PEAK.peak(), 0);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// The statistics for a heap with `used` and `free` bytes, raising the peak if `used` is
    /// above it. Values above `u32::MAX` saturate. Allocates nothing; never panics.
    ///
    /// ```
    /// use twine_engine::HeapPeak;
    /// let p = HeapPeak::new();
    /// assert_eq!(p.sample(500, 500).peak, 500);
    /// let m = p.sample(200, 800);
    /// assert_eq!((m.used, m.peak, m.free), (200, 500, 800));
    /// assert_eq!(p.peak(), 500);
    /// ```
    pub fn sample(&self, used: usize, free: usize) -> MemInfo {
        let used = u32::try_from(used).unwrap_or(u32::MAX);
        let free = u32::try_from(free).unwrap_or(u32::MAX);
        let peak = self.0.load(Ordering::Relaxed).max(used);
        self.0.store(peak, Ordering::Relaxed);
        MemInfo { used, peak, free }
    }

    /// The highest `used` sampled so far (0 before the first [`sample`](Self::sample)).
    /// Never panics.
    ///
    /// ```
    /// use twine_engine::HeapPeak;
    /// let p = HeapPeak::new();
    /// p.sample(700, 300);
    /// p.sample(400, 600);
    /// assert_eq!(p.peak(), 700);
    /// ```
    #[must_use]
    pub fn peak(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Accumulates frames and busy time over one-second windows (LVGL's `sysmon` performance
/// monitor): `fps` = frames per window, `cpu_percent` = busy time / wall time.
///
/// ```
/// use twine_core::{Duration, Instant};
/// use twine_engine::PerfMonitor;
/// let mut m = PerfMonitor::default();
/// let mut t = Instant::from_millis(0);
/// for _ in 0..=62 {
///     m.frame(t, 4_000); // 4 ms of work per frame
///     t += Duration::ms(16);
/// }
/// assert!(m.window_done(t));
/// assert_eq!(m.fps(), 62);
/// assert_eq!(m.cpu_percent(), 25);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerfMonitor {
    window_start: Option<Instant>,
    frames: u32,
    busy_us: u64,
    fps: u16,
    cpu: u8,
}

impl PerfMonitor {
    /// The window length.
    pub const WINDOW: Duration = Duration::secs(1);

    /// Records a frame at `now` that kept the CPU busy for `busy_us`.
    pub fn frame(&mut self, now: Instant, busy_us: u64) {
        if self.window_start.is_none() {
            self.window_start = Some(now);
        }
        self.frames += 1;
        self.busy_us += busy_us;
    }

    /// Closes the window if it is at least one second old at `now`: computes `fps` and
    /// `cpu_percent` and starts a new window. Returns whether a window was closed.
    pub fn window_done(&mut self, now: Instant) -> bool {
        let Some(start) = self.window_start else {
            return false;
        };
        let wall = now.saturating_duration_since(start).as_micros();
        if wall < Self::WINDOW.as_micros() {
            return false;
        }
        self.fps = (u64::from(self.frames) * 1_000_000 / wall).min(u64::from(u16::MAX)) as u16;
        self.cpu = (self.busy_us * 100 / wall).min(100) as u8;
        self.window_start = Some(now);
        self.frames = 0;
        self.busy_us = 0;
        true
    }

    /// Frames per second of the last closed window.
    #[must_use]
    pub fn fps(&self) -> u16 {
        self.fps
    }

    /// CPU load in percent of the last closed window.
    #[must_use]
    pub fn cpu_percent(&self) -> u8 {
        self.cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_needs_a_second() {
        let mut m = PerfMonitor::default();
        assert!(!m.window_done(Instant::from_millis(5000)));
        m.frame(Instant::from_millis(0), 500_000);
        assert!(!m.window_done(Instant::from_millis(999)));
        assert!(m.window_done(Instant::from_millis(1000)));
        assert_eq!((m.fps(), m.cpu_percent()), (1, 50));
        // Empty window: zero.
        assert!(m.window_done(Instant::from_millis(2000)));
        assert_eq!((m.fps(), m.cpu_percent()), (0, 0));
    }
}
