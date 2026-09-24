//! [`Interpolate`]: linear interpolation of animatable values with fixed-point progress.

use twine_core::{Angle, Color, Opa, Point, Scale, Size};

use crate::EASING_ONE;

/// A value that can be animated: linear interpolation between two values.
///
/// `t` is the eased progress in 1024ths (`0` = `a`, `1024` = `b`). It may leave `0..=1024`
/// (overshooting curves, custom curves); implementations then extrapolate and saturate to the
/// type's range instead of wrapping. The results at `t = 0` and `t = 1024` are exactly `a` and
/// `b`.
///
/// `Color` also has an inherent `Color::lerp` (per-channel, `u16` progress); call this trait's
/// version as `<Color as Interpolate>::lerp` or `Interpolate::lerp`.
///
/// ```
/// use twine_anim::Interpolate;
/// use twine_core::{Color, Point};
///
/// assert_eq!(i32::lerp(0, 100, 256), 25);
/// assert_eq!(u8::lerp(200, 250, 2048), 255); // overshoot saturates
/// assert_eq!(Point::lerp(Point::new(0, 0), Point::new(10, -10), 512), Point::new(5, -5));
/// assert_eq!(<Color as Interpolate>::lerp(Color::BLACK, Color::WHITE, 1024), Color::WHITE);
/// ```
pub trait Interpolate: Copy {
    /// The value at progress `t` (1024 = 1.0) between `a` and `b`.
    #[must_use]
    fn lerp(a: Self, b: Self, t: i32) -> Self;
}

/// `a + ((b − a) · t) >> 10` in 64 bits (the rounding of LVGL's `lv_anim_path_*` value formula,
/// so it agrees with [`Easing::value`](crate::Easing::value)).
#[inline]
fn lerp_i64(a: i32, b: i32, t: i32) -> i64 {
    match t {
        0 => i64::from(a),
        EASING_ONE => i64::from(b),
        _ => i64::from(a) + (((i64::from(b) - i64::from(a)) * i64::from(t)) >> 10),
    }
}

#[inline]
fn lerp_clamped(a: i32, b: i32, t: i32, min: i32, max: i32) -> i32 {
    lerp_i64(a, b, t).clamp(i64::from(min), i64::from(max)) as i32
}

impl Interpolate for i32 {
    fn lerp(a: i32, b: i32, t: i32) -> i32 {
        lerp_clamped(a, b, t, i32::MIN, i32::MAX)
    }
}

impl Interpolate for i16 {
    fn lerp(a: i16, b: i16, t: i32) -> i16 {
        lerp_clamped(a.into(), b.into(), t, i16::MIN.into(), i16::MAX.into()) as i16
    }
}

impl Interpolate for u8 {
    fn lerp(a: u8, b: u8, t: i32) -> u8 {
        lerp_clamped(a.into(), b.into(), t, 0, 255) as u8
    }
}

impl Interpolate for Opa {
    fn lerp(a: Opa, b: Opa, t: i32) -> Opa {
        Opa(u8::lerp(a.0, b.0, t))
    }
}

impl Interpolate for Color {
    /// Within `0..=1024` this is LVGL's `lv_color_mix(b, a, mix)` with `mix = t · 255 / 1024`
    /// (floor), i.e. [`Color::mix`]; outside, each channel is extrapolated and saturated.
    fn lerp(a: Color, b: Color, t: i32) -> Color {
        if (0..=EASING_ONE).contains(&t) {
            Color::mix(b, a, Opa(((t * 255) >> 10) as u8))
        } else {
            Color::new(
                u8::lerp(a.r, b.r, t),
                u8::lerp(a.g, b.g, t),
                u8::lerp(a.b, b.b, t),
            )
        }
    }
}

impl Interpolate for Point {
    fn lerp(a: Point, b: Point, t: i32) -> Point {
        Point::new(i32::lerp(a.x, b.x, t), i32::lerp(a.y, b.y, t))
    }
}

