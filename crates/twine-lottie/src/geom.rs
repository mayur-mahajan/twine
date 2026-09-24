//! Float geometry of the renderer: the affine [`Mat`], evaluated transforms, and the contour
//! arena ([`Geometry`]) that shape items fill and modifiers (trim paths, repeater) rewrite.
//!
//! Contours are stored as a start point followed by cubic segments `(c1, c2, end)`; lines are
//! cubics with the control points on the end points. All buffers are reused between frames.

use alloc::vec::Vec;

use twine_core::Transform as FxTransform;

use crate::eval::{Lerp, cubic_point};
use crate::fmath::{self, DEG, cos, fx, sin, sqrt, tan};
use crate::model::{PathData, Position, Transform, Vec2};

/// A 2-D affine transform in `f32`: `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty` (the same
/// layout as [`twine_core::Transform`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mat {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Mat {
    pub const IDENTITY: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Mat {
        Mat {
            tx: x,
            ty: y,
            ..Mat::IDENTITY
        }
    }

    pub fn scale(sx: f32, sy: f32) -> Mat {
        Mat {
            a: sx,
            d: sy,
            ..Mat::IDENTITY
        }
    }

    /// Clockwise on screen (y down) for positive `rad`.
    pub fn rotate(rad: f32) -> Mat {
        let (s, c) = (sin(rad), cos(rad));
        Mat {
            a: c,
            b: s,
            c: -s,
            d: c,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// `self · o` (apply `o` first, then `self`).
    pub fn mul(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        [
            self.a * p[0] + self.c * p[1] + self.tx,
            self.b * p[0] + self.d * p[1] + self.ty,
        ]
    }

    pub fn det(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Mat> {
        let det = self.det();
        if !(fmath::abs(det) > 1e-12) || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Mat {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }

    pub fn to_fx(self) -> FxTransform {
        FxTransform {
            a: fx(self.a),
            b: fx(self.b),
            c: fx(self.c),
            d: fx(self.d),
            tx: fx(self.tx),
            ty: fx(self.ty),
        }
    }

    /// Screen bounds of the rectangle `(0, 0)–(w, h)` under this transform.
    pub fn map_rect_bounds(&self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let pts = [
            self.apply([0.0, 0.0]),
            self.apply([w, 0.0]),
            self.apply([0.0, h]),
            self.apply([w, h]),
        ];
        let mut b = (pts[0][0], pts[0][1], pts[0][0], pts[0][1]);
        for p in &pts[1..] {
            b.0 = b.0.min(p[0]);
            b.1 = b.1.min(p[1]);
            b.2 = b.2.max(p[0]);
            b.3 = b.3.max(p[1]);
        }
        b
    }
}

/// Evaluated parts of a [`Transform`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct TransformValues {
    pub anchor: Vec2,
    pub position: Vec2,
    pub scale: Vec2,
    pub rotation: f32,
    pub skew: f32,
    pub skew_axis: f32,
}

impl TransformValues {
    pub fn eval(t: &Transform, frame: f32) -> Self {
        let position = match &t.position {
            Position::Combined(p) => p.value(frame),
            Position::Split(x, y) => [x.value(frame), y.value(frame)],
        };
        Self {
            anchor: t.anchor.value(frame),
            position,
            scale: t.scale.value(frame),
            rotation: t.rotation.value(frame),
            skew: t.skew.value(frame),
            skew_axis: t.skew_axis.value(frame),
        }
    }

    /// `translate(p) · rotate(r) · skew · scale(s/100) · translate(−a)` (lottie-web order).
    pub fn matrix(&self) -> Mat {
        let mut m = Mat::translate(self.position[0], self.position[1]);
        if self.rotation != 0.0 {
            m = m.mul(&Mat::rotate(self.rotation * DEG));
        }
        if self.skew != 0.0 {
            // lottie-web `skewFromAxis(-sk, sa)`: rotate(sa)⁻¹ · shearX(tan(−sk)) · rotate(sa),
            // applied in that (right-to-left) order.
            let sa = self.skew_axis * DEG;
            let shear = Mat {
                c: tan(-self.skew * DEG),
                ..Mat::IDENTITY
            };
            m = m.mul(&Mat::rotate(-sa)).mul(&shear).mul(&Mat::rotate(sa));
        }
        m.mul(&Mat::scale(self.scale[0] / 100.0, self.scale[1] / 100.0))
            .mul(&Mat::translate(-self.anchor[0], -self.anchor[1]))
    }
}

/// The matrix of `t` at `frame`.
pub(crate) fn transform_matrix(t: &Transform, frame: f32) -> Mat {
    TransformValues::eval(t, frame).matrix()
}

/// One contour in the [`Geometry`] arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Contour {
    /// Index of the start point in `Geometry::pts`.
    pub start: u32,
    /// Number of cubic segments (3 points each after the start point).
    pub segs: u32,
    pub closed: bool,
    /// Depth-first number of the shape item that produced it.
    pub owner: u32,
    /// Opacity factor (repeater copies).
    pub alpha: f32,
}

impl Contour {
    pub fn pts<'g>(&self, pts: &'g [Vec2]) -> &'g [Vec2] {
        let s = self.start as usize;
        &pts[s..s + 1 + 3 * self.segs as usize]
    }
}

