//! Evaluating animated properties at a frame: [`Lerp`], keyframe segment lookup (cached for
//! sequential playback), hold and cubic-bezier easing (per axis), spatial position tangents, and
//! layer time mapping.
//!
//! Evaluation never allocates once the output buffers have their capacity: values are written
//! into caller-owned outputs ([`Animatable::eval_into`]).

use alloc::vec::Vec;

use crate::fmath::abs;
use crate::model::{Animatable, Ease, Keyframe, Keyframes, Layer, PathData, Rgba, Vec2};

/// Progress of an interpolation, one value per component (Lottie eases each axis separately).
/// Components beyond the fourth use the fourth value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress(pub [f32; 4]);

impl Progress {
    /// The same progress for every component.
    #[must_use]
    pub const fn uniform(t: f32) -> Self {
        Self([t; 4])
    }

    /// Progress of component `i`.
    #[must_use]
    pub fn get(&self, i: usize) -> f32 {
        self.0[i.min(3)]
    }
}

/// Values that can be interpolated between keyframes.
///
/// ```
/// use twine_lottie::eval::{Lerp, Progress};
/// let mut out = [0.0f32; 2];
/// Lerp::lerp_into(&[0.0, 10.0], &[10.0, 20.0], Progress([0.5, 0.25, 0.0, 0.0]), &mut out);
/// assert_eq!(out, [5.0, 12.5]);
/// ```
pub trait Lerp: Clone {
    /// Writes the interpolation between `a` (progress 0) and `b` (progress 1) into `out`.
    /// Must not allocate once `out` has the needed capacity.
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self);

    /// Spatial interpolation along the cubic `a, a + to, b + ti, b` (positions); returns
    /// `false` when the type has no spatial interpolation.
    fn lerp_spatial(_a: &Self, _b: &Self, _to: Vec2, _ti: Vec2, _t: f32, _out: &mut Self) -> bool {
        false
    }

    /// Copies `src` into `out` (reusing `out`'s storage).
    fn assign(out: &mut Self, src: &Self) {
        out.clone_from(src);
    }
}

/// `a + (b − a)·t`.
#[inline]
pub(crate) fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Lerp for f32 {
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        *out = mix(*a, *b, t.get(0));
    }
}

impl Lerp for Vec2 {
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        *out = [mix(a[0], b[0], t.get(0)), mix(a[1], b[1], t.get(1))];
    }

    fn lerp_spatial(a: &Self, b: &Self, to: Vec2, ti: Vec2, t: f32, out: &mut Self) -> bool {
        let c1 = [a[0] + to[0], a[1] + to[1]];
        let c2 = [b[0] + ti[0], b[1] + ti[1]];
        *out = cubic_point(*a, c1, c2, *b, t);
        true
    }
}

impl Lerp for [f32; 3] {
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        *out = core::array::from_fn(|i| mix(a[i], b[i], t.get(i)));
    }
}

impl Lerp for Rgba {
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        *out = core::array::from_fn(|i| mix(a[i], b[i], t.get(i)));
    }
}

impl Lerp for Vec<f32> {
    /// Element-wise; lists of different lengths hold `a`.
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        out.clear();
        if a.len() == b.len() {
            let t = t.get(0);
            out.extend(a.iter().zip(b).map(|(x, y)| mix(*x, *y, t)));
        } else {
            out.extend_from_slice(a);
        }
    }
}

fn lerp_points(a: &[Vec2], b: &[Vec2], t: f32, out: &mut Vec<Vec2>) {
    out.clear();
    out.extend(
        a.iter()
            .zip(b)
            .map(|(p, q)| [mix(p[0], q[0], t), mix(p[1], q[1], t)]),
    );
}

impl Lerp for PathData {
    /// Vertex-wise; paths with different vertex counts hold `a`.
    fn lerp_into(a: &Self, b: &Self, t: Progress, out: &mut Self) {
        if a.v.len() != b.v.len() || a.i.len() != b.i.len() || a.o.len() != b.o.len() {
            Self::assign(out, a);
            return;
        }
        let t = t.get(0);
        out.closed = a.closed;
        lerp_points(&a.v, &b.v, t, &mut out.v);
        lerp_points(&a.i, &b.i, t, &mut out.i);
        lerp_points(&a.o, &b.o, t, &mut out.o);
    }

