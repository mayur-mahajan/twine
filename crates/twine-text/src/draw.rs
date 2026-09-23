//! Text drawing: [`draw_text`] with a [`TextDsc`] (LVGL `lv_draw_label` features).

use bitflags::bitflags;
use twine_core::{Color, Opa, Point, Rect};
use twine_render::{Painter, SubpxOrder};

use crate::cache::GlyphCache;
use crate::font::{Font, Subpx};
use crate::hit::{DOTS, TextAlign, TextDecor};
use crate::layout::{Pen, TextFlags, TextLayout};

bitflags! {
    /// Layout flags of a [`TextDsc`].
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct TextDrawFlags: u8 {
        /// No wrapping: only newlines break lines.
        const EXPAND = 1;
        /// Lines may break between any two characters.
        const BREAK_ALL = 2;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for TextDrawFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "TextDrawFlags({=u8:#x})", self.bits());
    }
}

/// How to draw a text (LVGL `lv_draw_label_dsc_t`).
#[derive(Clone, Copy, Debug)]
pub struct TextDsc {
    /// The font (its fallback chain is used for missing characters).
    pub font: &'static Font,
    /// Text color.
    pub color: Color,
    /// Opacity of everything drawn.
    pub opa: Opa,
    /// Line alignment inside the area.
    pub align: TextAlign,
    /// Underline / strikethrough.
    pub decor: TextDecor,
    /// Extra space after each glyph.
    pub letter_space: i32,
    /// Extra space between lines.
    pub line_space: i32,
    /// Selection start (byte index; with `sel_end`, the order does not matter).
    pub sel_start: Option<usize>,
    /// Selection end (exclusive).
    pub sel_end: Option<usize>,
    /// Text color of selected characters.
    pub sel_color: Color,
    /// Background of selected characters.
    pub sel_bg_color: Color,
    /// Layout flags.
    pub flags: TextDrawFlags,
    /// Draw at most this many lines; the last one ends with `"..."` when text is cut.
    pub ellipsis_lines: Option<usize>,
    /// Offset of the text inside the area (scrolling long modes).
    pub ofs: Point,
    /// Subpixel order of the panel (used by subpixel fonts).
    pub subpx_order: SubpxOrder,
}

impl TextDsc {
    /// Black, opaque, left-aligned text without decorations, spacing or selection.
    #[must_use]
    pub fn new(font: &'static Font) -> Self {
        Self {
            font,
            color: Color::BLACK,
            opa: Opa::COVER,
            align: TextAlign::Left,
            decor: TextDecor::empty(),
            letter_space: 0,
            line_space: 0,
            sel_start: None,
            sel_end: None,
            sel_color: Color::WHITE,
            sel_bg_color: Color::hex(0x0021_96F3),
            flags: TextDrawFlags::empty(),
            ellipsis_lines: None,
            ofs: Point::ZERO,
            subpx_order: SubpxOrder::Rgb,
        }
    }

    /// The layout this descriptor gives `text` in an area `width` px wide.
    #[must_use]
    pub fn layout<'a>(&self, text: &'a str, width: i32) -> TextLayout<'a> {
        let mut flags = TextFlags::empty();
        if self.flags.contains(TextDrawFlags::EXPAND) {
            flags |= TextFlags::EXPAND;
        }
        if self.flags.contains(TextDrawFlags::BREAK_ALL) {
            flags |= TextFlags::BREAK_ALL;
        }
        TextLayout {
            text,
            font: self.font,
            letter_space: self.letter_space,
            line_space: self.line_space,
            max_width: width,
            flags,
        }
    }
}

