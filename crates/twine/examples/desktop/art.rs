//! Generated album artwork: every album gets a cover drawn from a recipe — one of eight
//! motifs, two hues and a seed (see `catalog::Album`) — so the library looks like a record
//! collection without shipping a single image.
//!
//! **Drawing.** [`cover`] is a [`canvas`] whose value is a [`Cover`]; it redraws only when the
//! cover changes (a different track starts, a row scrolls to another album). The motif is
//! drawn with filled circles, rings, polygons and gradients in integer pixels, scaled to the
//! node's size, inside a rounded-rectangle mask so every shape is clipped to the cover's
//! corners. A faint radial highlight on top gives the printed-sleeve sheen.
//!
//! Covers are cheap to draw, but a list scrolling over 40 of them redraws each visible one
//! once per frame of the scroll; at 40–140 px that is a few thousand pixels per cover.

use twine::prelude::*;
use twine::render::{GradKind, Mask, Painter};

use crate::canvas::{Canvas, canvas};
use crate::catalog::{Motif, catalog};
use crate::paint::{
    circle, circle_grad, gradient, gradient_opa, hsv, line, mix, polar, polygon, polygon_grad, ring,
    rrect_grad,
};
use crate::util::mix as hash;

/// What a cover shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cover {
    /// Nothing yet.
    #[default]
    Empty,
    /// The cover of album `id`.
    Album(u32),
    /// The cover of album `id` cropped to a circle: an artist's portrait.
    Portrait(u32),
    /// A playlist's tile, in its hue.
    Playlist(u16),
}

/// A cover view: size it like any view (covers are square; a non-square area is filled from
/// its top left with the largest square that fits).
pub fn cover<M>(c: impl IntoProp<Cover, M>) -> WidgetView<Canvas<Cover>> {
    canvas(c, paint)
}

/// The corner radius for a cover of side `s`.
fn radius(s: i32) -> i32 {
    match s {
        0..64 => 4,
        64..160 => 8,
        _ => 12,
    }
}

fn paint(c: Cover, cx: &mut DrawCx<'_, '_>) {
    let full = cx.coords();
    let s = full.width().min(full.height());
    if s < 4 {
        return;
    }
    let a = Rect::from_xywh(full.x0, full.y0, s, s);
    let p = cx.painter();
    let corner = if matches!(c, Cover::Portrait(_)) {
        s / 2
    } else {
        radius(s)
    };
    let mask = p.push_mask(Mask::Radius {
        area: a,
        radius: corner,
        outer: false,
    });
    match c {
        Cover::Empty => {}
        Cover::Album(id) | Cover::Portrait(id) => {
            let album = catalog().album(id);
            let pal = Palette {
                main: hsv(album.hue, 190, 235),
                second: hsv(album.hue2, 170, 215),
                deep: hsv(album.hue, 170, 46),
                seed: u64::from(album.seed),
            };
            match album.motif {
                Motif::Rings => rings(p, a, s, &pal),
                Motif::Sunset => sunset(p, a, s, &pal, false),
                Motif::Bands => bands(p, a, s, &pal),
                Motif::Orbits => orbits(p, a, s, &pal),
                Motif::Peaks => peaks(p, a, s, &pal),
                Motif::Waves => waves(p, a, s, &pal),
                Motif::Grid => sunset(p, a, s, &pal, true),
                Motif::Bloom => bloom(p, a, s, &pal, 5),
            }
        }
        Cover::Playlist(hue) => {
            let pal = Palette {
                main: hsv(hue, 180, 240),
                second: hsv(hue + 50, 190, 200),
                deep: hsv(hue + 20, 200, 70),
                seed: u64::from(hue),
            };
            bloom(p, a, s, &pal, 3);
        }
    }
    // The sleeve's sheen: a soft white glow from the top-left corner.
    let sheen = gradient_opa(
        GradKind::Radial {
            center: Point::new(0, 0),
            radius: s,
            focal: Point::new(0, 0),
            focal_radius: 0,
        },
        (Color::WHITE, Opa::from_raw(34)),
        (Color::WHITE, Opa::TRANSP),
    );
    p.fill_gradient(a, &sheen, Opa::COVER);
    p.pop_mask(mask);
}

/// The colors and seed of one cover.
struct Palette {
    main: Color,
    second: Color,
    deep: Color,
    seed: u64,
}

impl Palette {
    /// A value in `0..n` derived from the cover's seed and `salt`.
    fn pick(&self, salt: u64, n: i32) -> i32 {
        (hash(self.seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)) % n.max(1) as u64) as i32
    }
}

/// `v` per mille of `s`.
fn pm(s: i32, v: i32) -> i32 {
    s * v / 1000
}

