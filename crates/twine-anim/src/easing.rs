//! Easing curves: integer ports of LVGL's animation path functions (verified against LVGL v9.6.0
//! `src/misc/lv_anim.c` and `src/misc/lv_math.c`; line numbers below refer to v9.6.0's
//! `lv_anim.c`).

use twine_core::math::{CUBIC_BEZIER_ONE, cubic_bezier};

/// 1.0 in easing progress units (LVGL `LV_ANIM_RESOLUTION` / `LV_BEZIER_VAL_MAX`).
pub const EASING_ONE: i32 = CUBIC_BEZIER_ONE;

/// `(x1, y1, x2, y2)` of LVGL's `lv_anim_path_ease_in` (lines 303–308): `LV_BEZIER_VAL_FLOAT(0.42), 0, 1, 1`
/// (`LV_BEZIER_VAL_FLOAT` truncates: 0.42 × 1024 = 430).
const EASE_IN: (i32, i32, i32, i32) = (430, 0, 1024, 1024);
/// LVGL `lv_anim_path_ease_out` (lines 310–315): `0, 0, LV_BEZIER_VAL_FLOAT(0.58), 1` (0.58 × 1024 → 593).
const EASE_OUT: (i32, i32, i32, i32) = (0, 0, 593, 1024);
/// LVGL `lv_anim_path_ease_in_out` (lines 317–322): `0.42, 0, 0.58, 1`.
const EASE_IN_OUT: (i32, i32, i32, i32) = (430, 0, 593, 1024);
/// LVGL `lv_anim_path_overshoot` (lines 324–328): `cubic_bezier(341, 0, 683, 1300)`.
const OVERSHOOT: (i32, i32, i32, i32) = (341, 0, 683, 1300);

/// Maps animation progress to eased progress (the LVGL `lv_anim_path_*` functions).
///
/// Progress is fixed point with [`EASING_ONE`] (1024) = 1.0. Every built-in curve is an exact
/// integer port of the LVGL v9 function of the same name: for an animation from `0` to `1024`,
/// [`Easing::apply`] returns bit-for-bit what LVGL's path callback returns, and
/// [`Easing::value`] reproduces LVGL for any start/end values.
///
/// | Variant | LVGL |
/// |---------|------|
/// | `Linear` | `lv_anim_path_linear` |
/// | `EaseIn` | `lv_anim_path_ease_in` (cubic-bezier 0.42, 0, 1, 1) |
/// | `EaseOut` | `lv_anim_path_ease_out` (cubic-bezier 0, 0, 0.58, 1) |
/// | `EaseInOut` | `lv_anim_path_ease_in_out` (cubic-bezier 0.42, 0, 0.58, 1) |
/// | `Overshoot` | `lv_anim_path_overshoot` |
/// | `Bounce` | `lv_anim_path_bounce` |
/// | `Step` | `lv_anim_path_step` |
/// | `CubicBezier` | `lv_anim_path_custom_bezier3` (`lv_cubic_bezier`) |
/// | `Custom` | a user path callback |
///
/// ```
/// use twine_anim::Easing;
///
/// assert_eq!(Easing::Linear.apply(512), 512);
/// assert_eq!(Easing::EaseInOut.apply(512), 512);
/// assert!(Easing::EaseIn.apply(256) < 256);
/// assert!(Easing::Overshoot.apply(896) > 1024); // overshoots before settling
/// assert_eq!(Easing::Step.apply(1023), 0);
/// assert_eq!(Easing::EaseOut.value(1024, 10, 90), 90);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq)]
pub enum Easing {
    /// Constant speed.
    #[default]
    Linear,
    /// Slow start.
    EaseIn,
    /// Slow end.
    EaseOut,
    /// Slow start and end.
    EaseInOut,
    /// Runs past the end value and comes back.
    Overshoot,
    /// Reaches the end value and bounces back twice.
    Bounce,
    /// Stays at the start value and jumps to the end at the very end.
    Step,
    /// CSS `cubic-bezier(x1, y1, x2, y2)` with control points scaled by 1024 (`x1`/`x2` must be
    /// in `0..=1024`; otherwise a warning is logged and the curve returns 0, as LVGL).
    CubicBezier(i16, i16, i16, i16),
    /// A user curve: progress `0..=1024` → eased progress (nominally `0..=1024`, may overshoot).
    Custom(fn(u16) -> i32),
}

