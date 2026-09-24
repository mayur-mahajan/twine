//! DPI scaling ([`dpx`], LVGL `LV_DPX_CALC`).

/// LVGL's default display DPI (`LV_DPI_DEF`), used when a display does not report one.
pub const DPI_DEF: u16 = 130;

/// `px` pixels designed for a 160 DPI display, scaled to `dpi` (LVGL `LV_DPX_CALC`):
/// `(dpi · px + 80) / 160`, at least 1 for positive `px`, 0 for 0; negative values are
/// scaled symmetrically.
///
/// ```
/// use twine_theme::dpx;
/// assert_eq!(dpx(1, 130), 1);
/// assert_eq!(dpx(10, 160), 10);
/// assert_eq!(dpx(10, 320), 20);
/// assert_eq!(dpx(-5, 160), -5);
/// ```
#[must_use]
pub const fn dpx(px: i32, dpi: u16) -> i32 {
    if px == 0 {
        0
    } else if px > 0 {
        let v = (dpi as i32 * px + 80) / 160;
        if v < 1 { 1 } else { v }
    } else {
        let v = (dpi as i32 * -px + 80) / 160;
        -(if v < 1 { 1 } else { v })
    }
}
