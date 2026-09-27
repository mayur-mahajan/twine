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
    /// Attaches a draw accelerator (e.g. `twine_accel_stm32::Dma2d`): every chunk the engine
    /// renders passes it to the `Painter`, which offloads fills, blits and glyph blending and
    /// draws whatever the accelerator reports `Unsupported` in software. The painter waits for
    /// queued operations before a chunk is flushed or presented. Replaces any previous
    /// accelerator; `None` returns to software rendering.
    pub fn set_accel(&mut self, accel: Option<Box<dyn DrawAccel>>) {
        twine_core::info!(target: "twine::engine", "draw accelerator {}", if accel.is_some() { "attached" } else { "detached" });
        if let Some(res) = self.res.as_mut() {
            res.accel.0 = accel;
        }
    }

    /// Whether a draw accelerator is attached.
    pub fn has_accel(&self) -> bool {
        self.res.as_ref().is_some_and(|r| r.accel.0.is_some())
    }
}