/// Concentric discs around an off-centre point, alternating between the two hues.
fn rings(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette) {
    rrect_grad(p, a, 0, &gradient(GradKind::Ver, pal.deep, Color::BLACK));
    let c = Point::new(
        a.x0 + pm(s, 600 + pal.pick(1, 200)),
        a.y0 + pm(s, 300 + pal.pick(2, 200)),
    );
    for k in (1..=8).rev() {
        let col = if k % 2 == 0 {
            mix(pal.main, pal.deep, 100 * (8 - k))
        } else {
            mix(pal.second, pal.deep, 90 * (8 - k))
        };
        circle(p, c, pm(s, 95 * k), col, Opa::COVER);
    }
}

/// A striped sun over a horizon; with `grid`, a perspective grid in front (synthwave).
fn sunset(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette, grid: bool) {
    let horizon = a.y0 + pm(s, if grid { 600 } else { 700 });
    let sky_top = mix(pal.deep, Color::BLACK, 400);
    let sky_low = mix(pal.second, pal.deep, 300);
    rrect_grad(p, a, 0, &gradient(GradKind::Ver, sky_top, sky_low));
    // The sun, a vertical gradient from amber-white to the main hue.
    let c = Point::new(a.x0 + s / 2, horizon - pm(s, 60));
    let r = pm(s, if grid { 260 } else { 300 });
    circle_grad(
        p,
        c,
        r,
        &gradient(
            GradKind::Ver,
            mix(Color::hex(0xFF_E6_A8), pal.main, 250),
            pal.main,
        ),
    );
    // Stripes cut out of its lower half, thicker towards the horizon (the 80s sun).
    for i in 0..5 {
        let y = c.y + pm(r, 120 + 180 * i);
        let h = 1 + pm(s, 6 + 6 * i);
        let t = (y - a.y0) * 1000 / s;
        let band = Rect::from_xywh(a.x0, y, s, h);
        p.fill(band, mix(sky_top, sky_low, t), Opa::COVER);
    }
    // The ground.
    let ground = Rect::new(a.x0, horizon, a.x1, a.y1);
    rrect_grad(
        p,
        ground,
        0,
        &gradient(GradKind::Ver, mix(pal.deep, Color::BLACK, 300), Color::BLACK),
    );
    if grid {
        let glow = mix(pal.main, Color::WHITE, 200);
        let w = (s / 160).max(1);
        // Horizontal lines, closer together towards the horizon: y grows with k².
        for k in 1..=7 {
            let y = horizon + (a.y1 - horizon) * k * k / 49;
            line(p, Point::new(a.x0, y), Point::new(a.x1, y), w, glow, Opa::P60);
        }
        // Lines converging on the vanishing point at the horizon's centre.
        let vx = a.x0 + s / 2;
        for i in -6..=6 {
            let bottom = Point::new(vx + i * s / 5, a.y1);
            let top = Point::new(vx + i * s / 30, horizon);
            line(p, top, bottom, w, glow, Opa::P50);
        }
        p.fill(Rect::new(a.x0, horizon, a.x1, horizon + w), glow, Opa::P80);
    } else {
        // The sun's reflection on water: short shimmering dashes.
        for i in 0..6 {
            let y = horizon + pm(s, 30 + 45 * i);
            let half = pm(s, 200 - 25 * i);
            let x = a.x0 + s / 2 + pal.pick(10 + i as u64, pm(s, 40) + 1) - pm(s, 20);
            line(
                p,
                Point::new(x - half, y),
                Point::new(x + half, y),
                (s / 90).max(1),
                pal.main,
                Opa::P50,
            );
        }
    }
}

/// Bold diagonal bands across a flat field.
fn bands(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette) {
    rrect_grad(p, a, 0, &gradient(GradKind::Hor, pal.main, pal.second));
    let bw = pm(s, 150);
    for i in 0..6 {
        let off = -s + i * pm(s, 330) + pal.pick(i as u64, pm(s, 60) + 1);
        let pts = [
            Point::new(a.x0 + off, a.y1 + 1),
            Point::new(a.x0 + off + bw, a.y1 + 1),
            Point::new(a.x0 + off + bw + s, a.y0 - 1),
            Point::new(a.x0 + off + s, a.y0 - 1),
        ];
        let col = if i % 2 == 0 {
            pal.deep
        } else {
            Color::hex(0xF7_EF_E3)
        };
        polygon(p, &pts, col, if i % 2 == 0 { Opa::P80 } else { Opa::P30 });
    }
}

