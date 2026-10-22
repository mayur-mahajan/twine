//! Helpers shared by the widgets: DPI scaling, per-part extra draw sizes, inclusive-area
//! arithmetic for ports of LVGL geometry code, and input-device queries.

use twine_core::Rect;
use twine_engine::{Engine, InputKind, MeasureCx, NodeId};
use twine_render::{ShadowDsc, shadow_ext_size};
use twine_style::{BaseDir, Length, Part, PropId};

/// LVGL `LV_DPI_DEF`: the DPI class default sizes are written for.
pub(crate) const DPI_DEF: i32 = 130;

/// The DPI of the display showing `id` (the default display's, else [`DPI_DEF`]).
pub(crate) fn display_dpi(e: &Engine, id: NodeId) -> u16 {
    e.display_of(id)
        .or_else(|| e.default_display())
        .and_then(|d| e.display_info(d))
        .map_or(DPI_DEF as u16, |i| i.dpi)
}

/// LVGL `LV_DPX_CALC(dpi, px)` / `lv_dpx`: `px` at 160 DPI scaled to the node's display
/// (saturated to `i32`; exact for every `px` whose scaled value fits).
pub(crate) fn dpx(e: &Engine, id: NodeId, px: i32) -> i32 {
    let dpi = i64::from(display_dpi(e, id));
    // SAFE-ARITH: `dpi < 2^16` and `|px| ≤ 2^31`, so `dpi · |px| + 80 < 2^48`.
    let scaled = |p: u32| (dpi.wrapping_mul(i64::from(p)).wrapping_add(80) / 160).max(1);
    match px {
        0 => 0,
        p if p > 0 => sat_i32(scaled(p.unsigned_abs())),
        // SAFE-ARITH: `scaled(..) ≤ 2^48`, its negation fits `i64`.
        p => sat_i32(scaled(p.unsigned_abs()).wrapping_neg()),
    }
}