    fn assign(out: &mut Self, src: &Self) {
        out.closed = src.closed;
        out.v.clear();
        out.v.extend_from_slice(&src.v);
        out.i.clear();
        out.i.extend_from_slice(&src.i);
        out.o.clear();
        out.o.extend_from_slice(&src.o);
    }
}

/// Point of the cubic bezier `p0, p1, p2, p3` at parameter `t`.
pub(crate) fn cubic_point(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
    ]
}

/// One coordinate of the easing cubic `0, c1, c2, 1` at `u`.
#[inline]
fn bez1(c1: f32, c2: f32, u: f32) -> f32 {
    let v = 1.0 - u;
    3.0 * v * v * u * c1 + 3.0 * v * u * u * c2 + u * u * u
}

/// Derivative of [`bez1`].
#[inline]
fn bez1_d(c1: f32, c2: f32, u: f32) -> f32 {
    let v = 1.0 - u;
    3.0 * v * v * c1 + 6.0 * v * u * (c2 - c1) + 3.0 * u * u * (1.0 - c2)
}

/// CSS-style cubic-bezier easing `(x1, y1, x2, y2)` at time progress `x ∈ [0, 1]`: solves
/// `bx(u) = x` with 4 Newton iterations, falling back to bisection, then returns `by(u)`.
///
/// ```
/// use twine_lottie::eval::cubic_bezier_ease;
/// assert_eq!(cubic_bezier_ease(0.0, 0.0, 1.0, 1.0, 0.3), 0.3); // linear
/// let v = cubic_bezier_ease(0.42, 0.0, 0.58, 1.0, 0.5); // ease-in-out
/// assert!((v - 0.5).abs() < 1e-4);
/// ```
#[must_use]
pub fn cubic_bezier_ease(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let x = crate::fmath::clamp(x, 0.0, 1.0);
    let (x1, x2) = (
        crate::fmath::clamp(x1, 0.0, 1.0),
        crate::fmath::clamp(x2, 0.0, 1.0),
    );
    if x1 == y1 && x2 == y2 {
        return x;
    }
    if x <= 0.0 || x >= 1.0 {
        return x;
    }
    let mut u = x;
    let mut ok = false;
    for _ in 0..4 {
        let err = bez1(x1, x2, u) - x;
        if abs(err) < 1e-6 {
            ok = true;
            break;
        }
        let d = bez1_d(x1, x2, u);
        if abs(d) < 1e-6 {
            break;
        }
        u -= err / d;
        if !(0.0..=1.0).contains(&u) {
            break;
        }
    }
    if !ok && (abs(bez1(x1, x2, u) - x) >= 1e-6 || !(0.0..=1.0).contains(&u)) {
        // Bisection: bx is monotonic for x1, x2 ∈ [0, 1].
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        u = x;
        for _ in 0..30 {
            let v = bez1(x1, x2, u);
            if abs(v - x) < 1e-7 {
                break;
            }
            if v < x {
                lo = u;
            } else {
                hi = u;
            }
            u = f32::midpoint(lo, hi);
        }
    }
    bez1(y1, y2, u)
}

impl Ease {
    /// Eased progress of every axis for linear progress `x`.
    #[must_use]
    pub fn progress(&self, x: f32) -> Progress {
        let n = usize::from(self.axes.clamp(1, 4));
        let mut out = [0.0; 4];
        for (i, o) in out.iter_mut().enumerate() {
            let a = i.min(n - 1);
            *o = cubic_bezier_ease(self.out[a][0], self.out[a][1], self.inn[a][0], self.inn[a][1], x);
        }
        Progress(out)
    }
}

impl<T: Lerp> Keyframes<T> {
    /// Index `k` of the segment `[t_k, t_{k+1})` containing `frame` (`frames.len() ≥ 2`,
    /// `t_0 ≤ frame < t_last`). Tries the cached segment and its successor first.
    fn segment(&self, frame: f32) -> usize {
        let f = &self.frames;
        let inside = |k: usize| k + 1 < f.len() && f[k].t <= frame && frame < f[k + 1].t;
        let last = self.last.get() as usize;
        let k = if inside(last) {
            last
        } else if inside(last + 1) {
            last + 1
        } else {
            f.partition_point(|kf| kf.t <= frame).saturating_sub(1)
        };
        let k = k.min(f.len().saturating_sub(2));
        self.last.set(k as u32);
        k
    }