/// Draws one character with its pen at `(pen_x, line_top)`; returns nothing drawn when the
/// glyph is outside the clip.
#[allow(clippy::too_many_arguments)]
fn draw_char(
    p: &mut Painter<'_>,
    font: &'static Font,
    c: char,
    next: Option<char>,
    pen_x: i32,
    line_top: i32,
    color: Color,
    opa: Opa,
    order: SubpxOrder,
    cache: &mut GlyphCache,
) {
    let clip = p.clip();
    let Some((gfont, info)) = font.glyph(c, next) else {
        if !crate::font::has_placeholder(c) {
            return;
        }
        if cache.first_warning((font.provider_key(), u32::from(c))) {
            twine_core::warn!(target: "twine::text", "missing glyph U+{:04X}", u32::from(c));
        }
        let b = Rect::from_xywh(pen_x, line_top, font.placeholder_width(), font.ascent());
        if b.intersects(&clip) && !b.is_empty() {
            p.fill(Rect::new(b.x0, b.y0, b.x1, b.y0 + 1), color, opa);
            p.fill(Rect::new(b.x0, b.y1 - 1, b.x1, b.y1), color, opa);
            p.fill(Rect::new(b.x0, b.y0 + 1, b.x0 + 1, b.y1 - 1), color, opa);
            p.fill(Rect::new(b.x1 - 1, b.y0 + 1, b.x1, b.y1 - 1), color, opa);
        }
        return;
    };
    if info.box_w == 0 || info.box_h == 0 {
        return;
    }
    let (bw, bh) = (i32::from(info.box_w), i32::from(info.box_h));
    let (w_px, h_px) = match gfont.subpx {
        Subpx::None => (bw, bh),
        Subpx::Hor => ((bw + 2) / 3, bh),
        Subpx::Ver => (bw, (bh + 2) / 3),
    };
    let gx = pen_x + i32::from(info.ofs_x);
    // LVGL: y = line top + ascent (of the drawing font) − box height − ofs_y.
    let gy = line_top + font.ascent() - h_px - i32::from(info.ofs_y);
    let area = Rect::from_xywh(gx, gy, w_px, h_px);
    if !area.intersects(&clip) {
        return;
    }
    cache.count_render();
    let provider = gfont.provider;
    match gfont.subpx {
        Subpx::None => {
            provider.render_rows(&info, cache, &mut |y, row| {
                let gy = area.y0 + y as i32;
                if gy >= clip.y0 && gy < clip.y1 {
                    p.glyph_a8(Rect::from_xywh(area.x0, gy, w_px, 1), row, row.len(), color, opa);
                }
            });
        }
        Subpx::Hor => {
            let mut lcd = core::mem::take(&mut cache.lcd_row);
            lcd.resize(3 * w_px as usize, 0);
            provider.render_rows(&info, cache, &mut |y, row| {
                let gy = area.y0 + y as i32;
                if gy >= clip.y0 && gy < clip.y1 {
                    lcd.fill(0);
                    lcd[..row.len()].copy_from_slice(row);
                    p.glyph_lcd(
                        Rect::from_xywh(area.x0, gy, w_px, 1),
                        &lcd,
                        lcd.len(),
                        color,
                        opa,
                        order,
                    );
                }
            });
            cache.lcd_row = lcd;
        }
        Subpx::Ver => {
            let mut lcd = core::mem::take(&mut cache.lcd_row);
            lcd.resize(3 * w_px as usize, 0);
            lcd.fill(0);
            let rows = bh as usize;
            provider.render_rows(&info, cache, &mut |y, row| {
                let t = y % 3;
                for (x, v) in row.iter().enumerate() {
                    lcd[x * 3 + t] = *v;
                }
                if t == 2 || y + 1 == rows {
                    let gy = area.y0 + (y / 3) as i32;
                    if gy >= clip.y0 && gy < clip.y1 {
                        p.glyph_lcd(
                            Rect::from_xywh(area.x0, gy, w_px, 1),
                            &lcd,
                            lcd.len(),
                            color,
                            opa,
                            order,
                        );
                    }
                    lcd.fill(0);
                }
            });
            cache.lcd_row = lcd;
        }
    }
}

/// Draws `text` into `area` (clipped to it) with `dsc`.
///
/// Lines are laid out with the area's width (unless [`TextDrawFlags::EXPAND`]), start at
/// `area.y0 + dsc.ofs.y` and are aligned per `dsc.align`. Only visible lines are processed:
/// lines above the clip are skipped and drawing stops after the first line below it. Glyphs
/// are positioned like LVGL: `x = pen + ofs_x`, `y = line_top + ascent − box_h − ofs_y`.
/// Decorations follow LVGL `lv_draw_label.c`: underline at
/// `line_top + ascent − underline_position`, strikethrough at
/// `line_top + ascent × 2 / 3 + underline_thickness / 2`, both `max(1, thickness)` px tall and
/// as wide as the line's glyphs. Missing characters are drawn as a 1 px outlined box
/// (`line_height / 2` × `ascent`) and logged once. Never allocates.
///
/// ```
/// use twine_assets::fonts::MONTSERRAT_14;
/// use twine_core::{Color, ColorFormat, Rect};
/// use twine_render::{DrawBuf, Painter, RenderCaches};
/// use twine_text::{GlyphCache, TextDsc, draw_text};
///
/// let mut caches = RenderCaches::default();
/// let mut glyphs = GlyphCache::default();
/// let mut px = vec![255u8; 60 * 20];
/// let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, Rect::from_xywh(0, 0, 60, 20)).unwrap();
/// let mut p = Painter::new(buf, &mut caches);
/// draw_text(&mut p, Rect::from_xywh(0, 0, 60, 20), "Hi!", &TextDsc::new(&MONTSERRAT_14), &mut glyphs);
/// drop(p);
/// assert!(px.iter().any(|&v| v < 128), "some dark text pixels");
/// ```
pub fn draw_text(p: &mut Painter<'_>, area: Rect, text: &str, dsc: &TextDsc, cache: &mut GlyphCache) {
    if dsc.opa.is_transparent() || area.is_empty() {
        return;
    }
    p.with_clip(area, |p| draw_clipped(p, area, text, dsc, cache));
}

