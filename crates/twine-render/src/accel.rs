//! [`DrawAccel`]: the optional hardware acceleration hook (e.g. STM32 DMA2D).

use twine_core::{Color, Opa, Rect};

use crate::{DrawBuf, ImagePixels};

/// Result of an accelerated operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AccelResult {
    /// Finished.
    Done,
    /// Started; [`DrawAccel::wait`] must be called before the CPU touches the buffer again.
    Queued,
    /// Not supported for these arguments: the software path is used.
    Unsupported,
}

/// A hardware accelerator for common operations. The software renderer is always complete;
/// an accelerator only speeds up what it supports.
///
/// ```
/// use twine_core::{Color, Opa, Rect};
/// use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};
///
/// struct Nothing;
/// impl DrawAccel for Nothing {
///     fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult { AccelResult::Unsupported }
///     fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult { AccelResult::Unsupported }
///     fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult { AccelResult::Unsupported }
///     fn wait(&mut self) {}
/// }
/// ```
pub trait DrawAccel {
    /// Fills `area` (inside `dst` and the clip) with `color` at `opa`.
    fn fill(&mut self, dst: &mut DrawBuf<'_>, area: Rect, color: Color, opa: Opa) -> AccelResult;
    /// Copies/blends `src` onto `dst_area` (same size as `src`).
    fn blit(&mut self, dst: &mut DrawBuf<'_>, dst_area: Rect, src: &ImagePixels<'_>, opa: Opa)
    -> AccelResult;
    /// Blends `color` through an 8-bit alpha map (`stride` bytes per row) onto `area`.
    fn blend_a8(
        &mut self,
        dst: &mut DrawBuf<'_>,
        area: Rect,
        color: Color,
        alpha: &[u8],
        stride: usize,
    ) -> AccelResult;
    /// Waits until all queued operations have finished.
    fn wait(&mut self);
}
