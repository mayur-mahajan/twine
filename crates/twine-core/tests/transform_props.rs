//! Property tests for `Transform`.

use proptest::prelude::*;
use twine_core::{Angle, Fx, Point, Scale, Transform};

proptest! {
    #[test]
    fn transform_invert_roundtrip(
        angle in 0i32..3600,
        sx in 64u16..1024,
        sy in 64u16..1024,
        tx in -2000i32..2000,
        ty in -2000i32..2000,
        px in -500i32..500,
        py in -500i32..500,
        x in -1000i32..1000,
        y in -1000i32..1000,
    ) {
        let t = Transform::from_rotate_scale(Angle(angle), Scale(sx), Scale(sy), Point::new(px, py))
            .then(Transform::translate(Fx::from_int(tx), Fx::from_int(ty)));
        let inv = t.invert().expect("scale >= 0.25 is invertible");
        let p = Point::new(x, y);
        let back = inv.map_point(t.map_point(p));
        // map_point rounds to whole pixels; the inverse can scale that error up by 1/scale.
        let tol = 1 + 256 / i32::from(sx.min(sy));
        prop_assert!((back.x - p.x).abs() <= tol && (back.y - p.y).abs() <= tol,
            "{:?} -> {:?} -> {:?} (tol {})", p, t.map_point(p), back, tol);
        // Without intermediate rounding the round trip is within ±1 px.
        let (fx, fy) = t.map(Fx::from_int(x), Fx::from_int(y));
        let (bx, by) = inv.map(fx, fy);
        prop_assert!((bx.to_int_round() - x).abs() <= 1 && (by.to_int_round() - y).abs() <= 1);
    }
}
