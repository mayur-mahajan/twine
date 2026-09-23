//! `cargo xtask sim text_gallery`: fonts and text drawing.
//!
//! Pages: Montserrat sizes (two pages), alignment & wrapping, letter/line spacing,
//! decorations / selection / opacity, ellipsis, symbols, unscii, subpixel vs normal. Switch
//! pages with ←/→ or by clicking the left/right third of the screen; pages also advance every
//! 3 s. The page title is drawn at the top and logged with what to look at.
//!
//! Every panel format works (`TWINE_SIM_FORMAT=rgb565swapped|rgb888|xrgb8888|argb8888|l8|i1`).
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB

use twine_assets::fonts::*;
use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_hal::Key;
use twine_render::{DrawBuf, Painter, RenderCaches, RenderConfig, SubpxOrder};
use twine_sim::{SimConfig, SimFrame, show_framebuffer_with_input};
use twine_text::{Font, GlyphCache, TextAlign, TextDecor, TextDrawFlags, TextDsc, draw_text, symbols};

const W: i32 = 480;
const H: i32 = 320;
/// Height of the title bar.
const TOP: i32 = 26;
/// Frames between automatic page switches (3 s at 60 fps).
const AUTO_FRAMES: u32 = 180;

const BG: Color = Color::hex(0xFAFAFA);
const INK: Color = Color::hex(0x212121);
const MUTED: Color = Color::hex(0x757575);
const ACCENT: Color = Color::hex(0x1565C0);