impl Interpolate for Size {
    fn lerp(a: Size, b: Size, t: i32) -> Size {
        Size::new(i32::lerp(a.w, b.w, t), i32::lerp(a.h, b.h, t))
    }
}

impl Interpolate for Angle {
    fn lerp(a: Angle, b: Angle, t: i32) -> Angle {
        Angle(i32::lerp(a.0, b.0, t))
    }
}

impl Interpolate for Scale {
    fn lerp(a: Scale, b: Scale, t: i32) -> Scale {
        Scale(lerp_clamped(a.0.into(), b.0.into(), t, 0, u16::MAX.into()) as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Easing;

    #[test]
    fn interp_endpoints_and_midpoints() {
        assert_eq!(i32::lerp(-10, 10, 0), -10);
        assert_eq!(i32::lerp(-10, 10, 1024), 10);
        assert_eq!(i32::lerp(-10, 10, 512), 0);
        assert_eq!(i16::lerp(i16::MIN, i16::MAX, 1024), i16::MAX);
        assert_eq!(i16::lerp(0, 30_000, 2048), i16::MAX);
        assert_eq!(Opa::lerp(Opa(0), Opa(255), 512), Opa(127));
        assert_eq!(
            Size::lerp(Size::new(0, 10), Size::new(100, 20), 256),
            Size::new(25, 12)
        );
        assert_eq!(Angle::lerp(Angle(0), Angle(3600), 512), Angle(1800));
        assert_eq!(Scale::lerp(Scale(256), Scale(512), 512), Scale(384));
        assert_eq!(Scale::lerp(Scale(256), Scale(0), 2048), Scale(0));
        assert_eq!(i32::lerp(i32::MIN, i32::MAX, 1024), i32::MAX);
        assert_eq!(i32::lerp(0, i32::MAX, 2048), i32::MAX);
    }

    #[test]
    fn interp_agrees_with_easing_value() {
        for e in [Easing::Linear, Easing::EaseInOut, Easing::Overshoot] {
            for t in (0..=1024u16).step_by(7) {
                assert_eq!(
                    i32::lerp(-300, 700, e.apply(t)),
                    e.value(t, -300, 700),
                    "{e:?} {t}"
                );
            }
        }
    }

    #[test]
    fn color_lerp_matches_mix() {
        let a = Color::hex(0x10_80_F0);
        let b = Color::hex(0xF0_20_08);
        for t in 0..=1024 {
            let want = Color::mix(b, a, Opa(((t * 255) >> 10) as u8));
            assert_eq!(<Color as Interpolate>::lerp(a, b, t), want, "t = {t}");
        }
        assert_eq!(<Color as Interpolate>::lerp(a, b, 0), a);
        assert_eq!(<Color as Interpolate>::lerp(a, b, 1024), b);
        // Overshoot extrapolates per channel and saturates.
        assert_eq!(
            <Color as Interpolate>::lerp(Color::new(0, 128, 255), Color::new(255, 128, 0), 1300),
            Color::new(255, 128, 0)
        );
        assert_eq!(
            <Color as Interpolate>::lerp(Color::new(100, 0, 0), Color::new(200, 0, 0), 1536),
            Color::new(250, 0, 0)
        );
        assert_eq!(
            <Color as Interpolate>::lerp(Color::new(100, 0, 0), Color::new(200, 0, 0), -512),
            Color::new(50, 0, 0)
        );
    }

    #[test]
    fn u8_lerp_saturates_on_overshoot() {
        assert_eq!(u8::lerp(0, 255, 1300), 255);
        assert_eq!(u8::lerp(255, 0, 1300), 0);
        assert_eq!(u8::lerp(10, 20, -2048), 0);
        assert_eq!(u8::lerp(0, 200, 1100), 214);
        assert_eq!(
            Opa::lerp(Opa(0), Opa(255), Easing::Overshoot.apply(896)),
            Opa(255)
        );
    }
}
