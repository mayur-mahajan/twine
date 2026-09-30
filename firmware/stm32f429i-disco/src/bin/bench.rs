//! Render benchmark on the STM32F429I-DISC1: the Phase 03/04 renderer scenarios (the host
//! criterion benches of `twine-render`, adapted to the 240 × 320 panel) drawn into an RGB565
//! framebuffer in SDRAM, each once in software and once with the DMA2D attached to the
//! `Painter`. Cycle counts come from the DWT cycle counter (168 cycles per µs); the result of
//! each scenario stays on screen for a moment.
//!
//! `cargo run --release --bin bench` prints one defmt line per scenario:
//! `bench <name>: sw <cycles> (<µs>) dma2d <cycles> (<µs>) x<speed-up>`.
#![no_std]
#![no_main]

use cortex_m::peripheral::DWT;
use embassy_executor::Spawner;
use twine::core::{Angle, Color, ColorFormat, Fraction, Opa, Point, Rect};
use twine::hal::FramebufferDisplay;
use twine::render::{
    ArcDsc, DrawAccel, DrawBuf, GradKind, GradStop, Gradient, ImageDsc, ImagePixels, LayerDsc, LineDsc, Mask,
    Painter, RectDsc, RenderCaches, ShadowDsc, TriangleDsc,
};
use twine_accel_stm32::{Dma2d, PacRegs};
use twine_example_stm32f429i_disco::{Board, FB_BYTES, HEIGHT, WIDTH, clocks, init_heap};
use {defmt_rtt as _, panic_probe as _};

const W: i32 = WIDTH as i32;
const H: i32 = HEIGHT as i32;
/// Timed runs per scenario and mode (the minimum is reported).
const RUNS: u32 = 5;
/// Core clock (cycles per µs).
const MHZ: u32 = 168;