struct Page {
    title: &'static str,
    hint: &'static str,
    draw: fn(&mut Painter<'_>, &mut GlyphCache),
}

const PAGES: &[Page] = &[
    Page {
        title: "Montserrat 8 - 28 px",
        hint: "every size renders crisply on a common left edge; kerning in 'AV'",
        draw: page_sizes_small,
    },
    Page {
        title: "Montserrat 30 - 48 px",
        hint: "large sizes (RLE-compressed bitmaps, decoded through the glyph cache)",
        draw: page_sizes_large,
    },
    Page {
        title: "Alignment & wrapping",
        hint: "left / center / right in bordered boxes, wrapping at spaces and hyphens, a non-wrapping line",
        draw: page_align,
    },
    Page {
        title: "Letter & line spacing",
        hint: "letter space -1 / 0 / 3 (columns) x line space -2 / 0 / 6 (rows)",
        draw: page_spacing,
    },
    Page {
        title: "Decorations, selection, opacity",
        hint: "underline, strikethrough, selection highlight, 25/50/75/100 % opacity",
        draw: page_decor,
    },
    Page {
        title: "Ellipsis",
        hint: "the same paragraph cut to 1, 2 and 3 lines with '...'",
        draw: page_ellipsis,
    },
    Page {
        title: "Symbols",
        hint: "every built-in symbol with its name (Font Awesome glyphs merged into Montserrat)",
        draw: page_symbols,
    },
    Page {
        title: "unscii 8 & 16",
        hint: "1 bpp pixel fonts: the ASCII table",
        draw: page_unscii,
    },
    Page {
        title: "Subpixel vs normal",
        hint: "Montserrat 14 grayscale (left) vs horizontal RGB subpixel (right), light and dark",
        draw: page_subpx,
    },
];

fn text(p: &mut Painter<'_>, c: &mut GlyphCache, font: &'static Font, color: Color, area: Rect, s: &str) {
    let mut d = TextDsc::new(font);
    d.color = color;
    draw_text(p, area, s, &d, c);
}

fn border(p: &mut Painter<'_>, r: Rect) {
    let c = Color::hex(0xBDBDBD);
    p.fill(Rect::new(r.x0 - 1, r.y0 - 1, r.x1 + 1, r.y0), c, Opa::COVER);
    p.fill(Rect::new(r.x0 - 1, r.y1, r.x1 + 1, r.y1 + 1), c, Opa::COVER);
    p.fill(Rect::new(r.x0 - 1, r.y0, r.x0, r.y1), c, Opa::COVER);
    p.fill(Rect::new(r.x1, r.y0, r.x1 + 1, r.y1), c, Opa::COVER);
}

fn size_list(
    p: &mut Painter<'_>,
    c: &mut GlyphCache,
    fonts: &[(&str, &'static Font)],
    x: i32,
    y0: i32,
    sample: &str,
) {
    let mut y = y0;
    for (name, font) in fonts {
        let lh = i32::from(font.line_height);
        text(
            p,
            c,
            &MONTSERRAT_10,
            MUTED,
            Rect::from_xywh(x, y + font.ascent() - 10, 24, 14),
            name,
        );
        text(
            p,
            c,
            font,
            INK,
            Rect::from_xywh(x + 26, y, W - x - 30, lh),
            sample,
        );
        y += lh;
    }
}

fn page_sizes_small(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let fonts: [(&str, &'static Font); 11] = [
        ("8", &MONTSERRAT_8),
        ("10", &MONTSERRAT_10),
        ("12", &MONTSERRAT_12),
        ("14", &MONTSERRAT_14),
        ("16", &MONTSERRAT_16),
        ("18", &MONTSERRAT_18),
        ("20", &MONTSERRAT_20),
        ("22", &MONTSERRAT_22),
        ("24", &MONTSERRAT_24),
        ("26", &MONTSERRAT_26),
        ("28", &MONTSERRAT_28),
    ];
    size_list(p, c, &fonts, 8, TOP + 4, "Aa Bb 0123 \u{b0} AV Wave");
}

fn page_sizes_large(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let left: [(&str, &'static Font); 5] = [
        ("30", &MONTSERRAT_30),
        ("32", &MONTSERRAT_32),
        ("34", &MONTSERRAT_34),
        ("36", &MONTSERRAT_36),
        ("38", &MONTSERRAT_38),
    ];
    let right: [(&str, &'static Font); 5] = [
        ("40", &MONTSERRAT_40),
        ("42", &MONTSERRAT_42),
        ("44", &MONTSERRAT_44),
        ("46", &MONTSERRAT_46),
        ("48", &MONTSERRAT_48),
    ];
    size_list(p, c, &left, 8, TOP + 4, "Aa 0\u{b0}");
    size_list(p, c, &right, 244, TOP + 2, "Aa 0\u{b0}");
}

const PARA: &str = "Twine wraps text at spaces and hyphens like LVGL: long-words-are-split where \
                    they must be, and a newline\nforces a break.";

fn page_align(p: &mut Painter<'_>, c: &mut GlyphCache) {
    for (i, align) in [TextAlign::Left, TextAlign::Center, TextAlign::Right]
        .into_iter()
        .enumerate()
    {
        let r = Rect::from_xywh(10 + i as i32 * 157, TOP + 10, 145, 210);
        border(p, r);
        let mut d = TextDsc::new(&MONTSERRAT_14);
        d.align = align;
        d.color = INK;
        draw_text(p, r.inset(twine_core::Insets::all(4)), PARA, &d, c);
    }
    let r = Rect::from_xywh(10, TOP + 236, 459, 44);
    border(p, r);
    let mut d = TextDsc::new(&MONTSERRAT_16);
    d.flags = TextDrawFlags::EXPAND;
    d.color = ACCENT;
    draw_text(
        p,
        r.inset(twine_core::Insets::all(4)),
        "EXPAND: this line never wraps, it is clipped at the box edge instead of breaking into two lines",
        &d,
        c,
    );
}

fn page_spacing(p: &mut Painter<'_>, c: &mut GlyphCache) {
    for (col, ls) in [-1, 0, 3].into_iter().enumerate() {
        for (row, lsp) in [-2, 0, 6].into_iter().enumerate() {
            let r = Rect::from_xywh(10 + col as i32 * 157, TOP + 22 + row as i32 * 90, 145, 82);
            border(p, r);
            let mut d = TextDsc::new(&MONTSERRAT_14);
            d.letter_space = ls;
            d.line_space = lsp;
            d.color = INK;
            let label = format!("letter {ls}, line {lsp}\nSpacing test\nWAVE text");
            draw_text(p, r.inset(twine_core::Insets::all(3)), &label, &d, c);
        }
    }
    text(
        p,
        c,
        &MONTSERRAT_12,
        MUTED,
        Rect::from_xywh(10, TOP + 4, 460, 16),
        "columns: letter space -1 / 0 / 3 px     rows: line space -2 / 0 / 6 px",
    );
}

fn page_decor(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let mut d = TextDsc::new(&MONTSERRAT_22);
    d.color = INK;
    d.decor = TextDecor::UNDERLINE;
    draw_text(p, Rect::from_xywh(12, TOP + 8, 460, 30), "Underlined text", &d, c);
    d.decor = TextDecor::STRIKETHROUGH;
    d.color = Color::hex(0xC62828);
    draw_text(p, Rect::from_xywh(12, TOP + 42, 460, 30), "Struck through", &d, c);
    d.decor = TextDecor::UNDERLINE | TextDecor::STRIKETHROUGH;
    d.color = INK;
    draw_text(p, Rect::from_xywh(12, TOP + 76, 460, 30), "Both at once", &d, c);

    let mut d = TextDsc::new(&MONTSERRAT_16);
    d.color = INK;
    d.sel_start = Some(10);
    d.sel_end = Some(38);
    draw_text(
        p,
        Rect::from_xywh(12, TOP + 116, 300, 50),
        "Selection highlights a byte range that spans a line break",
        &d,
        c,
    );

    p.fill(Rect::from_xywh(0, TOP + 176, W, 118), ACCENT, Opa::COVER);
    for (i, o) in [64u8, 128, 191, 255].into_iter().enumerate() {
        let mut d = TextDsc::new(&MONTSERRAT_20);
        d.opa = Opa(o);
        d.color = Color::WHITE;
        let label = format!("Opacity {} %", (u32::from(o) * 100 + 127) / 255);
        draw_text(
            p,
            Rect::from_xywh(
                12 + (i as i32 % 2) * 236,
                TOP + 184 + (i as i32 / 2) * 52,
                230,
                30,
            ),
            &label,
            &d,
            c,
        );
    }
}

fn page_ellipsis(p: &mut Painter<'_>, c: &mut GlyphCache) {
    for (i, n) in [1usize, 2, 3].into_iter().enumerate() {
        let r = Rect::from_xywh(10 + i as i32 * 157, TOP + 30, 145, 120);
        border(p, r);
        text(
            p,
            c,
            &MONTSERRAT_12,
            MUTED,
            Rect::from_xywh(r.x0, TOP + 8, 145, 16),
            &format!("{n} line(s)"),
        );
        let mut d = TextDsc::new(&MONTSERRAT_14);
        d.color = INK;
        d.ellipsis_lines = Some(n);
        draw_text(p, r.inset(twine_core::Insets::all(3)), PARA, &d, c);
    }
    let r = Rect::from_xywh(10, TOP + 170, 459, 30);
    border(p, r);
    let mut d = TextDsc::new(&MONTSERRAT_20);
    d.color = ACCENT;
    d.align = TextAlign::Center;
    d.ellipsis_lines = Some(1);
    draw_text(p, r.inset(twine_core::Insets::all(3)), PARA, &d, c);
}

fn page_symbols(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let cols = 6;
    let (cw, ch) = (W / cols, 26);
    let mut name_dsc = TextDsc::new(&MONTSERRAT_8);
    name_dsc.color = INK;
    name_dsc.flags = TextDrawFlags::EXPAND;
    for (i, (name, sym)) in symbols::NAMED.iter().enumerate() {
        let (col, row) = (i as i32 % cols, i as i32 / cols);
        let (x, y) = (col * cw + 4, TOP + 1 + row * ch);
        text(p, c, &MONTSERRAT_16, ACCENT, Rect::from_xywh(x, y, 22, 22), sym);
        draw_text(p, Rect::from_xywh(x + 22, y + 6, cw - 24, 12), name, &name_dsc, c);
    }
}

fn page_unscii(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let ascii: String = (0x20u8..0x7F).map(char::from).collect();
    let mut d = TextDsc::new(&UNSCII_8);
    d.color = INK;
    draw_text(p, Rect::from_xywh(10, TOP + 8, 460, 40), &ascii, &d, c);
    let mut d = TextDsc::new(&UNSCII_16);
    d.color = Color::hex(0x2E7D32);
    draw_text(p, Rect::from_xywh(10, TOP + 60, 460, 120), &ascii, &d, c);
    d.color = ACCENT;
    draw_text(
        p,
        Rect::from_xywh(10, TOP + 190, 460, 90),
        "unscii: 8x8 and 8x16 pixel fonts, 1 bpp,\nno anti-aliasing, no kerning.",
        &d,
        c,
    );
}

fn page_subpx(p: &mut Painter<'_>, c: &mut GlyphCache) {
    let sample = "Subpixel rendering\nHamburgefonstiv 0123\nThe quick brown fox";
    for (row, (bg, fg)) in [(Color::WHITE, INK), (Color::hex(0x202020), Color::WHITE)]
        .into_iter()
        .enumerate()
    {
        let y = TOP + 10 + row as i32 * 140;
        p.fill(Rect::from_xywh(0, y - 6, W, 130), bg, Opa::COVER);
        for (col, (font, label)) in [
            (&MONTSERRAT_14, "grayscale"),
            (&MONTSERRAT_14_SUBPX, "subpixel (RGB)"),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 12 + col as i32 * 236;
            text(p, c, &MONTSERRAT_10, MUTED, Rect::from_xywh(x, y, 220, 14), label);
            let mut d = TextDsc::new(font);
            d.color = fg;
            d.subpx_order = SubpxOrder::Rgb;
            draw_text(p, Rect::from_xywh(x, y + 18, 220, 100), sample, &d, c);
        }
    }
}

struct Gallery {
    page: usize,
    since: u32,
    caches: RenderCaches,
    glyphs: GlyphCache,
    announced: bool,
}

impl Gallery {
    fn announce(&self) {
        let p = &PAGES[self.page];
        twine_core::info!(target: "twine::text", "page {}: {}", self.page + 1, p.title);
        twine_core::info!(target: "twine::text", "  look at: {}", p.hint);
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
            twine_core::warn!(target: "twine::text", "cannot draw into {}", format);
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
        (page.draw)(&mut p, &mut self.glyphs);
    }
}

fn main() {
    let cfg = SimConfig::new(W as u16, H as u16).title("text_gallery").scale(2);
    let mut g = Gallery {
        page: 0,
        since: 0,
        caches: RenderCaches::new(&RenderConfig::default()),
        glyphs: GlyphCache::default(),
        announced: false,
    };
    show_framebuffer_with_input(cfg, move |fb, format, frame| g.draw(fb, format, frame));
}
