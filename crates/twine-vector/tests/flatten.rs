//! Curve flattening.

use twine_core::{Angle, Fx, Transform};
use twine_vector::{DEFAULT_TOLERANCE, FxPoint, Line, MAX_CURVE_SEGMENTS, Path, flatten};

fn f(p: FxPoint) -> (f64, f64) {
    (f64::from(p.x.0) / 65536.0, f64::from(p.y.0) / 65536.0)
}

/// Distance from `p` to the polyline `lines`.
fn dist_to_lines(p: (f64, f64), lines: &[Line]) -> f64 {
    lines
        .iter()
        .map(|l| {
            let (a, b) = (f(l.p0), f(l.p1));
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 {
                0.0
            } else {
                (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
            };
            let (x, y) = (a.0 + t * dx, a.1 + t * dy);
            ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt()
        })
        .fold(f64::MAX, f64::min)
}

fn curve_lines(p: &Path, t: &Transform) -> Vec<Line> {
    let mut out = Vec::new();
    flatten(p, t, DEFAULT_TOLERANCE, &mut out);
    out
}

#[test]
fn quad_flatten_error_below_tolerance() {
    let (p0, c, p1) = ((0.0, 0.0), (150.0, 200.0), (300.0, 10.0));
    let mut p = Path::new();
    p.move_to(FxPoint::from_int(0, 0))
        .quad_to(FxPoint::from_int(150, 200), FxPoint::from_int(300, 10));
    let lines = curve_lines(&p, &Transform::IDENTITY);
    // The closing line is included; sample the curve.
    for i in 0..=1000 {
        let t = f64::from(i) / 1000.0;
        let m = 1.0 - t;
        let q = (
            m * m * p0.0 + 2.0 * m * t * c.0 + t * t * p1.0,
            m * m * p0.1 + 2.0 * m * t * c.1 + t * t * p1.1,
        );
        let d = dist_to_lines(q, &lines);
        assert!(d < 0.25, "t={t} d={d}");
    }
}

#[test]
fn circle_r100_at_most_64_segments_error_below_quarter_px() {
    let mut p = Path::new();
    p.circle(FxPoint::from_int(0, 0), Fx::from_int(100));
    let lines = curve_lines(&p, &Transform::IDENTITY);
    assert!(lines.len() <= 64, "{}", lines.len());
    for l in &lines {
        // Chord midpoints are the furthest from the circle.
        let (a, b) = (f(l.p0), f(l.p1));
        let m = (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1));
        let r = (m.0 * m.0 + m.1 * m.1).sqrt();
        assert!((100.0 - r).abs() < 0.25, "{r}");
    }
}

#[test]
fn cubic_flatten_segment_count_bounded() {
    let mut p = Path::new();
    p.move_to(FxPoint::from_int(0, 0)).cubic_to(
        FxPoint::from_int(30_000, -30_000),
        FxPoint::from_int(-30_000, 30_000),
        FxPoint::from_int(10, 0),
    );
    let lines = curve_lines(&p, &Transform::IDENTITY);
    assert_eq!(lines.len(), MAX_CURVE_SEGMENTS as usize + 1); // + closing line
}

#[test]
fn degenerate_curve_single_segment() {
    let mut p = Path::new();
    // Control points on the chord: a straight "curve".
    p.move_to(FxPoint::from_int(0, 0))
        .cubic_to(
            FxPoint::from_int(10, 0),
            FxPoint::from_int(20, 0),
            FxPoint::from_int(30, 0),
        )
        .quad_to(FxPoint::from_int(40, 0), FxPoint::from_int(50, 0));
    let lines = curve_lines(&p, &Transform::IDENTITY);
    // cubic (1) + quad (1) + closing line (1).
    assert_eq!(lines.len(), 3);
    // A zero-size curve yields nothing.
    let mut q = Path::new();
    q.move_to(FxPoint::from_int(5, 5)).cubic_to(
        FxPoint::from_int(5, 5),
        FxPoint::from_int(5, 5),
        FxPoint::from_int(5, 5),
    );
    assert!(curve_lines(&q, &Transform::IDENTITY).is_empty());
}

#[test]
fn transform_applied_before_flatten() {
    // A small circle scaled 20×: the tolerance must hold in device space, so it needs many more
    // segments than the untransformed circle.
    let mut p = Path::new();
    p.circle(FxPoint::from_int(0, 0), Fx::from_int(5));
    let small = curve_lines(&p, &Transform::IDENTITY);
    let t = Transform::scale(Fx::from_int(20), Fx::from_int(20)).then(Transform::rotate(Angle::deg(10)));
    let big = curve_lines(&p, &t);
    assert!(big.len() > 2 * small.len(), "{} vs {}", big.len(), small.len());
    for l in &big {
        let (a, b) = (f(l.p0), f(l.p1));
        let m = (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1));
        let r = (m.0 * m.0 + m.1 * m.1).sqrt();
        assert!((100.0 - r).abs() < 0.3, "{r}");
    }
}

#[test]
fn open_subpaths_are_closed_for_filling() {
    let mut p = Path::new();
    p.move_to(FxPoint::from_int(0, 0))
        .line_to(FxPoint::from_int(10, 0))
        .line_to(FxPoint::from_int(10, 10))
        .move_to(FxPoint::from_int(20, 20))
        .line_to(FxPoint::from_int(30, 20))
        .line_to(FxPoint::from_int(30, 30));
    let lines = curve_lines(&p, &Transform::IDENTITY);
    assert_eq!(lines.len(), 6);
    assert_eq!(
        lines[2],
        Line {
            p0: FxPoint::from_int(10, 10),
            p1: FxPoint::from_int(0, 0)
        }
    );
}
