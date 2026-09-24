//! The performance overlay (feature `perf-monitor`): a small box on the system layer showing
//! FPS, CPU load and the render / flush times, updated once per second.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use twine_core::{Color, Opa, Size};
use twine_style::{Align, Selector, StyleProp};
use twine_text::TextDsc;

use crate::{DisplayId, DrawCx, Engine, EngineError, MeasureCx, NodeId, Widget, WidgetClass};

/// Class of [`PerfOverlay`].
pub static PERF_OVERLAY_CLASS: WidgetClass = WidgetClass::new("perf_overlay");

/// Margin between the overlay and the screen corner, and around the text.
const MARGIN: i32 = 4;

static WARN_NO_FONT: AtomicBool = AtomicBool::new(false);

/// The overlay widget: draws its text on a translucent dark box with the configured
/// `default_font` (nothing without a font).
#[derive(Debug, Default)]
pub struct PerfOverlay {
    text: heapless::String<64>,
}

impl Widget for PerfOverlay {
    fn class(&self) -> &'static WidgetClass {
        &PERF_OVERLAY_CLASS
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        let Some(font) = cx.engine().config().default_font else {
            if !WARN_NO_FONT.load(Ordering::Relaxed) {
                WARN_NO_FONT.store(true, Ordering::Relaxed);
                twine_core::warn!(target: "twine::perf", "perf overlay: no default_font configured, nothing drawn");
            }
            return;
        };
        let area = cx.coords();
        cx.painter().fill(area, Color::BLACK, Opa::P70);
        let mut t = TextDsc::new(font);
        t.color = Color::WHITE;
        let inner = cx.content_area();
        cx.draw_text(inner, &self.text, &t);
    }

    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        cx.engine().config().default_font.map_or(Size::ZERO, |f| {
            TextDsc::new(f).layout(&self.text, i32::MAX).measure()
        })
    }

    fn text(&self) -> Option<&str> {
        Some(&self.text)
    }
}

impl Engine {
    /// Shows or hides the performance overlay of `display` (bottom-right corner of the system
    /// layer). Its own redraws are excluded from the frame statistics.
    pub fn set_perf_overlay(&mut self, display: DisplayId, on: bool) -> Result<(), EngineError> {
        let d = display.index();
        let disp = self
            .displays
            .get(d)
            .ok_or(EngineError::DisplayNotFound(display))?;
        match (disp.perf_overlay, on) {
            (None, true) => {
                let sys = disp.sys_layer;
                let id = self.create(sys, alloc::boxed::Box::new(PerfOverlay::default()))?;
                self.set_flag(id, crate::ObjFlags::CLICKABLE, false);
                // Content-sized (the text) plus a margin, in the bottom-right corner.
                for p in [
                    StyleProp::PadLeft(MARGIN),
                    StyleProp::PadTop(MARGIN),
                    StyleProp::PadRight(MARGIN),
                    StyleProp::PadBottom(MARGIN),
                ] {
                    self.set_local_prop(id, Selector::MAIN, p);
                }
                self.align(id, Align::BottomRight, -MARGIN, -MARGIN);
                self.displays[d].perf_overlay = Some(id);
                self.update_perf_overlay(d, id);
            }
            (Some(id), false) => {
                self.displays[d].perf_overlay = None;
                self.delete(id)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// The overlay node of `display`, if shown.
    #[must_use]
    pub fn perf_overlay(&self, display: DisplayId) -> Option<NodeId> {
        self.displays.get(display.index()).and_then(|d| d.perf_overlay)
    }

    pub(crate) fn update_perf_overlays(&mut self) {
        for d in 0..self.displays.len() {
            if let Some(id) = self.displays[d].perf_overlay {
                self.update_perf_overlay(d, id);
            }
        }
    }

    fn update_perf_overlay(&mut self, d: usize, id: NodeId) {
        let s = self.displays[d].refresher.stats;
        let mut text: heapless::String<64> = heapless::String::new();
        let _ = write!(
            text,
            "{} FPS {}% CPU\n{}.{} ms R {}.{} ms F",
            self.perf.fps(),
            self.perf.cpu_percent(),
            s.render_us / 1000,
            s.render_us % 1000 / 100,
            s.flush_us / 1000,
            s.flush_us % 1000 / 100
        );
        if let Some(w) = self.tree.get_mut::<PerfOverlay>(id) {
            if w.text == text {
                return;
            }
            w.text = text;
        }
        // The overlay's invalidations go to its own area (kept out of the statistics).
        self.invalidate(id, crate::InvalidateReason::WidgetSetter("perf_overlay"));
        self.mark_layout_dirty(id);
    }
}
