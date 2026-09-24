//! Drawing a [`Code128`] with a [`Painter`].

use twine_core::{Color, Opa, Rect};
use twine_render::Painter;

use super::Code128;

/// Direction in which a barcode is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Orientation {
    /// Read left to right; bars are vertical.
    #[default]
    Horizontal,
    /// Read top to bottom; bars are horizontal.
    Vertical,
}

/// Colors, module size, direction and quiet zone used by [`draw_barcode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BarcodeStyle {
    /// Color of the bars.
    pub dark: Color,
    /// Color of the spaces, the quiet zones and the rest of the area.
    pub light: Color,
    /// Pixels per module (0 is treated as 1).
    pub scale: u8,
    /// Reading direction.
    pub orientation: Orientation,
    /// Light margin before and after the symbol in modules (Code 128 needs at least 10).
    pub quiet_zone: u8,
}

impl Default for BarcodeStyle {
    /// Black on white, 1 px per module, horizontal, 10-module quiet zones.
    fn default() -> Self {
        Self {
            dark: Color::BLACK,
            light: Color::WHITE,
            scale: 1,
            orientation: Orientation::Horizontal,
            quiet_zone: 10,
        }
    }
}

impl BarcodeStyle {
    /// Length in pixels along the reading direction: `(modules + 2 × quiet_zone) × scale`.
    ///
    /// ```
    /// use twine_extra::barcode::{BarcodeStyle, Code128};
    ///
    /// let code = Code128::encode("A").unwrap(); // 4 symbols: 3 × 11 + 13 = 46 modules
    /// let style = BarcodeStyle { scale: 2, ..BarcodeStyle::default() };
    /// assert_eq!(style.length_px(&code), (46 + 20) * 2);
    /// ```
    #[must_use]
    pub fn length_px(&self, code: &Code128) -> i32 {
        (code.module_count() as i32 + 2 * i32::from(self.quiet_zone)) * i32::from(self.scale.max(1))
    }
}

/// Draws `code` in `area`: the area is filled with `style.light` once, then the bars span the
/// whole area across the reading direction. Along it the symbol (with its quiet zones) is centred;
/// a symbol longer than the area is clipped at both ends (make the area at least
/// [`BarcodeStyle::length_px`] long). Only bars inside the painter's clip are drawn.
///
/// ```
/// use twine_core::{ColorFormat, Rect};
/// use twine_extra::barcode::{BarcodeStyle, Code128, draw_barcode};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
///
/// let code = Code128::encode("twine").unwrap();
/// let style = BarcodeStyle::default();
/// let area = Rect::from_xywh(0, 0, style.length_px(&code), 20);
/// let mut caches = RenderCaches::default();
/// let mut px = vec![0u8; area.area() as usize];
/// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, area).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// draw_barcode(&mut p, area, &code, &style);
/// ```
pub fn draw_barcode(p: &mut Painter<'_>, area: Rect, code: &Code128, style: &BarcodeStyle) {
    p.fill(area, style.light, Opa::COVER);
    let s = i32::from(style.scale.max(1));
    let vertical = style.orientation == Orientation::Vertical;
    let (start, extent) = if vertical {
        (area.y0, area.height())
    } else {
        (area.x0, area.width())
    };
    let first = start + (extent - style.length_px(code)) / 2 + i32::from(style.quiet_zone) * s;
    let clip = p.clip();
    let (clip0, clip1) = if vertical {
        (clip.y0, clip.y1)
    } else {
        (clip.x0, clip.x1)
    };
    for (m, w) in code.bars() {
        let a = first + m as i32 * s;
        let b = a + i32::from(w) * s;
        if b <= clip0 {
            continue;
        }
        if a >= clip1 {
            break;
        }
        let bar = if vertical {
            Rect::new(area.x0, a, area.x1, b)
        } else {
            Rect::new(a, area.y0, b, area.y1)
        };
        // Bars never leave the area, even when the symbol is longer than it.
        if let Some(bar) = bar.intersection(&area) {
            p.fill(bar, style.dark, Opa::COVER);
        }
    }
}