fn draw_clipped(p: &mut Painter<'_>, area: Rect, text: &str, dsc: &TextDsc, cache: &mut GlyphCache) {
    let clip = p.clip();
    if clip.is_empty() {
        return;
    }
    let font = dsc.font;
    let layout = dsc.layout(text, area.width());
    let ls = dsc.letter_space;
    let font_lh = i32::from(font.line_height);
    let step = layout.line_height();
    let ellipsis = dsc.ellipsis_lines.and_then(|n| layout.ellipsize(n));
    let max_lines = dsc.ellipsis_lines.map_or(usize::MAX, |n| n.max(1));
    let sel = match (dsc.sel_start, dsc.sel_end) {
        (Some(a), Some(b)) if a != b => Some(a.min(b)..a.max(b)),
        _ => None,
    };
    let thick = i32::from(font.underline_thickness).max(1);
    let mut y = area.y0 + dsc.ofs.y;
    for (k, line) in layout.lines().enumerate() {
        if k >= max_lines {
            break;
        }
        if y + font_lh <= clip.y0 {
            y += step;
            continue;
        }
        if y >= clip.y1 {
            break;
        }
        let dots = ellipsis.as_ref().filter(|e| e.line_index == k);
        let range = dots.map_or(line.range.clone(), |e| e.keep.clone());
        let width = if dots.is_some() {
            // Aligned as if the kept text plus "..." were the line.
            let kept = layout.line_width(range.clone());
            if kept > 0 {
                kept + ls + layout.dots_width()
            } else {
                layout.dots_width()
            }
        } else {
            line.width
        };
        let line_obj = crate::layout::Line {
            range: range.clone(),
            width,
        };
        let x0 = area.x0 + dsc.ofs.x + layout.line_x(&line_obj, area.width(), dsc.align);
        let mut pen = Pen::default();
        let end = range.end;
        let chars = text[range.clone()]
            .char_indices()
            .map(|(i, c)| (range.start + i, c, false));
        let dot_chars = dots
            .into_iter()
            .flat_map(|_| DOTS.char_indices().map(|(i, c)| (end + i, c, true)));
        let mut it = chars.chain(dot_chars).peekable();
        while let Some((b, c, is_dot)) = it.next() {
            let next = if is_dot || b + c.len_utf8() < end {
                it.peek().map(|&(_, n, _)| n)
            } else if dots.is_some() {
                Some('.')
            } else {
                layout.char_at_byte(b + c.len_utf8())
            };
            let a = font.advance_px(c, next);
            let pen_x = x0 + pen.x;
            // Skip glyphs entirely right of the clip (they cannot come back into view).
            if ls >= 0 && pen_x - font_lh > clip.x1 {
                break;
            }
            let selected = !is_dot && sel.as_ref().is_some_and(|s| s.contains(&b));
            if selected && a > 0 {
                p.fill(Rect::from_xywh(pen_x, y, a, font_lh), dsc.sel_bg_color, dsc.opa);
            }
            if pen_x + a + font_lh >= clip.x0 {
                let color = if selected { dsc.sel_color } else { dsc.color };
                draw_char(p, font, c, next, pen_x, y, color, dsc.opa, dsc.subpx_order, cache);
            }
            pen.add(a, ls);
        }
        let w = pen.width(ls);
        if w > 0 {
            if dsc.decor.contains(TextDecor::UNDERLINE) {
                let uy = y + font.ascent() - i32::from(font.underline_position);
                p.fill(Rect::new(x0, uy, x0 + w, uy + thick), dsc.color, dsc.opa);
            }
            if dsc.decor.contains(TextDecor::STRIKETHROUGH) {
                let sy = y + font.ascent() * 2 / 3 + i32::from(font.underline_thickness) / 2;
                p.fill(Rect::new(x0, sy, x0 + w, sy + thick), dsc.color, dsc.opa);
            }
        }
        y += step;
    }
}
