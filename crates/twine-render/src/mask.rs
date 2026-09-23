//! Coverage masks ([`Mask`]) and the fixed-capacity [`MaskStack`].
//!
//! A mask multiplies a span's coverage in place ([`Mask::apply`]). The painter applies every
//! pushed mask to every span it blends, so masks affect all primitives: rounded clipping of
//! children, arcs, fades and bitmap masks.
//!
//! Geometry uses continuous coordinates: pixel `(x, y)` covers `[x, x+1) × [y, y+1)` and its
//! center is `(x + ½, y + ½)`. Anti-aliasing is 1 px wide.

use twine_core::math::{cos, isqrt64, sin, udiv255};
use twine_core::{Angle, Opa, Point, Rect};

use crate::circle::Quarter;
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
    /// clamped to half the shorter side.
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
    /// Keeps one side of the infinite line through `p1` and `p2`.
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
    *c = udiv255(u32::from(*c) * u32::from(m)) as u8;
}

/// Coverage of a half plane from a signed distance in 1/256 px (0.5 px at the boundary).
#[inline(always)]
fn half_plane(d256: i64) -> u8 {
    (d256 + 128).clamp(0, 255) as u8
}

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
        let n = cov.len() as i32;
        if n == 0 {
            return MaskResult::FullCover;
        }
        let x1 = x0 + n;
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
                    opa_top.0
                } else if y >= y_bottom {
                    opa_bottom.0
                } else {
                    let t = i64::from(y - y_top) * 256 / i64::from(y_bottom - y_top);
                    let v = i64::from(opa_top.0) + (i64::from(opa_bottom.0) - i64::from(opa_top.0)) * t / 256;
                    v.clamp(0, 255) as u8
                };
                let a = (area.x0.max(x0) - x0).clamp(0, n) as usize;
                let b = (area.x1.min(x1) - x0).clamp(0, n) as usize;
                if a >= b || opa == 255 {
                    return MaskResult::FullCover;
                }
                if opa == 0 && a == 0 && b == cov.len() {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                cov[a..b].iter_mut().for_each(|c| mul(c, opa));
                MaskResult::Changed
            }
            Mask::Map { area, alpha } => {
                if y < area.y0 || y >= area.y1 {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                let a = (area.x0.max(x0) - x0).clamp(0, n) as usize;
                let b = (area.x1.min(x1) - x0).clamp(0, n) as usize;
                if a >= b {
                    cov.fill(0);
                    return MaskResult::Transparent;
                }
                cov[..a].fill(0);
                cov[b..].fill(0);
                let w = area.width() as usize;
                let row_start = (y - area.y0) as usize * w;
                let ax = (x0 + a as i32 - area.x0) as usize;
                for (i, c) in cov[a..b].iter_mut().enumerate() {
                    mul(c, alpha.get(row_start + ax + i).copied().unwrap_or(0));
                }
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
    let idx = |x: i32| (x.clamp(x0, x1) - x0) as usize;
    #[allow(clippy::if_not_else)] // the inner (common) case first
    if !outer {
        if rs.is_empty() || rs.aa_r1 <= x0 || rs.aa_l0 >= x1 {
            cov.fill(0);
            return MaskResult::Transparent;
        }
        if rs.full_l <= x0 && rs.full_r >= x1 {
            return MaskResult::FullCover;
        }
        cov[..idx(rs.aa_l0)].fill(0);
        cov[idx(rs.aa_r1)..].fill(0);
        for x in rs.aa_l0.max(x0)..rs.full_l.min(x1) {
            mul(&mut cov[(x - x0) as usize], rs.cov(x));
        }
        for x in rs.full_r.max(x0)..rs.aa_r1.min(x1) {
            mul(&mut cov[(x - x0) as usize], rs.cov(x));
        }
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
            cov[fa..fb].fill(0);
        }
        for x in rs.aa_l0.max(x0)..rs.full_l.min(x1) {
            mul(&mut cov[(x - x0) as usize], 255 - rs.cov(x));
        }
        for x in rs.full_r.max(x0)..rs.aa_r1.min(x1) {
            mul(&mut cov[(x - x0) as usize], 255 - rs.cov(x));
        }
        MaskResult::Changed
    }
}

