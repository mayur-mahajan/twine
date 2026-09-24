//! Alignment, ellipsis and hit testing on a [`TextLayout`].
//!
//! Coordinates are relative to the text area's top-left corner:
//!
//! ```text
//!  (0,0) ─────────────── area_w ───────────────►  x
//!    │   ┌──────────────────────────────────────┐
//!    │   │"Hello "            ← line 0, y = 0    │  Left:   line_x = 0
//!    │   │      "wonderful"   ← line 1           │  Center: line_x = (area_w − width) / 2
//!    │   │            "world" ← line 2           │  Right:  line_x = area_w − width
//!    ▼   └──────────────────────────────────────┘
//!    y   line k: top = k × (line_height + line_space)
//!
//!  pos_of(i)  = top-left of the cursor before byte i  (x = line_x + pen position)
//!  char_at(p) = the char boundary closest to p        (x past a glyph's middle → after it)
//! ```

use core::ops::Range;

use bitflags::bitflags;
use twine_core::Point;

use crate::layout::{Line, Pen, TextFlags, TextLayout, floor_boundary};

/// Horizontal alignment of lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TextAlign {
    /// Left edge.
    #[default]
    Left,
    /// Centered.
    Center,
    /// Right edge.
    Right,
    /// By base direction: right for right-to-left text, else left (see
    /// [`TextAlign::resolve`]; [`draw_text`](crate::draw_text) resolves it with
    /// [`TextDsc::base_dir`](crate::TextDsc::base_dir), layout functions that get no direction
    /// treat it as left).
    Auto,
}

bitflags! {
    /// Text decorations.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct TextDecor: u8 {
        /// Line under the baseline.
        const UNDERLINE = 1;
        /// Line through the glyphs.
        const STRIKETHROUGH = 2;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for TextDecor {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "TextDecor({=u8:#x})", self.bits());
    }
}

/// How a label handles text longer than its area (the behaviour lives in the label widget).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum LongMode {
    /// Wrap into lines; the label grows in height.
    #[default]
    Wrap,
    /// Cut and end with "...".
    Dots,
    /// Scroll back and forth.
    Scroll,
    /// Scroll in a circle.
    ScrollCircular,
    /// Clip.
    Clip,
}

/// The result of [`TextLayout::ellipsize`]: the last visible line and the part of it to keep
/// before the `"..."`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ellipsis {
    /// Index of the last visible line.
    pub line_index: usize,
    /// Bytes of that line drawn before `"..."`.
    pub keep: Range<usize>,
}

/// The ellipsis string.
pub const DOTS: &str = "...";