impl PartialEq for Easing {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Easing::Linear, Easing::Linear)
            | (Easing::EaseIn, Easing::EaseIn)
            | (Easing::EaseOut, Easing::EaseOut)
            | (Easing::EaseInOut, Easing::EaseInOut)
            | (Easing::Overshoot, Easing::Overshoot)
            | (Easing::Bounce, Easing::Bounce)
            | (Easing::Step, Easing::Step) => true,
            (Easing::CubicBezier(a, b, c, d), Easing::CubicBezier(e, f, g, h)) => {
                (a, b, c, d) == (e, f, g, h)
            }
            (Easing::Custom(f), Easing::Custom(g)) => core::ptr::fn_addr_eq(f, g),
            _ => false,
        }
    }
}

impl core::hash::Hash for Easing {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match *self {
            Easing::CubicBezier(a, b, c, d) => (a, b, c, d).hash(state),
            // Consistent with `PartialEq`, which compares function addresses.
            Easing::Custom(f) => (f as usize).hash(state),
            _ => {}
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Easing {
    fn format(&self, f: defmt::Formatter<'_>) {
        match *self {
            Easing::Linear => defmt::write!(f, "Linear"),
            Easing::EaseIn => defmt::write!(f, "EaseIn"),
            Easing::EaseOut => defmt::write!(f, "EaseOut"),
            Easing::EaseInOut => defmt::write!(f, "EaseInOut"),
            Easing::Overshoot => defmt::write!(f, "Overshoot"),
            Easing::Bounce => defmt::write!(f, "Bounce"),
            Easing::Step => defmt::write!(f, "Step"),
            Easing::CubicBezier(a, b, c, d) => defmt::write!(f, "CubicBezier({}, {}, {}, {})", a, b, c, d),
            Easing::Custom(_) => defmt::write!(f, "Custom"),
        }
    }
}

/// LVGL `lv_anim_path_bounce` pieces (lines 330–381, segment table at 339–369): the bezier input `t` and the divisor of the value range
/// for progress `t` (`0..=1024`).
const fn bounce_segment(t: i32) -> (i32, i32) {
    // 3 bounces have 5 parts: 3 down and 2 up.
    let (t, div) = if t < 408 {
        (EASING_ONE - ((t * 2500) >> 10), 1) // go down
    } else if t < 614 {
        ((t - 408) * 5, 20) // first bounce back
    } else if t < 819 {
        (EASING_ONE - (t - 614) * 5, 20) // fall back
    } else if t < 921 {
        ((t - 819) * 10, 40) // second bounce back
    } else {
        (EASING_ONE - (t - 921) * 10, 40) // fall back
    };
    let t = if t > EASING_ONE {
        EASING_ONE
    } else if t < 0 {
        0
    } else {
        t
    };
    (t, div)
}

/// LVGL `lv_bezier3(t, 0, 500, 800, 1024)` as used by the bounce path (LVGL v9 maps it to
/// `lv_cubic_bezier(t, 341, 500, 683, 800)`).
fn bounce_step(t: i32) -> i32 {
    cubic_bezier(t, 341, 500, 683, 800)
}

impl Easing {
    /// Eased progress for progress `t` (`0..=1024`; larger values are treated as 1024).
    ///
    /// The result is nominally `0..=1024`; [`Easing::Overshoot`], custom curves and
    /// out-of-range bezier control points may leave that range. Identical to LVGL's path
    /// callback for an animation from 0 to 1024.
    #[must_use]
    pub fn apply(self, t: u16) -> i32 {
        let t = i32::from(t).min(EASING_ONE);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => bezier(t, EASE_IN),
            Easing::EaseOut => bezier(t, EASE_OUT),
            Easing::EaseInOut => bezier(t, EASE_IN_OUT),
            Easing::Overshoot => bezier(t, OVERSHOOT),
            Easing::Bounce => {
                let (bt, div) = bounce_segment(t);
                EASING_ONE - ((bounce_step(bt) * (EASING_ONE / div)) >> 10)
            }
            Easing::Step => {
                if t >= EASING_ONE {
                    EASING_ONE
                } else {
                    0
                }
            }
            Easing::CubicBezier(x1, y1, x2, y2) => {
                cubic_bezier(t, i32::from(x1), i32::from(y1), i32::from(x2), i32::from(y2))
            }
            Easing::Custom(f) => f(t as u16),
        }
    }

