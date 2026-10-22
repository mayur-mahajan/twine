//! No division by zero through the public API (R5.S10, coding guideline G3): zero (and
//! degenerate) sizes, ranges, scales, durations and periods that an application can pass never
//! reach a division — each API returns its documented fallback, and nothing panics.
//!
//! - **Core arithmetic:** `Duration::div`, `Duration::fraction_1024`, `Fx::from_ratio`,
//!   `Fx::div`, `Scale::inverse_apply`, `math::map`, `XorShift32::range`, `Rect::round_out`,
//!   `Transform::invert` and `Anim::speed` / zero durations.
//! - **Rendering:** zero-size and tiled images, gradients of every kind over zero-size areas and
//!   with degenerate geometry, dashed lines with zero dashes or gaps.
//! - **Widgets:** empty and reversed ranges (bar, slider, arc, spinbox), zero widget sizes,
//!   empty option lists (roller, dropdown), zero-size tile and tab views, a zero-size image and
//!   a zero image scale — laid out, drawn and pressed.
//!
//! The sites themselves are proven in the source (`// DIV:` proofs, `NonZero*` divisors and
//! guards, audited by `cargo xtask divisions`); these tests check the fallbacks end to end.
//!
//! req: SAF-033

use proptest::prelude::*;
use twine::core::math::map;
use twine::core::{
    Angle, Color, ColorFormat, Duration, Fraction, Fx, Opa, Point, Rect, Scale, Transform, XorShift32,
};
use twine::engine::{Engine, NodeId};
use twine::image::{Image, ImageHeader, ImageSource};
use twine::render::{GradKind, GradStop, Gradient, ImageDsc, ImagePixels, LineDsc, RectDsc};
use twine::style::Side;
use twine::widgets::image::ImageAlign;
use twine::widgets::{arc, bar, image, slider, spinbox};
use twine::widgets_ext::{dropdown, roller, tabview, tileview};
use twine_testing::{EngineHarness, RenderHarness};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Zero divisors in the core arithmetic give the documented fallbacks.
    #[test]
    fn core_arithmetic_falls_back_on_zero(v in any::<i32>(), a in any::<i32>(), o0 in any::<i32>(), o1 in any::<i32>(), us in any::<u64>()) {
        let d = Duration::us(us);
        prop_assert_eq!(d.div(0), Duration::MAX);
        prop_assert_eq!(Duration::fraction_1024(d, Duration::ZERO), 1024);
        prop_assert_eq!(Fx::from_ratio(v, 0), Fx::ZERO);
        let q = Fx::from_raw(v).div(Fx::ZERO);
        prop_assert_eq!(q, match v { 0 => Fx::ZERO, v if v > 0 => Fx::MAX, _ => Fx::MIN });
        prop_assert_eq!(Scale::from_raw_256(0).inverse_apply(v), v);
        // An empty input range maps to the start of the output range.
        prop_assert_eq!(map(v, a, a, o0, o1), o0);
        prop_assert_eq!(XorShift32::new(a.unsigned_abs() | 1).range(a, a), a);
        let r = Rect::new(v.min(a), v.min(a), v.max(a), v.max(a));
        prop_assert_eq!(r.round_out(0), r);
        prop_assert_eq!(r.round_out(1), r);
        // A transform that collapses an axis has no inverse.
        prop_assert!(Transform::scale(Fx::ZERO, Fx::from_raw(v)).invert().is_none());
    }

    /// A zero speed keeps the duration; a zero duration jumps to the end.
    #[test]
    fn zero_speeds_and_durations(start in -1000i32..1000, end in -1000i32..1000, t in 0u64..1_000_000) {
        let a = twine::anim::Anim::new(start, end).duration(Duration::ms(300)).speed(0);
        prop_assert_eq!(a.spec.duration, Duration::ms(300));
        let z = twine::anim::Anim::new(start, end).duration(Duration::ZERO);
        let _ = z.sample(Duration::us(t));
    }

    /// Zero-size and tiled images, degenerate gradients and dashes draw without dividing by
    /// zero; a zero-size image draws nothing.
    #[test]
    fn degenerate_drawing(
        w in 0u16..3, h in 0u16..3, tile in any::<bool>(),
        x0 in -4i32..20, y0 in -4i32..20, aw in 0i32..20, ah in 0i32..20,
        sx in -2i32..18, sy in -2i32..18, ex in -2i32..18, ey in -2i32..18,
        radius in 0i32..4, kind in 0u8..5, dash in 0i32..3, gap in 0i32..3,
    ) {
        let mut hs = RenderHarness::new(16, 16, ColorFormat::Rgb888);
        let before = hs.data().to_vec();
        let data = vec![0x80u8; usize::from(w) * usize::from(h)];
        let area = Rect::from_xywh(x0, y0, aw, ah);
        hs.paint(|p| {
            let px = ImagePixels::new(ColorFormat::L8, w, h, &data);
            p.image(area, &px, &ImageDsc { tile, ..ImageDsc::default() });
        });
        if w == 0 || h == 0 {
            prop_assert_eq!(hs.data(), &before[..]);
        }
        let (start, end) = (Point::new(sx, sy), Point::new(ex, ey));
        let kind = match kind {
            0 => GradKind::Hor,
            1 => GradKind::Ver,
            2 => GradKind::Linear { start, end },
            3 => GradKind::Radial { center: start, radius, focal: end, focal_radius: radius / 2 },
            _ => GradKind::Conical { center: start, start_angle: Angle::deg(ex), end_angle: Angle::deg(ex) },
        };
        let stops = [
            GradStop::new(Color::BLACK, Fraction::ZERO),
            GradStop::new(Color::WHITE, Fraction::ZERO),
        ];
        let g = Gradient::new(kind, &stops);
        hs.paint(|p| {
            p.rect(area, &RectDsc { bg_opa: Opa::COVER, bg_grad: Some(&g), ..RectDsc::default() });
            p.line(start, end, &LineDsc { width: radius, dash_width: dash, dash_gap: gap, ..LineDsc::default() });
        });
    }
}