/// `v` saturated to `i32`.
pub(crate) const fn sat_i32(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

/// `(a + b) / 2` rounded toward zero, for every pair of `i32`s (no overflow).
pub(crate) const fn mid(a: i32, b: i32) -> i32 {
    // SAFE-ARITH: the sum of two `i32`s fits in `i64` and half of it fits back in `i32`.
    ((a as i64).wrapping_add(b as i64) / 2) as i32
}

/// `max − min` of a value range in `i64`: exact for every pair of `i32`s.
pub(crate) const fn span(min: i32, max: i32) -> i64 {
    // SAFE-ARITH: the difference of two `i32`s fits `i64`.
    (max as i64).wrapping_sub(min as i64)
}

/// `r · t >> 15`: a radius `r` scaled by a trigonometric value `t` of
/// [`sin`](twine_core::math::sin)/[`cos`](twine_core::math::cos) (`|t| ≤ 32767`); the result
/// is at most `|r|` in magnitude.
pub(crate) const fn trig_scale(r: i32, t: i32) -> i32 {
    // SAFE-ARITH: `|r| ≤ 2^31` and `|t| < 2^15`, so the product is below 2^46 in magnitude.
    ((r as i64).wrapping_mul(t as i64) >> 15) as i32
}

/// `len · (v − min) / span`: the position of the value `v` along `len` pixels for a value
/// range `min ..= min + span` (`span` from [`span`], `0` taken as 1), truncated toward zero.
///
/// Exact for every `i32` value, range and length: `|len| ≤ 2^31` and `|v − min| < 2^32`, so
/// the product stays below 2^63 in `i64`. A position outside `i32` (a `v` far outside the
/// range) saturates.
pub(crate) const fn value_pos(len: i32, v: i32, min: i32, span: i64) -> i32 {
    let span = if span == 0 { 1 } else { span };
    // SAFE-ARITH: see above.
    let num = (len as i64).wrapping_mul((v as i64).wrapping_sub(min as i64));
    match num.checked_div(span) {
        Some(q) => sat_i32(q),
        // `span ≠ 0`, and `num` is never `i64::MIN` (`|num| < 2^63`): not taken.
        None => 0,
    }
}

/// The value at the pointer offset `rel` along `len` pixels for a value range
/// `min ..= min + span` (LVGL `update_knob_pos`): `(rel · span + len / 2) / len + min`, or
/// `rel` when `len` is 0.
///
/// `rel` is first clamped between 0 and `len` (a pointer beyond either end gives the value of
/// that end, which the callers clamp to anyway), so for every `i32` length, offset and range
/// with `0 ≤ span < 2^32` the computation is exact in `i64`: `|rel · span| ≤ 2^31 · (2^32 − 1)`
/// and `|len / 2| ≤ 2^30` sum to less than 2^63.
pub(crate) const fn pos_value(rel: i32, len: i32, min: i32, span: i64) -> i32 {
    if len == 0 {
        return rel;
    }
    let (lo, hi) = if len < 0 { (len, 0) } else { (0, len) };
    let rel = if rel < lo {
        lo
    } else if rel > hi {
        hi
    } else {
        rel
    };
    // SAFE-ARITH: see above (`span < 2^32` for a value range of two `i32`s).
    let num = (rel as i64).wrapping_mul(span).wrapping_add((len / 2) as i64);
    match num.checked_div(len as i64) {
        // SAFE-ARITH: `|q| ≤ |span| + 1 < 2^33`, so adding an `i32` fits.
        Some(q) => sat_i32(q.wrapping_add(min as i64)),
        // `len ≠ 0` and `num > i64::MIN`: not taken.
        None => min,
    }
}

/// `a + (b − a) / 2` (truncated toward zero), the middle of the inclusive span `a ..= b`
/// as LVGL computes it, for every pair of `i32`s.
pub(crate) const fn span_mid(a: i32, b: i32) -> i32 {
    // SAFE-ARITH: `|b − a| < 2^33` in `i64`; the result lies between `a` and `b`.
    (a as i64).wrapping_add((b as i64).wrapping_sub(a as i64) / 2) as i32
}

/// `from + (to − from) · s / 256`: an animation step `s ∈ 0 ..= 256` between two `i32`s
/// (exact for every pair; truncated toward zero like LVGL's integer animation; a step outside
/// `0 ..= 256` is clamped into it).
pub(crate) const fn lerp256(from: i32, to: i32, s: i32) -> i32 {
    let s = if s < 0 {
        0
    } else if s > 256 {
        256
    } else {
        s
    };
    // SAFE-ARITH: `|to − from| < 2^33` and `s ≤ 256`, so the product is below 2^42; the
    // result lies between `from` and `to`.
    let d = (to as i64).wrapping_sub(from as i64).wrapping_mul(s as i64) / 256;
    sat_i32((from as i64).wrapping_add(d))
}

/// `max + min − v`: `v` reflected inside the range `min ..= max` (either order), exact for
/// every `i32` range (the result of a `v` in the range is in the range; others saturate).
pub(crate) const fn reflect(v: i32, min: i32, max: i32) -> i32 {
    // SAFE-ARITH: three `i32` terms sum to less than 2^33 in magnitude.
    sat_i32((max as i64).wrapping_add(min as i64).wrapping_sub(v as i64))
}

/// A circle's circumference `2πr` (π ≈ 3.14, LVGL's `2 · r · 314 / 100`) in `i64`, exact for
/// every radius, at least 1.
pub(crate) const fn circumference(r: i32) -> i64 {
    // SAFE-ARITH: `|r| ≤ 2^31`, so `628 · r` stays below 2^41 in magnitude.
    let c = (r as i64).wrapping_mul(628) / 100;
    if c < 1 { 1 } else { c }
}

/// `turn · len / circumference`: the angle (in the unit of `turn`) that an arc of length `len`
/// spans (saturated to `i32`). `circumference` comes from [`circumference`] (`≥ 1`).
pub(crate) fn per_circumference(turn: i32, len: i32, circumference: i64) -> i32 {
    // SAFE-ARITH: the product of two `i32`s fits `i64`.
    let num = i64::from(turn).wrapping_mul(i64::from(len));
    // `circumference ≥ 1`: the fallback is never taken.
    num.checked_div(circumference).map_or(0, sat_i32)
}

/// `x² + y²` of a pixel offset, exact for every pair of `i32`s (`≤ 2^63`).
pub(crate) const fn hyp2(x: i32, y: i32) -> u64 {
    let (x, y) = (x.unsigned_abs() as u64, y.unsigned_abs() as u64);
    // SAFE-ARITH: `x, y ≤ 2^31`, so `x² + y² ≤ 2^63 < 2^64`.
    x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y))
}

