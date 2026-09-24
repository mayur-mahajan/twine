//! `cargo xtask sim image_gallery`: images.
//!
//! Pages: (1) the Twine logo in every color format on a checkerboard; (2) opacity, recolor,
//! chroma key, tiling and clip radius; (3) a rotating, pulsing image with and without
//! anti-aliasing; (4) the same photo decoded from QOI, PNG, BMP and JPEG; (5) an animated GIF;
//! (6) RLE- and LZ4-compressed images with the image cache statistics. Switch pages with ←/→ or
//! by clicking the left/right third of the screen; pages also advance every 3 s. The page title
//! is drawn at the top and logged with what to look at.
//!
//! Every panel format works (`TWINE_SIM_FORMAT=rgb565swapped|rgb888|xrgb8888|argb8888|l8|i1`).
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

use twine_assets::fonts::{MONTSERRAT_10, MONTSERRAT_12, MONTSERRAT_16};
use twine_core::{Angle, Color, ColorFormat, Duration, Instant, Opa, Point, Rect, Scale};
use twine_examples::assets as img;
use twine_hal::Key;
use twine_image::decoders::gif::GifPlayer;
use twine_image::{
    DecoderRegistry, Image, ImageCache, ImageContext, ImageHeaderCache, ImageSource, with_pixels,
};
use twine_render::{DrawBuf, ImageDsc, ImagePixels, Painter, RenderCaches, RenderConfig};
use twine_sim::{SimConfig, SimFrame, show_framebuffer_with_input};
use twine_text::{GlyphCache, TextAlign, TextDsc, draw_text, symbols};

const W: i32 = 480;
const H: i32 = 320;
/// Height of the title bar.
const TOP: i32 = 26;
/// Frames between automatic page switches (3 s at 60 fps).
const AUTO_FRAMES: u32 = 180;
/// Image cache budget (as recommended for the demos).
const CACHE_BYTES: usize = 64 * 1024;

const BG: Color = Color::hex(0xFAFAFA);
const INK: Color = Color::hex(0x212121);
const ACCENT: Color = Color::hex(0x6A1B9A);

static PHOTO_QOI: &[u8] = include_bytes!("../../../assets/images/photo.qoi");
static PHOTO_PNG: &[u8] = include_bytes!("../../../assets/images/photo.png");
static PHOTO_BMP: &[u8] = include_bytes!("../../../assets/images/photo.bmp");
static PHOTO_JPG: &[u8] = include_bytes!("../../../assets/images/photo.jpg");
static SPINNER_GIF: &[u8] = include_bytes!("../../../assets/images/spinner.gif");

/// Every format of the logo, with its name.
const FORMATS: [(&str, &Image); 16] = [
    ("L8", &img::logo_l8::LOGO_L8),
    ("A1", &img::logo_a1::LOGO_A1),
    ("A2", &img::logo_a2::LOGO_A2),
    ("A4", &img::logo_a4::LOGO_A4),
    ("A8", &img::logo_a8::LOGO_A8),
    ("I1", &img::logo_i1::LOGO_I1),
    ("I2", &img::logo_i2::LOGO_I2),
    ("I4", &img::logo_i4::LOGO_I4),
    ("I8", &img::logo_i8::LOGO_I8),
    ("RGB565", &img::logo_rgb565::LOGO_RGB565),
    ("565 SWAP", &img::logo_rgb565_swapped::LOGO_RGB565_SWAPPED),
    ("RGB565A8", &img::logo_rgb565a8::LOGO_RGB565A8),
    ("RGB888", &img::logo_rgb888::LOGO_RGB888),
    ("ARGB8888", &img::logo_argb8888::LOGO_ARGB8888),
    ("XRGB8888", &img::logo_xrgb8888::LOGO_XRGB8888),
    (
        "PREMUL",
        &img::logo_argb8888_premultiplied::LOGO_ARGB8888_PREMULTIPLIED,
    ),
];

