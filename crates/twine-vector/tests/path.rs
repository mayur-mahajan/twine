//! Path model and builder.

use proptest::prelude::*;
use twine_core::{Angle, Fx, Rect, Transform};
use twine_vector::{FxPoint, FxRect, Path, PathEl, Verb};

fn fx(v: f64) -> Fx {
    #[allow(clippy::cast_possible_truncation)]
    Fx((v * 65536.0).round() as i32)
}

fn near(a: FxPoint, b: FxPoint, lsb: i32) -> bool {
    (a.x.0 - b.x.0).abs() <= lsb && (a.y.0 - b.y.0).abs() <= lsb
}

#[test]
fn rect_with_radius_has_4_cubics() {
    let mut p = Path::new();
    p.rect(Rect::from_xywh(10, 20, 100, 50), 8);
    let cubics = p.verbs().iter().filter(|v| **v == Verb::CubicTo).count();
    let lines = p.verbs().iter().filter(|v| **v == Verb::LineTo).count();
    assert_eq!((cubics, lines), (4, 4));
    assert_eq!(p.verbs().first(), Some(&Verb::MoveTo));
    assert_eq!(p.verbs().last(), Some(&Verb::Close));
    assert_eq!(p.bounds(), FxRect::from(Rect::from_xywh(10, 20, 100, 50)));
    // Radius 0 → plain rectangle; a huge radius is clamped to a stadium.
    let mut q = Path::new();
    q.rect(Rect::from_xywh(0, 0, 10, 10), 0);
    assert!(!q.verbs().contains(&Verb::CubicTo));
    let mut s = Path::new();
    s.rect(Rect::from_xywh(0, 0, 40, 10), 1000);
    assert_eq!(s.bounds(), FxRect::from(Rect::from_xywh(0, 0, 40, 10)));
}

#[test]
fn circle_bounds() {
    let mut p = Path::new();
    p.circle(FxPoint::from_int(50, 40), Fx::from_int(30));
    assert_eq!(p.bounds(), FxRect::from(Rect::new(20, 10, 80, 70)));
    assert_eq!(p.verbs().iter().filter(|v| **v == Verb::CubicTo).count(), 4);
    let mut e = Path::new();
    e.ellipse(FxPoint::from_int(0, 0), Fx::from_int(20), Fx::from_int(5));
    assert_eq!(e.bounds(), FxRect::from(Rect::new(-20, -5, 20, 5)));
}

#[test]
fn arc_to_quarter_circle_endpoint_exact() {
    let mut p = Path::new();
    let end = FxPoint::from_int(0, 100);
    p.move_to(FxPoint::from_int(100, 0));
    p.arc_to(FxPoint::from_int(100, 100), Angle(0), false, true, end);
    assert_eq!(p.verbs(), &[Verb::MoveTo, Verb::CubicTo]);
    assert_eq!(p.current_point(), end);
    // Compare with the ideal quarter circle around the origin (clockwise on screen).
    let pts = p.points();
    let k = 100.0 * 0.552_284_75;
    assert!(near(pts[1], FxPoint::new(fx(100.0), fx(k)), 200), "{:?}", pts[1]);
    assert!(near(pts[2], FxPoint::new(fx(k), fx(100.0)), 200), "{:?}", pts[2]);
    // The large arc the other way round: 270° → 3 cubics, same end point.
    let mut q = Path::new();
    q.move_to(FxPoint::from_int(100, 0));
    q.arc_to(FxPoint::from_int(100, 100), Angle(0), true, false, end);
    assert_eq!(q.verbs().len(), 4);
    assert_eq!(q.current_point(), end);
    // Every curve point stays near the circle of radius 100 around (0, 0).
    for el in &q {
        if let PathEl::CubicTo(_, _, e) = el {
            let (x, y) = (f64::from(e.x.0) / 65536.0, f64::from(e.y.0) / 65536.0);
            assert!(((x * x + y * y).sqrt() - 100.0).abs() < 0.05, "{x} {y}");
        }
    }
}

#[test]
fn arc_rotated_ellipse_end_exact() {
    let mut p = Path::new();
    p.move_to(FxPoint::from_int(10, 10));
    let end = FxPoint::new(fx(60.25), fx(-12.5));
    p.arc_to(FxPoint::from_int(40, 20), Angle::deg(30), true, true, end);
    assert_eq!(p.current_point(), end);
    assert!(p.verbs().len() >= 3);
}

#[test]
fn transform_translate_scale_rotate() {
    let mut p = Path::new();
    p.move_to(FxPoint::from_int(10, 0))
        .line_to(FxPoint::new(fx(2.5), fx(-4.0)));
    let t = Transform::scale(Fx::from_int(2), Fx::from_ratio(1, 2))
        .then(Transform::rotate(Angle::deg(90)))
        .then(Transform::translate(Fx::from_int(100), Fx::from_int(50)));
    p.transform(&t);
    // (10, 0) → scale (20, 0) → rotate 90° cw (0, 20) → translate (100, 70).
    assert!(
        near(p.points()[0], FxPoint::from_int(100, 70), 1),
        "{:?}",
        p.points()[0]
    );
    // (2.5, −4) → (5, −2) → (2, 5) → (102, 55).
    assert!(
        near(p.points()[1], FxPoint::from_int(102, 55), 1),
        "{:?}",
        p.points()[1]
    );
}

#[test]
fn clear_keeps_capacity() {
    let mut p = Path::with_capacity(16, 32);
    p.circle(FxPoint::from_int(0, 0), Fx::ONE);
    p.clear();
    assert!(p.is_empty());
    assert_eq!(p.bounds(), FxRect::ZERO);
}

fn arb_point() -> impl Strategy<Value = FxPoint> {
    (-20_000_000i32..20_000_000, -20_000_000i32..20_000_000).prop_map(|(x, y)| FxPoint::new(Fx(x), Fx(y)))
}

proptest! {
    #[test]
    fn bounds_contain_all_points(
        cmds in prop::collection::vec((0u8..6, arb_point(), arb_point(), arb_point(), any::<bool>(), any::<bool>()), 1..20)
    ) {
        let mut p = Path::new();
        for (k, a, b, c, f1, f2) in cmds {
            match k {
                0 => { p.move_to(a); }
                1 => { p.line_to(a); }
                2 => { p.quad_to(a, b); }
                3 => { p.cubic_to(a, b, c); }
                4 => { p.arc_to(FxPoint::new(b.x.abs(), b.y.abs()), Angle(c.x.0 % 3600), f1, f2, a); }
                _ => { p.close(); }
            }
        }
        let b = p.bounds();
        for &q in p.points() {
            prop_assert!(b.contains_incl(q));
        }
        // Verb/point bookkeeping is consistent.
        let n: usize = p.verbs().iter().map(|v| v.points()).sum();
        prop_assert_eq!(n, p.points().len());
    }
}
