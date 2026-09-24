//! Shape modifiers that rewrite the contours collected so far in a group: trim paths
//! ([`trim`]) and the repeater ([`repeat`]). Both write into a scratch [`Geometry`] and splice
//! the result back, so nothing is allocated once the buffers are warm.

use crate::eval::mix;
use crate::fmath::{clamp, floor, powf};
use crate::geom::{
    Contour, Geometry, LEN_SAMPLES, Mat, TransformValues, cubic_lengths, sub_cubic, t_at_length,
};
use crate::model::{Composite, TrimMode, Vec2};

/// Most repeater copies drawn (more are clamped).
pub(crate) const MAX_COPIES: usize = 256;

/// Most contour points per shape layer: repeaters (and trims) that would exceed it are
/// reduced or skipped, which bounds memory for hostile files (nested repeaters multiply).
pub(crate) const MAX_POINTS: usize = 1 << 18;

/// Replaces the contours `geo.contours[from..]` by `tmp`'s contours.
fn splice(geo: &mut Geometry, from: usize, tmp: &Geometry) {
    let Some(first) = geo.contours.get(from) else {
        return;
    };
    let base = first.start as usize;
    geo.contours.truncate(from);
    geo.pts.truncate(base);
    for c in &tmp.contours {
        geo.contours.push(Contour {
            start: c.start + base as u32,
            ..*c
        });
    }
    geo.pts.extend_from_slice(&tmp.pts);
}

fn seg(pts: &[Vec2], k: usize) -> [Vec2; 4] {
    let i = 3 * k;
    [pts[i], pts[i + 1], pts[i + 2], pts[i + 3]]
}

/// Length of a contour.
fn contour_length(c: &Contour, pts: &[Vec2]) -> f32 {
    let p = c.pts(pts);
    let mut tab = [0.0; LEN_SAMPLES + 1];
    (0..c.segs as usize)
        .map(|k| cubic_lengths(seg(p, k), &mut tab))
        .sum()
}

/// Appends the part `[a, b]` (length units) of contour `c` to `out`. With `cont`, the part
/// continues the last contour of `out` instead of starting a new one. Returns whether anything
/// was appended.
fn emit(out: &mut Geometry, c: &Contour, pts: &[Vec2], a: f32, b: f32, cont: bool) -> bool {
    let p = c.pts(pts);
    let mut tab = [0.0; LEN_SAMPLES + 1];
    let mut acc = 0.0;
    let mut started = cont;
    let start_idx = out.pts.len();
    let mut segs = 0u32;
    for k in 0..c.segs as usize {
        let s = seg(p, k);
        let len = cubic_lengths(s, &mut tab);
        let (s0, s1) = (acc, acc + len);
        acc = s1;
        if len <= 0.0 || s1 <= a || s0 >= b {
            continue;
        }
        let t0 = t_at_length(&tab, a - s0);
        let t1 = t_at_length(&tab, b - s0);
        if t1 <= t0 {
            continue;
        }
        let sub = sub_cubic(s, t0, t1);
        if !started {
            out.begin(sub[0]);
            started = true;
        }
        out.cubic(sub[1], sub[2], sub[3]);
        segs += 1;
    }
    if segs == 0 {
        if !cont {
            out.pts.truncate(start_idx);
        }
        return false;
    }
    if cont {
        if let Some(last) = out.contours.last_mut() {
            last.segs += segs;
        }
    } else {
        out.contours.push(Contour {
            start: start_idx as u32,
            segs,
            closed: false,
            owner: c.owner,
            alpha: c.alpha,
        });
    }
    true
}

