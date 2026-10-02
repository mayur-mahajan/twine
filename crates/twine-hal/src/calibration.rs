//! Touch calibration: [`Calibration`], a 3-point affine mapping from raw touch-controller
//! coordinates to the display's native (unrotated) pixels.

/// A 3-point affine touch calibration.
///
/// Maps raw controller coordinates `(x, y)` to **native panel coordinates** — the pixels of the
/// display at [`Rotation::Deg0`](crate::Rotation::Deg0), whatever rotation it runs in:
///
/// ```text
/// x' = (a·x + b·y + c) / div
/// y' = (d·x + e·y + f) / div
/// ```
///
/// All math is integer (no floating point); `div` is always positive and results are rounded to
/// the nearest pixel. The coefficients are `i64` because `c`/`f` easily exceed `i32` for 12-bit
/// raw values (they are products of three coordinates).
///
/// Resistive panels (XPT2046, STMPE811) need a calibration per module: show three targets, read
/// the raw coordinates and call [`from_points`](Self::from_points) with the targets' **native**
/// positions (a target drawn at a logical point `p` of a rotated display is at
/// `TouchTransform::for_display(&info).inverse().apply(p)`). The drivers map the calibrated
/// native point to the logical screen with the display's
/// [`TouchTransform`](crate::TouchTransform) when they are fitted to it, so one calibration
/// stays valid in every rotation, also when the display is rotated at run time.
///
/// ```
/// use twine_hal::Calibration;
///
/// // Raw 12-bit readings of three targets and where the targets were drawn.
/// let raw = [(400, 3600), (3700, 2000), (2000, 400)];
/// let screen = [(32, 24), (288, 120), (160, 216)];
/// let cal = Calibration::from_points(raw, screen).expect("points are not collinear");
/// for (r, s) in raw.iter().zip(screen) {
///     assert_eq!(cal.apply(r.0, r.1), s);
/// }
/// assert!(Calibration::from_points([(0, 0), (1, 1), (2, 2)], screen).is_none());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Calibration {
    /// `x` coefficient of `x'`.
    pub a: i64,
    /// `y` coefficient of `x'`.
    pub b: i64,
    /// Constant term of `x'`.
    pub c: i64,
    /// `x` coefficient of `y'`.
    pub d: i64,
    /// `y` coefficient of `y'`.
    pub e: i64,
    /// Constant term of `y'`.
    pub f: i64,
    /// Common divisor (> 0).
    pub div: i64,
}

impl Calibration {
    /// The identity mapping (raw coordinates are passed through), e.g. for a calibration screen
    /// that needs raw values.
    pub const IDENTITY: Self = Self {
        a: 1,
        b: 0,
        c: 0,
        d: 0,
        e: 1,
        f: 0,
        div: 1,
    };

    /// A typical calibration of the common 2.8" ILI9341 + XPT2046 module, in the panel's
    /// native 240 × 320 portrait coordinates: raw X (≈ 3900…200) runs along the native x
    /// axis (reversed), raw Y (≈ 200…3900) along the native y axis. Valid in every rotation
    /// (landscape `Rotation::Deg90` is the usual one).
    ///
    /// Every module differs by tens of pixels (and some mount the touch film mirrored);
    /// calibrate each device with [`from_points`](Self::from_points) for accurate touch.
    ///
    /// A constant: [`apply`](Self::apply) with it never panics (its coefficients cannot
    /// overflow the `i64` arithmetic for any `i32` raw point).
    ///
    /// ```
    /// use twine_hal::Calibration;
    /// let c = Calibration::DEFAULT_240X320;
    /// assert_eq!(c.apply(3900, 200), (0, 0)); // the native top-left corner
    /// assert_eq!(c.apply(200, 3900), (240, 320));
    /// ```
    pub const DEFAULT_240X320: Self = Self {
        a: -240,
        b: 0,
        c: 240 * 3900,
        d: 0,
        e: 320,
        f: -320 * 200,
        div: 3700,
    };

