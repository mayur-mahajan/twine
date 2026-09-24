//! Benchmark scenarios shared by the criterion and iai-callgrind benches. Host targets:
//! full-screen fill < 40 µs, full-screen rounded rect r=16 with border < 250 µs, 100 × 100
//! shadow w=20 (warm cache) < 60 µs, 60 px spinner arc < 30 µs.

use twine_core::{Angle, Color, ColorFormat, Fx, Opa, Point, Rect, Rotation, Transform};
use twine_render::{
    ArcDsc, BlitDsc, DrawBuf, GradKind, GradStop, Gradient, ImageDsc, ImagePixels, LayerDsc, LayerTransform,
    LineDsc, Mask, Painter, RectDsc, RenderCaches, ShadowDsc, TriangleDsc, rotate_buffer,
};

/// Screen size of the full-screen scenarios.
pub const W: i32 = 320;
/// Screen height.
pub const H: i32 = 240;

/// A named drawing scenario.
pub struct Scene {
    /// Benchmark name.
    pub name: &'static str,
    /// Draws the scenario.
    pub draw: fn(&mut Painter<'_>),
}

const GRAD_VER: Gradient = Gradient::new(
    GradKind::Ver,
    &[
        GradStop::new(Color::hex(0x10_20_40), 0),
        GradStop::new(Color::hex(0xF0_A0_20), 255),
    ],
);
const GRAD_RADIAL: Gradient = Gradient::new(
    GradKind::Radial {
        center: Point::new(100, 100),
        radius: 100,
        focal: Point::new(100, 100),
        focal_radius: 0,
    },
    &[GradStop::new(Color::WHITE, 0), GradStop::new(Color::BLUE, 255)],
);

fn fill_fullscreen(p: &mut Painter<'_>) {
    p.fill(Rect::from_xywh(0, 0, W, H), Color::hex(0x33_66_99), Opa::COVER);
}

fn rounded_rect_r16_border_fullscreen(p: &mut Painter<'_>) {
    p.rect(
        Rect::from_xywh(0, 0, W, H),
        &RectDsc {
            radius: 16,
            bg_color: Color::hex(0xEE_EE_EE),
            bg_opa: Opa::COVER,
            border_width: 2,
            border_color: Color::hex(0x22_88_EE),
            border_opa: Opa::COVER,
            ..RectDsc::default()
        },
    );
}

fn shadow_100_w20(p: &mut Painter<'_>) {
    p.rect(
        Rect::from_xywh(100, 70, 100, 100),
        &RectDsc {
            radius: 10,
            bg_opa: Opa::COVER,
            shadow: ShadowDsc {
                width: 20,
                opa: Opa(160),
                ..ShadowDsc::default()
            },
            ..RectDsc::default()
        },
    );
}

fn gradient_ver_fullscreen(p: &mut Painter<'_>) {
    p.fill_gradient(Rect::from_xywh(0, 0, W, H), &GRAD_VER, Opa::COVER);
}

fn gradient_radial_200(p: &mut Painter<'_>) {
    p.fill_gradient(Rect::from_xywh(20, 20, 200, 200), &GRAD_RADIAL, Opa::COVER);
}

fn masked_fill_radius_fade(p: &mut Painter<'_>) {
    let a = p.push_mask(Mask::Radius {
        area: Rect::from_xywh(10, 10, 300, 220),
        radius: 30,
        outer: false,
    });
    let b = p.push_mask(Mask::Fade {
        area: Rect::from_xywh(0, 0, W, H),
        y_top: 20,
        y_bottom: 220,
        opa_top: Opa::COVER,
        opa_bottom: Opa(20),
    });
    p.fill(Rect::from_xywh(0, 0, W, H), Color::hex(0xAA_33_55), Opa::COVER);
    p.pop_mask(b);
    p.pop_mask(a);
}

fn layer_opa_200x200(p: &mut Painter<'_>) {
    let area = Rect::from_xywh(60, 20, 200, 200);
    p.layer(
        area,
        &LayerDsc {
            opa: Opa(128),
            ..LayerDsc::default()
        },
        |p| {
            p.fill(area, Color::RED, Opa::COVER);
            p.fill(area.expand(-40), Color::BLUE, Opa::COVER);
        },
    );
}

fn line_diag_w5_x100(p: &mut Painter<'_>) {
    let d = LineDsc {
        width: 5,
        color: Color::BLACK,
        ..LineDsc::default()
    };
    for i in 0..100 {
        p.line(Point::new(i * 2, 10), Point::new(120 + i, 230), &d);
    }
}

fn arc_spinner_60px(p: &mut Painter<'_>) {
    let c = Point::new(160, 120);
    p.arc(
        c,
        30,
        Angle::deg(0),
        Angle::deg(360),
        &ArcDsc {
            color: Color::hex(0xDD_DD_DD),
            width: 6,
            ..ArcDsc::default()
        },
    );
    p.arc(
        c,
        30,
        Angle::deg(40),
        Angle::deg(110),
        &ArcDsc {
            color: Color::hex(0x22_88_EE),
            width: 6,
            rounded: true,
            ..ArcDsc::default()
        },
    );
}

fn polygon_star_200(p: &mut Painter<'_>) {
    let pts: [Point; 10] = core::array::from_fn(|i| {
        let r = if i % 2 == 0 { 100 } else { 40 };
        let a = Angle::deg(i as i32 * 36 - 90);
        Point::new(
            160 + r * twine_core::math::cos(a) / 32767,
            120 + r * twine_core::math::sin(a) / 32767,
        )
    });
    p.polygon(&pts, &TriangleDsc::default());
}

/// A 100 × 100 `Argb8888` test image.
pub fn image_100() -> &'static [u8] {
    static IMG: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    IMG.get_or_init(|| {
        (0..100 * 100)
            .flat_map(|i| {
                let (x, y) = (i % 100, i / 100);
                [(x * 2) as u8, (y * 2) as u8, 128, 255]
            })
            .collect()
    })
}

