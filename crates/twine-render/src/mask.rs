//! Coverage masks ([`Mask`]) and the fixed-capacity [`MaskStack`].
//!
//! A mask multiplies a span's coverage in place ([`Mask::apply`]). The painter applies every
//! pushed mask to every span it blends, so masks affect all primitives: rounded clipping of
//! children, arcs, fades and bitmap masks.
//!
//! Geometry uses continuous coordinates: pixel `(x, y)` covers `[x, x+1) × [y, y+1)` and its
//! center is `(x + ½, y + ½)`. Anti-aliasing is 1 px wide.

use twine_core::math::{cos, isqrt64, sin};
use twine_core::{Angle, Opa, Point, Rect};

use crate::circle::Quarter;
use crate::num::{inv8, mul8, span};
use crate::rrect::{RowSpan, eff_radius};

/// Which side of a [`Mask::Line`] is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LineSide {
    /// Pixels left of the line (for a horizontal line: above).
    Left,
    /// Pixels right of the line (for a horizontal line: below).
    Right,
    /// Pixels above the line (for a vertical line: left).
    Top,
    /// Pixels below the line (for a vertical line: right).
    Bottom,
}

/// A coverage mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mask<'a> {
    /// A rounded rectangle; `outer = false` keeps the inside, `true` the outside. The radius is
    /// clamped to half the shorter side and to 2^22 px (never panics).
    Radius {
        /// The rectangle.
        area: Rect,
        /// Corner radius.
        radius: i32,
        /// Keep the outside instead of the inside.
        outer: bool,
    },
    /// Keeps the sector from `start` clockwise to `end` around the point `center` (the
    /// top-left corner of pixel `center`). Angles are clockwise from 3 o'clock;
    /// `end − start ≥ 360°` keeps everything, `start == end` nothing.
    Angle {
        /// Vertex of the sector.
        center: Point,
        /// Start angle.
        start: Angle,
        /// End angle.
        end: Angle,
    },
    /// Keeps one side of the infinite line through `p1` and `p2` (a direction longer than
    /// 2^23 px is clamped by scaling it down, which keeps the line and rounds its slope; never
    /// panics).
    Line {
        /// First point.
        p1: Point,
        /// Second point.
        p2: Point,
        /// Kept side.
        side: LineSide,
    },
    /// Vertical fade inside `area` (pixels outside `area` are unchanged): rows above `y_top` get
    /// `opa_top`, rows below `y_bottom` get `opa_bottom`, rows in between are interpolated.
    Fade {
        /// Affected area.
        area: Rect,
        /// Start of the fade.
        y_top: i32,
        /// End of the fade.
        y_bottom: i32,
        /// Opacity at and above `y_top`.
        opa_top: Opa,
        /// Opacity at and below `y_bottom`.
        opa_bottom: Opa,
    },
    /// An 8-bit alpha map covering `area` (row stride = `area.width()`); pixels outside `area`
    /// become transparent.
    Map {
        /// Covered area.
        area: Rect,
        /// Alpha values, row-major.
        alpha: &'a [u8],
    },
}

/// What [`Mask::apply`] did to a span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MaskResult {
    /// The whole span is now 0 (callers skip it).
    Transparent,
    /// The span was left unchanged.
    FullCover,
    /// Some values changed.
    Changed,
}

#[inline(always)]
fn mul(c: &mut u8, m: u8) {
    *c = mul8(*c, m);
}

/// Coverage of a half plane from a signed distance in 1/256 px (0.5 px at the boundary).
/// Callers pass distances below `2^51` in magnitude.
#[inline(always)]
fn half_plane(d256: i64) -> u8 {
    // SAFE-ARITH: `|d256| < 2^51` (the callers' range arguments).
    d256.wrapping_add(128).clamp(0, 255) as u8
}

/// Index of screen x `x` in a span starting at `x0` with `n` values: `x − x0` clamped to
/// `0..=n`.
#[inline(always)]
fn rel(x: i32, x0: i32, n: usize) -> usize {
    span(x0, x).min(n)
}

/// Longest part of a span the angle and line masks step through from one start value
/// (longer spans are split): keeps their per-pixel distance steps in range (`2^24 · 2^24 =
/// 2^48`).
const MAX_CHUNK: usize = 1 << 24;

/// Most parts of [`MAX_CHUNK`] values evaluated (2^32 values: every pixel an `i32` coordinate
/// can address; values beyond are not pixels and are left unchanged).
const MAX_PARTS: i64 = 1 << 8;