/// `(2r)²` for a radius `r` (saturating at `u64::MAX`; exact for `|r| < 2^31`), to compare
/// with the [`hyp2`] of doubled pixel-center offsets.
pub(crate) const fn diam2(r: i32) -> u64 {
    let d = 2 * (r.unsigned_abs() as u64);
    d.saturating_mul(d)
}

/// The doubled offset `2·(a − b) + 1` of a pixel center `a` from a pixel corner `b`
/// (saturating; exact for screen coordinates).
pub(crate) const fn center_offset2(a: i32, b: i32) -> i32 {
    a.saturating_sub(b).saturating_mul(2).saturating_add(1)
}

/// The transform width and height of `part` in pixels (LVGL
/// `lv_obj_get_style_transform_width/height`; percentages of the node's size).
pub(crate) fn transform_wh(cx: &MeasureCx<'_>, part: Part) -> (i32, i32) {
    let c = cx.coords();
    let px = |p, basis: i32| match cx.style(part, p).as_length() {
        Some(Length::Px(v)) => v,
        // SAFE-ARITH: the product of two `i32`s fits `i64`; a percentage of an `i32` basis
        // within ±100 % fits back (larger ones saturate).
        Some(Length::Pct(pct)) => sat_i32(i64::from(basis).wrapping_mul(i64::from(pct)) / 100),
        _ => 0,
    };
    (
        px(PropId::TransformWidth, c.width()),
        px(PropId::TransformHeight, c.height()),
    )
}

/// LVGL `lv_obj_calculate_ext_draw_size(obj, part)`: how far `part` draws outside its area
/// (shadow, outline, transform size).
pub(crate) fn part_ext_draw(cx: &MeasureCx<'_>, part: Part) -> i32 {
    let mut ext = 0;
    let sh = ShadowDsc {
        width: cx.style_i32(part, PropId::ShadowWidth),
        ofs_x: cx.style_i32(part, PropId::ShadowOffsetX),
        ofs_y: cx.style_i32(part, PropId::ShadowOffsetY),
        spread: cx.style_i32(part, PropId::ShadowSpread),
        color: twine_core::Color::BLACK,
        opa: cx.engine().style_opa(cx.node(), part, PropId::ShadowOpacity),
    };
    if sh.is_visible() {
        ext = ext.max(shadow_ext_size(&sh));
    }
    let ow = cx.style_i32(part, PropId::OutlineWidth);
    if ow > 0
        && !cx
            .engine()
            .style_opa(cx.node(), part, PropId::OutlineOpacity)
            .is_transparent()
    {
        ext = ext.max(ow.saturating_add(cx.style_i32(part, PropId::OutlineOffset).max(0)));
    }
    let (tw, th) = transform_wh(cx, part);
    ext.max(tw).max(th)
}

/// Clamps an extra draw size to the `u16` the engine stores.
pub(crate) fn ext_u16(v: i32) -> u16 {
    u16::try_from(v.max(0)).unwrap_or(u16::MAX)
}

/// Whether the node's `Main` part has a right-to-left base direction.
pub(crate) fn is_rtl(cx: &MeasureCx<'_>) -> bool {
    cx.style(Part::Main, PropId::BaseDir).get::<BaseDir>() == Some(BaseDir::Rtl)
}