/// Contours in layer space.
#[derive(Clone, Debug, Default)]
pub(crate) struct Geometry {
    pub contours: Vec<Contour>,
    pub pts: Vec<Vec2>,
}

/// Bezier circle constant.
pub(crate) const KAPPA: f32 = 0.552_284_8;

impl Geometry {
    pub fn clear(&mut self) {
        self.contours.clear();
        self.pts.clear();
    }

    pub fn begin(&mut self, p: Vec2) {
        self.pts.push(p);
    }

    pub fn cubic(&mut self, c1: Vec2, c2: Vec2, p: Vec2) {
        self.pts.extend_from_slice(&[c1, c2, p]);
    }

    pub fn line(&mut self, p: Vec2) {
        let last = self.pts.last().copied().unwrap_or(p);
        self.cubic(last, p, p);
    }

    /// Finishes the contour begun at point index `start`, mapping its points by `m`.
    pub fn finish(&mut self, start: usize, closed: bool, owner: u32, m: &Mat) {
        if self.pts.len() <= start + 1 {
            self.pts.truncate(start);
            return;
        }
        for p in &mut self.pts[start..] {
            *p = m.apply(*p);
        }
        let segs = ((self.pts.len() - start - 1) / 3) as u32;
        self.contours.push(Contour {
            start: start as u32,
            segs,
            closed,
            owner,
            alpha: 1.0,
        });
    }

    /// Lottie rectangle (starts at the top-right corner, clockwise unless `reversed`).
    pub fn rect(&mut self, p: Vec2, s: Vec2, r: f32, reversed: bool, owner: u32, m: &Mat) {
        let (hw, hh) = (fmath::abs(s[0]) / 2.0, fmath::abs(s[1]) / 2.0);
        let (x0, y0, x1, y1) = (p[0] - hw, p[1] - hh, p[0] + hw, p[1] + hh);
        let r = fmath::clamp(r, 0.0, hw.min(hh));
        let start = self.pts.len();
        if r <= 0.0 {
            let v = [[x1, y0], [x1, y1], [x0, y1], [x0, y0]];
            self.polygon(&v, reversed);
        } else {
            let k = r * (1.0 - KAPPA);
            // (vertex before the corner, control 1, control 2, vertex after the corner)
            let corners = [
                ([x1, y1 - r], [x1, y1 - k], [x1 - k, y1], [x1 - r, y1]),
                ([x0 + r, y1], [x0 + k, y1], [x0, y1 - k], [x0, y1 - r]),
                ([x0, y0 + r], [x0, y0 + k], [x0 + k, y0], [x0 + r, y0]),
                ([x1 - r, y0], [x1 - k, y0], [x1, y0 + k], [x1, y0 + r]),
            ];
            if reversed {
                self.begin([x1, y0 + r]);
                for (i, &(a, c1, c2, b)) in corners.iter().rev().enumerate() {
                    if i > 0 {
                        self.line(b);
                    }
                    self.cubic(c2, c1, a);
                }
                self.line([x1, y0 + r]);
            } else {
                self.begin([x1, y0 + r]);
                for &(a, c1, c2, b) in &corners {
                    self.line(a);
                    self.cubic(c1, c2, b);
                }
            }
        }
        self.finish(start, true, owner, m);
    }

