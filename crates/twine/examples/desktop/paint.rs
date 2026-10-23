//! Drawing helpers for the canvases: integer HSV colors, shapes and gradients on top of the
//! renderer's [`Painter`].
//!
//! Twine's renderer works in integer pixels and fixed-point angles (no floating point while
//! drawing), so these helpers do too: hues are degrees, saturation and value 0–255, and
//! [`polar`] uses the library's `sin`/`cos`, which return the sine scaled by 2¹⁵.

use twine::core::math::{TRIG_SHIFT, cos, sin};
use twine::prelude::*;
use twine::render::{ArcDsc, GradKind, GradStop, LineDsc, Painter, PolygonDsc, RADIUS_CIRCLE, RectDsc};

/// The color of hue `h` (degrees), saturation `s` and value `v` (0–255).
pub fn hsv(h: u16, s: u8, v: u8) -> Color {
    let h = u32::from(h % 360);
    let (s, v) = (u32::from(s), u32::from(v));
    let sector = h / 60;
    let f = (h % 60) * 255 / 60; // position inside the sector, 0–255
    let p = v * (255 - s) / 255;
    let q = v * (255 - s * f / 255) / 255;
    let t = v * (255 - s * (255 - f) / 255) / 255;
    let (r, g, b) = match sector {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::new(r as u8, g as u8, b as u8)
}

/// `a` → `b` at `t` per mille.
pub fn mix(a: Color, b: Color, t: i32) -> Color {
    Color::lerp(a, b, (t.clamp(0, 1000) * 1024 / 1000) as u16)
}

/// The point `r` px from `c` at `deci_deg` tenths of a degree (clockwise from 3 o'clock).
pub fn polar(c: Point, r: i32, deci_deg: i32) -> Point {
    let a = Angle::deci_deg(deci_deg);
    Point::new(
        c.x + ((r * cos(a)) >> TRIG_SHIFT),
        c.y + ((r * sin(a)) >> TRIG_SHIFT),
    )
}

/// A two-stop gradient.
pub fn gradient(kind: GradKind, from: Color, to: Color) -> Gradient {
    Gradient::new(
        kind,
        &[
            GradStop::new(from, Fraction::ZERO),
            GradStop::new(to, Fraction::ONE),
        ],
    )
}

/// A two-stop gradient whose ends have their own opacity.
pub fn gradient_opa(kind: GradKind, from: (Color, Opa), to: (Color, Opa)) -> Gradient {
    Gradient::new(
        kind,
        &[
            GradStop::with_opa(from.0, from.1, Fraction::ZERO),
            GradStop::with_opa(to.0, to.1, Fraction::ONE),
        ],
    )
}

/// A filled rectangle with rounded corners.
pub fn rrect(p: &mut Painter<'_>, area: Rect, radius: i32, color: Color, opa: Opa) {
    p.rect(
        area,
        &RectDsc {
            radius,
            bg_color: color,
            bg_opa: opa,
            ..RectDsc::default()
        },
    );
}

/// A rectangle with rounded corners filled with `grad`.
pub fn rrect_grad(p: &mut Painter<'_>, area: Rect, radius: i32, grad: &Gradient) {
    p.rect(
        area,
        &RectDsc {
            radius,
            bg_opa: Opa::COVER,
            bg_grad: Some(grad),
            ..RectDsc::default()
        },
    );
}

/// The square around a circle.
pub fn circle_area(c: Point, r: i32) -> Rect {
    Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)
}

/// A filled circle.
pub fn circle(p: &mut Painter<'_>, c: Point, r: i32, color: Color, opa: Opa) {
    if r > 0 {
        rrect(p, circle_area(c, r), RADIUS_CIRCLE, color, opa);
    }
}

/// A circle filled with `grad` (geometry relative to its bounding square).
pub fn circle_grad(p: &mut Painter<'_>, c: Point, r: i32, grad: &Gradient) {
    if r > 0 {
        rrect_grad(p, circle_area(c, r), RADIUS_CIRCLE, grad);
    }
}

/// A ring of `width` px whose outer edge has radius `r` (a full circle outline).
pub fn ring(p: &mut Painter<'_>, c: Point, r: i32, width: i32, color: Color, opa: Opa) {
    if r > 0 && width > 0 {
        let dsc = ArcDsc {
            color,
            opa,
            width,
            ..ArcDsc::default()
        };
        p.arc(c, r, Angle::deg(0), Angle::deg(360), &dsc);
    }
}

/// A straight line with round caps.
pub fn line(p: &mut Painter<'_>, a: Point, b: Point, width: i32, color: Color, opa: Opa) {
    p.line(
        a,
        b,
        &LineDsc {
            color,
            opa,
            width,
            round_start: true,
            round_end: true,
            ..LineDsc::default()
        },
    );
}

/// A filled polygon.
pub fn polygon(p: &mut Painter<'_>, pts: &[Point], color: Color, opa: Opa) {
    p.polygon(
        pts,
        &PolygonDsc {
            color,
            opa,
            grad: None,
        },
    );
}

/// A polygon filled with `grad` (over its bounding box).
pub fn polygon_grad(p: &mut Painter<'_>, pts: &[Point], grad: &Gradient) {
    p.polygon(
        pts,
        &PolygonDsc {
            color: Color::BLACK,
            opa: Opa::COVER,
            grad: Some(grad),
        },
    );
}
