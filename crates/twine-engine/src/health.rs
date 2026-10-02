//! Display health: what the engine does when a driver fails to flush or present
//! ([`FlushPolicy`]) and what the application can observe ([`DisplayHealth`]).
//!
//! Nothing here allocates: the state is a few words per display.

use twine_core::fault::FaultKind;
use twine_core::{Duration, Instant, Rect, Rotation};

use crate::display::Backend;
use crate::refresh::Strategy;
use crate::{DisplayId, DriverErrorCode, Engine, EngineError, FaultRecord, InvalidateReason};

/// The engine's reaction to a failed flush or present.
///
/// Every failed `begin_flush` (partial buffers), `present` (framebuffers) or async flush
/// reported through [`Engine::report_flush`]
///
/// 1. raises [`FaultKind::FlushError`] for the display, with the driver's error code
///    ([`DriverErrorCode`], from `DisplayDriver::error_code`) as the record's `code`;
/// 2. counts one more consecutive error for the display (a successful flush resets it);
/// 3. applies [`EngineConfig::flush_policy`](crate::EngineConfig::flush_policy): the area that
///    did not reach the panel is marked dirty again and redrawn on the next frame
///    ([`FlushPolicy::Reinvalidate`], [`FlushPolicy::Halt`]) or dropped
///    ([`FlushPolicy::Ignore`]).
///
/// After [`max_consecutive_flush_errors`](crate::EngineConfig::max_consecutive_flush_errors)
/// consecutive errors the display is [`DisplayState::Failed`]. With [`FlushPolicy::Halt`] it
/// then stops refreshing ([`DisplayState::Halted`]) until the application calls
/// [`Engine::recover_display`] — typically after resetting the panel or its bus.
///
/// Whatever the policy, every failure raises [`FaultKind::FlushError`] and is counted in
/// [`DisplayHealth`]; the policy only decides what happens to the pixels that did not reach
/// the panel and whether a failed display keeps refreshing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FlushPolicy {
    /// Mark the failed area dirty again: it is redrawn and flushed on the next frame (at most
    /// one frame per `refr_period`), for as long as the driver keeps failing. A later
    /// successful flush makes the display healthy again on its own.
    #[default]
    Reinvalidate,
    /// Like [`Reinvalidate`](Self::Reinvalidate) until the display is
    /// [`Failed`](DisplayState::Failed); then stop refreshing it (the frame in progress is
    /// abandoned, invalidations accumulate) and report [`DisplayState::Halted`] until the
    /// application calls [`Engine::recover_display`]. For products whose fail-safe reaction
    /// must not be fought by the engine retrying a broken bus.
    Halt,
    /// Drop the failed area (the panel shows stale pixels there until the area changes again).
    /// The fault is still raised and the health still tracked.
    Ignore,
}

/// The state of a display, derived from its consecutive flush errors (see
/// [`DisplayHealth`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DisplayState {
    /// The last flush succeeded (or none failed yet).
    Healthy,
    /// The last flush failed, fewer than `max_consecutive_flush_errors` times in a row.
    Degraded,
    /// At least `max_consecutive_flush_errors` flushes failed in a row, or a flush did not
    /// finish within [`flush_timeout`](crate::EngineConfig::flush_timeout). The display keeps
    /// refreshing (unless [`FlushPolicy::Halt`]); one successful flush makes it healthy.
    Failed,
    /// Failed under [`FlushPolicy::Halt`]: not refreshed until [`Engine::recover_display`].
    Halted,
}

/// The flush health of a display ([`Engine::display_health`]).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_engine::{DisplayState, Engine, EngineConfig};
/// use twine_hal::DisplayInfo;
///
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let d = e.add_chunked_display(DisplayInfo::new(8, 8, ColorFormat::L8), 64).unwrap();
/// let h = e.display_health(d).unwrap();
/// assert_eq!((h.state, h.consecutive_errors, h.last_ok), (DisplayState::Healthy, 0, None));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct DisplayHealth {
    /// The state.
    pub state: DisplayState,
    /// Failed flushes / presents in a row (each chunk counts; `0` after a success). A flush
    /// timeout raises it to at least `max_consecutive_flush_errors`.
    pub consecutive_errors: u32,
    /// When the last flush succeeded (engine time), `None` before the first one.
    pub last_ok: Option<Instant>,
    /// The driver's code of the last failure, `None` if none failed yet.
    pub last_error: Option<DriverErrorCode>,
}

/// Per-display health bookkeeping.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HealthTracker {
    consecutive: u32,
    last_ok: Option<Instant>,
    last_error: Option<DriverErrorCode>,
    /// Stopped by [`FlushPolicy::Halt`] until `recover_display`.
    pub(crate) halted: bool,
}