    fn polygon(&mut self, v: &[Vec2], reversed: bool) {
        if reversed {
            self.begin(v[0]);
            for p in v[1..].iter().rev() {
                self.line(*p);
            }
        } else {
            self.begin(v[0]);
            for p in &v[1..] {
                self.line(*p);
            }
        }
        self.line(v[0]);
    }

    /// Lottie ellipse (starts at the top, clockwise unless `reversed`).
    pub fn ellipse(&mut self, p: Vec2, s: Vec2, reversed: bool, owner: u32, m: &Mat) {
        let (rx, ry) = (s[0] / 2.0, s[1] / 2.0);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let [cx, cy] = p;
        let start = self.pts.len();
        let top = [cx, cy - ry];
        self.begin(top);
        if reversed {
            self.cubic([cx - kx, cy - ry], [cx - rx, cy - ky], [cx - rx, cy]);
            self.cubic([cx - rx, cy + ky], [cx - kx, cy + ry], [cx, cy + ry]);
            self.cubic([cx + kx, cy + ry], [cx + rx, cy + ky], [cx + rx, cy]);
            self.cubic([cx + rx, cy - ky], [cx + kx, cy - ry], top);
        } else {
            self.cubic([cx + kx, cy - ry], [cx + rx, cy - ky], [cx + rx, cy]);
            self.cubic([cx + rx, cy + ky], [cx + kx, cy + ry], [cx, cy + ry]);
            self.cubic([cx - kx, cy + ry], [cx - rx, cy + ky], [cx - rx, cy]);
            self.cubic([cx - rx, cy - ky], [cx - kx, cy - ry], top);
        }
        self.finish(start, true, owner, m);
    }

    /// A bezier path from vertices and relative tangents.
    pub fn path(&mut self, d: &PathData, reversed: bool, owner: u32, m: &Mat) {
        let n = d.v.len().min(d.i.len()).min(d.o.len());
        if n == 0 {
            return;
        }
        let add = |a: Vec2, b: Vec2| [a[0] + b[0], a[1] + b[1]];
        let start = self.pts.len();
        let segs = if d.closed { n } else { n - 1 };
        if reversed {
            // Walk the vertices backwards: out tangents become in tangents.
            self.begin(d.v[n - 1]);
            for s in 0..segs {
                let i = n - 1 - s;
                let j = (i + n - 1) % n;
                self.cubic(add(d.v[i], d.i[i]), add(d.v[j], d.o[j]), d.v[j]);
            }
        } else {
            self.begin(d.v[0]);
            for i in 0..segs {
                let j = (i + 1) % n;
                self.cubic(add(d.v[i], d.o[i]), add(d.v[j], d.i[j]), d.v[j]);
            }
        }
        self.finish(start, d.closed, owner, m);
    }
}

/// Splits the cubic `p` at `t`, returning the two halves.
pub(crate) fn split_cubic(p: [Vec2; 4], t: f32) -> ([Vec2; 4], [Vec2; 4]) {
    let l = |a: Vec2, b: Vec2| {
        let mut o = [0.0; 2];
        Lerp::lerp_into(&a, &b, crate::eval::Progress::uniform(t), &mut o);
        o
    };
    let p01 = l(p[0], p[1]);
    let p12 = l(p[1], p[2]);
    let p23 = l(p[2], p[3]);
    let p012 = l(p01, p12);
    let p123 = l(p12, p23);
    let m = l(p012, p123);
    ([p[0], p01, p012, m], [m, p123, p23, p[3]])
}