    /// The animated value at progress `t` (`0..=1024`) of an animation from `start` to `end`,
    /// computed exactly like LVGL's path callbacks (`start + (eased · (end − start)) >> 10`, with
    /// `Bounce` scaling the value range as `lv_anim_path_bounce` does). Uses 64-bit
    /// intermediates, so it never overflows; the result saturates to `i32`.
    #[must_use]
    pub fn value(self, t: u16, start: i32, end: i32) -> i32 {
        let diff = i64::from(end) - i64::from(start);
        let v = match self {
            Easing::Bounce => {
                let (bt, div) = bounce_segment(i32::from(t).min(EASING_ONE));
                // C integer division truncates toward zero, as Rust's `/`.
                i64::from(end) - ((i64::from(bounce_step(bt)) * (diff / i64::from(div))) >> 10)
            }
            _ => i64::from(start) + ((i64::from(self.apply(t)) * diff) >> 10),
        };
        v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }

    /// Whether the curve stays within `0..=1024` and rises from 0 to 1024 without bouncing (every
    /// built-in curve except `Overshoot` and `Bounce`; bezier curves with all control points in
    /// `0..=1024`). As in LVGL, the bezier solver's ±1 tolerance can make neighbouring samples
    /// step back by up to 2/1024. Custom curves report `false`.
    #[must_use]
    pub const fn is_monotonic(self) -> bool {
        match self {
            Easing::Linear | Easing::EaseIn | Easing::EaseOut | Easing::EaseInOut | Easing::Step => true,
            Easing::CubicBezier(x1, y1, x2, y2) => {
                x1 >= 0
                    && x1 <= 1024
                    && x2 >= 0
                    && x2 <= 1024
                    && y1 >= 0
                    && y1 <= 1024
                    && y2 >= 0
                    && y2 <= 1024
            }
            Easing::Overshoot | Easing::Bounce | Easing::Custom(_) => false,
        }
    }
}

fn bezier(t: i32, (x1, y1, x2, y2): (i32, i32, i32, i32)) -> i32 {
    cubic_bezier(t, x1, y1, x2, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Progress samples of the reference tables below.
    const SAMPLES: [u16; 17] = [
        0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 832, 896, 960, 1024,
    ];

    /// Reference values computed by compiling LVGL v9.6.0's C sources (`lv_anim_path_*` from
    /// `src/misc/lv_anim.c`, `lv_cubic_bezier`, `lv_bezier3` and `lv_map` from
    /// `src/misc/lv_math.c`, `CUBIC_PRECISION_BITS = 10`) and running each path callback for an
    /// animation with `duration = 1024`, `act_time = t`, from `start` to `end`.
    const LVGL_0_1024: [(Easing, [i32; 17]); 8] = [
        (
            Easing::Linear,
            [
                0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 832, 896, 960, 1024,
            ],
        ),
        (
            Easing::EaseIn,
            [
                0, 7, 26, 57, 96, 143, 196, 256, 322, 394, 469, 552, 636, 727, 821, 919, 1024,
            ],
        ),
        (
            Easing::EaseOut,
            [
                0, 107, 204, 298, 389, 474, 556, 632, 702, 769, 828, 882, 928, 967, 997, 1016, 1024,
            ],
        ),
        (
            Easing::EaseInOut,
            [
                0, 7, 32, 73, 133, 210, 301, 404, 512, 622, 725, 816, 893, 951, 992, 1015, 1024,
            ],
        ),
        (
            Easing::Overshoot,
            [
                0, 14, 55, 118, 198, 292, 396, 505, 615, 721, 820, 908, 980, 1031, 1058, 1056, 1024,
            ],
        ),
        (
            Easing::Bounce,
            [
                0, 109, 230, 368, 524, 701, 901, 1011, 994, 980, 978, 991, 1008, 1020, 1004, 1007, 1024,
            ],
        ),
        (
            Easing::Step,
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1024],
        ),
        // CSS `ease` through `lv_anim_path_custom_bezier3`.
        (
            Easing::CubicBezier(256, 102, 256, 1024),
            [
                0, 46, 141, 272, 418, 552, 663, 752, 822, 878, 922, 957, 982, 1001, 1013, 1021, 1024,
            ],
        ),
    ];

    /// Same source and method, animation from -300 to 700.
    const LVGL_M300_700: [(Easing, [i32; 17]); 8] = [
        (
            Easing::Linear,
            [
                -300, -238, -175, -113, -50, 12, 75, 137, 200, 262, 325, 387, 450, 512, 575, 637, 700,
            ],
        ),
        (
            Easing::EaseIn,
            [
                -300, -294, -275, -245, -207, -161, -109, -50, 14, 84, 158, 239, 321, 409, 501, 597, 700,
            ],
        ),
        (
            Easing::EaseOut,
            [
                -300, -196, -101, -9, 79, 162, 242, 317, 385, 450, 508, 561, 606, 644, 673, 692, 700,
            ],
        ),
        (
            Easing::EaseInOut,
            [
                -300, -294, -269, -229, -171, -95, -7, 94, 200, 307, 408, 496, 572, 628, 668, 691, 700,
            ],
        ),
        (
            Easing::Overshoot,
            [
                -300, -287, -247, -185, -107, -15, 86, 193, 300, 404, 500, 586, 657, 706, 733, 731, 700,
            ],
        ),
        (
            Easing::Bounce,
            [
                -300, -193, -75, 60, 212, 385, 580, 687, 670, 657, 655, 668, 684, 696, 680, 683, 700,
            ],
        ),
        (
            Easing::Step,
            [
                -300, -300, -300, -300, -300, -300, -300, -300, -300, -300, -300, -300, -300, -300, -300,
                -300, 700,
            ],
        ),
        (
            Easing::CubicBezier(256, 102, 256, 1024),
            [
                -300, -256, -163, -35, 108, 239, 347, 434, 502, 557, 600, 634, 658, 677, 689, 697, 700,
            ],
        ),
    ];

    const ALL: [Easing; 7] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::Overshoot,
        Easing::Bounce,
        Easing::Step,
    ];