/// Maps a flush area (physical for software rotation) of a `w × h` logical display back to
/// logical coordinates.
fn unrotate(area: Rect, rotation: Rotation, w: i32, h: i32) -> Rect {
    match rotation {
        Rotation::Deg0 => area,
        Rotation::Deg90 => area.rotate_in(Rotation::Deg270, h, w),
        Rotation::Deg180 => area.rotate_in(Rotation::Deg180, w, h),
        Rotation::Deg270 => area.rotate_in(Rotation::Deg90, h, w),
    }
}

impl Engine {
    /// The flush health of `display` (`None` for unknown displays): state, consecutive
    /// errors, last success and last error code. See [`FlushPolicy`].
    /// Never panics; O(1).
    ///
    /// ```
    /// use twine_core::fault::FaultKind;
    /// use twine_core::{ColorFormat, Rect};
    /// use twine_engine::{DisplayState, DriverErrorCode, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(8, 8, ColorFormat::L8), 64).unwrap();
    /// let area = Rect::from_xywh(0, 0, 8, 8);
    /// e.report_flush(d, area, Err(DriverErrorCode::new(5))); // the driver's flush failed
    /// let h = e.display_health(d).unwrap();
    /// assert_eq!(h.state, DisplayState::Degraded);
    /// assert_eq!((h.consecutive_errors, h.last_error), (1, Some(DriverErrorCode::new(5))));
    /// assert_eq!(e.last_fault(FaultKind::FlushError).map(|r| r.code), Some(5));
    /// e.recover_display(d).unwrap(); // e.g. after resetting the panel
    /// assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    /// ```
    #[must_use]
    pub fn display_health(&self, display: DisplayId) -> Option<DisplayHealth> {
        let h = &self.displays.get(display.index())?.health;
        let max = u32::from(self.config.max_consecutive_flush_errors.max(1));
        let state = if h.halted {
            DisplayState::Halted
        } else if h.consecutive == 0 {
            DisplayState::Healthy
        } else if h.consecutive >= max {
            DisplayState::Failed
        } else {
            DisplayState::Degraded
        };
        Some(DisplayHealth {
            state,
            consecutive_errors: h.consecutive,
            last_ok: h.last_ok,
            last_error: h.last_error,
        })
    }

    /// Recovers `display` after a failure (typically once the application has reset the panel
    /// or its bus): clears the consecutive error count, resumes refreshing a
    /// [`Halted`](DisplayState::Halted) display and redraws the whole display (its content is
    /// unknown). The display is [`Healthy`](DisplayState::Healthy) again until the next
    /// failure. A no-op for the counters of a healthy display (it is still redrawn).
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`] for unknown displays (never panics).
    ///
    /// ```
    /// use twine_core::{ColorFormat, Rect};
    /// use twine_engine::{DisplayState, DriverErrorCode, Engine, EngineConfig, FlushPolicy};
    /// use twine_hal::DisplayInfo;
    ///
    /// let config = EngineConfig {
    ///     flush_policy: FlushPolicy::Halt,
    ///     max_consecutive_flush_errors: 1,
    ///     ..EngineConfig::default()
    /// };
    /// let mut e = Engine::new(config).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(8, 8, ColorFormat::L8), 64).unwrap();
    /// e.report_flush(d, Rect::from_xywh(0, 0, 8, 8), Err(DriverErrorCode::NONE));
    /// assert_eq!(e.display_health(d).unwrap().state, DisplayState::Halted);
    /// e.recover_display(d).unwrap(); // the application reset the panel
    /// assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    /// ```
    pub fn recover_display(&mut self, display: DisplayId) -> Result<(), EngineError> {
        let Some(disp) = self.displays.get_mut(display.index()) else {
            twine_core::warn!(target: "twine::refresh", "recover_display: display {} not found", display);
            return Err(EngineError::DisplayNotFound(display));
        };
        let was = disp.health;
        disp.health.consecutive = 0;
        disp.health.halted = false;
        let area = disp.area();
        // A flush that hung before is given a fresh `flush_timeout`: if it is still hung, the
        // fault is raised again.
        self.restart_flush_timeouts(display.index());
        if was.halted || was.consecutive > 0 {
            twine_core::info!(
                target: "twine::refresh",
                "display {} recovered after {} consecutive flush errors",
                display,
                was.consecutive
            );
        }
        self.invalidate_area(display, area, InvalidateReason::Explicit);
        Ok(())
    }