impl Mask<'_> {
    /// Multiplies the coverage of the span starting at `(x0, y)` in place.
    ///
    /// ```
    /// use twine_core::{Point, Rect};
    /// use twine_render::{Mask, MaskResult};
    ///
    /// let m = Mask::Radius { area: Rect::from_xywh(0, 0, 10, 10), radius: 0, outer: false };
    /// let mut cov = [255u8; 4];
    /// assert_eq!(m.apply(5, 8, &mut cov), MaskResult::Changed);
    /// assert_eq!(cov, [255, 255, 0, 0]);
    /// assert_eq!(m.apply(20, 0, &mut cov), MaskResult::Transparent);
    /// ```
    pub fn apply(&self, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
        let n = cov.len();
        if n == 0 {
            return MaskResult::FullCover;
        }
        // The span's end; past `i32::MAX` it saturates (only the first `i32::MAX − x0` values
        // can be pixels).
        let x1 = x0.saturating_add(i32::try_from(n).unwrap_or(i32::MAX));
        match *self {
            Mask::Radius { area, radius, outer } => apply_radius(area, radius, outer, y, x0, x1, cov),
            Mask::Angle { center, start, end } => apply_angle(center, start, end, y, x0, cov),
            Mask::Line { p1, p2, side } => apply_line(p1, p2, side, y, x0, cov),
            Mask::Fade {
                area,
                y_top,
                y_bottom,
                opa_top,
                opa_bottom,
            } => {
                if y < area.y0 || y >= area.y1 {
                    return MaskResult::FullCover;
                }
                let opa = if y <= y_top {
                    opa_top.raw()
                } else if y >= y_bottom {
                    opa_bottom.raw()
                } else {
                    // SAFE-ARITH: here `y_top < y < y_bottom` (the branches above): the
                    // differences of `i32`s fit in `i64`, `t ∈ 0..256` and the opacity
                    // difference is within `±255`. The division's fallback (`y_bottom > y_top`)
                    // is never taken.
                    let t = i64::from(y)
                        .wrapping_sub(i64::from(y_top))
                        .wrapping_mul(256)
                        .checked_div(i64::from(y_bottom).wrapping_sub(i64::from(y_top)))
                        .unwrap_or(0);
                    let (top, bottom) = (i64::from(opa_top.raw()), i64::from(opa_bottom.raw()));
                    let v = top.wrapping_add(bottom.wrapping_sub(top).wrapping_mul(t) / 256);
                    v.clamp(0, 255) as u8
                };
                let a = rel(area.x0, x0, n);
                let b = rel(area.x1, x0, n);
                if a >= b || opa == 255 {
                    return MaskResult::FullCover;
                }
                if opa == 0 && a == 0 && b == n {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                // SAFE-INDEX: `a < b ≤ n`; the fallback is never taken.
                if let Some(part) = cov.get_mut(a..b) {
                    for c in part {
                        mul(c, opa);
                    }
                }
                MaskResult::Changed
            }
            Mask::Map { area, alpha } => {
                if y < area.y0 || y >= area.y1 {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                let a = rel(area.x0, x0, n);
                let b = rel(area.x1, x0, n);
                if a >= b {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                // SAFE-INDEX: `a < b ≤ n`; the fallbacks are never taken.
                let Some((head, rest)) = cov.split_at_mut_checked(a) else {
                    return MaskResult::Changed;
                };
                head.fill(0);
                // SAFE-ARITH: `a < b`.
                let Some((mid, tail)) = rest.split_at_mut_checked(b.wrapping_sub(a)) else {
                    return MaskResult::Changed;
                };
                tail.fill(0);
                // The alpha row of `y`, from the first covered pixel (`max(x0, area.x0)`);
                // pixels past the end of `alpha` are transparent.
                let start = span(area.y0, y)
                    .saturating_mul(span(area.x0, area.x1))
                    .saturating_add(span(area.x0, x0));
                let src = alpha.get(start..).unwrap_or_default();
                let k = mid.len().min(src.len());
                let (with_alpha, past_end) = mid.split_at_mut(k);
                for (c, &m) in with_alpha.iter_mut().zip(src) {
                    mul(c, m);
                }
                past_end.fill(0);
                MaskResult::Changed
            }
        }
    }
}

fn apply_radius(
    area: Rect,
    radius: i32,
    outer: bool,
    y: i32,
    x0: i32,
    x1: i32,
    cov: &mut [u8],
) -> MaskResult {
    let r = eff_radius(area, radius);
    let q = if r == 0 {
        Quarter::Square
    } else {
        Quarter::Computed { r }
    };
    let rs = RowSpan::new(area, &q, y);
    let n = cov.len();
    let idx = |x: i32| rel(x, x0, n);
    // Multiplies the pixels `[a, b)` (screen x, clipped to the span) by the row's coverage at
    // x (`invert`: by its complement).
    let edge = |cov: &mut [u8], a: i32, b: i32, invert: bool| {
        let (a, b) = (a.max(x0), b.min(x1));
        // SAFE-INDEX: `x0 ≤ a` and `b ≤ x1 ≤ x0 + n`; the fallback is never taken.
        if let Some(part) = cov.get_mut(idx(a)..idx(b).max(idx(a))) {
            for (c, x) in part.iter_mut().zip(a..b) {
                let v = rs.cov(x);
                mul(c, if invert { inv8(v) } else { v });
            }
        }
    };
    #[allow(clippy::if_not_else)] // the inner (common) case first
    if !outer {
        if rs.is_empty() || rs.aa_r1 <= x0 || rs.aa_l0 >= x1 {
            cov.fill(0);
            return MaskResult::Transparent;
        }
        if rs.full_l <= x0 && rs.full_r >= x1 {
            return MaskResult::FullCover;
        }
        // SAFE-INDEX: `idx` is at most `n`; the fallbacks are never taken.
        if let Some(c) = cov.get_mut(..idx(rs.aa_l0)) {
            c.fill(0);
        }
        if let Some(c) = cov.get_mut(idx(rs.aa_r1)..) {
            c.fill(0);
        }
        edge(cov, rs.aa_l0, rs.full_l, false);
        edge(cov, rs.full_r, rs.aa_r1, false);
        MaskResult::Changed
    } else {
        if rs.is_empty() || rs.aa_r1 <= x0 || rs.aa_l0 >= x1 {
            return MaskResult::FullCover;
        }
        if rs.full_l <= x0 && rs.full_r >= x1 {
            cov.fill(0);
            return MaskResult::Transparent;
        }
        let (fa, fb) = (idx(rs.full_l), idx(rs.full_r));
        if fa < fb {
            // SAFE-INDEX: `fa < fb ≤ n`; the fallback is never taken.
            if let Some(c) = cov.get_mut(fa..fb) {
                c.fill(0);
            }
        }
        edge(cov, rs.aa_l0, rs.full_l, true);
        edge(cov, rs.full_r, rs.aa_r1, true);
        MaskResult::Changed
    }
}

fn apply_angle(center: Point, start: Angle, end: Angle, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
    // SAFE-ARITH: the difference of two `i32`s fits in `i64`.
    let span = i64::from(end.as_deci_deg()).wrapping_sub(i64::from(start.as_deci_deg()));
    if span >= 3600 {
        return MaskResult::FullCover;
    }
    let s = start.normalized();
    let mut e = end.normalized();
    if span <= 0 && e.as_deci_deg() == s.as_deci_deg() {
        cov.fill(0);
        return MaskResult::Transparent;
    }
    // SAFE-ARITH: normalized angles are in `0..3600`, so `e < 7200` and `e − s ∈ 1..7200`.
    if e.as_deci_deg() <= s.as_deci_deg() {
        e = Angle::deci_deg(e.as_deci_deg().wrapping_add(3600));
    }
    let wide = e.as_deci_deg().wrapping_sub(s.as_deci_deg()) >= 1800;
    let (cs, ss) = (i64::from(cos(s)), i64::from(sin(s)));
    let (ce, se) = (i64::from(cos(e)), i64::from(sin(e)));
    // Pixel center relative to the vertex, in half pixels.
    // SAFE-ARITH: differences of `i32`s are below 2^32 in magnitude, so `|ry|, |rx0| ≤ 2^33`
    // (and a part's `|rx| < 2^34`); `|cos|, |sin| ≤ 32_767 < 2^15`, so `|ds|, |de| < 2^50` and
    // each step is at most 2^16: after at most `MAX_CHUNK = 2^24` pixels they stay below
    // 2^51 (`half_plane`'s range).
    let ry = i64::from(y)
        .wrapping_sub(i64::from(center.y))
        .wrapping_mul(2)
        .wrapping_add(1);
    let rx0 = i64::from(x0)
        .wrapping_sub(i64::from(center.x))
        .wrapping_mul(2)
        .wrapping_add(1);
    // cross(dir_s, rel) = cs·ry − ss·rx  (> 0: clockwise of the start ray)
    // cross(rel, dir_e) = rx·se − ry·ce  (> 0: counter-clockwise of the end ray)
    // Distance in 1/256 px = cross · 256 / (2 · 32767) ≈ cross >> 8.
    let (step_s, step_e) = (ss.wrapping_mul(-2), se.wrapping_mul(2));
    let mut all_zero = true;
    let mut all_full = true;
    // One part for every span the renderer produces (`max_span` is a `u16`); the distances
    // restart from the exact value at each part, so their steps stay in range. At most
    // `MAX_PARTS` parts (2^32 values: every pixel an `i32` coordinate can address).
    for (k, part) in (0..MAX_PARTS).zip(cov.chunks_mut(MAX_CHUNK)) {
        // SAFE-ARITH: `k < 2^8`, so the part's offset is below 2^33 half pixels and `|rx| <
        // 2^34`.
        let rx = rx0.wrapping_add(k.wrapping_mul(2 << 24));
        let mut ds = cs.wrapping_mul(ry).wrapping_sub(ss.wrapping_mul(rx));
        let mut de = rx.wrapping_mul(se).wrapping_sub(ry.wrapping_mul(ce));
        for c in part.iter_mut() {
            let a = half_plane(ds >> 8);
            let b = half_plane(de >> 8);
            let m = if wide { a.max(b) } else { a.min(b) };
            if m != 255 {
                all_full = false;
                mul(c, m);
            }
            if *c != 0 {
                all_zero = false;
            }
            // SAFE-ARITH: see above.
            ds = ds.wrapping_add(step_s);
            de = de.wrapping_add(step_e);
        }
    }
    if all_full {
        MaskResult::FullCover
    } else if all_zero {
        MaskResult::Transparent
    } else {
        MaskResult::Changed
    }
}

/// Largest direction component of a line mask: longer directions are scaled down (the
/// distance to the line depends only on the direction, so the result is the same up to
/// rounding); keeps `|n|² · 2^16` inside `u64` (`(2 · 2^46) · 2^16 = 2^63`).
const LINE_DIR_MAX: i64 = 1 << 23;

fn apply_line(p1: Point, p2: Point, side: LineSide, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
    // SAFE-ARITH: differences of two `i32`s fit in `i64`.
    let (mut dx, mut dy) = (
        i64::from(p2.x).wrapping_sub(i64::from(p1.x)),
        i64::from(p2.y).wrapping_sub(i64::from(p1.y)),
    );
    if dx == 0 && dy == 0 {
        return MaskResult::FullCover;
    }
    // Directions longer than `LINE_DIR_MAX` (2^23 px) are halved until they fit.
    while dx.abs() > LINE_DIR_MAX || dy.abs() > LINE_DIR_MAX {
        dx /= 2;
        dy /= 2;
    }
    // Normal (dy, −dx), oriented towards the kept side.
    // SAFE-ARITH: `|dx|, |dy| ≤ 2^23` from here on.
    let (mut nx, mut ny) = (dy, dx.wrapping_neg());
    let flip = match side {
        LineSide::Left => nx > 0 || (nx == 0 && ny > 0),
        LineSide::Right => nx < 0 || (nx == 0 && ny < 0),
        LineSide::Top => ny > 0 || (ny == 0 && nx > 0),
        LineSide::Bottom => ny < 0 || (ny == 0 && nx < 0),
    };
    if flip {
        // SAFE-ARITH: `|nx|, |ny| ≤ 2^23`.
        nx = nx.wrapping_neg();
        ny = ny.wrapping_neg();
    }
    // |n| in 1/256 px.
    // SAFE-ARITH: `|dx|, |dy| ≤ 2^23`, so `dx² + dy² ≤ 2^47` and `· 2^16 ≤ 2^63`.
    let n2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    let len256 = i64::from(isqrt64((n2 as u64) << 16)).max(1);
    // Signed distance of the pixel center in 1/256 px, as 16.16 fixed point:
    // dist = dot(n, rel2) / 2 / |n|, rel2 in half pixels.
    // SAFE-ARITH: `|rx|, |ry| ≤ 2^33` (differences of `i32`s, doubled; a part's `|rx| <
    // 2^35`), so `|nx·rx + ny·ry| < 2^59` and `· 2^31 < 2^90` in `i128`. The distance itself is at most `|rel| · 2^24 <
    // 2^57` (saturated for safety), and the step `2·|nx| / |n| · 2^23 ≤ 2^24`: after at most
    // `MAX_CHUNK = 2^24` pixels `|d| < 2^58`, so `d >> 16 < 2^42` (`half_plane`'s range).
    let rx = i64::from(x0)
        .wrapping_sub(i64::from(p1.x))
        .wrapping_mul(2)
        .wrapping_add(1);
    let ry = i64::from(y)
        .wrapping_sub(i64::from(p1.y))
        .wrapping_mul(2)
        .wrapping_add(1);
    let scale = |v: i64| -> i64 {
        let q = i128::from(v)
            .wrapping_mul(128 << 16)
            .wrapping_mul(256)
            .checked_div(i128::from(len256))
            .unwrap_or(0);
        q.clamp(-(1 << 57), 1 << 57) as i64
    };
    let step = scale(nx.wrapping_mul(2));
    let mut all_zero = true;
    let mut all_full = true;
    // Parts as in `apply_angle`: the distance restarts from the exact value at each part.
    for (k, part) in (0..MAX_PARTS).zip(cov.chunks_mut(MAX_CHUNK)) {
        // SAFE-ARITH: `k < 2^8`: the part's offset is below 2^33 half pixels, `|rx| < 2^35`.
        let rx = rx.wrapping_add(k.wrapping_mul(2 << 24));
        let mut d = scale(nx.wrapping_mul(rx).wrapping_add(ny.wrapping_mul(ry)));
        for c in part.iter_mut() {
            let m = half_plane(d >> 16);
            if m != 255 {
                all_full = false;
                mul(c, m);
            }
            if *c != 0 {
                all_zero = false;
            }
            // SAFE-ARITH: see above.
            d = d.wrapping_add(step);
        }
    }
    if all_full {
        MaskResult::FullCover
    } else if all_zero {
        MaskResult::Transparent
    } else {
        MaskResult::Changed
    }
}

/// Identifies a pushed mask for [`Painter::pop_mask`](crate::Painter::pop_mask).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MaskId(pub(crate) u8);

impl MaskId {
    /// The id returned when the stack is full; popping it does nothing.
    pub const INVALID: MaskId = MaskId(u8::MAX);
}

/// Maximum number of simultaneously pushed masks.
pub const MAX_MASKS: usize = 16;

/// A fixed-capacity stack of masks (no allocation).
#[derive(Clone, Copy, Debug)]
pub struct MaskStack<'a> {
    masks: [Option<Mask<'a>>; MAX_MASKS],
    count: u8,
}

impl Default for MaskStack<'_> {
    fn default() -> Self {
        Self {
            masks: [None; MAX_MASKS],
            count: 0,
        }
    }
}

impl<'a> MaskStack<'a> {
    /// Whether no mask is pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Number of pushed masks.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.count)
    }

    /// Pushes `m`; `None` when the stack is full.
    pub fn push(&mut self, m: Mask<'a>) -> Option<MaskId> {
        let (i, slot) = self.masks.iter_mut().enumerate().find(|(_, s)| s.is_none())?;
        *slot = Some(m);
        // SAFE-ARITH: a free slot exists, so `count < MAX_MASKS = 16`.
        self.count = self.count.wrapping_add(1);
        // `i < MAX_MASKS = 16`: the cast is exact.
        Some(MaskId(i as u8))
    }

    /// Removes the mask `id` (unknown ids are ignored). Returns whether a mask was removed.
    pub fn pop(&mut self, id: MaskId) -> bool {
        match self.masks.get_mut(usize::from(id.0)) {
            Some(slot @ Some(_)) => {
                *slot = None;
                // SAFE-ARITH: the slot was occupied, so `count ≥ 1`.
                self.count = self.count.wrapping_sub(1);
                true
            }
            _ => false,
        }
    }

    /// Applies every mask to the span; stops early when it becomes transparent. Returns
    /// [`MaskResult::FullCover`] if no mask changed anything.
    pub fn apply(&self, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
        let mut res = MaskResult::FullCover;
        if self.count == 0 {
            return res;
        }
        for m in self.masks.iter().flatten() {
            match m.apply(y, x0, cov) {
                MaskResult::Transparent => return MaskResult::Transparent,
                MaskResult::Changed => res = MaskResult::Changed,
                MaskResult::FullCover => {}
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_returns_transparent_outside() {
        let area = Rect::from_xywh(10, 10, 20, 20);
        let mut cov = [255u8; 8];
        let m = Mask::Radius {
            area,
            radius: 5,
            outer: false,
        };
        assert_eq!(m.apply(5, 12, &mut cov), MaskResult::Transparent);
        assert_eq!(cov, [0; 8]);
        let mut cov = [255u8; 8];
        assert_eq!(m.apply(20, 0, &mut cov), MaskResult::Transparent);
        let mut cov = [255u8; 8];
        let m = Mask::Map {
            area,
            alpha: &[255; 400],
        };
        assert_eq!(m.apply(40, 12, &mut cov), MaskResult::Transparent);
        let mut cov = [255u8; 8];
        let m = Mask::Angle {
            center: Point::new(0, 0),
            start: Angle::deg(0),
            end: Angle::deg(90),
        };
        // Row above the vertex: outside the sector 0°..90° (which points down-right).
        assert_eq!(m.apply(-10, 5, &mut cov), MaskResult::Transparent);
    }

    #[test]
    fn apply_full_cover_inside() {
        let area = Rect::from_xywh(10, 10, 20, 20);
        let mut cov = [200u8; 6];
        let m = Mask::Radius {
            area,
            radius: 5,
            outer: false,
        };
        assert_eq!(m.apply(20, 12, &mut cov), MaskResult::FullCover);
        assert_eq!(cov, [200; 6]);
        let m = Mask::Angle {
            center: Point::new(0, 0),
            start: Angle::deg(-30),
            end: Angle::deg(400),
        };
        assert_eq!(m.apply(3, -3, &mut cov), MaskResult::FullCover);
        let m = Mask::Fade {
            area,
            y_top: 12,
            y_bottom: 20,
            opa_top: Opa::COVER,
            opa_bottom: Opa::TRANSP,
        };
        assert_eq!(m.apply(5, 12, &mut cov), MaskResult::FullCover);
        assert_eq!(m.apply(11, 12, &mut cov), MaskResult::FullCover);
        let mut cov = [255u8; 40];
        assert_eq!(m.apply(25, 0, &mut cov), MaskResult::Changed);
        assert_eq!(&cov[..10], &[255; 10]);
        assert_eq!(&cov[10..30], &[0; 20]);
    }

    #[test]
    fn angle_quadrant_signs() {
        // Sector 0°..90° around (0,0): the bottom-right quadrant (y down).
        let m = Mask::Angle {
            center: Point::new(0, 0),
            start: Angle::deg(0),
            end: Angle::deg(90),
        };
        let mut row = [255u8; 20];
        m.apply(5, -10, &mut row);
        assert!(row[..9].iter().all(|&v| v == 0), "{row:?}");
        assert!(row[11..].iter().all(|&v| v == 255), "{row:?}");
    }

    #[test]
    fn line_mask_sides() {
        let m = Mask::Line {
            p1: Point::new(0, 0),
            p2: Point::new(0, 10),
            side: LineSide::Left,
        };
        let mut row = [255u8; 4];
        m.apply(3, -2, &mut row);
        assert_eq!(row, [255, 255, 0, 0]);
        let m = Mask::Line {
            p1: Point::new(0, 5),
            p2: Point::new(10, 5),
            side: LineSide::Bottom,
        };
        let mut row = [255u8; 4];
        assert_eq!(m.apply(4, 0, &mut row), MaskResult::Transparent);
        let mut row = [255u8; 4];
        assert_eq!(m.apply(5, 0, &mut row), MaskResult::FullCover);
    }

    #[test]
    fn stack_push_pop() {
        let mut s = MaskStack::default();
        let area = Rect::from_xywh(0, 0, 4, 4);
        let ids: alloc::vec::Vec<_> = (0..MAX_MASKS)
            .map(|_| {
                s.push(Mask::Radius {
                    area,
                    radius: 0,
                    outer: false,
                })
                .unwrap()
            })
            .collect();
        assert!(
            s.push(Mask::Radius {
                area,
                radius: 0,
                outer: false
            })
            .is_none()
        );
        assert_eq!(s.len(), MAX_MASKS);
        assert!(s.pop(ids[3]));
        assert!(!s.pop(ids[3]));
        assert!(!s.pop(MaskId::INVALID));
        assert_eq!(s.len(), MAX_MASKS - 1);
    }
}