/// A named drawing scenario.
struct Scene {
    name: &'static str,
    draw: fn(&mut Painter<'_>, &Images),
}

/// Test images in SDRAM.
struct Images {
    /// Full-screen RGB565.
    full_565: &'static [u8],
    /// 100 × 100 ARGB8888.
    argb_100: &'static [u8],
}

const GRAD_VER: Gradient = Gradient::new(
    GradKind::Ver,
    &[
        GradStop::new(Color::hex(0x10_20_40), Fraction::ZERO),
        GradStop::new(Color::hex(0xF0_A0_20), Fraction::ONE),
    ],
);
const GRAD_RADIAL: Gradient = Gradient::new(
    GradKind::Radial {
        center: Point::new(100, 100),
        radius: 100,
        focal: Point::new(100, 100),
        focal_radius: 0,
    },
    &[
        GradStop::new(Color::WHITE, Fraction::ZERO),
        GradStop::new(Color::BLUE, Fraction::ONE),
    ],
);

const SCENES: &[Scene] = &[
    Scene {
        name: "fill_fullscreen",
        draw: |p, _| p.fill(Rect::from_xywh(0, 0, W, H), Color::hex(0x33_66_99), Opa::COVER),
    },
    Scene {
        name: "fill_fullscreen_opa50",
        draw: |p, _| p.fill(Rect::from_xywh(0, 0, W, H), Color::hex(0xCC_33_66), Opa::P50),
    },
    Scene {
        name: "rounded_rect_r16_border_fullscreen",
        draw: |p, _| {
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
        },
    },
    Scene {
        name: "shadow_100_w20_warm",
        draw: |p, _| {
            p.rect(
                Rect::from_xywh(70, 110, 100, 100),
                &RectDsc {
                    radius: 10,
                    bg_opa: Opa::COVER,
                    shadow: ShadowDsc {
                        width: 20,
                        opa: Opa::from_raw(160),
                        ..ShadowDsc::default()
                    },
                    ..RectDsc::default()
                },
            );
        },
    },
    Scene {
        name: "gradient_ver_fullscreen",
        draw: |p, _| p.fill_gradient(Rect::from_xywh(0, 0, W, H), &GRAD_VER, Opa::COVER),
    },
    Scene {
        name: "gradient_radial_200",
        draw: |p, _| p.fill_gradient(Rect::from_xywh(20, 20, 200, 200), &GRAD_RADIAL, Opa::COVER),
    },
    Scene {
        name: "masked_fill_radius_fade",
        draw: |p, _| {
            let a = p.push_mask(Mask::Radius {
                area: Rect::from_xywh(10, 10, W - 20, H - 20),
                radius: 30,
                outer: false,
            });
            let b = p.push_mask(Mask::Fade {
                area: Rect::from_xywh(0, 0, W, H),
                y_top: 20,
                y_bottom: H - 20,
                opa_top: Opa::COVER,
                opa_bottom: Opa::from_raw(20),
            });
            p.fill(Rect::from_xywh(0, 0, W, H), Color::hex(0xAA_33_55), Opa::COVER);
            p.pop_mask(b);
            p.pop_mask(a);
        },
    },
    Scene {
        name: "layer_opa_200x200",
        draw: |p, _| {
            let area = Rect::from_xywh(20, 60, 200, 200);
            p.layer(
                area,
                &LayerDsc {
                    opa: Opa::from_raw(128),
                    ..LayerDsc::default()
                },
                |p| {
                    p.fill(area, Color::RED, Opa::COVER);
                    p.fill(area.expand(-40), Color::BLUE, Opa::COVER);
                },
            );
        },
    },
    Scene {
        name: "line_diag_w5_x100",
        draw: |p, _| {
            let d = LineDsc {
                width: 5,
                color: Color::BLACK,
                ..LineDsc::default()
            };
            for i in 0..100 {
                p.line(Point::new(i * 2, 10), Point::new(120 + i, 300), &d);
            }
        },
    },
    Scene {
        name: "arc_spinner_60px",
        draw: |p, _| {
            let c = Point::new(120, 160);
            let track = ArcDsc {
                color: Color::hex(0xDD_DD_DD),
                width: 6,
                ..ArcDsc::default()
            };
            p.arc(c, 30, Angle::deg(0), Angle::deg(360), &track);
            let ind = ArcDsc {
                color: Color::hex(0x22_88_EE),
                width: 6,
                rounded: true,
                ..ArcDsc::default()
            };
            p.arc(c, 30, Angle::deg(40), Angle::deg(110), &ind);
        },
    },
    Scene {
        name: "polygon_star_200",
        draw: |p, _| {
            let pts: [Point; 10] = core::array::from_fn(|i| {
                let r = if i % 2 == 0 { 100 } else { 40 };
                let a = Angle::deg(i as i32 * 36 - 90);
                Point::new(
                    120 + r * twine::core::math::cos(a) / 32767,
                    160 + r * twine::core::math::sin(a) / 32767,
                )
            });
            p.polygon(&pts, &TriangleDsc::default());
        },
    },
    Scene {
        name: "blit_rgb565_same_format_fullscreen",
        draw: |p, img| {
            let px = ImagePixels::new(ColorFormat::Rgb565, WIDTH, HEIGHT, img.full_565);
            p.image(Rect::from_xywh(0, 0, W, H), &px, &ImageDsc::default());
        },
    },
    Scene {
        name: "blit_argb8888_on_rgb565_100x100",
        draw: |p, img| {
            let px = ImagePixels::new(ColorFormat::Argb8888, 100, 100, img.argb_100);
            p.image(Rect::from_xywh(70, 110, 100, 100), &px, &ImageDsc::default());
        },
    },
    Scene {
        name: "rotate_100x100_aa",
        draw: |p, img| {
            let px = ImagePixels::new(ColorFormat::Argb8888, 100, 100, img.argb_100);
            p.image(
                Rect::from_xywh(70, 110, 100, 100),
                &px,
                &ImageDsc {
                    angle: Angle::deg(30),
                    pivot: Point::new(50, 50),
                    ..ImageDsc::default()
                },
            );
        },
    },
];

/// Draws `scene` [`RUNS`] times into `fb` and returns the fewest cycles of one run (the painter
/// is dropped inside the timed region, so queued DMA2D work is included).
fn measure(
    fb: &mut [u8],
    caches: &mut RenderCaches,
    mut accel: Option<&mut dyn DrawAccel>,
    scene: &Scene,
    images: &Images,
) -> u32 {
    let mut best = u32::MAX;
    for _ in 0..RUNS {
        let start = DWT::cycle_count();
        {
            let Ok(buf) = DrawBuf::new_packed(fb, ColorFormat::Rgb565, Rect::from_xywh(0, 0, W, H)) else {
                defmt::panic!("framebuffer does not fit {}x{}", W, H);
            };
            let mut p = Painter::new(buf, caches);
            if let Some(a) = accel.as_deref_mut() {
                p = p.with_accel(a);
            }
            (scene.draw)(&mut p, images);
        }
        best = best.min(DWT::cycle_count().wrapping_sub(start));
    }
    best
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    init_heap();
    let mut cp = cortex_m::Peripherals::take().unwrap_or_else(|| defmt::panic!("core peripherals taken"));
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();
    let p = embassy_stm32::init(clocks());
    let mut board = Board::init(p, &mut embassy_time::Delay);
    let Some((mut fb, _)) = board.display.framebuffers() else {
        defmt::panic!("framebuffers already taken");
    };
    let _ = board.display.present(0);

    // Test images after the framebuffers, in SDRAM.
    let (full_565, rest) = board.sdram_rest.split_at_mut(FB_BYTES);
    let (argb_100, _) = rest.split_at_mut(100 * 100 * 4);
    for (i, px) in full_565.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let (x, y) = (i % WIDTH as usize, i / WIDTH as usize);
        *px = Color::new(x as u8, y as u8, 90).to_rgb565().to_le_bytes();
    }
    for (i, px) in argb_100.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % 100, i / 100);
        *px = [(x * 2) as u8, (y * 2) as u8, 128, 255];
    }
    let images = Images { full_565, argb_100 };

    let mut caches = RenderCaches::default();
    let mut dma2d = Dma2d::new(PacRegs::new());
    defmt::info!(
        "bench: {} scenarios, {}x{} RGB565 in SDRAM, best of {} runs",
        SCENES.len(),
        W,
        H,
        RUNS
    );
    for scene in SCENES {
        let sw = measure(fb.as_mut_slice(), &mut caches, None, scene, &images);
        embassy_time::Timer::after_millis(300).await;
        let hw = measure(fb.as_mut_slice(), &mut caches, Some(&mut dma2d), scene, &images);
        embassy_time::Timer::after_millis(300).await;
        defmt::info!(
            "bench {}: sw {} ({} us) dma2d {} ({} us) x{}.{}",
            scene.name,
            sw,
            sw / MHZ,
            hw,
            hw / MHZ,
            sw / hw.max(1),
            (sw % hw.max(1)) * 10 / hw.max(1)
        );
    }
    defmt::info!("bench: done");
    loop {
        embassy_time::Timer::after_secs(3600).await;
    }
}
