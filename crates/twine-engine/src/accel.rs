//! Draw accelerator of the engine ([`Engine::set_accel`]).

use alloc::boxed::Box;

use twine_render::DrawAccel;

use crate::Engine;

/// The accelerator handed to every `Painter` the engine creates (`None`: software only).
#[derive(Default)]
pub(crate) struct AccelSlot(pub(crate) Option<Box<dyn DrawAccel>>);

impl core::fmt::Debug for AccelSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(if self.0.is_some() {
            "AccelSlot(Some)"
        } else {
            "AccelSlot(None)"
        })
    }
}

impl Engine {
    /// Attaches a draw accelerator (e.g. `twine_accel_stm32::Dma2d`), taking ownership of it
    /// (and so of the peripheral it drives): every chunk the engine renders passes it to the
    /// `Painter`, which offloads fills, blits and glyph blending and draws whatever the
    /// accelerator reports `Unsupported` in software. The painter waits for queued operations
    /// before a chunk is flushed or presented. Replaces (drops) any previous accelerator.
    ///
    /// With the declarative runtime, pass it to the builder instead
    /// (`Ui::builder_fb(display).accel(..)`). Boxed once here; each accelerated operation is
    /// one dynamic call, as before.
    ///
    /// ```
    /// use twine_core::{Color, Opa, Rect};
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};
    ///
    /// /// An accelerator that declines everything (the software renderer draws it all).
    /// struct Decline;
    /// impl DrawAccel for Decline {
    ///     fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult { AccelResult::Unsupported }
    ///     fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult { AccelResult::Unsupported }
    ///     fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult { AccelResult::Unsupported }
    ///     fn wait(&mut self) {}
    /// }
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// e.set_accel(Decline);
    /// assert!(e.has_accel());
    /// e.clear_accel();
    /// assert!(!e.has_accel());
    /// ```
    pub fn set_accel(&mut self, accel: impl DrawAccel + 'static) {
        self.install_accel(Some(Box::new(accel)));
    }

    /// Detaches (drops) the draw accelerator: rendering returns to software only. No-op
    /// without one. Never panics (dropping the accelerator runs its `Drop`, if any).
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// e.clear_accel(); // no accelerator: nothing to do
    /// assert!(!e.has_accel());
    /// ```
    pub fn clear_accel(&mut self) {
        self.install_accel(None);
    }

    #[cold]
    #[inline(never)]
    fn install_accel(&mut self, accel: Option<Box<dyn DrawAccel>>) {
        twine_core::info!(target: "twine::engine", "draw accelerator {}", if accel.is_some() { "attached" } else { "detached" });
        if let Some(res) = self.res.as_mut() {
            res.accel.0 = accel;
        }
    }

    /// Whether a draw accelerator is attached.
    #[must_use]
    pub fn has_accel(&self) -> bool {
        self.res.as_ref().is_some_and(|r| r.accel.0.is_some())
    }
}