fn blit_rotate_100x100_aa(p: &mut Painter<'_>) {
    let img = ImagePixels::new(ColorFormat::Argb8888, 100, 100, image_100());
    p.blit_transformed(
        &img,
        &BlitDsc {
            transform: Transform::rotate(Angle::deg(30))
                .around(Point::new(50, 50))
                .then(Transform::translate(Fx::from_int(110), Fx::from_int(70))),
            ..BlitDsc::default()
        },
    );
}

/// A full-screen RGB565 image and a 100 × 100 RGB565A8 image (color plane, then alpha plane).
fn images_565() -> &'static (Vec<u8>, Vec<u8>) {
    static IMG: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();
    IMG.get_or_init(|| {
        let full = (0..W * H)
            .flat_map(|i| {
                Color::new((i % W) as u8, (i / W) as u8, 90)
                    .to_rgb565()
                    .to_le_bytes()
            })
            .collect();
        let mut a8: Vec<u8> = (0..100 * 100)
            .flat_map(|i| {
                Color::new((i % 100) as u8 * 2, 60, (i / 100) as u8 * 2)
                    .to_rgb565()
                    .to_le_bytes()
            })
            .collect();
        a8.extend((0..100 * 100).map(|i| ((i % 100) * 255 / 99) as u8));
        (full, a8)
    })
}

fn image_rotate_100x100_aa(p: &mut Painter<'_>) {
    let img = ImagePixels::new(ColorFormat::Argb8888, 100, 100, image_100());
    p.image(
        Rect::from_xywh(110, 70, 100, 100),
        &img,
        &ImageDsc {
            angle: Angle::deg(30),
            pivot: Point::new(50, 50),
            ..ImageDsc::default()
        },
    );
}

fn blit_rgb565_same_format_320x240(p: &mut Painter<'_>) {
    let img = ImagePixels::new(ColorFormat::Rgb565, W as u16, H as u16, &images_565().0);
    p.image(Rect::from_xywh(0, 0, W, H), &img, &ImageDsc::default());
}

fn blit_argb8888_on_rgb565_100x100(p: &mut Painter<'_>) {
    let img = ImagePixels::new(ColorFormat::Argb8888, 100, 100, image_100());
    p.image(Rect::from_xywh(110, 70, 100, 100), &img, &ImageDsc::default());
}

fn blit_rgb565a8_100x100(p: &mut Painter<'_>) {
    let img = ImagePixels::new(ColorFormat::Rgb565A8, 100, 100, &images_565().1);
    p.image(Rect::from_xywh(110, 70, 100, 100), &img, &ImageDsc::default());
}