/// State shared by the pages.
struct Ctx<'a> {
    glyphs: &'a mut GlyphCache,
    images: &'a mut Images,
    frame: &'a SimFrame,
}

/// Image decoding state.
struct Images {
    cache: ImageCache,
    headers: ImageHeaderCache,
    registry: DecoderRegistry,
    gif: Option<GifPlayer<'static>>,
    /// When the GIF's next frame is due.
    gif_due: Instant,
}

impl Images {
    /// Draws `src` at `(x, y)` (decoding through the cache when needed).
    fn draw(&mut self, p: &mut Painter<'_>, src: &ImageSource, x: i32, y: i32, dsc: &ImageDsc<'_>) {
        let mut cx = ImageContext {
            cache: &mut self.cache,
            header_cache: &mut self.headers,
            registry: &self.registry,
            fs: None,
        };
        let r = with_pixels(src, &mut cx, |px| {
            p.image(Rect::from_xywh(x, y, i32::from(px.w), i32::from(px.h)), px, dsc);
        });
        if let Err(e) = r {
            twine_core::warn!(target: "twine::image", "cannot draw {}: {}", src, e);
        }
    }
}

struct Page {
    title: &'static str,
    hint: &'static str,
    draw: fn(&mut Painter<'_>, &mut Ctx<'_>),
}

const PAGES: &[Page] = &[
    Page {
        title: "Every color format",
        hint: "the logo in all 16 formats; A* are masks (black), I1/I2/I4 dithered palettes, XRGB/RGB are opaque",
        draw: page_formats,
    },
    Page {
        title: "Opacity, recolor, chroma key, tiling",
        hint: "opa 25/50/100 %, recolor, white threads keyed out, tiled background, clip radius 16",
        draw: page_effects,
    },
    Page {
        title: "Rotation & zoom",
        hint: "1 deg per frame, zoom pulsing 50-200 %; bilinear (left) vs nearest (right)",
        draw: page_transform,
    },
    Page {
        title: "Decoders",
        hint: "the same photo decoded from QOI, PNG, BMP and JPEG (JPEG is lossy)",
        draw: page_decoders,
    },
    Page {
        title: "Animated GIF",
        hint: "8-frame spinner (80 ms per frame) composited over a checkerboard, 1x and 3x",
        draw: page_gif,
    },
    Page {
        title: "Compressed images & cache",
        hint: "RLE and LZ4 images decompressed into the image cache; the counters below",
        draw: page_compressed,
    },
];

fn text(p: &mut Painter<'_>, c: &mut GlyphCache, area: Rect, s: &str, align: TextAlign) {
    let mut d = TextDsc::new(&MONTSERRAT_12);
    d.color = INK;
    d.align = align;
    draw_text(p, area, s, &d, c);
}

fn checker(p: &mut Painter<'_>, area: Rect) {
    p.fill(area, Color::hex(0xE0E0E0), Opa::COVER);
    for y in (area.y0..area.y1).step_by(8) {
        for x in (area.x0..area.x1).step_by(8) {
            if ((x - area.x0) / 8 + (y - area.y0) / 8) % 2 == 1 {
                let r = Rect::new(x, y, (x + 8).min(area.x1), (y + 8).min(area.y1));
                p.fill(r, Color::hex(0xA0A0A0), Opa::COVER);
            }
        }
    }
}

fn logo() -> ImagePixels<'static> {
    img::logo_argb8888::LOGO_ARGB8888.pixels().expect("valid logo")
}

fn page_formats(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    checker(p, Rect::new(0, TOP, W, H));
    for (i, (name, img)) in FORMATS.iter().enumerate() {
        let (col, row) = (i as i32 % 6, i as i32 / 6);
        let (x, y) = (8 + col * 79, TOP + 6 + row * 96);
        if let Some(px) = img.pixels() {
            p.image(Rect::from_xywh(x + 7, y, 64, 64), &px, &ImageDsc::default());
        }
        p.fill(Rect::from_xywh(x, y + 68, 78, 16), Color::WHITE, Opa(200));
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(x, y + 69, 78, 16),
            name,
            TextAlign::Center,
        );
    }
}