/// The kind of the input device being processed, if any.
pub(crate) fn active_input_kind(e: &Engine) -> Option<InputKind> {
    e.active_input().and_then(|i| e.input_kind(i))
}

/// Whether an ancestor of `id` is being scrolled (a press that turned into a scroll).
pub(crate) fn ancestor_scrolling(e: &Engine, id: NodeId) -> bool {
    e.tree().ancestors(id).any(|a| a != id && e.is_scrolling(a))
}

/// An area with **inclusive** corners, as LVGL's `lv_area_t`: ports of LVGL geometry code
/// use it so that every `+ 1` / `- 1` stays exactly where LVGL has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Area {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl Area {
    /// The inclusive form of a (half-open) [`Rect`].
    pub(crate) fn from_rect(r: Rect) -> Self {
        Self {
            x1: r.x0,
            y1: r.y0,
            x2: r.x1.saturating_sub(1),
            y2: r.y1.saturating_sub(1),
        }
    }

    /// The half-open [`Rect`] (empty when inverted).
    pub(crate) fn to_rect(self) -> Rect {
        if self.x2 < self.x1 || self.y2 < self.y1 {
            return Rect::new(self.x1, self.y1, self.x1, self.y1);
        }
        Rect::new(
            self.x1,
            self.y1,
            self.x2.saturating_add(1),
            self.y2.saturating_add(1),
        )
    }

    /// LVGL `lv_area_get_width`.
    pub(crate) fn width(self) -> i32 {
        self.x2.saturating_sub(self.x1).saturating_add(1)
    }

    /// LVGL `lv_area_get_height`.
    pub(crate) fn height(self) -> i32 {
        self.y2.saturating_sub(self.y1).saturating_add(1)
    }
}

/// Whether the pixel `(x, y)` lies inside `r` with corners rounded by `radius` (LVGL
/// `lv_area_is_point_on` with a radius).
pub(crate) fn point_in_rounded(r: Rect, radius: i32, x: i32, y: i32) -> bool {
    if x < r.x0 || x >= r.x1 || y < r.y0 || y >= r.y1 {
        return false;
    }
    let rad = radius.clamp(0, r.width().min(r.height()) / 2);
    if rad == 0 {
        return true;
    }
    // SAFE-ARITH: `0 ≤ rad ≤ (x1 − x0) / 2` (and the same for `y`), so `x0 + rad` and
    // `x1 − rad` lie within `x0 ..= x1`.
    let (x0r, x1r) = (r.x0.wrapping_add(rad), r.x1.wrapping_sub(rad));
    // SAFE-ARITH: as above.
    let (y0r, y1r) = (r.y0.wrapping_add(rad), r.y1.wrapping_sub(rad));
    let cx0 = if x < x0r {
        x0r
    } else if x >= x1r {
        x1r
    } else {
        return true;
    };
    let cy0 = if y < y0r {
        y0r
    } else if y >= y1r {
        y1r
    } else {
        return true;
    };
    // `|x − cx0| ≤ rad ≤ 2^30` in a corner, so the doubled offsets and their squares are exact.
    let off = |a: i32, c: i32| {
        // SAFE-ARITH: `|a − c| < 2^32`, so `2·(a − c) + 1` fits `i64`.
        (i64::from(a)
            .wrapping_sub(i64::from(c))
            .wrapping_mul(2)
            .wrapping_add(1))
        .unsigned_abs()
    };
    let (dx, dy) = (off(x, cx0), off(y, cy0));
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)) <= diam2(rad)
}

