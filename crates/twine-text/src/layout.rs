//! [`TextLayout`]: line breaking and measuring (LVGL `lv_text_get_next_line` rules).
//!
//! ```text
//!   area origin (0, 0)
//!   ┌──────────────────────────────── max_width ───────────────┐
//!   │ line 0: "The quick brown "      width excludes the trailing space
//!   │ ← line_x →┌───────────┐  line 1 top = 1 × (line_height + line_space)
//!   │           │ fox jumps │  (line_x from TextLayout::line_x per alignment)
//!   │           └───────────┘
//!   │ line 2 …
//! ```
//!
//! All positions are relative to the text area's top-left corner. Line `k` spans
//! `y ∈ [k × (line_height + line_space), … + line_height)`; x = 0 is the area's left edge before
//! alignment.
//!
//! **Widths.** A glyph advances by [`Font::advance_px`] (whole pixels, LVGL rounding) with
//! kerning against the next character of the text, plus `letter_space` after every glyph with
//! a non-zero advance; the width of a slice excludes the last letter space. `\n` and `\r` are
//! zero-width.

use core::ops::Range;

use bitflags::bitflags;
use twine_core::Size;

use crate::font::Font;

bitflags! {
    /// Line-breaking flags.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct TextFlags: u8 {
        /// No wrapping: only newlines break lines.
        const EXPAND = 1;
        /// Lines may break between any two characters.
        const BREAK_ALL = 2;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for TextFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "TextFlags({=u8:#x})", self.bits());
    }
}

/// A line may break **after** these characters (LVGL `LV_TXT_BREAK_CHARS`).
pub const BREAK_CHARS: &[char] = &[' ', ',', '.', ';', ':', '-', '_', ')', ']', '}'];

/// Whether `c` is a wide (CJK-like) character that may break anywhere: U+2E80–U+9FFF,
/// U+AC00–U+D7AF, U+F900–U+FAFF, U+FF00–U+FFEF.
#[must_use]
pub const fn is_wide(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// One laid-out line: its byte range (without the terminating newline) and its width in px
/// (without trailing spaces).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Line {
    /// Byte range of the line's text in [`TextLayout::text`].
    pub range: Range<usize>,
    /// Width in px, trailing spaces excluded.
    pub width: i32,
}

/// Text plus the parameters that determine its layout.
///
/// ```
/// use twine_assets::fonts::MONTSERRAT_14;
/// use twine_text::TextLayout;
///
/// let mut l = TextLayout::new("Hello world", &MONTSERRAT_14);
/// let one_line = l.measure();
/// l.max_width = one_line.w - 1;
/// let lines: Vec<_> = l.lines().map(|line| &l.text[line.range]).collect();
/// assert_eq!(lines, ["Hello ", "world"]);
/// assert_eq!(l.measure().h, 2 * i32::from(MONTSERRAT_14.line_height));
/// ```
#[derive(Clone, Copy, Debug)]
pub struct TextLayout<'a> {
    /// The UTF-8 text.
    pub text: &'a str,
    /// The font.
    pub font: &'static Font,
    /// Extra space after every glyph (px, may be negative).
    pub letter_space: i32,
    /// Extra space between lines (px, may be negative).
    pub line_space: i32,
    /// Width available for a line (ignored with [`TextFlags::EXPAND`]).
    pub max_width: i32,
    /// Breaking flags.
    pub flags: TextFlags,
}

/// Rounds `i` down to a char boundary of `s` (and clamps it to `s.len()`).
#[must_use]
pub(crate) fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Pen accumulator implementing the width rule.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pen {
    /// Sum of `advance + letter_space` of every glyph with a non-zero advance.
    pub x: i32,
    any: bool,
}

impl Pen {
    /// Advances by a glyph of advance `a`.
    #[inline]
    pub fn add(&mut self, a: i32, ls: i32) {
        if a > 0 {
            self.x += a + ls;
            self.any = true;
        }
    }

    /// The width so far (the last letter space excluded).
    #[inline]
    pub fn width(self, ls: i32) -> i32 {
        if self.any { self.x - ls } else { 0 }
    }
}

