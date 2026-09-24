//! Drawing a [`QrMatrix`] with a [`Painter`].

use twine_core::{Color, Opa, Point, Rect};
use twine_render::Painter;

use super::QrMatrix;

/// Colors and quiet zone used by [`draw_qr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct QrStyle {
    /// Color of dark modules.
    pub dark: Color,
    /// Color of light modules, the quiet zone and the rest of the area.
    pub light: Color,
    /// Width of the light margin around the code in modules. The standard asks for 4; phones
    /// read codes with 1–2 as long as the surrounding area is light.
    pub quiet_zone: u8,
}

impl Default for QrStyle {
    /// Black on white with a 2-module quiet zone.
    fn default() -> Self {
        Self {
            dark: Color::BLACK,
            light: Color::WHITE,
            quiet_zone: 2,
        }
    }
}

/// Where a QR code lands in an area: whole pixels per module, centred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct QrLayout {
    /// Pixels per module (at least 1).
    pub scale: i32,
    /// Top-left corner of module (0, 0) (inside the quiet zone).
    pub origin: Point,
}

impl QrLayout {
    /// Fits a code of `modules` modules per side plus `quiet_zone` modules on each side into
    /// `area`: `scale = floor(min(w, h) / (modules + 2 × quiet_zone))`, centred. `None` when not
    /// even one pixel per module fits.
    ///
    /// ```
    /// use twine_core::{Point, Rect};
    /// use twine_extra::qrcode::QrLayout;
    ///
    /// // 21 modules + 2 × 2 quiet = 25 → 4 px per module in 100 × 120 px.
    /// let l = QrLayout::fit(Rect::from_xywh(0, 0, 100, 120), 21, 2).unwrap();
    /// assert_eq!((l.scale, l.origin), (4, Point::new(8, 18)));
    /// assert_eq!(QrLayout::fit(Rect::from_xywh(0, 0, 24, 24), 21, 2), None);
    /// ```
    #[must_use]
    pub fn fit(area: Rect, modules: u8, quiet_zone: u8) -> Option<Self> {
        let n = i32::from(modules);
        let total = n + 2 * i32::from(quiet_zone);
        let scale = area.width().min(area.height()) / total.max(1);
        if scale < 1 {
            return None;
        }
        let side = n * scale;
        Some(Self {
            scale,
            origin: Point::new(
                area.x0 + (area.width() - side) / 2,
                area.y0 + (area.height() - side) / 2,
            ),
        })
    }
}

/// Draws `qr` centred in `area`: the whole area is filled with `style.light` once, then every
/// run of dark modules becomes one filled rectangle. Only rows inside the painter's clip are
/// visited. Returns the layout used, or `None` (after a warning, drawing
/// [`draw_qr_placeholder`] instead) when the area is too small for one pixel per module.
///
/// ```
/// use twine_core::{Color, ColorFormat, Rect};
/// use twine_extra::qrcode::{Ecc, QrMatrix, QrStyle, draw_qr};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
///
/// let qr = QrMatrix::encode(b"twine", Ecc::Medium).unwrap();
/// let mut caches = RenderCaches::default();
/// let mut px = vec![0u8; 100 * 100];
/// let area = Rect::from_xywh(0, 0, 100, 100);
/// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, area).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// let layout = draw_qr(&mut p, area, &qr, &QrStyle::default()).unwrap();
/// assert_eq!(layout.scale, 4); // 100 / (21 + 2 × 2)
/// ```
pub fn draw_qr(p: &mut Painter<'_>, area: Rect, qr: &QrMatrix, style: &QrStyle) -> Option<QrLayout> {
    let Some(layout) = QrLayout::fit(area, qr.size(), style.quiet_zone) else {
        twine_core::warn!(
            target: "twine::extra",
            "QR code area {} too small for {} modules",
            area,
            u32::from(qr.size()) + 2 * u32::from(style.quiet_zone)
        );
        draw_qr_placeholder(p, area, style);
        return None;
    };
    p.fill(area, style.light, Opa::COVER);
    let s = layout.scale;
    let clip = p.clip();
    // Rows of modules whose pixel band meets the clip.
    let first = ((clip.y0 - layout.origin.y).max(0) / s).min(i32::from(qr.size()));
    let last = ((clip.y1 - layout.origin.y + s - 1).max(0) / s).min(i32::from(qr.size()));
    for y in first..last {
        let py = layout.origin.y + y * s;
        for (x, n) in qr.dark_runs(y as u8) {
            let px = layout.origin.x + i32::from(x) * s;
            p.fill(
                Rect::from_xywh(px, py, i32::from(n) * s, s),
                style.dark,
                Opa::COVER,
            );
        }
    }
    Some(layout)
}

/// Draws the stand-in for a code that cannot be shown (data too long, area too small): the
/// area filled with `style.light` and outlined with `style.dark` (1/16 of the shorter side,
/// at least 1 px).
pub fn draw_qr_placeholder(p: &mut Painter<'_>, area: Rect, style: &QrStyle) {
    p.fill(area, style.light, Opa::COVER);
    let b = (area.width().min(area.height()) / 16).max(1);
    let (x0, y0, x1, y1) = (area.x0, area.y0, area.x1, area.y1);
    for r in [
        Rect::new(x0, y0, x1, y0 + b),
        Rect::new(x0, y1 - b, x1, y1),
        Rect::new(x0, y0 + b, x0 + b, y1 - b),
        Rect::new(x1 - b, y0 + b, x1, y1 - b),
    ] {
        p.fill(r, style.dark, Opa::COVER);
    }
}
