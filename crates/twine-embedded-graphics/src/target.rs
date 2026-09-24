//! [`PainterTarget`]: a twine [`Painter`] as an embedded-graphics [`DrawTarget`] (feature
//! `drawtarget`).

use core::convert::Infallible;
use core::marker::PhantomData;

use embedded_graphics_core::Pixel;
use embedded_graphics_core::draw_target::DrawTarget;
use embedded_graphics_core::geometry::{Dimensions, Point, Size};
use embedded_graphics_core::primitives::Rectangle;
use twine_core::{Opa, Rect};
use twine_render::Painter;

use crate::EgColor;

/// Draws embedded-graphics primitives, text and images through a twine [`Painter`]: clipping,
/// masks and the accelerator of the painter apply. The bounding box is the painter's clip.
///
/// The colour type `C` is the one the embedded-graphics styles use; any [`EgColor`] works on any
/// draw buffer format (colours are converted).
///
/// ```
/// use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}};
/// use twine_core::{ColorFormat, Rect};
/// use twine_embedded_graphics::PainterExt;
/// use twine_render::{DrawBuf, Painter, RenderCaches};
///
/// let mut caches = RenderCaches::default();
/// let mut px = vec![0u8; 8 * 4];
/// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, 8, 4)).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// Rectangle::new(Point::new(2, 1), Size::new(3, 2))
///     .into_styled(PrimitiveStyle::with_fill(Rgb565::WHITE))
///     .draw(&mut p.as_draw_target::<Rgb565>())
///     .unwrap();
/// drop(p);
/// assert_eq!(&px[8..16], &[0, 0, 255, 255, 255, 0, 0, 0]);
/// ```
#[derive(Debug)]
pub struct PainterTarget<'p, 'a, C> {
    painter: &'p mut Painter<'a>,
    opa: Opa,
    _color: PhantomData<C>,
}

impl<'p, 'a, C: EgColor> PainterTarget<'p, 'a, C> {
    /// Draws into `painter` at full opacity.
    pub fn new(painter: &'p mut Painter<'a>) -> Self {
        Self {
            painter,
            opa: Opa::COVER,
            _color: PhantomData,
        }
    }

    /// Draws everything at opacity `opa`.
    #[must_use]
    pub fn with_opa(mut self, opa: Opa) -> Self {
        self.opa = opa;
        self
    }

    fn fill(&mut self, r: Rect, c: C) {
        self.painter.fill(r, c.to_color(), self.opa);
    }
}

/// `Rectangle` → half-open twine [`Rect`] (saturating for huge sizes).
fn to_rect(r: &Rectangle) -> Rect {
    let w = i32::try_from(r.size.width).unwrap_or(i32::MAX);
    let h = i32::try_from(r.size.height).unwrap_or(i32::MAX);
    Rect::new(
        r.top_left.x,
        r.top_left.y,
        r.top_left.x.saturating_add(w),
        r.top_left.y.saturating_add(h),
    )
}

impl<C: EgColor> Dimensions for PainterTarget<'_, '_, C> {
    fn bounding_box(&self) -> Rectangle {
        let c = self.painter.clip();
        Rectangle::new(
            Point::new(c.x0, c.y0),
            Size::new(c.width().max(0) as u32, c.height().max(0) as u32),
        )
    }
}

impl<C: EgColor> DrawTarget for PainterTarget<'_, '_, C> {
    type Color = C;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = Pixel<C>>,
    {
        let clip = self.painter.clip();
        for Pixel(p, c) in pixels {
            if clip.contains(twine_core::Point::new(p.x, p.y)) {
                self.fill(Rect::new(p.x, p.y, p.x + 1, p.y + 1), c);
            }
        }
        Ok(())
    }

    /// Row by row, one fill per run of equal colours.
    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = C>,
    {
        let r = to_rect(area);
        if r.is_empty() {
            return Ok(());
        }
        let mut it = colors.into_iter();
        for y in r.y0..r.y1 {
            let mut run: Option<(i32, C)> = None;
            for x in r.x0..r.x1 {
                let Some(c) = it.next() else {
                    if let Some((x0, rc)) = run {
                        self.fill(Rect::new(x0, y, x, y + 1), rc);
                    }
                    return Ok(());
                };
                match run {
                    Some((_, rc)) if rc == c => {}
                    Some((x0, rc)) => {
                        self.fill(Rect::new(x0, y, x, y + 1), rc);
                        run = Some((x, c));
                    }
                    None => run = Some((x, c)),
                }
            }
            if let Some((x0, rc)) = run {
                self.fill(Rect::new(x0, y, r.x1, y + 1), rc);
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: C) -> Result<(), Infallible> {
        self.fill(to_rect(area), color);
        Ok(())
    }

    fn clear(&mut self, color: C) -> Result<(), Infallible> {
        let clip = self.painter.clip();
        self.painter.fill(clip, color.to_color(), Opa::COVER);
        Ok(())
    }
}

/// Adds [`as_draw_target`](Self::as_draw_target) to [`Painter`].
pub trait PainterExt<'a> {
    /// The painter as an embedded-graphics draw target with colour type `C`.
    fn as_draw_target<C: EgColor>(&mut self) -> PainterTarget<'_, 'a, C>;
}

impl<'a> PainterExt<'a> for Painter<'a> {
    fn as_draw_target<C: EgColor>(&mut self) -> PainterTarget<'_, 'a, C> {
        PainterTarget::new(self)
    }
}