impl<'a> TextLayout<'a> {
    /// A layout of `text` in `font` with no letter/line spacing, unlimited width and no flags.
    #[must_use]
    pub fn new(text: &'a str, font: &'static Font) -> Self {
        Self {
            text,
            font,
            letter_space: 0,
            line_space: 0,
            max_width: i32::MAX,
            flags: TextFlags::empty(),
        }
    }

    /// The character after byte `i` (kerning partner), `None` at the end.
    #[inline]
    pub(crate) fn char_at_byte(&self, i: usize) -> Option<char> {
        self.text.get(i..).and_then(|s| s.chars().next())
    }

    /// Advance of the char `c` that starts at byte `b`, kerned with the following char.
    #[inline]
    pub(crate) fn adv(&self, c: char, b: usize) -> i32 {
        self.font.advance_px(c, self.char_at_byte(b + c.len_utf8()))
    }

    /// The lines (always at least one; an empty text is one empty line).
    #[must_use]
    pub fn lines(&self) -> LineIter<'_, 'a> {
        LineIter {
            layout: self,
            pos: 0,
            state: IterState::Start,
        }
    }

    /// Width of the slice `range` laid out on one line (kerning against the character after the
    /// slice included). Indices are rounded down to char boundaries.
    #[must_use]
    pub fn line_width(&self, range: Range<usize>) -> i32 {
        let s = floor_boundary(self.text, range.start);
        let e = floor_boundary(self.text, range.end).max(s);
        let mut pen = Pen::default();
        for (i, c) in self.text[s..e].char_indices() {
            pen.add(self.adv(c, s + i), self.letter_space);
        }
        pen.width(self.letter_space)
    }

    /// Size of the laid-out text: the widest line and the total height
    /// (`n × line_height + (n − 1) × line_space`).
    #[must_use]
    pub fn measure(&self) -> Size {
        let mut n = 0i32;
        let mut w = 0;
        for l in self.lines() {
            n += 1;
            w = w.max(l.width);
        }
        Size::new(
            w,
            n * i32::from(self.font.line_height) + (n - 1) * self.line_space,
        )
    }