/// Trim paths: keeps the part `[s, e]` (percent) shifted by `o` degrees of the contours
/// `geo.contours[from..]`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn trim(
    geo: &mut Geometry,
    from: usize,
    s: f32,
    e: f32,
    o: f32,
    mode: TrimMode,
    tmp: &mut Geometry,
) {
    if from >= geo.contours.len() || geo.pts.len() > MAX_POINTS / 2 {
        return;
    }
    let (mut s, mut e) = (clamp(s / 100.0, 0.0, 1.0), clamp(e / 100.0, 0.0, 1.0));
    if s > e {
        core::mem::swap(&mut s, &mut e);
    }
    let span = e - s;
    if span >= 1.0 - 1e-6 {
        return;
    }
    tmp.clear();
    if span <= 1e-6 {
        splice(geo, from, tmp);
        return;
    }
    let off = o / 360.0;
    let off = if off.is_finite() { off } else { 0.0 };
    let mut start = s + off;
    start -= floor(start);
    let end = start + span;
    // Up to two intervals in [0, 1] (the second when the range wraps around the end).
    let first = (start, end.min(1.0));
    let second = (end > 1.0).then_some((0.0, end - 1.0));
    match mode {
        TrimMode::Simultaneously => {
            for c in &geo.contours[from..] {
                let len = contour_length(c, &geo.pts);
                if !(len > 0.0) {
                    continue;
                }
                let got = emit(tmp, c, &geo.pts, first.0 * len, first.1 * len, false);
                if let Some((a, b)) = second {
                    // On a closed contour the wrapped part continues the first piece.
                    let cont = got && c.closed;
                    emit(tmp, c, &geo.pts, a * len, b * len, cont);
                }
            }
        }
        TrimMode::Individually => {
            let total: f32 = geo.contours[from..]
                .iter()
                .map(|c| contour_length(c, &geo.pts))
                .sum();
            if !(total > 0.0) {
                splice(geo, from, tmp);
                return;
            }
            let mut acc = 0.0;
            for c in &geo.contours[from..] {
                let len = contour_length(c, &geo.pts);
                for (a, b) in core::iter::once(first).chain(second) {
                    let (a, b) = (a * total - acc, b * total - acc);
                    if b > 0.0 && a < len {
                        emit(tmp, c, &geo.pts, a.max(0.0), b.min(len), false);
                    }
                }
                acc += len;
            }
        }
    }
    splice(geo, from, tmp);
}

/// Evaluated repeater parameters.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RepeatParams {
    pub copies: f32,
    pub offset: f32,
    pub tr: TransformValues,
    pub start_opacity: f32,
    pub end_opacity: f32,
    pub composite: Composite,
}

/// Repeater: replaces the contours `geo.contours[from..]` by `copies` transformed copies.
/// `group` maps the group's space (where the repeater transform applies) to layer space.
pub(crate) fn repeat(geo: &mut Geometry, from: usize, rp: &RepeatParams, group: &Mat, tmp: &mut Geometry) {
    if from >= geo.contours.len() {
        return;
    }
    tmp.clear();
    let c = clamp(rp.copies, 0.0, MAX_COPIES as f32);
    let n = -floor(-c) as usize; // ceil
    let Some(first) = geo.contours.get(from) else {
        return;
    };
    let range_pts = (geo.pts.len() - first.start as usize).max(1);
    let n = n.min((MAX_POINTS.saturating_sub(first.start as usize)) / range_pts);
    let Some(inv) = group.invert() else {
        return;
    };
    for step in 0..n {
        let i = match rp.composite {
            Composite::Above => step,
            Composite::Below => n - 1 - step,
        };
        let k = rp.offset + i as f32;
        let t = &rp.tr;
        let copy = TransformValues {
            anchor: t.anchor,
            position: [t.anchor[0] + t.position[0] * k, t.anchor[1] + t.position[1] * k],
            scale: [
                powf(t.scale[0] / 100.0, k) * 100.0,
                powf(t.scale[1] / 100.0, k) * 100.0,
            ],
            rotation: t.rotation * k,
            skew: 0.0,
            skew_axis: 0.0,
        };
        let m = group.mul(&copy.matrix()).mul(&inv);
        let f = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
        let alpha = clamp(mix(rp.start_opacity, rp.end_opacity, f) / 100.0, 0.0, 1.0);
        for src in &geo.contours[from..] {
            let start = tmp.pts.len() as u32;
            tmp.pts.extend(src.pts(&geo.pts).iter().map(|p| m.apply(*p)));
            tmp.contours.push(Contour {
                start,
                alpha: src.alpha * alpha,
                ..*src
            });
        }
    }
    splice(geo, from, tmp);
}