fn apply_angle(center: Point, start: Angle, end: Angle, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
    let span = i64::from(end.0) - i64::from(start.0);
    if span >= 3600 {
        return MaskResult::FullCover;
    }
    let s = start.normalized();
    let mut e = end.normalized();
    if span <= 0 && e.0 == s.0 {
        cov.fill(0);
        return MaskResult::Transparent;
    }
    if e.0 <= s.0 {
        e = Angle(e.0 + 3600);
    }
    let wide = e.0 - s.0 >= 1800;
    let (cs, ss) = (i64::from(cos(s)), i64::from(sin(s)));
    let (ce, se) = (i64::from(cos(e)), i64::from(sin(e)));
    // Pixel center relative to the vertex, in half pixels.
    let ry = i64::from(2 * (y - center.y) + 1);
    let rx0 = i64::from(2 * (x0 - center.x) + 1);
    // cross(dir_s, rel) = cs·ry − ss·rx  (> 0: clockwise of the start ray)
    // cross(rel, dir_e) = rx·se − ry·ce  (> 0: counter-clockwise of the end ray)
    // Distance in 1/256 px = cross · 256 / (2 · 32767) ≈ cross >> 8.
    let mut ds = cs * ry - ss * rx0;
    let mut de = rx0 * se - ry * ce;
    let (step_s, step_e) = (-2 * ss, 2 * se);
    let mut all_zero = true;
    let mut all_full = true;
    for c in cov.iter_mut() {
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
        ds += step_s;
        de += step_e;
    }
    if all_full {
        MaskResult::FullCover
    } else if all_zero {
        MaskResult::Transparent
    } else {
        MaskResult::Changed
    }
}

fn apply_line(p1: Point, p2: Point, side: LineSide, y: i32, x0: i32, cov: &mut [u8]) -> MaskResult {
    let (dx, dy) = (
        i64::from(p2.x) - i64::from(p1.x),
        i64::from(p2.y) - i64::from(p1.y),
    );
    if dx == 0 && dy == 0 {
        return MaskResult::FullCover;
    }
    // Normal (dy, −dx), oriented towards the kept side.
    let (mut nx, mut ny) = (dy, -dx);
    let flip = match side {
        LineSide::Left => nx > 0 || (nx == 0 && ny > 0),
        LineSide::Right => nx < 0 || (nx == 0 && ny < 0),
        LineSide::Top => ny > 0 || (ny == 0 && nx > 0),
        LineSide::Bottom => ny < 0 || (ny == 0 && nx < 0),
    };
    if flip {
        nx = -nx;
        ny = -ny;
    }
    // |n| in 1/256 px.
    let len256 = i64::from(isqrt64(((dx * dx + dy * dy) as u64) << 16)).max(1);
    // Signed distance of the pixel center in 1/256 px, as 16.16 fixed point:
    // dist = dot(n, rel2) / 2 / |n|, rel2 in half pixels.
    let rx = i64::from(2 * (x0 - p1.x) + 1);
    let ry = i64::from(2 * (y - p1.y) + 1);
    let scale = |v: i64| -> i64 { (i128::from(v) * (128 << 16) * 256 / i128::from(len256)) as i64 };
    let mut d = scale(nx * rx + ny * ry);
    let step = scale(2 * nx);
    let mut all_zero = true;
    let mut all_full = true;
    for c in cov.iter_mut() {
        let m = half_plane(d >> 16);
        if m != 255 {
            all_full = false;
            mul(c, m);
        }
        if *c != 0 {
            all_zero = false;
        }
        d += step;
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
        let i = self.masks.iter().position(Option::is_none)?;
        self.masks[i] = Some(m);
        self.count += 1;
        Some(MaskId(i as u8))
    }

    /// Removes the mask `id` (unknown ids are ignored). Returns whether a mask was removed.
    pub fn pop(&mut self, id: MaskId) -> bool {
        match self.masks.get_mut(usize::from(id.0)) {
            Some(slot @ Some(_)) => {
                *slot = None;
                self.count -= 1;
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