    #[test]
    fn easing_endpoints() {
        for e in ALL {
            assert_eq!(e.apply(0), 0, "{e:?}");
            assert_eq!(e.apply(1024), 1024, "{e:?}");
            assert_eq!(e.apply(u16::MAX), 1024, "{e:?} clamps progress");
            assert_eq!(e.value(0, -5, 77), -5, "{e:?}");
            assert_eq!(e.value(1024, -5, 77), 77, "{e:?}");
        }
    }

    #[test]
    fn easing_matches_lvgl_table() {
        for (e, table) in LVGL_0_1024 {
            for (t, want) in SAMPLES.into_iter().zip(table) {
                assert_eq!(e.apply(t), want, "{e:?} apply({t})");
                assert_eq!(e.value(t, 0, 1024), want, "{e:?} value({t})");
            }
        }
        for (e, table) in LVGL_M300_700 {
            for (t, want) in SAMPLES.into_iter().zip(table) {
                assert_eq!(e.value(t, -300, 700), want, "{e:?} value({t}, -300, 700)");
            }
        }
    }

    /// LVGL's bezier solver stops once `|x(t) − x| ≤ 1`, so its curves (and therefore these
    /// bit-exact ports) may step back by up to 2/1024 between neighbouring samples; the running
    /// maximum never drops by more than that.
    #[test]
    fn easing_monotonic_for_non_overshoot() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::Step,
        ] {
            assert!(e.is_monotonic());
            let mut max = e.apply(0);
            for t in 1..=1024u16 {
                let v = e.apply(t);
                assert!(v >= max - 2, "{e:?} decreases at {t}: {max} -> {v}");
                assert!((0..=1024).contains(&v), "{e:?} out of range at {t}: {v}");
                max = max.max(v);
            }
            // Coarser steps (≥ 16/1024) are strictly non-decreasing.
            for t in (16..=1024u16).step_by(16) {
                assert!(e.apply(t) >= e.apply(t - 16), "{e:?} at {t}");
            }
        }
        assert!(!Easing::Overshoot.is_monotonic());
        assert!(!Easing::Bounce.is_monotonic());
    }

    #[test]
    fn custom_and_invalid_bezier() {
        fn half(t: u16) -> i32 {
            i32::from(t) / 2
        }
        assert_eq!(Easing::Custom(half).apply(1000), 500);
        assert_eq!(Easing::Custom(half).value(1024, 0, 100), 50);
        // x1 out of range: LVGL returns 0.
        assert_eq!(Easing::CubicBezier(-1, 0, 1024, 1024).apply(500), 0);
        assert!(!Easing::CubicBezier(0, -10, 1024, 1024).is_monotonic());
    }

    #[test]
    fn value_never_overflows() {
        for e in ALL {
            assert_eq!(e.value(0, i32::MIN, i32::MAX), i32::MIN, "{e:?}");
            assert_eq!(e.value(1024, i32::MIN, i32::MAX), i32::MAX, "{e:?}");
            assert_eq!(e.value(1024, i32::MAX, i32::MIN), i32::MIN, "{e:?}");
            // Overshoot leaves the range and saturates instead of wrapping.
            assert_eq!(Easing::Overshoot.value(900, 0, i32::MAX), i32::MAX);
        }
    }

    #[test]
    fn equality() {
        fn a(t: u16) -> i32 {
            i32::from(t)
        }
        assert_eq!(Easing::Custom(a), Easing::Custom(a));
        assert_ne!(Easing::Linear, Easing::Step);
        assert_eq!(Easing::CubicBezier(1, 2, 3, 4), Easing::CubicBezier(1, 2, 3, 4));
        assert_ne!(Easing::CubicBezier(1, 2, 3, 4), Easing::CubicBezier(1, 2, 3, 5));
        assert_eq!(Easing::default(), Easing::Linear);
    }
    #[test]
    fn easings_endpoints() {
        for e in ALL.into_iter().chain([Easing::CubicBezier(256, 102, 256, 1024)]) {
            assert_eq!(e.apply(0), 0, "{e:?}");
            assert_eq!(e.apply(1024), 1024, "{e:?}");
        }
    }

    #[test]
    fn linear_identity() {
        for t in 0..=1024u16 {
            assert_eq!(Easing::Linear.apply(t), i32::from(t));
        }
    }

    /// Monotonic within LVGL's bezier-solver tolerance (see `easing_monotonic_for_non_overshoot`).
    #[test]
    fn ease_in_out_monotonic() {
        let mut max = 0;
        for t in 0..=1024u16 {
            let v = Easing::EaseInOut.apply(t);
            assert!(v >= max - 2, "decreases at {t}: {max} -> {v}");
            max = max.max(v);
        }
    }

    /// Floating-point cubic-bezier solver (bisection on x), the reference for the integer port.
    fn reference_bezier(x: f64, (x1, y1, x2, y2): (f64, f64, f64, f64)) -> f64 {
        let b = |s: f64, p1: f64, p2: f64| {
            let r = 1.0 - s;
            3.0 * r * r * s * p1 + 3.0 * r * s * s * p2 + s * s * s
        };
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..100 {
            let mid = lo.midpoint(hi);
            if b(mid, x1, x2) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        b(lo.midpoint(hi), y1, y2)
    }

    /// The port agrees with an exact floating-point solver. LVGL's Newton solver stops once
    /// `|x(t) − x| ≤ 1/1024`, so the reference is the range of `y` over `x ± 1/1024`; the
    /// integer result (whose fixed-point evaluation truncates) must lie within ±3/1024 of that
    /// range. Bit-exactness with LVGL itself is checked by `easing_matches_lvgl_table`.
    #[test]
    fn bezier_matches_reference() {
        let curves = [
            (Easing::EaseIn, EASE_IN),
            (Easing::EaseOut, EASE_OUT),
            (Easing::EaseInOut, EASE_IN_OUT),
            (Easing::Overshoot, OVERSHOOT),
            (Easing::CubicBezier(256, 102, 256, 1024), (256, 102, 256, 1024)),
            (Easing::CubicBezier(0, 1024, 1024, 0), (0, 1024, 1024, 0)),
            (Easing::CubicBezier(700, 100, 300, 900), (700, 100, 300, 900)),
        ];
        let mut worst = 0.0f64;
        for (e, (x1, y1, x2, y2)) in curves {
            let pts = (
                f64::from(x1) / 1024.0,
                f64::from(y1) / 1024.0,
                f64::from(x2) / 1024.0,
                f64::from(y2) / 1024.0,
            );
            let reference = |x: f64| reference_bezier(x.clamp(0.0, 1024.0) / 1024.0, pts) * 1024.0;
            for i in 0..=64u16 {
                let t = i * 16;
                let x = f64::from(t);
                let (a, b) = (reference(x - 1.0), reference(x + 1.0));
                let (lo, hi) = (a.min(b).min(reference(x)), a.max(b).max(reference(x)));
                let got = f64::from(e.apply(t));
                let err = (lo - got).max(got - hi).max(0.0);
                worst = worst.max(err);
                assert!(err <= 3.0, "{e:?} at {t}: {got} vs reference {lo:.2}..={hi:.2}");
            }
        }
        assert!(worst > 0.0 && worst <= 3.0);
    }

    #[test]
    fn overshoot_exceeds_1024_mid_curve() {
        let peak = (0..=1024u16).map(|t| Easing::Overshoot.apply(t)).max().unwrap();
        assert!(peak > 1050, "peak {peak}");
        let over = (0..=1024u16)
            .filter(|&t| Easing::Overshoot.apply(t) > 1024)
            .collect::<alloc::vec::Vec<_>>();
        assert!(!over.is_empty() && *over.last().unwrap() < 1024);
        assert!(over[0] > 512, "overshoot starts late in the curve: {}", over[0]);
    }

    #[test]
    fn bounce_hits_1024_multiple_times() {
        // Count separate runs of progress values where the curve touches 1024.
        let mut runs = 0;
        let mut prev = false;
        for t in 0..=1024u16 {
            let hit = Easing::Bounce.apply(t) == 1024;
            if hit && !prev {
                runs += 1;
            }
            prev = hit;
        }
        assert!(runs >= 3, "{runs} touches");
        assert!((0..=1024u16).all(|t| (0..=1024).contains(&Easing::Bounce.apply(t))));
    }

    #[test]
    fn step_is_step() {
        assert!((0..1024u16).all(|t| Easing::Step.apply(t) == 0));
        assert_eq!(Easing::Step.apply(1024), 1024);
        assert_eq!(Easing::Step.value(1023, 5, 9), 5);
    }
}