/// Planets on thin orbits around a glowing star.
fn orbits(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette) {
    rrect_grad(
        p,
        a,
        0,
        &gradient(GradKind::Ver, Color::hex(0x07_08_0D), pal.deep),
    );
    let c = Point::new(a.x0 + s / 2, a.y0 + s / 2);
    let glow = gradient_opa(
        GradKind::Radial {
            center: Point::new(pm(s, 300), pm(s, 300)),
            radius: pm(s, 300),
            focal: Point::new(pm(s, 300), pm(s, 300)),
            focal_radius: 0,
        },
        (pal.main, Opa::P70),
        (pal.main, Opa::TRANSP),
    );
    circle_grad(p, c, pm(s, 300), &glow);
    circle(p, c, pm(s, 70), mix(pal.main, Color::WHITE, 500), Opa::COVER);
    let w = (s / 200).max(1);
    for k in 1..=4 {
        let r = pm(s, 110 + 90 * k);
        ring(p, c, r, w, mix(pal.second, Color::WHITE, 300), Opa::P40);
        let at = polar(c, r, pal.pick(20 + k as u64, 3600));
        let col = if k % 2 == 0 { pal.main } else { pal.second };
        circle(p, at, pm(s, 22 + 9 * k), col, Opa::COVER);
    }
}

/// Three mountain ridges in aerial perspective under a pale moon.
fn peaks(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette) {
    rrect_grad(
        p,
        a,
        0,
        &gradient(GradKind::Ver, mix(pal.second, Color::WHITE, 250), pal.main),
    );
    circle(
        p,
        Point::new(a.x0 + pm(s, 250 + pal.pick(3, 500)), a.y0 + pm(s, 260)),
        pm(s, 90),
        Color::hex(0xFF_F6_E8),
        Opa::P90,
    );
    for layer in 0..3 {
        let base = a.y0 + pm(s, 520 + 160 * layer);
        let mut pts = [Point::new(0, 0); 11];
        pts[0] = Point::new(a.x0 - 1, a.y1 + 1);
        for (i, pt) in pts.iter_mut().enumerate().take(10).skip(1) {
            let x = a.x0 + (i as i32 - 1) * s / 8;
            let up = pal.pick((layer * 16 + i as i32) as u64, pm(s, 200) + 1);
            *pt = Point::new(x, base - up);
        }
        pts[10] = Point::new(a.x1 + 1, a.y1 + 1);
        let col = mix(pal.main, pal.deep, 400 + 300 * layer);
        polygon(p, &pts, col, Opa::COVER);
    }
}

/// Stacked sine waves sweeping from one hue to the other.
fn waves(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette) {
    rrect_grad(
        p,
        a,
        0,
        &gradient(GradKind::Ver, pal.deep, Color::hex(0x06_07_0A)),
    );
    let w = (s / 26).max(2);
    for k in 0..7 {
        let base = a.y0 + pm(s, 160 + 115 * k);
        let amp = pm(s, 40 + pal.pick(30 + k as u64, 50));
        let phase = pal.pick(40 + k as u64, 3600);
        let mut pts = [Point::new(0, 0); 25];
        for (i, pt) in pts.iter_mut().enumerate() {
            let x = a.x0 + (i as i32) * s / 24;
            // Two periods across the cover; `polar` on a zero centre gives r·sin(angle).
            let y = base + polar(Point::new(0, 0), amp, phase + i as i32 * 300).y;
            *pt = Point::new(x, y);
        }
        let col = mix(pal.main, pal.second, k * 160);
        p.polyline(
            &pts,
            &twine::render::LineDsc {
                color: col,
                opa: Opa::COVER,
                width: w,
                round_start: true,
                round_end: true,
                ..twine::render::LineDsc::default()
            },
        );
    }
}

/// Overlapping translucent discs, soft at their edges.
fn bloom(p: &mut Painter<'_>, a: Rect, s: i32, pal: &Palette, n: u64) {
    rrect_grad(
        p,
        a,
        0,
        &gradient(
            GradKind::Linear {
                start: Point::new(0, 0),
                end: Point::new(s, s),
            },
            pal.deep,
            Color::hex(0x08_09_0E),
        ),
    );
    for k in 0..n {
        let r = pm(s, 280 + pal.pick(50 + k, 200));
        let c = Point::new(a.x0 + pal.pick(60 + k, s), a.y0 + pal.pick(70 + k, s));
        let col = if k % 2 == 0 { pal.main } else { pal.second };
        let soft = gradient_opa(
            GradKind::Radial {
                center: Point::new(r, r),
                radius: r,
                focal: Point::new(r, r),
                focal_radius: 0,
            },
            (col, Opa::P90),
            (col, Opa::TRANSP),
        );
        circle_grad(p, c, r, &soft);
    }
    // A clean geometric accent over the haze.
    let c = Point::new(a.x0 + pm(s, 500), a.y0 + pm(s, 500));
    ring(p, c, pm(s, 230), (s / 60).max(1), Color::WHITE, Opa::P60);
    let tri = [
        polar(c, pm(s, 120), 2700),
        polar(c, pm(s, 120), 300),
        polar(c, pm(s, 120), 1500),
    ];
    polygon_grad(
        p,
        &tri,
        &gradient(GradKind::Ver, Color::WHITE, mix(pal.main, Color::WHITE, 400)),
    );
}