/// An engine with `make` building one widget on the screen, sized `w` × `h`, drawn, then
/// pressed and dragged across it.
fn exercise(w: i32, h: i32, make: impl FnOnce(&mut Engine, NodeId) -> NodeId) {
    let mut hs = EngineHarness::new(64, 64);
    let screen = hs.screen();
    let n = make(hs.engine_mut(), screen);
    hs.engine_mut().set_pos(n, 4, 4);
    hs.engine_mut().set_size(n, w, h);
    hs.render_full();
    let c = Point::new(4 + w / 2, 4 + h / 2);
    let _ = hs.tap(c);
    let _ = hs.drag(c, Point::new(c.x + 9, c.y + 9), Duration::ms(100));
    hs.run_until_idle();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Empty and reversed ranges on zero-size and tiny widgets: laid out, drawn and pressed
    /// without a division by zero. (Ranges near `i32::MIN`/`MAX` overflow in the widgets'
    /// subtractions — guideline G2, not a division: `NOTE(R5.S14)` cleans them up.)
    #[test]
    fn widgets_with_empty_ranges_and_sizes(
        min in prop_oneof![Just(0), -1000i32..1000],
        span in prop_oneof![Just(0), -3i32..3],
        value in -5i32..5, w in 0i32..5, h in 0i32..5,
    ) {
        let max = min + span;
        exercise(w, h, |e, p| {
            let b = bar::create(e, p).unwrap();
            e.with_widget_mut(b, |x: &mut bar::Bar, cx| {
                x.set_range(cx, min, max);
                x.set_value(cx, value, false);
            });
            b
        });
        exercise(w, h, |e, p| {
            let s = slider::create(e, p).unwrap();
            e.with_widget_mut(s, |x: &mut slider::Slider, cx| {
                x.set_range(cx, min, max);
                x.set_value(cx, value, false);
            });
            s
        });
        exercise(w, h, |e, p| {
            let a = arc::create(e, p).unwrap();
            e.with_widget_mut(a, |x: &mut arc::Arc, cx| {
                x.set_range(cx, min, max);
                x.set_value(cx, value);
            });
            a
        });
        exercise(w, h, |e, p| {
            let s = spinbox::create(e, p).unwrap();
            e.with_widget_mut(s, |x: &mut spinbox::Spinbox, cx| {
                x.set_range(cx, min, max);
                x.set_step(cx, 0);
                x.set_value(cx, value);
            });
            s
        });
    }

    /// Empty option lists, zero visible rows and zero-size containers of tiles and tabs.
    #[test]
    fn widgets_with_empty_contents(rows in 0u8..3, w in 0i32..4, h in 0i32..4, infinite in any::<bool>()) {
        let mode = if infinite { roller::RollerMode::Infinite } else { roller::RollerMode::Normal };
        exercise(w, h, |e, p| {
            let r = roller::create(e, p).unwrap();
            e.with_widget_mut(r, |x: &mut roller::Roller, cx| {
                x.set_options(cx, "", mode);
                x.set_visible_row_count(cx, rows);
                x.set_selected(cx, 3, true);
            });
            r
        });
        exercise(w, h, |e, p| {
            let d = dropdown::create(e, p).unwrap();
            e.with_widget_mut(d, |x: &mut dropdown::Dropdown, cx| {
                x.set_options(cx, "");
                x.open(cx);
            });
            d
        });
        exercise(w, h, |e, p| {
            let t = tileview::create(e, p).unwrap();
            let _ = tileview::add_tile(e, t, 0, 0, Side::Right);
            let _ = tileview::add_tile(e, t, 1, 0, Side::Left);
            t
        });
        exercise(w, h, |e, p| {
            let t = tabview::create(e, p).unwrap();
            e.with_widget_mut(t, |x: &mut tabview::Tabview, cx| {
                let _ = x.add_tab(cx, "A");
                let _ = x.add_tab(cx, "B");
                x.set_active(cx, 1, true);
            });
            t
        });
    }
}

static EMPTY: Image = Image::new_static(ImageHeader::new(ColorFormat::L8, 0, 0), &[]);
static PIXEL: Image = Image::new_static(ImageHeader::new(ColorFormat::L8, 1, 1), &[0x80]);

/// A zero-size image (tiled or not) and a zero image scale (logged, taken as 1/256).
#[test]
fn zero_size_images_and_zero_scales() {
    for align in [
        ImageAlign::Tile,
        ImageAlign::Center,
        ImageAlign::Stretch,
        ImageAlign::Contain,
    ] {
        for src in [&EMPTY, &PIXEL] {
            exercise(8, 8, |e, p| {
                let i = image::create(e, p).unwrap();
                e.with_widget_mut(i, |x: &mut image::Image, cx| {
                    x.set_src(cx, ImageSource::Static(src));
                    x.set_inner_align(cx, align);
                });
                i
            });
            exercise(8, 8, |e, p| {
                let i = image::create(e, p).unwrap();
                e.with_widget_mut(i, |x: &mut image::Image, cx| {
                    x.set_src(cx, ImageSource::Static(src));
                    x.set_scale(cx, Scale::from_raw_256(0));
                });
                i
            });
        }
    }
}
