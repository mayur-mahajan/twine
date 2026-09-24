//! Draw traversal: decorations, clipping, layers, the top-cover optimization, and chunk-size
//! independence.

mod common;

use common::{boxed, screen, style, white_screen};
use twine_core::{Angle, Color, ColorFormat, Opa, Rect};
use twine_engine::{Engine, InvalidateReason, MeasureCx, NodeId, ObjFlags, default_covers};
use twine_hal::BufferSpec;
use twine_image::{Image, ImageHeader, ImageSource};
use twine_render::{BlendMode, GradKind, GradStop, Gradient};
use twine_style::{GradDir, Length, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::scenes::{engine_boxes, styled_box};

type Scene = fn(&mut Engine);

fn bg_border_outline(e: &mut Engine) {
    let s = white_screen(e);
    styled_box(
        e,
        s,
        Rect::from_xywh(20, 16, 60, 40),
        &[
            StyleProp::BgColor(Color::hex(0x90_CA_F9)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(10),
            StyleProp::BorderWidth(4),
            StyleProp::BorderColor(Color::hex(0x0D_47_A1)),
            StyleProp::OutlineWidth(3),
            StyleProp::OutlinePad(3),
            StyleProp::OutlineColor(Color::hex(0xE5_39_35)),
        ],
    );
}

fn shadow(e: &mut Engine) {
    let s = white_screen(e);
    styled_box(
        e,
        s,
        Rect::from_xywh(26, 18, 48, 36),
        &[
            StyleProp::BgColor(Color::hex(0xFF_EE_58)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(8),
            StyleProp::ShadowWidth(16),
            StyleProp::ShadowOffsetX(4),
            StyleProp::ShadowOffsetY(6),
            StyleProp::ShadowSpread(2),
            StyleProp::ShadowOpa(Opa::P70),
        ],
    );
}

static CONICAL: Gradient = Gradient::new(
    GradKind::Hor,
    &[
        GradStop::new(Color::RED, 0),
        GradStop::new(Color::GREEN, 128),
        GradStop::new(Color::BLUE, 255),
    ],
);

fn gradient(e: &mut Engine) {
    let s = white_screen(e);
    styled_box(
        e,
        s,
        Rect::from_xywh(8, 8, 40, 56),
        &[
            StyleProp::BgColor(Color::hex(0x7B_1F_A2)),
            StyleProp::BgGradColor(Color::hex(0xFF_CC_80)),
            StyleProp::BgGradDir(GradDir::Ver),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(6),
        ],
    );
    styled_box(
        e,
        s,
        Rect::from_xywh(52, 8, 40, 56),
        &[StyleProp::BgGrad(&CONICAL), StyleProp::BgOpa(Opa::COVER)],
    );
}

/// A 6 × 6 RGB565 checkerboard.
static CHECKER: [u8; 72] = {
    let mut d = [0u8; 72];
    let mut i = 0;
    while i < 36 {
        let (x, y) = (i % 6, i / 6);
        let v: u16 = if (x / 2 + y / 2) % 2 == 0 { 0xF800 } else { 0x001F };
        d[2 * i] = v as u8;
        d[2 * i + 1] = (v >> 8) as u8;
        i += 1;
    }
    d
};
static CHECKER_IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::Rgb565, 6, 6), &CHECKER);
static CHECKER_SRC: ImageSource = ImageSource::Static(&CHECKER_IMG);

fn bg_image(e: &mut Engine) {
    let s = white_screen(e);
    styled_box(
        e,
        s,
        Rect::from_xywh(10, 10, 30, 30),
        &[
            StyleProp::BgColor(Color::hex(0xDD_DD_DD)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::BgImageSrc(&CHECKER_SRC),
            StyleProp::BorderWidth(2),
        ],
    );
    styled_box(
        e,
        s,
        Rect::from_xywh(50, 10, 30, 30),
        &[StyleProp::BgImageSrc(&CHECKER_SRC), StyleProp::BgImageTiled(true)],
    );
}

fn radius_clip_corner(e: &mut Engine) {
    let s = white_screen(e);
    let p = styled_box(
        e,
        s,
        Rect::from_xywh(16, 8, 64, 56),
        &[
            StyleProp::BgColor(Color::hex(0x26_32_38)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(20),
            StyleProp::ClipCorner(true),
        ],
    );
    for i in 0..4 {
        boxed(
            e,
            p,
            Rect::from_xywh(16, 8 + i * 14, 64, 7),
            Color::hex(0xFF_A7_26),
        );
    }
}

fn overflow_visible(e: &mut Engine) {
    let s = white_screen(e);
    let a = boxed(e, s, Rect::from_xywh(10, 10, 30, 30), Color::hex(0xB0_BE_C5));
    boxed(e, a, Rect::from_xywh(30, 30, 20, 20), Color::RED);
    let b = boxed(e, s, Rect::from_xywh(56, 10, 30, 30), Color::hex(0xB0_BE_C5));
    e.set_flag(b, ObjFlags::OVERFLOW_VISIBLE, true);
    boxed(e, b, Rect::from_xywh(76, 30, 20, 20), Color::RED);
}

fn opa_layer(e: &mut Engine) {
    let s = white_screen(e);
    let g = styled_box(
        e,
        s,
        Rect::from_xywh(10, 10, 70, 50),
        &[StyleProp::OpaLayered(Opa::P50)],
    );
    boxed(e, g, Rect::from_xywh(10, 10, 45, 35), Color::RED);
    boxed(e, g, Rect::from_xywh(35, 25, 45, 35), Color::BLUE);
}

fn transform_rotated(e: &mut Engine) {
    let s = white_screen(e);
    let g = styled_box(
        e,
        s,
        Rect::from_xywh(26, 20, 44, 30),
        &[
            StyleProp::BgColor(Color::hex(0x00_96_88)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::Radius(4),
            StyleProp::TransformRotation(Angle::deg(30)),
            StyleProp::TransformPivotX(Length::Pct(50)),
            StyleProp::TransformPivotY(Length::Pct(50)),
        ],
    );
    boxed(e, g, Rect::from_xywh(30, 24, 12, 12), Color::WHITE);
}

fn blend_additive(e: &mut Engine) {
    let s = white_screen(e);
    boxed(e, s, Rect::from_xywh(10, 10, 50, 50), Color::hex(0x80_00_00));
    styled_box(
        e,
        s,
        Rect::from_xywh(35, 20, 50, 30),
        &[
            StyleProp::BgColor(Color::hex(0x00_80_40)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::BlendMode(BlendMode::Additive),
        ],
    );
}

fn border_post(e: &mut Engine) {
    let s = white_screen(e);
    let p = styled_box(
        e,
        s,
        Rect::from_xywh(16, 10, 60, 50),
        &[
            StyleProp::BgColor(Color::hex(0xEC_EF_F1)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::BorderWidth(6),
            StyleProp::BorderColor(Color::hex(0x37_47_4F)),
            StyleProp::BorderPost(true),
        ],
    );
    boxed(e, p, Rect::from_xywh(10, 20, 80, 20), Color::hex(0xFF_70_43));
}

/// A 32 × 32 A8 mask: a diagonal ramp.
static RAMP: [u8; 32 * 32] = {
    let mut d = [0u8; 32 * 32];
    let mut i = 0;
    while i < 32 * 32 {
        d[i] = ((i % 32 + i / 32) * 4) as u8;
        i += 1;
    }
    d
};
static RAMP_IMG: Image = Image::new_static(ImageHeader::new(ColorFormat::A8, 32, 32), &RAMP);
static RAMP_SRC: ImageSource = ImageSource::Static(&RAMP_IMG);

fn bitmap_mask(e: &mut Engine) {
    let s = white_screen(e);
    let b = styled_box(
        e,
        s,
        Rect::from_xywh(20, 10, 48, 48),
        &[
            StyleProp::BgColor(Color::hex(0x6A_1B_9A)),
            StyleProp::BgOpa(Opa::COVER),
            StyleProp::BitmapMaskSrc(&RAMP_SRC),
        ],
    );
    boxed(e, b, Rect::from_xywh(40, 30, 20, 20), Color::hex(0xFF_EB_3B));
}

fn boxes_scene(e: &mut Engine) {
    engine_boxes(e);
}

const SCENES: &[(&str, Scene, (u16, u16))] = &[
    ("draw_base_bg_border_outline", bg_border_outline, (100, 72)),
    ("draw_base_shadow", shadow, (100, 72)),
    ("draw_base_gradient", gradient, (100, 72)),
    ("draw_base_bg_image", bg_image, (100, 50)),
    ("draw_radius_clip_corner_children", radius_clip_corner, (100, 72)),
    ("draw_overflow_visible", overflow_visible, (100, 60)),
    ("draw_opa_layer_subtree", opa_layer, (100, 72)),
    ("draw_transform_rotated_group", transform_rotated, (100, 72)),
    ("draw_blend_additive", blend_additive, (100, 72)),
    ("border_post_draws_after_children", border_post, (100, 72)),
    ("draw_bitmap_mask", bitmap_mask, (100, 72)),
    ("engine_boxes", boxes_scene, (320, 240)),
];

fn render(scene: Scene, size: (u16, u16), buffers: BufferSpec) -> EngineHarness {
    let mut h = EngineHarness::new(size.0, size.1)
        .no_theme()
        .buffers(buffers)
        .mount_engine(scene);
    h.run_until_idle();
    h
}

#[test]
fn draw_snapshots() {
    for (name, scene, size) in SCENES {
        let mut h = render(*scene, *size, BufferSpec::PartialDouble { rows: 40 });
        h.assert_snapshot(name);
    }
}

#[test]
fn draw_is_deterministic_across_chunk_sizes() {
    for (name, scene, size) in SCENES {
        let reference = render(*scene, *size, BufferSpec::PartialDouble { rows: 40 }).panel_rgb888();
        for rows in [1, 7] {
            let got = render(*scene, *size, BufferSpec::PartialSingle { rows }).panel_rgb888();
            let diff = reference.chunks(3).zip(got.chunks(3)).position(|(a, b)| a != b);
            assert!(
                diff.is_none(),
                "{name}: {rows}-row chunks differ at pixel {diff:?}"
            );
        }
    }
}

#[test]
fn opa_layer_does_not_darken_overlap() {
    let h = render(opa_layer, (100, 72), BufferSpec::PartialDouble { rows: 40 });
    // Inside the overlap the blue box is on top, blended once at 50 % over white.
    let overlap = h.pixel(45, 35);
    let blue_only = h.pixel(70, 50);
    assert_eq!(overlap, blue_only);
}

#[test]
fn top_cover_skips_hidden_background() {
    let mut cover: Option<NodeId> = None;
    let mut h = EngineHarness::new(100, 80).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        for i in 0..5 {
            boxed(e, s, Rect::from_xywh(i * 10, i * 5, 30, 30), Color::RED);
        }
        cover = Some(boxed(e, s, Rect::new(0, 0, 100, 80), Color::BLUE));
    });
    h.run_until_idle();
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(40, 30, 10, 10), InvalidateReason::Explicit);
    h.advance(twine_core::Duration::ms(16));
    // The cover node plus the (empty) top and system layers; not the screen, not the 5 boxes.
    assert_eq!(h.last_frame().nodes_drawn, 3);
    assert!(cover.is_some());
}

#[test]
fn rounded_parent_does_not_cover_corner_area() {
    let mut b: Option<NodeId> = None;
    let mut h = EngineHarness::new(100, 80).no_theme().mount_engine(|e| {
        let s = white_screen(e);
        let n = boxed(e, s, Rect::from_xywh(10, 10, 60, 60), Color::BLUE);
        style(e, n, &[StyleProp::Radius(10)]);
        b = Some(n);
    });
    h.run_until_idle();
    let b = b.unwrap();
    let e = h.engine();
    let cx = MeasureCx::new(e, b);
    assert!(!default_covers(&cx, Rect::from_xywh(11, 11, 4, 4)));
    assert!(default_covers(&cx, Rect::from_xywh(20, 10, 40, 60)));
    assert!(default_covers(&cx, Rect::from_xywh(10, 20, 60, 40)));
    assert!(!default_covers(&cx, Rect::from_xywh(15, 15, 20, 20)));
    // The corner area is drawn from the screen.
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::from_xywh(11, 11, 4, 4), InvalidateReason::Explicit);
    h.advance(twine_core::Duration::ms(16));
    assert_eq!(h.last_frame().nodes_drawn, 4); // screen, box, top, sys
    let _ = screen(h.engine());
}

#[test]
fn bitmap_mask_fades_the_group() {
    let h = render(bitmap_mask, (100, 72), BufferSpec::PartialDouble { rows: 40 });
    // Outside the 32 × 32 mask (centered in the 48 × 48 box) nothing of the box is drawn.
    assert_eq!(h.pixel(22, 12), Color::WHITE);
    // The ramp: nearly transparent at the mask's top-left, opaque at its bottom-right.
    let light = h.pixel(29, 19);
    assert!(light.r > 230 && light.g > 230, "{light:?}");
    let child_faint = h.pixel(41, 31);
    let child_full = h.pixel(58, 48);
    assert!(child_full.r == 255 && child_full.b < 80, "{child_full:?}");
    assert!(
        child_faint.b > child_full.b + 60,
        "{child_faint:?} vs {child_full:?}"
    );
}