    /// Solves the affine mapping that sends the three `raw` points to the three `screen` points
    /// (the classic 3-point solution, e.g. TI application report SLYT277).
    ///
    /// Returns `None` if the raw points are collinear (degenerate).
    #[must_use]
    pub fn from_points(raw: [(i32, i32); 3], screen: [(i32, i32); 3]) -> Option<Self> {
        let [(x0, y0), (x1, y1), (x2, y2)] = raw.map(|(x, y)| (i64::from(x), i64::from(y)));
        let [(sx0, sy0), (sx1, sy1), (sx2, sy2)] = screen.map(|(x, y)| (i64::from(x), i64::from(y)));

        let div = (x0 - x2) * (y1 - y2) - (x1 - x2) * (y0 - y2);
        if div == 0 {
            return None;
        }
        let a = (sx0 - sx2) * (y1 - y2) - (sx1 - sx2) * (y0 - y2);
        let b = (x0 - x2) * (sx1 - sx2) - (sx0 - sx2) * (x1 - x2);
        let c = y0 * (x2 * sx1 - x1 * sx2) + y1 * (x0 * sx2 - x2 * sx0) + y2 * (x1 * sx0 - x0 * sx1);
        let d = (sy0 - sy2) * (y1 - y2) - (sy1 - sy2) * (y0 - y2);
        let e = (x0 - x2) * (sy1 - sy2) - (sy0 - sy2) * (x1 - x2);
        let f = y0 * (x2 * sy1 - x1 * sy2) + y1 * (x0 * sy2 - x2 * sy0) + y2 * (x1 * sy0 - x0 * sy1);

        let s = div.signum();
        Some(Self {
            a: a * s,
            b: b * s,
            c: c * s,
            d: d * s,
            e: e * s,
            f: f * s,
            div: div * s,
        })
    }

    /// Maps a raw point to screen coordinates (rounded to the nearest pixel).
    ///
    /// A calibration with `div <= 0` (not produced by this type's constructors) maps everything
    /// to `(0, 0)`.
    #[must_use]
    pub fn apply(&self, x: i32, y: i32) -> (i32, i32) {
        if self.div <= 0 {
            return (0, 0);
        }
        let (x, y) = (i64::from(x), i64::from(y));
        let nx = self.a * x + self.b * y + self.c;
        let ny = self.d * x + self.e * y + self.f;
        (round_div(nx, self.div), round_div(ny, self.div))
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// `n / d` rounded to nearest (halves away from zero), saturated to `i32`; `d > 0`.
fn round_div(n: i64, d: i64) -> i32 {
    let half = d / 2;
    let q = if n >= 0 { (n + half) / d } else { (n - half) / d };
    q.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_default() {
        assert_eq!(Calibration::default(), Calibration::IDENTITY);
        assert_eq!(Calibration::IDENTITY.apply(123, -7), (123, -7));
    }

    #[test]
    fn default_maps_native_corners() {
        let c = Calibration::DEFAULT_240X320;
        assert_eq!(c.apply(3900, 200), (0, 0));
        assert_eq!(c.apply(200, 3900), (240, 320));
        // In landscape (Deg90) the raw corners land on the logical corners, as on the module.
        let t = crate::TouchTransform::for_rotation(crate::Rotation::Deg90, 240, 320);
        let (x, y) = c.apply(3900, 3900);
        assert_eq!(t.apply(x, y), twine_core::Point::new(0, 0));
        let (x, y) = c.apply(200, 200);
        assert_eq!(t.apply(x, y), twine_core::Point::new(319, 239));
    }

    #[test]
    fn negative_div_is_normalized() {
        // Swapping two points flips the sign of the determinant.
        let raw = [(3700, 2000), (400, 3600), (2000, 400)];
        let screen = [(288, 120), (32, 24), (160, 216)];
        let c = Calibration::from_points(raw, screen).unwrap();
        assert!(c.div > 0);
        assert_eq!(c.apply(400, 3600), (32, 24));
    }

    #[test]
    fn rounding() {
        assert_eq!(round_div(5, 2), 3);
        assert_eq!(round_div(-5, 2), -3);
        assert_eq!(round_div(4, 3), 1);
        assert_eq!(round_div(i64::MAX, 1), i32::MAX);
    }

    #[test]
    fn invalid_div_maps_to_origin() {
        let c = Calibration {
            div: 0,
            ..Calibration::IDENTITY
        };
        assert_eq!(c.apply(5, 5), (0, 0));
    }
}