fn page_effects(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    // Tiled background: the RGB565 logo repeated, faded.
    let bg = img::logo_rgb565::LOGO_RGB565.pixels().expect("valid logo");
    p.image(
        Rect::new(0, TOP, W, H),
        &bg,
        &ImageDsc {
            tile: true,
            opa: Opa(60),
            ..ImageDsc::default()
        },
    );
    let px = logo();
    let cells: [(&str, ImageDsc<'_>); 6] = [
        (
            "opa 25 %",
            ImageDsc {
                opa: Opa::from_percent(25),
                ..ImageDsc::default()
            },
        ),
        (
            "opa 50 %",
            ImageDsc {
                opa: Opa::from_percent(50),
                ..ImageDsc::default()
            },
        ),
        ("opa 100 %", ImageDsc::default()),
        (
            "recolor",
            ImageDsc {
                recolor: Color::hex(0xE65100),
                recolor_opa: Opa(170),
                ..ImageDsc::default()
            },
        ),
        (
            "chroma key white",
            ImageDsc {
                chroma_key: Some(Color::WHITE),
                ..ImageDsc::default()
            },
        ),
        (
            "clip radius 16",
            ImageDsc {
                clip_radius: 16,
                ..ImageDsc::default()
            },
        ),
    ];
    for (i, (name, dsc)) in cells.iter().enumerate() {
        let (col, row) = (i as i32 % 3, i as i32 / 3);
        let (x, y) = (40 + col * 150, TOP + 20 + row * 140);
        checker(p, Rect::from_xywh(x, y, 96, 96));
        p.image(Rect::from_xywh(x + 16, y + 16, 64, 64), &px, dsc);
        p.fill(
            Rect::from_xywh(x - 20, y + 100, 136, 18),
            Color::WHITE,
            Opa::COVER,
        );
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(x - 20, y + 101, 136, 18),
            name,
            TextAlign::Center,
        );
    }
}

/// Triangle wave between `lo` and `hi` with `period` frames.
fn pulse(frame: u32, period: u32, lo: u16, hi: u16) -> u16 {
    let t = frame % period;
    let half = period / 2;
    let k = if t < half { t } else { period - t };
    lo + ((u32::from(hi - lo) * k) / half) as u16
}

fn page_transform(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    checker(p, Rect::new(0, TOP, W, H));
    let px = logo();
    let f = cx.frame.index;
    let scale = Scale(pulse(f, 240, 128, 512));
    for (i, aa) in [true, false].into_iter().enumerate() {
        let (x, y) = (88 + i as i32 * 240, TOP + 110);
        p.image(
            Rect::from_xywh(x, y, 64, 64),
            &px,
            &ImageDsc {
                angle: Angle::decideg((f % 360) as i32 * 10),
                scale_x: scale,
                scale_y: scale,
                pivot: Point::new(32, 32),
                antialias: aa,
                ..ImageDsc::default()
            },
        );
        let label = if aa { "bilinear" } else { "nearest" };
        p.fill(Rect::from_xywh(x - 28, H - 24, 120, 18), Color::WHITE, Opa::COVER);
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(x - 28, H - 23, 120, 18),
            label,
            TextAlign::Center,
        );
    }
    let zoom = format!("zoom {} %", u32::from(scale.0) * 100 / 256);
    p.fill(
        Rect::from_xywh(W / 2 - 50, TOP + 4, 100, 18),
        Color::WHITE,
        Opa::COVER,
    );
    text(
        p,
        cx.glyphs,
        Rect::from_xywh(W / 2 - 50, TOP + 5, 100, 18),
        &zoom,
        TextAlign::Center,
    );
}