impl TextLayout<'_> {
    /// X of `line`'s left edge in an area `area_w` wide. Left: 0, Center:
    /// `(area_w − width).div_euclid(2)`, Right: `area_w − width`. Like LVGL, the result is not
    /// clamped: it is negative when the line is wider than the area.
    #[must_use]
    pub fn line_x(&self, line: &Line, area_w: i32, align: TextAlign) -> i32 {
        match align {
            TextAlign::Left | TextAlign::Auto => 0,
            TextAlign::Center => (area_w - line.width).div_euclid(2),
            TextAlign::Right => area_w - line.width,
        }
    }

    /// Width of `"..."` (letter spacing included).
    #[must_use]
    pub fn dots_width(&self) -> i32 {
        let dot = self.font.advance_px('.', Some('.'));
        let last = self.font.advance_px('.', None);
        let mut pen = Pen::default();
        pen.add(dot, self.letter_space);
        pen.add(dot, self.letter_space);
        pen.add(last, self.letter_space);
        pen.width(self.letter_space)
    }

    /// If the text needs more than `max_lines` lines (at least 1), the last visible line keeps
    /// its longest prefix for which `width(prefix) + letter_space + width("...") ≤ max_width`,
    /// without trailing spaces. `None` when the text fits.
    #[must_use]
    pub fn ellipsize(&self, max_lines: usize) -> Option<Ellipsis> {
        let max_lines = max_lines.max(1);
        let mut it = self.lines();
        let line = it.nth(max_lines - 1)?;
        it.next()?;
        let ls = self.letter_space;
        let budget = if self.flags.contains(TextFlags::EXPAND) {
            i32::MAX
        } else {
            self.max_width
        }
        .saturating_sub(self.dots_width());
        let mut pen = Pen::default();
        let mut end = line.range.start;
        for (i, c) in self.text[line.range.clone()].char_indices() {
            let b = line.range.start + i;
            // The prefix's last glyph is followed by '.' when drawn.
            let mut with_dot = pen;
            with_dot.add(self.font.advance_px(c, Some('.')), ls);
            // `with_dot.x` includes the letter space before the dots.
            if with_dot.x > budget {
                break;
            }
            end = b + c.len_utf8();
            pen.add(self.adv(c, b), ls);
        }
        let keep_end = line.range.start + self.text[line.range.start..end].trim_end_matches(' ').len();
        Some(Ellipsis {
            line_index: max_lines - 1,
            keep: line.range.start..keep_end,
        })
    }

    /// Index of the line containing byte `byte_index` (rounded down to a char boundary). An
    /// index at the end of a wrapped line belongs to the next line; one inside a line's
    /// newline belongs to that line.
    #[must_use]
    pub fn line_of(&self, byte_index: usize) -> usize {
        self.find_line(byte_index).0
    }

    fn find_line(&self, byte_index: usize) -> (usize, Line) {
        let i = floor_boundary(self.text, byte_index);
        let mut it = self.lines();
        let mut k = 0;
        let mut line = it.next().unwrap_or_default();
        loop {
            let next_start = it.next_start();
            if i < next_start {
                return (k, line);
            }
            match it.next() {
                Some(l) => {
                    line = l;
                    k += 1;
                }
                None => return (k, line),
            }
        }
    }

    fn line_at(&self, index: usize) -> Line {
        let mut last = Line::default();
        for (k, l) in self.lines().enumerate() {
            if k == index {
                return l;
            }
            last = l;
        }
        last
    }

    /// Byte index of the char boundary closest to `p` (relative to the area's top-left): the
    /// line is chosen by `y` (clamped to the first/last line), then the glyphs are walked; past
    /// a glyph's midpoint the boundary after it is returned.
    #[must_use]
    pub fn char_at(&self, p: Point, align: TextAlign, area_w: i32) -> usize {
        let lh = self.line_height();
        let idx = if p.y <= 0 || lh <= 0 {
            0
        } else {
            (p.y / lh) as usize
        };
        let line = self.line_at(idx);
        let x = p.x - self.line_x(&line, area_w, align);
        let mut pen = Pen::default();
        for (i, c) in self.text[line.range.clone()].char_indices() {
            let b = line.range.start + i;
            let a = self.adv(c, b);
            if x < pen.x + (a.max(0) + 1) / 2 {
                return b;
            }
            pen.add(a, self.letter_space);
        }
        line.range.end
    }

    /// Top-left of the cursor before byte `byte_index` (rounded down to a char boundary): the
    /// start x of that glyph, or the line's end x at the end of a line.
    #[must_use]
    pub fn pos_of(&self, byte_index: usize, align: TextAlign, area_w: i32) -> Point {
        let i = floor_boundary(self.text, byte_index);
        let (k, line) = self.find_line(i);
        let end = i.clamp(line.range.start, line.range.end);
        let mut pen = Pen::default();
        for (j, c) in self.text[line.range.start..end].char_indices() {
            pen.add(self.adv(c, line.range.start + j), self.letter_space);
        }
        Point::new(
            self.line_x(&line, area_w, align) + pen.x,
            k as i32 * self.line_height(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_font::TEST_FONT;
    use alloc::vec;
    use proptest::prelude::*;

    fn layout(text: &str, w: i32) -> TextLayout<'_> {
        let mut l = TextLayout::new(text, &TEST_FONT);
        l.max_width = w;
        l
    }

    #[test]
    fn center_align_offsets() {
        let l = layout("CC", 100);
        let line = l.lines().next().unwrap();
        assert_eq!(l.line_x(&line, 20, TextAlign::Center), 8);
        assert_eq!(l.line_x(&line, 5, TextAlign::Center), 0);
        assert_eq!(l.line_x(&line, 2, TextAlign::Center), -1, "not clamped, floors");
        assert_eq!(l.line_x(&line, 20, TextAlign::Left), 0);
        assert_eq!(l.line_x(&line, 20, TextAlign::Auto), 0);
    }

    #[test]
    fn right_align_offsets() {
        let l = layout("CC", 100);
        let line = l.lines().next().unwrap();
        assert_eq!(l.line_x(&line, 20, TextAlign::Right), 16);
        assert_eq!(l.line_x(&line, 1, TextAlign::Right), -3);
    }

    #[test]
    fn ellipsis_fits_with_dots() {
        // '.' is missing in TEST_FONT → 7 px each, "..." = 21. Lines of "CC CC CC" at 10 px:
        // "CC ", "CC ", "CC".
        let l = layout("CC CC CC", 30);
        assert_eq!(l.line_count(), 1);
        let l = layout("CC CC CC", 10);
        assert_eq!(l.line_count(), 3);
        // max_width 10 - 21 < 0: nothing kept.
        let e = l.ellipsize(2).unwrap();
        assert_eq!(e.line_index, 1);
        assert_eq!(e.keep, 3..3);
        // Wider area with lines broken by newlines.
        let l = layout("CCCC\nC\nC", 27);
        let e = l.ellipsize(1).unwrap();
        // 27 - 21 = 6 → "CCC" = 6 fits.
        assert_eq!(e.keep, 0..3);
        let mut l2 = l;
        l2.letter_space = 1;
        // "..." = 21 + 2 = 23, budget 4: "C"+ls = 3 ≤ 4, "CC"+ls = 6 > 4.
        assert_eq!(l2.ellipsize(1).unwrap().keep, 0..1);
    }

    #[test]
    fn ellipsis_drops_trailing_spaces() {
        let l = layout("C  C\nx", 30);
        // budget 30 - 21 = 9: "C" 2, "C " 6, "C  " 10 > 9 → "C " → trimmed to "C".
        assert_eq!(l.ellipsize(1).unwrap().keep, 0..1);
    }

    #[test]
    fn ellipsis_none_when_fits() {
        let l = layout("CC\nCC", 100);
        assert!(l.ellipsize(2).is_none());
        assert!(l.ellipsize(5).is_none());
        assert!(l.ellipsize(1).is_some());
    }

    #[test]
    fn char_at_clamps_outside() {
        let l = layout("CC\nCCC", 100);
        assert_eq!(l.char_at(Point::new(-10, -10), TextAlign::Left, 100), 0);
        assert_eq!(l.char_at(Point::new(1000, -10), TextAlign::Left, 100), 2);
        assert_eq!(l.char_at(Point::new(1000, 1000), TextAlign::Left, 100), 6);
        assert_eq!(l.char_at(Point::new(-5, 1000), TextAlign::Left, 100), 3);
        // Midpoint rule: C is 2 px wide; x = 0 → before, x = 1 → after.
        assert_eq!(l.char_at(Point::new(0, 0), TextAlign::Left, 100), 0);
        assert_eq!(l.char_at(Point::new(1, 0), TextAlign::Left, 100), 1);
    }

    #[test]
    fn pos_of_end_of_text() {
        let l = layout("CC\nCCC", 100);
        assert_eq!(l.pos_of(6, TextAlign::Left, 100), Point::new(6, 10));
        assert_eq!(l.pos_of(2, TextAlign::Left, 100), Point::new(4, 0));
        assert_eq!(l.pos_of(3, TextAlign::Left, 100), Point::new(0, 10));
        assert_eq!(l.pos_of(99, TextAlign::Right, 100), Point::new(100, 10));
        let l = layout("C\n", 100);
        assert_eq!(
            l.pos_of(2, TextAlign::Left, 100),
            Point::new(0, 10),
            "trailing empty line"
        );
        assert_eq!(l.line_of(2), 1);
        assert_eq!(l.line_of(1), 0);
    }

    #[test]
    fn line_of_wrapped_boundary_goes_to_next_line() {
        let l = layout("CC CC", 10);
        assert_eq!(l.line_of(2), 0);
        assert_eq!(l.line_of(3), 1);
        assert_eq!(l.pos_of(3, TextAlign::Left, 10), Point::new(0, 10));
    }

    #[test]
    fn non_boundary_index_rounds_down() {
        let l = layout("°C", 100);
        assert_eq!(
            l.pos_of(1, TextAlign::Left, 100),
            l.pos_of(0, TextAlign::Left, 100)
        );
        assert_eq!(l.line_of(1), 0);
        assert_eq!(l.line_width(0..1), 0);
    }

    proptest! {
        #[test]
        fn char_at_roundtrips_pos_of(
            chars in prop::collection::vec(prop::sample::select(vec!['C', ' ', 'x', '\n', '中', '°']), 0..40),
            w in 1i32..120, ls in 0i32..3, align in prop::sample::select(vec![TextAlign::Left, TextAlign::Center, TextAlign::Right]),
        ) {
            let text: alloc::string::String = chars.into_iter().collect();
            let mut l = layout(&text, w);
            l.letter_space = ls;
            for i in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
                let p = l.pos_of(i, align, w);
                prop_assert_eq!(l.char_at(p, align, w), i, "text {:?} index {}", text, i);
            }
        }
    }
}