/// LVGL `lv_area_is_in(inner, outer, radius)`: `inner` lies inside `outer` whose corners are
/// rounded by `radius`.
pub(crate) fn rect_in_rounded(inner: Rect, outer: Rect, radius: i32) -> bool {
    if !outer.contains_rect(&inner) {
        return false;
    }
    if radius == 0 || inner.is_empty() {
        return true;
    }
    [
        (inner.x0, inner.y0),
        // SAFE-ARITH: `inner` is not empty, so `x1 > x0` and `y1 > y0`.
        (inner.x1.wrapping_sub(1), inner.y0),
        // SAFE-ARITH: as above.
        (inner.x0, inner.y1.wrapping_sub(1)),
        // SAFE-ARITH: as above.
        (inner.x1.wrapping_sub(1), inner.y1.wrapping_sub(1)),
    ]
    .into_iter()
    .all(|(x, y)| point_in_rounded(outer, radius, x, y))
}

/// Logs a value change (`debug!` at `twine::engine`).
pub(crate) fn log_value(class: &str, v: i32) {
    twine_core::debug!(target: "twine::engine", "{} value {}", class, v);
    let _ = (class, v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_round_trip() {
        let r = Rect::new(3, 4, 10, 20);
        let a = Area::from_rect(r);
        assert_eq!((a.width(), a.height()), (7, 16));
        assert_eq!(a.to_rect(), r);
        assert!(
            Area {
                x1: 5,
                y1: 5,
                x2: 4,
                y2: 9
            }
            .to_rect()
            .is_empty()
        );
    }

    /// The exact value of `num / den` truncated toward zero, saturated to `i32`.
    fn exact(num: i128, den: i128) -> i32 {
        i32::try_from(num / den).unwrap_or(if (num < 0) == (den < 0) {
            i32::MAX
        } else {
            i32::MIN
        })
    }

    proptest::proptest! {
        /// The value ↔ position helpers equal the `i128` formulas for every `i32` input
        /// (exact where the result fits, saturated where it does not).
        #[test]
        fn value_mapping_is_exact(len: i32, v: i32, a: i32, b: i32, s in 0i32..=256, rel: i32) {
            let (min, max) = (a.min(b), a.max(b));
            let span = i128::from(max) - i128::from(min);
            let den = if span == 0 { 1 } else { span };
            proptest::prop_assert_eq!(
                value_pos(len, v, min, super::span(min, max)),
                exact(i128::from(len) * (i128::from(v) - i128::from(min)), den)
            );
            let lerp = i128::from(a) + (i128::from(b) - i128::from(a)) * i128::from(s) / 256;
            proptest::prop_assert_eq!(i128::from(lerp256(a, b, s)), lerp);
            proptest::prop_assert_eq!(mid(a, b), exact(i128::from(a) + i128::from(b), 2));
            proptest::prop_assert_eq!(
                i128::from(span_mid(a, b)),
                i128::from(a) + (i128::from(b) - i128::from(a)) / 2
            );
            proptest::prop_assert_eq!(reflect(v.clamp(min, max), min, max), exact(
                i128::from(max) + i128::from(min) - i128::from(v.clamp(min, max)),
                1
            ));
            proptest::prop_assert_eq!(
                u128::from(hyp2(a, b)),
                i128::from(a).unsigned_abs().pow(2) + i128::from(b).unsigned_abs().pow(2)
            );
            if len != 0 {
                let (lo, hi) = if len < 0 { (len, 0) } else { (0, len) };
                let r = i128::from(rel.clamp(lo, hi));
                let q = (r * span + i128::from(len / 2)) / i128::from(len) + i128::from(min);
                proptest::prop_assert_eq!(pos_value(rel, len, min, super::span(min, max)), exact(q, 1));
            }
        }
    }

    #[test]
    fn rounded_containment() {
        let outer = Rect::new(0, 0, 20, 10);
        assert!(point_in_rounded(outer, 5, 10, 0));
        assert!(!point_in_rounded(outer, 5, 0, 0));
        assert!(rect_in_rounded(Rect::new(5, 0, 15, 10), outer, 5));
        assert!(!rect_in_rounded(Rect::new(0, 0, 15, 10), outer, 5));
    }
}