fn page_decoders(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    let files: [(&str, &'static [u8]); 4] = [
        ("QOI", PHOTO_QOI),
        ("PNG", PHOTO_PNG),
        ("BMP", PHOTO_BMP),
        ("JPEG", PHOTO_JPG),
    ];
    for (i, (name, bytes)) in files.into_iter().enumerate() {
        let (col, row) = (i as i32 % 2, i as i32 / 2);
        let (x, y) = (48 + col * 208, TOP + 2 + row * 146);
        // Decoded images are shown at 2× (a transformed draw).
        cx.images.draw(
            p,
            &ImageSource::Encoded(bytes),
            x,
            y,
            &ImageDsc {
                scale_x: Scale(512),
                scale_y: Scale(512),
                antialias: false,
                ..ImageDsc::default()
            },
        );
        let label = format!("{name} ({} bytes)", bytes.len());
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(x, y + 129, 192, 16),
            &label,
            TextAlign::Center,
        );
    }
}

fn page_gif(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    checker(p, Rect::new(0, TOP, W, H));
    let now = cx.frame.time;
    let images = &mut *cx.images;
    let Some(gif) = images.gif.as_mut() else {
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(0, 150, W, 20),
            "GIF failed to load",
            TextAlign::Center,
        );
        return;
    };
    // Advance by the accumulated frame time (catching up at most one GIF loop).
    let mut steps = 0;
    while now >= images.gif_due && steps < gif.frame_count() {
        let d: Duration = gif.advance();
        images.gif_due = if images.gif_due == Instant::ZERO {
            now + d
        } else {
            images.gif_due + d
        };
        steps += 1;
    }
    let px = gif.pixels();
    p.image(Rect::from_xywh(90, TOP + 110, 48, 48), &px, &ImageDsc::default());
    p.image(
        Rect::from_xywh(290, TOP + 110, 48, 48),
        &px,
        &ImageDsc {
            scale_x: Scale(768),
            scale_y: Scale(768),
            pivot: Point::new(24, 24),
            ..ImageDsc::default()
        },
    );
    let label = format!("frame {} / {}", gif.frame_index() + 1, gif.frame_count());
    p.fill(
        Rect::from_xywh(W / 2 - 60, H - 24, 120, 18),
        Color::WHITE,
        Opa::COVER,
    );
    text(
        p,
        cx.glyphs,
        Rect::from_xywh(W / 2 - 60, H - 23, 120, 18),
        &label,
        TextAlign::Center,
    );
}