fn layer_rotate_card_120(p: &mut Painter<'_>) {
    let area = Rect::from_xywh(100, 70, 120, 48);
    p.layer(
        area,
        &LayerDsc {
            transform: Some(LayerTransform {
                rotation: Angle::deg(20),
                pivot: Point::new(60, 24),
                ..LayerTransform::default()
            }),
            ..LayerDsc::default()
        },
        |p| {
            p.rect(
                area.expand(-4),
                &RectDsc {
                    radius: 8,
                    bg_color: Color::WHITE,
                    bg_opa: Opa::COVER,
                    border_width: 2,
                    border_opa: Opa::COVER,
                    ..RectDsc::default()
                },
            );
        },
    );
}

/// Every painter scenario.
pub const SCENES: &[Scene] = &[
    Scene {
        name: "fill_fullscreen",
        draw: fill_fullscreen,
    },
    Scene {
        name: "rounded_rect_r16_border_fullscreen",
        draw: rounded_rect_r16_border_fullscreen,
    },
    Scene {
        name: "shadow_100_w20_warm",
        draw: shadow_100_w20,
    },
    Scene {
        name: "gradient_ver_fullscreen",
        draw: gradient_ver_fullscreen,
    },
    Scene {
        name: "gradient_radial_200",
        draw: gradient_radial_200,
    },
    Scene {
        name: "masked_fill_radius_fade",
        draw: masked_fill_radius_fade,
    },
    Scene {
        name: "layer_opa_200x200",
        draw: layer_opa_200x200,
    },
    Scene {
        name: "line_diag_w5_x100",
        draw: line_diag_w5_x100,
    },
    Scene {
        name: "arc_spinner_60px",
        draw: arc_spinner_60px,
    },
    Scene {
        name: "polygon_star_200",
        draw: polygon_star_200,
    },
    Scene {
        name: "blit_rotate_100x100_aa",
        draw: blit_rotate_100x100_aa,
    },
    Scene {
        name: "layer_rotate_card_120",
        draw: layer_rotate_card_120,
    },
    Scene {
        name: "rotate_100x100_aa",
        draw: image_rotate_100x100_aa,
    },
    Scene {
        name: "blit_rgb565_same_format_320x240",
        draw: blit_rgb565_same_format_320x240,
    },
    Scene {
        name: "blit_argb8888_on_rgb565_100x100",
        draw: blit_argb8888_on_rgb565_100x100,
    },
    Scene {
        name: "blit_rgb565a8_100x100",
        draw: blit_rgb565a8_100x100,
    },
];

/// A 320 × 240 screen buffer with warm caches.
pub struct Target {
    data: Vec<u8>,
    caches: RenderCaches,
    format: ColorFormat,
}

impl Target {
    /// A screen in `format`; `scene` is drawn once to warm the caches.
    pub fn new(format: ColorFormat, scene: &Scene) -> Self {
        let mut t = Self {
            data: vec![0; format.stride(W as u32) as usize * H as usize],
            caches: RenderCaches::default(),
            format,
        };
        t.run(scene);
        t
    }

    /// Draws `scene` over the whole screen.
    pub fn run(&mut self, scene: &Scene) {
        let buf =
            DrawBuf::new_packed(&mut self.data, self.format, Rect::from_xywh(0, 0, W, H)).expect("buffer");
        (scene.draw)(&mut Painter::new(buf, &mut self.caches));
    }
}

/// `rotate_buffer` of a 320 × 40 RGB565 chunk by 90°.
pub struct RotateChunk {
    src: Vec<u8>,
    dst: Vec<u8>,
}

impl Default for RotateChunk {
    fn default() -> Self {
        Self::new()
    }
}

impl RotateChunk {
    /// Buffers for the benchmark.
    pub fn new() -> Self {
        Self {
            src: (0..320 * 40 * 2).map(|i| i as u8).collect(),
            dst: vec![0; 320 * 40 * 2],
        }
    }

    /// Rotates once.
    pub fn run(&mut self) {
        rotate_buffer(&self.src, 640, &mut self.dst, 80, 320, 40, Rotation::Deg90, 2).expect("rotate");
    }
}
