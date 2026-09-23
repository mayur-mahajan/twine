//! Accuracy and property tests of the integer math against `f64` references.
#![allow(clippy::float_arithmetic, clippy::cast_precision_loss, clippy::cast_lossless)]

use proptest::prelude::*;
use twine_core::Angle;
use twine_core::math::{atan2, cos, isqrt64, sin, udiv255};

#[test]
fn sin_within_2_of_f64() {
    for d in 0..3600 {
        let exact = (f64::from(d) / 10.0).to_radians().sin() * 32_767.0;
        let s = sin(Angle(d));
        assert!(
            (f64::from(s) - exact).abs() <= 2.0,
            "sin({d}) = {s}, expected {exact}"
        );
        let c = cos(Angle(d));
        let exact_c = (f64::from(d) / 10.0).to_radians().cos() * 32_767.0;
        assert!(
            (f64::from(c) - exact_c).abs() <= 2.0,
            "cos({d}) = {c}, expected {exact_c}"
        );
    }
}

#[test]
fn udiv255_equals_floor_div() {
    for x in 0..=65_535u32 {
        assert_eq!(udiv255(x), x / 255, "{x}");
    }
}

/// Angular distance in decidegrees, accounting for wrap-around at 3600.
fn ang_dist(a: i32, b: i32) -> i32 {
    let d = (a - b).rem_euclid(3600);
    d.min(3600 - d)
}

proptest! {
    #[test]
    fn atan2_within_1_decideg(y in -1_000_000i32..=1_000_000, x in -1_000_000i32..=1_000_000) {
        prop_assume!(x != 0 || y != 0);
        let exact = f64::from(y).atan2(f64::from(x)).to_degrees() * 10.0;
        let exact = exact.rem_euclid(3600.0);
        let got = atan2(y, x).0;
        prop_assert!((0..3600).contains(&got));
        let d = (f64::from(got) - exact).rem_euclid(3600.0);
        let d = d.min(3600.0 - d);
        prop_assert!(d <= 1.0, "atan2({}, {}) = {}, expected {}", y, x, got, exact);
        prop_assert!(ang_dist(got, exact.round() as i32) <= 1);
    }

    #[test]
    fn isqrt64_floor_property(x in any::<u64>()) {
        let r = u128::from(isqrt64(x));
        prop_assert!(r * r <= u128::from(x));
        prop_assert!((r + 1) * (r + 1) > u128::from(x));
    }
}