    /// Reports the outcome of a flush of a display added with
    /// [`add_chunked_display`](Self::add_chunked_display): `area` as returned by
    /// [`render_chunk`](Self::render_chunk) (physical coordinates under software rotation),
    /// `result` the driver's outcome. Applies the [`FlushPolicy`] and updates the
    /// [`DisplayHealth`] exactly as for the engine's own flushes (the failed area is redrawn by a
    /// later frame). Call it for every flush, successful or not, also after
    /// [`refresh_end`](Self::refresh_end). Other displays report their flushes themselves; a
    /// call for them logs `warn!` and is ignored. Never panics (an unknown display is
    /// ignored with a `warn!`).
    ///
    /// ```
    /// use twine_core::fault::FaultKind;
    /// use twine_core::{ColorFormat, Rect};
    /// use twine_engine::{DisplayState, DriverErrorCode, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(8, 8, ColorFormat::L8), 64).unwrap();
    /// let area = Rect::from_xywh(0, 0, 8, 4);
    /// e.report_flush(d, area, Err(DriverErrorCode::new(3))); // area redrawn by a later frame
    /// assert!(e.take_faults().contains(FaultKind::FlushError));
    /// e.report_flush(d, area, Ok(())); // a success makes the display healthy again
    /// assert_eq!(e.display_health(d).unwrap().state, DisplayState::Healthy);
    /// ```
    pub fn report_flush(&mut self, display: DisplayId, area: Rect, result: Result<(), DriverErrorCode>) {
        let d = display.index();
        let Some(disp) = self.displays.get(d) else {
            twine_core::warn!(target: "twine::refresh", "report_flush: display {} not found", display);
            return;
        };
        if !matches!(disp.backend, Backend::External(())) {
            twine_core::warn!(target: "twine::refresh", "report_flush: display {} has its own driver", display);
            return;
        }
        match result {
            Ok(()) => self.flush_ok(d),
            Err(code) => {
                let rotation = match &disp.refresher.strategy {
                    Strategy::Partial(p) => p.rotation,
                    _ => Rotation::Deg0,
                };
                let bounds = disp.area();
                let logical = unrotate(area, rotation, bounds.width(), bounds.height());
                if self.flush_failed(d, code) {
                    if let Some(a) = logical.intersection(&bounds) {
                        self.add_dirty(d, a, InvalidateReason::FlushRetry);
                    }
                }
            }
        }
    }

    /// Records a successful flush / present of display `d`.
    pub(crate) fn flush_ok(&mut self, d: usize) {
        let now = self.anim_now();
        let h = &mut self.displays[d].health;
        if h.consecutive > 0 {
            twine_core::info!(
                target: "twine::refresh",
                "display {} flushes again after {} errors",
                d,
                h.consecutive
            );
        }
        h.consecutive = 0;
        h.last_ok = Some(now);
    }

    /// Records a failed flush / present of display `d` with the driver's `code`: raises
    /// [`FaultKind::FlushError`], counts the error, halts the display when the policy says
    /// so. Returns whether the caller must mark the failed area dirty again.
    pub(crate) fn flush_failed(&mut self, d: usize, code: DriverErrorCode) -> bool {
        let policy = self.config.flush_policy;
        let max = u32::from(self.config.max_consecutive_flush_errors.max(1));
        let h = &mut self.displays[d].health;
        h.consecutive = h.consecutive.saturating_add(1);
        h.last_error = Some(code);
        let n = h.consecutive;
        if n == max {
            twine_core::error!(
                target: "twine::refresh",
                "display {} failed: {} consecutive flush errors (last {})",
                d,
                n,
                code
            );
        }
        if n >= max && policy == FlushPolicy::Halt && !h.halted {
            h.halted = true;
            twine_core::error!(target: "twine::refresh", "display {} halted until recover_display", d);
        }
        self.raise_fault(
            FaultRecord::new(FaultKind::FlushError)
                .display(DisplayId(d as u8))
                .code(code.get()),
        );
        policy != FlushPolicy::Ignore
    }

    /// Records that a flush or swap of display `d` did not finish within `flush_timeout`
    /// (after `waited`): raises [`FaultKind::FlushTimeout`] (`code`: ms waited), makes the
    /// display [`DisplayState::Failed`] at once and halts it under [`FlushPolicy::Halt`].
    /// Returns whether the caller must mark the areas in flight dirty again.
    pub(crate) fn flush_timed_out(&mut self, d: usize, waited: Duration) -> bool {
        let policy = self.config.flush_policy;
        let max = u32::from(self.config.max_consecutive_flush_errors.max(1));
        let h = &mut self.displays[d].health;
        h.consecutive = h.consecutive.saturating_add(1).max(max);
        twine_core::error!(
            target: "twine::refresh",
            "display {} failed: flush not done after {} ms (driver hung?)",
            d,
            waited.as_millis()
        );
        if policy == FlushPolicy::Halt && !h.halted {
            h.halted = true;
            twine_core::error!(target: "twine::refresh", "display {} halted until recover_display", d);
        }
        self.raise_fault(
            FaultRecord::new(FaultKind::FlushTimeout)
                .display(DisplayId(d as u8))
                .code(waited.as_millis().min(u64::from(u32::MAX)) as u32),
        );
        policy != FlushPolicy::Ignore
    }

    /// Whether display `d` is halted by [`FlushPolicy::Halt`].
    pub(crate) fn display_halted(&self, d: usize) -> bool {
        self.displays[d].health.halted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrotate_inverts_rotate_in() {
        let (w, h) = (40, 30);
        let r = Rect::new(3, 5, 17, 11);
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            assert_eq!(unrotate(r.rotate_in(rot, w, h), rot, w, h), r, "{rot:?}");
        }
    }
}