/// Total length of all contours of `geo` (tests).
#[cfg(test)]
pub(crate) fn total_length(geo: &Geometry) -> f32 {
    geo.contours.iter().map(|c| contour_length(c, &geo.pts)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmath;

    fn square() -> Geometry {
        // 100 × 100 square, perimeter 400.
        let mut g = Geometry::default();
        g.rect([50.0, 50.0], [100.0, 100.0], 0.0, false, 0, &Mat::IDENTITY);
        g
    }

    fn close(a: f32, b: f32) -> bool {
        fmath::abs(a - b) < 0.05
    }

    #[test]
    fn trim_half_and_wrap() {
        let mut tmp = Geometry::default();
        let mut g = square();
        trim(&mut g, 0, 0.0, 50.0, 0.0, TrimMode::Simultaneously, &mut tmp);
        assert!(close(total_length(&g), 200.0));
        assert_eq!(g.contours.len(), 1);
        // Offset 270° shifts [0, 50 %] to [75 %, 125 %]: one joined piece on a closed contour.
        let mut g = square();
        trim(&mut g, 0, 0.0, 50.0, 270.0, TrimMode::Simultaneously, &mut tmp);
        assert_eq!(g.contours.len(), 1);
        assert!(close(total_length(&g), 200.0));
        // Rectangles start at the top-right corner and run clockwise: 75 % is the top-left
        // corner, the piece ends half-way down the right edge (25 % after the wrap).
        let p = g.contours[0].pts(&g.pts);
        assert!(
            fmath::abs(p[0][0]) < 0.01 && fmath::abs(p[0][1]) < 0.01,
            "{:?}",
            p[0]
        );
        let end = p[p.len() - 1];
        assert!(
            fmath::abs(end[0] - 100.0) < 0.01 && fmath::abs(end[1] - 100.0) < 0.01,
            "{end:?}"
        );
    }

    #[test]
    fn trim_empty_and_full() {
        let mut tmp = Geometry::default();
        let mut g = square();
        trim(&mut g, 0, 30.0, 30.0, 0.0, TrimMode::Simultaneously, &mut tmp);
        assert!(g.contours.is_empty() && g.pts.is_empty());
        let mut g = square();
        trim(&mut g, 0, 0.0, 100.0, 45.0, TrimMode::Individually, &mut tmp);
        assert_eq!(g.contours.len(), 1);
        assert!(g.contours[0].closed);
    }

    #[test]
    fn repeater_copies_alpha() {
        let mut tmp = Geometry::default();
        let mut g = square();
        let rp = RepeatParams {
            copies: 3.0,
            offset: 0.0,
            tr: TransformValues {
                anchor: [0.0, 0.0],
                position: [10.0, 0.0],
                scale: [100.0, 100.0],
                rotation: 0.0,
                skew: 0.0,
                skew_axis: 0.0,
            },
            start_opacity: 100.0,
            end_opacity: 0.0,
            composite: Composite::Above,
        };
        repeat(&mut g, 0, &rp, &Mat::IDENTITY, &mut tmp);
        assert_eq!(g.contours.len(), 3);
        let alphas: [f32; 3] = core::array::from_fn(|i| g.contours[i].alpha);
        assert_eq!(alphas, [1.0, 0.5, 0.0]);
        assert!(fmath::abs(g.contours[2].pts(&g.pts)[0][0] - 120.0) < 1e-3);
    }
}