fn page_compressed(p: &mut Painter<'_>, cx: &mut Ctx<'_>) {
    checker(p, Rect::new(0, TOP, W, H - 40));
    let list: [(&str, &'static Image); 4] = [
        ("RGB565A8 + RLE", &img::logo_rle::LOGO_RLE),
        ("ARGB8888 + LZ4", &img::logo_lz4::LOGO_LZ4),
        ("RGB565 + RLE", &img::photo_rle::PHOTO_RLE),
        ("RGB565 + LZ4", &img::photo_lz4::PHOTO_LZ4),
    ];
    for (i, (name, img)) in list.into_iter().enumerate() {
        let x = 16 + i as i32 * 116;
        let y = TOP + 40 + if img.header.w == 64 { 16 } else { 0 };
        cx.images
            .draw(p, &ImageSource::Static(img), x, y, &ImageDsc::default());
        let stored = match img.data {
            twine_image::ImageData::Compressed { data, .. } => data.len(),
            _ => 0,
        };
        let label = format!("{name}\n{stored} / {} B", img.header.data_size());
        p.fill(
            Rect::from_xywh(x - 8, TOP + 130, 112, 34),
            Color::WHITE,
            Opa::COVER,
        );
        text(
            p,
            cx.glyphs,
            Rect::from_xywh(x - 8, TOP + 131, 112, 34),
            &label,
            TextAlign::Center,
        );
    }
    let s = cx.images.cache.stats();
    let line = format!(
        "image cache: {} hits, {} misses, {} evictions, {} of {} bytes used",
        s.hits,
        s.misses,
        s.evictions,
        s.bytes_used,
        cx.images.cache.budget()
    );
    let mut d = TextDsc::new(&MONTSERRAT_10);
    d.color = INK;
    d.align = TextAlign::Center;
    draw_text(p, Rect::from_xywh(0, H - 30, W, 16), &line, &d, cx.glyphs);
}

struct Gallery {
    page: usize,
    since: u32,
    caches: RenderCaches,
    glyphs: GlyphCache,
    images: Images,
    announced: bool,
}

impl Gallery {
    fn announce(&self) {
        let p = &PAGES[self.page];
        twine_core::info!(target: "twine::image", "page {}: {}", self.page + 1, p.title);
        twine_core::info!(target: "twine::image", "  look at: {}", p.hint);
    }

    fn input(&mut self, frame: &SimFrame) {
        let mut delta = 0i32;
        for k in &frame.keys {
            match k {
                Key::Right => delta += 1,
                Key::Left => delta -= 1,
                _ => {}
            }
        }
        if let Some(pt) = frame.clicked {
            if pt.x < W / 3 {
                delta -= 1;
            } else if pt.x >= 2 * W / 3 {
                delta += 1;
            }
        }
        if delta == 0 && frame.index.saturating_sub(self.since) >= AUTO_FRAMES {
            delta = 1;
        }
        if delta != 0 {
            self.page = (self.page as i32 + delta).rem_euclid(PAGES.len() as i32) as usize;
            self.since = frame.index;
            self.announce();
        }
    }

    fn draw(&mut self, fb: &mut [u8], format: ColorFormat, frame: &SimFrame) {
        if !self.announced {
            self.announced = true;
            self.since = frame.index;
            self.announce();
        }
        self.input(frame);
        let area = Rect::from_xywh(0, 0, W, H);
        let Ok(buf) = DrawBuf::new_packed(fb, format, area) else {
            twine_core::warn!(target: "twine::image", "cannot draw into {}", format);
            return;
        };
        let mut p = Painter::new(buf, &mut self.caches);
        p.fill(area, BG, Opa::COVER);
        p.fill(Rect::from_xywh(0, 0, W, TOP - 4), ACCENT, Opa::COVER);
        let page = &PAGES[self.page];
        let title = format!("{}/{}  {}", self.page + 1, PAGES.len(), page.title);
        let mut d = TextDsc::new(&MONTSERRAT_16);
        d.color = Color::WHITE;
        draw_text(
            &mut p,
            Rect::from_xywh(8, 1, W - 100, TOP - 4),
            &title,
            &d,
            &mut self.glyphs,
        );
        d.align = TextAlign::Right;
        d.font = &MONTSERRAT_12;
        let nav = format!("{}  {}", symbols::LEFT, symbols::RIGHT);
        draw_text(
            &mut p,
            Rect::new(W - 90, 4, W - 8, TOP - 4),
            &nav,
            &d,
            &mut self.glyphs,
        );
        let mut cx = Ctx {
            glyphs: &mut self.glyphs,
            images: &mut self.images,
            frame,
        };
        p.with_clip(Rect::new(0, TOP - 4, W, H), |p| (page.draw)(p, &mut cx));
    }
}

fn main() {
    let cfg = SimConfig::new(W as u16, H as u16).title("image_gallery").scale(2);
    let gif = match GifPlayer::new(SPINNER_GIF) {
        Ok(g) => Some(g),
        Err(e) => {
            twine_core::warn!(target: "twine::image", "spinner.gif: {}", e);
            None
        }
    };
    let mut g = Gallery {
        page: 0,
        since: 0,
        caches: RenderCaches::new(&RenderConfig::default()),
        glyphs: GlyphCache::default(),
        images: Images {
            cache: ImageCache::new(CACHE_BYTES, 16),
            headers: ImageHeaderCache::new(),
            registry: DecoderRegistry::with_defaults(),
            gif,
            gif_due: Instant::ZERO,
        },
        announced: false,
    };
    show_framebuffer_with_input(cfg, move |fb, format, frame| g.draw(fb, format, frame));
}