    /// Writes the value at `frame` into `out`.
    pub fn eval_into(&self, frame: f32, out: &mut T) {
        let f = &self.frames;
        let (Some(first), Some(last)) = (f.first(), f.last()) else {
            return;
        };
        if f.len() == 1 || !(frame > first.t) {
            T::assign(out, &first.s);
            return;
        }
        if frame >= last.t {
            T::assign(out, &last.s);
            return;
        }
        let k = self.segment(frame);
        let (a, b) = (&f[k], &f[k + 1]);
        eval_segment(a, b, frame, out);
    }
}

fn eval_segment<T: Lerp>(a: &Keyframe<T>, b: &Keyframe<T>, frame: f32, out: &mut T) {
    if a.hold {
        T::assign(out, &a.s);
        return;
    }
    let end = a.e.as_ref().unwrap_or(&b.s);
    let span = b.t - a.t;
    let x = if span > 0.0 { (frame - a.t) / span } else { 1.0 };
    let p = match &a.ease {
        Some(e) => e.progress(x),
        None => Progress::uniform(x),
    };
    if a.to.is_some() || a.ti.is_some() {
        let to = a.to.unwrap_or([0.0, 0.0]);
        let ti = a.ti.unwrap_or([0.0, 0.0]);
        if T::lerp_spatial(&a.s, end, to, ti, p.get(0), out) {
            return;
        }
    }
    T::lerp_into(&a.s, end, p, out);
}

impl<T: Lerp> Animatable<T> {
    /// Writes the value at `frame` into `out` (reusing its storage).
    ///
    /// ```
    /// use twine_lottie::model::{Animatable, Keyframe, Keyframes};
    /// let kf = |t: f32, s: f32| Keyframe { t, s, e: None, hold: false, ease: None, to: None, ti: None };
    /// let a = Animatable::Keyframed(Keyframes::new(vec![kf(0.0, 0.0), kf(10.0, 100.0)]));
    /// let mut v = 0.0;
    /// a.eval_into(2.5, &mut v);
    /// assert_eq!(v, 25.0);
    /// ```
    pub fn eval_into(&self, frame: f32, out: &mut T) {
        match self {
            Animatable::Static(v) => T::assign(out, v),
            Animatable::Keyframed(k) => k.eval_into(frame, out),
        }
    }
}

impl<T: Lerp + Copy + Default> Animatable<T> {
    /// The value at `frame`.
    #[must_use]
    pub fn value(&self, frame: f32) -> T {
        match self {
            Animatable::Static(v) => *v,
            Animatable::Keyframed(k) => {
                let mut out = T::default();
                k.eval_into(frame, &mut out);
                out
            }
        }
    }
}

impl Layer {
    /// The layer's local time for the containing composition's `frame`: `(frame − st) / sr`.
    #[must_use]
    pub fn local_frame(&self, frame: f32) -> f32 {
        (frame - self.st) / self.sr
    }

    /// Whether the layer is visible at the containing composition's `frame` (`ip ≤ frame < op`
    /// and not hidden).
    #[must_use]
    pub fn is_visible_at(&self, frame: f32) -> bool {
        !self.hidden && self.ip <= frame && frame < self.op
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values of the cubic-bezier (0.25, 0.1, 0.25, 1.0) (CSS "ease"), computed by
    /// bisecting the bezier in double precision.
    #[test]
    fn ease_reference() {
        let refs = [
            (0.1, 0.094_8),
            (0.25, 0.408_51),
            (0.5, 0.802_4),
            (0.75, 0.960_46),
            (0.9, 0.994_32),
        ];
        for (x, y) in refs {
            let v = cubic_bezier_ease(0.25, 0.1, 0.25, 1.0, x);
            assert!(abs(v - y) < 1e-3, "{x}: {v} vs {y}");
        }
    }
}