    /// Number of lines (≥ 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines().count()
    }

    /// Distance between the tops of two lines: `line_height + line_space`.
    #[must_use]
    pub fn line_height(&self) -> i32 {
        i32::from(self.font.line_height) + self.line_space
    }

    /// Computes the line starting at byte `s`; returns it and the start of the next line.
    fn next_line(&self, s: usize) -> (Line, usize) {
        let text = self.text;
        let ls = self.letter_space;
        let wrap = !self.flags.contains(TextFlags::EXPAND);
        let break_all = self.flags.contains(TextFlags::BREAK_ALL);
        let mut pen = Pen::default();
        let mut w_ns = 0; // width through the last non-space char
        let mut last_break: Option<(usize, i32)> = None;
        let mut iter = text[s..].char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            let b = s + i;
            let len = c.len_utf8();
            if c == '\n' || c == '\r' {
                let nl = if c == '\r' && iter.peek().is_some_and(|&(_, n)| n == '\n') {
                    2
                } else {
                    1
                };
                return (
                    Line {
                        range: s..b,
                        width: w_ns,
                    },
                    b + nl,
                );
            }
            if b > s && (break_all || is_wide(c)) {
                last_break = Some((b, w_ns));
            }
            let mut after = pen;
            after.add(self.adv(c, b), ls);
            let wc = after.width(ls);
            if wrap && c != ' ' && wc > self.max_width {
                if let Some((bp, bw)) = last_break {
                    return (
                        Line {
                            range: s..bp,
                            width: bw,
                        },
                        bp,
                    );
                }
                if b > s {
                    return (
                        Line {
                            range: s..b,
                            width: w_ns,
                        },
                        b,
                    );
                }
                // A single character wider than the line: it gets a line of its own; a newline
                // right after it ends this line.
                let mut next = b + len;
                match self.char_at_byte(next) {
                    Some('\r') if self.char_at_byte(next + 1) == Some('\n') => next += 2,
                    Some('\n' | '\r') => next += 1,
                    _ => {}
                }
                return (
                    Line {
                        range: s..b + len,
                        width: wc,
                    },
                    next,
                );
            }
            pen = after;
            if c != ' ' {
                w_ns = wc;
            }
            if BREAK_CHARS.contains(&c) || is_wide(c) {
                last_break = Some((b + len, w_ns));
            }
        }
        (
            Line {
                range: s..text.len(),
                width: w_ns,
            },
            text.len(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IterState {
    Start,
    Running,
    /// The previous line ended with a newline at the very end: one empty line follows.
    TrailingEmpty,
    Done,
}

/// Iterator over the [`Line`]s of a [`TextLayout`]; holds indices only (no allocation).
#[derive(Clone, Debug)]
pub struct LineIter<'l, 'a> {
    layout: &'l TextLayout<'a>,
    pos: usize,
    state: IterState,
}

impl LineIter<'_, '_> {
    /// Byte index where the next line starts (after the last yielded line and its newline).
    #[must_use]
    pub fn next_start(&self) -> usize {
        self.pos
    }
}

impl Iterator for LineIter<'_, '_> {
    type Item = Line;

    fn next(&mut self) -> Option<Line> {
        let len = self.layout.text.len();
        match self.state {
            IterState::Done => return None,
            IterState::TrailingEmpty => {
                self.state = IterState::Done;
                return Some(Line {
                    range: len..len,
                    width: 0,
                });
            }
            IterState::Start if len == 0 => {
                self.state = IterState::Done;
                return Some(Line::default());
            }
            IterState::Start | IterState::Running => {}
        }
        if self.pos >= len {
            self.state = IterState::Done;
            return None;
        }
        let (line, next) = self.layout.next_line(self.pos);
        debug_assert!(next > self.pos, "line breaking must progress");
        let ended_by_newline = next > line.range.end;
        self.pos = next;
        self.state = if next >= len {
            if ended_by_newline {
                IterState::TrailingEmpty
            } else {
                IterState::Done
            }
        } else {
            IterState::Running
        };
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_font::TEST_FONT;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use proptest::prelude::*;

    // TEST_FONT: 'A' 0 px (7/16), 'B' 1 px, 'C' 2 px, ' ' 4 px, '°' 2 px, '•' 2 px; everything
    // else is missing and advances by the placeholder width 5 + 2 = 7 px.

    fn lines(l: &TextLayout<'_>) -> Vec<(String, i32)> {
        l.lines()
            .map(|x| (String::from(&l.text[x.range]), x.width))
            .collect()
    }

    fn layout(text: &str, w: i32) -> TextLayout<'_> {
        let mut l = TextLayout::new(text, &TEST_FONT);
        l.max_width = w;
        l
    }

    #[test]
    fn single_line_fits() {
        let l = layout("CC CC", 100);
        assert_eq!(lines(&l), [("CC CC".into(), 12)]);
        assert_eq!(l.measure(), Size::new(12, 10));
    }

    #[test]
    fn wrap_at_space() {
        // "CC " = 8 wide, "CC CC" = 12.
        assert_eq!(lines(&layout("CC CC", 10)), [("CC ".into(), 4), ("CC".into(), 4)]);
    }

    #[test]
    fn trailing_space_not_counted() {
        assert_eq!(lines(&layout("CC   ", 4)), [("CC   ".into(), 4)]);
        assert_eq!(layout("CC  ", 100).measure().w, 4);
    }

    #[test]
    fn break_after_hyphen() {
        // "xx-" = 7+7+7 = 21, "xx-xx" = 35.
        assert_eq!(
            lines(&layout("xx-xx", 25)),
            [("xx-".into(), 21), ("xx".into(), 14)]
        );
    }

    #[test]
    fn long_word_split_at_last_fitting_char() {
        assert_eq!(
            lines(&layout("xxxxx", 15)),
            [("xx".into(), 14), ("xx".into(), 14), ("x".into(), 7)]
        );
        // A single character wider than the line still makes progress.
        assert_eq!(lines(&layout("xx", 3)), [("x".into(), 7), ("x".into(), 7)]);
        assert_eq!(lines(&layout("x\ny", 3)), [("x".into(), 7), ("y".into(), 7)]);
    }

    #[test]
    fn cjk_breaks_anywhere() {
        // Each CJK char is a missing glyph (7 px) and a break opportunity.
        assert_eq!(
            lines(&layout("中文字", 15)),
            [("中文".into(), 14), ("字".into(), 7)]
        );
        assert!(is_wide('中') && is_wide('한') && is_wide('！') && !is_wide('a'));
    }

    #[test]
    fn newline_variants() {
        let l = layout("C\nC\r\nC\rC\n", 100);
        let got: Vec<_> = l.lines().map(|x| x.range).collect();
        assert_eq!(got, [0..1, 2..3, 5..6, 7..8, 9..9]);
        assert_eq!(l.line_count(), 5);
    }

    #[test]
    fn expand_ignores_width() {
        let mut l = layout("CC CC CC", 1);
        l.flags = TextFlags::EXPAND;
        assert_eq!(l.line_count(), 1);
    }

    #[test]
    fn break_all_breaks_anywhere() {
        let mut l = layout("xxxx", 15);
        l.flags = TextFlags::BREAK_ALL;
        assert_eq!(lines(&l), [("xx".into(), 14), ("xx".into(), 14)]);
    }

    #[test]
    fn empty_text_is_one_line() {
        let l = layout("", 100);
        assert_eq!(
            l.lines().collect::<Vec<_>>(),
            [Line {
                range: 0..0,
                width: 0
            }]
        );
        assert_eq!(l.measure(), Size::new(0, 10));
    }

    #[test]
    fn letter_space_not_added_after_last_glyph() {
        let mut l = layout("CCC", 100);
        l.letter_space = 3;
        assert_eq!(l.line_width(0..3), 2 * 3 + 2 * 3);
        assert_eq!(l.measure().w, 12);
        l.line_space = 4;
        l.text = "C\nC";
        assert_eq!(l.measure().h, 10 + 4 + 10);
    }

    #[test]
    fn kerning_affects_width() {
        // 'B' then 'A': B kerned +8/16 → 16/16 = 1 px; A = 0 px; without kerning 'B' = 1 px too,
        // so use 'A' 'B': A kerned −16 → clamps to 0.
        let l = layout("BA", 100);
        assert_eq!(l.line_width(0..2), 1);
        let l = layout("CB", 100);
        assert_eq!(l.line_width(0..2), 3);
        // The last glyph is kerned with the char after the slice (LVGL).
        let l = layout("BAB", 100);
        assert_eq!(l.line_width(0..1), 1);
    }

    #[test]
    fn non_boundary_range_is_rounded() {
        let l = layout("°C", 100);
        assert_eq!(l.line_width(1..3), l.line_width(0..3));
    }

    fn text_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec!['C', 'B', ' ', ' ', 'x', '\n', '中', '-', '°']),
            0..60,
        )
        .prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        #[test]
        fn lines_cover_all_bytes_in_order(text in text_strategy(), w in 1i32..300, all in any::<bool>()) {
            let mut l = layout(&text, w);
            if all { l.flags = TextFlags::BREAK_ALL; }
            let mut rebuilt = String::new();
            let mut prev_end = 0;
            for line in l.lines() {
                prop_assert!(line.range.start >= prev_end);
                // Bytes between lines are newlines.
                let gap = &text[prev_end..line.range.start];
                prop_assert!(gap.chars().all(|c| c == '\n'));
                rebuilt.push_str(gap);
                rebuilt.push_str(&text[line.range.clone()]);
                prev_end = line.range.end;
            }
            rebuilt.push_str(&text[prev_end..]);
            prop_assert_eq!(rebuilt, text);
        }

        #[test]
        fn each_line_width_le_max_unless_single_char(text in text_strategy(), w in 1i32..300) {
            let l = layout(&text, w);
            for line in l.lines() {
                let s = &text[line.range.clone()];
                prop_assert!(line.width <= w || s.trim_end_matches(' ').chars().count() == 1,
                    "line {:?} width {} > {}", s, line.width, w);
                prop_assert_eq!(line.width, l.line_width(line.range.start..line.range.start + s.trim_end_matches(' ').len()));
            }
        }

        #[test]
        fn always_progresses(text in text_strategy(), w in -5i32..300) {
            let l = layout(&text, w);
            let n = l.line_count();
            prop_assert!(n >= 1);
            prop_assert!(n <= text.len() + 1);
        }
    }
}