/// The part `[t0, t1]` of the cubic `p`.
pub(crate) fn sub_cubic(p: [Vec2; 4], t0: f32, t1: f32) -> [Vec2; 4] {
    let (head, _) = if t1 < 1.0 { split_cubic(p, t1) } else { (p, p) };
    if t0 <= 0.0 {
        return head;
    }
    let rel = if t1 > 0.0 { t0 / t1 } else { 0.0 };
    split_cubic(head, rel).1
}

/// Samples per cubic when measuring lengths.
pub(crate) const LEN_SAMPLES: usize = 16;

/// Length of the cubic `p` and the cumulative lengths at `LEN_SAMPLES` equal parameter steps.
pub(crate) fn cubic_lengths(p: [Vec2; 4], out: &mut [f32; LEN_SAMPLES + 1]) -> f32 {
    out[0] = 0.0;
    let mut prev = p[0];
    let mut acc = 0.0;
    for (i, o) in out.iter_mut().enumerate().skip(1) {
        let q = cubic_point(p[0], p[1], p[2], p[3], i as f32 / LEN_SAMPLES as f32);
        let (dx, dy) = (q[0] - prev[0], q[1] - prev[1]);
        acc += sqrt(dx * dx + dy * dy);
        *o = acc;
        prev = q;
    }
    acc
}

/// Parameter `t` where the cumulative length reaches `len` (linear between samples).
pub(crate) fn t_at_length(table: &[f32; LEN_SAMPLES + 1], len: f32) -> f32 {
    let total = table[LEN_SAMPLES];
    if !(len > 0.0) {
        return 0.0;
    }
    if len >= total {
        return 1.0;
    }
    let i = table.partition_point(|&l| l < len).clamp(1, LEN_SAMPLES);
    let (l0, l1) = (table[i - 1], table[i]);
    let f = if l1 > l0 { (len - l0) / (l1 - l0) } else { 0.0 };
    ((i - 1) as f32 + f) / LEN_SAMPLES as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec2, b: Vec2) -> bool {
        fmath::abs(a[0] - b[0]) < 1e-3 && fmath::abs(a[1] - b[1]) < 1e-3
    }

    #[test]
    fn matrix_order_and_inverse() {
        let t = TransformValues {
            anchor: [10.0, 0.0],
            position: [100.0, 50.0],
            scale: [200.0, 100.0],
            rotation: 90.0,
            skew: 0.0,
            skew_axis: 0.0,
        };
        let m = t.matrix();
        // anchor → position
        assert!(near(m.apply([10.0, 0.0]), [100.0, 50.0]));
        // +x 1 → scaled 2 → rotated clockwise 90° → +y 2
        assert!(near(m.apply([11.0, 0.0]), [100.0, 52.0]));
        let inv = m.invert().unwrap();
        assert!(near(inv.apply(m.apply([3.0, 4.0])), [3.0, 4.0]));
        assert!(Mat::scale(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn split_and_measure() {
        let line = [[0.0, 0.0], [0.0, 0.0], [10.0, 0.0], [10.0, 0.0]];
        let mut tab = [0.0; LEN_SAMPLES + 1];
        let len = cubic_lengths(
            [[0.0, 0.0], [10.0 / 3.0, 0.0], [20.0 / 3.0, 0.0], [10.0, 0.0]],
            &mut tab,
        );
        assert!(fmath::abs(len - 10.0) < 1e-4);
        assert!(fmath::abs(t_at_length(&tab, 5.0) - 0.5) < 1e-4);
        let (a, b) = split_cubic(line, 0.5);
        assert!(near(a[3], b[0]));
        let s = sub_cubic(
            [[0.0, 0.0], [10.0 / 3.0, 0.0], [20.0 / 3.0, 0.0], [10.0, 0.0]],
            0.25,
            0.75,
        );
        assert!(near(s[0], [2.5, 0.0]) && near(s[3], [7.5, 0.0]));
    }
}
