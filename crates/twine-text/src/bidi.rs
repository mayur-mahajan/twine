//! Bidirectional text (feature `bidi`): visual reordering of mixed left-to-right and
//! right-to-left lines (Unicode Bidirectional Algorithm via `unicode-bidi`), base direction
//! detection (LVGL `lv_bidi_detect_base_dir`) and bracket mirroring.
//!
//! Text is stored and edited in **logical** order; each laid-out line is reordered for display
//! only. [`BidiLine`] holds a line's runs in visual order: characters of an LTR run are drawn
//! first to last, those of an RTL run last to first (with brackets mirrored).
//!
//! Lines without any right-to-left character are never reordered (no allocation, no table
//! lookups beyond a range check). Reordering a line that contains RTL text allocates the
//! algorithm's temporary level tables.

use alloc::vec::Vec;
use core::ops::Range;

use twine_core::Point;
use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

use crate::dir::TextDir;
use crate::hit::TextAlign;
use crate::layout::{Line, TextLayout, floor_boundary};

/// Whether `c` has a strong or numeric right-to-left class (R, AL, AN). Characters below
/// U+0590 are never right-to-left, so Latin text costs one comparison per character.
#[must_use]
pub fn is_rtl_char(c: char) -> bool {
    u32::from(c) >= 0x0590 && matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::AN)
}

/// Whether `text` contains a right-to-left character (i.e. may need reordering).
#[must_use]
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_char)
}

/// The direction of the first strong character of `text` (LVGL `lv_bidi_detect_base_dir`):
/// [`TextDir::Rtl`] for Hebrew/Arabic letters, else [`TextDir::Ltr`] (also for text without
/// strong characters).
///
/// ```
/// use twine_text::{TextDir, detect_base_dir};
/// assert_eq!(detect_base_dir("123 שלום abc"), TextDir::Rtl);
/// assert_eq!(detect_base_dir("(abc) שלום"), TextDir::Ltr);
/// assert_eq!(detect_base_dir("123"), TextDir::Ltr);
/// ```
#[must_use]
pub fn detect_base_dir(text: &str) -> TextDir {
    for c in text.chars() {
        match bidi_class(c) {
            BidiClass::L => return TextDir::Ltr,
            BidiClass::R | BidiClass::AL => return TextDir::Rtl,
            _ => {}
        }
    }
    TextDir::Ltr
}

/// Resolves `dir` for `text`: [`TextDir::Auto`] becomes the detected direction.
#[must_use]
pub fn resolve_dir(dir: TextDir, text: &str) -> TextDir {
    match dir {
        TextDir::Auto => detect_base_dir(text),
        d => d,
    }
}

/// The mirrored form of a bracket drawn inside a right-to-left run (`(` ↔ `)`, `[` ↔ `]`,
/// `{` ↔ `}`, `<` ↔ `>`, `«` ↔ `»`); other characters are returned unchanged.
#[must_use]
pub const fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        c => c,
    }
}

/// A run of characters with one direction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BidiRun {
    /// Byte range in the text given to [`BidiLine::reorder`] / [`visual_runs`].
    pub range: Range<usize>,
    /// Whether the run is right-to-left (drawn last character first, brackets mirrored).
    pub rtl: bool,
}

/// One line's runs in visual (left-to-right display) order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BidiLine {
    runs: Vec<BidiRun>,
    base_rtl: bool,
}

/// The visual runs of `line` (one line of text, without newlines) with base direction `base`.
///
/// ```
/// use twine_text::{TextDir, visual_runs};
///
/// let line = "שלום 123";
/// let v = visual_runs(line, TextDir::Rtl);
/// // Visually: "123" at the left, then the Hebrew word (reversed) at the right.
/// let shown: String = v.visual_chars(line).map(|(_, c)| c).collect();
/// assert_eq!(shown, "123 םולש");
/// ```
#[must_use]
pub fn visual_runs(line: &str, base: TextDir) -> BidiLine {
    let mut l = BidiLine::default();
    l.reorder(line, base);
    l
}

impl BidiLine {
    /// Reorders `line` into this value (reusing its run buffer).
    pub fn reorder(&mut self, line: &str, base: TextDir) {
        self.runs.clear();
        let base = resolve_dir(base, line);
        self.base_rtl = base == TextDir::Rtl;
        if line.is_empty() {
            return;
        }
        if !self.base_rtl && !has_rtl(line) {
            self.runs.push(BidiRun {
                range: 0..line.len(),
                rtl: false,
            });
            return;
        }
        let level = if self.base_rtl { Level::rtl() } else { Level::ltr() };
        let info = ParagraphBidiInfo::new(line, Some(level));
        let (levels, runs) = info.visual_runs(0..line.len());
        self.runs.extend(runs.into_iter().map(|r| BidiRun {
            rtl: levels.get(r.start).is_some_and(Level::is_rtl),
            range: r,
        }));
    }

    /// The runs, left to right on screen.
    #[must_use]
    pub fn runs(&self) -> &[BidiRun] {
        &self.runs
    }

    /// Whether the (resolved) base direction is right-to-left.
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        self.base_rtl
    }

    /// The characters of `line` (the text this value was built from) in display order:
    /// `(logical byte index, char)`, brackets mirrored inside RTL runs.
    pub fn visual_chars<'a>(&'a self, line: &'a str) -> impl Iterator<Item = (usize, char)> + 'a {
        self.runs.iter().flat_map(move |r| {
            let s = line.get(r.range.clone()).unwrap_or("");
            let start = r.range.start;
            let fwd = (!r.rtl).then(|| s.char_indices().map(move |(i, c)| (start + i, c)));
            let back = r
                .rtl
                .then(|| s.char_indices().rev().map(move |(i, c)| (start + i, mirror(c))));
            fwd.into_iter().flatten().chain(back.into_iter().flatten())
        })
    }

    /// Whether the character at logical byte `b` lies in an RTL run.
    #[must_use]
    pub fn is_rtl_at(&self, b: usize) -> bool {
        self.runs.iter().any(|r| r.rtl && r.range.contains(&b))
    }
}

/// A glyph of a line in display order: logical byte, char (mirrored), x of its left edge
/// relative to the line start, advance, whether it is in an RTL run.
struct VisualGlyph {
    byte: usize,
    len: usize,
    x: i32,
    adv: i32,
    rtl: bool,
}

impl TextLayout<'_> {
    /// Walks `line` in display order, calling `f` per glyph (stops when `f` returns `true`).
    fn walk_visual(&self, line: &Line, base: TextDir, f: &mut dyn FnMut(&VisualGlyph) -> bool) {
        // Trailing spaces are excluded from the line width and not drawn.
        let text = self.text[line.range.clone()].trim_end_matches(' ');
        let bidi = visual_runs(text, base);
        let mut x = 0;
        let mut it = bidi.visual_chars(text).peekable();
        while let Some((b, c)) = it.next() {
            let next = it.peek().map(|&(_, n)| n);
            let adv = self.font.advance_px(c, next);
            let g = VisualGlyph {
                byte: line.range.start + b,
                len: c.len_utf8(),
                x,
                adv,
                rtl: bidi.is_rtl_at(b),
            };
            if f(&g) {
                break;
            }
            if adv > 0 {
                x += adv + self.letter_space;
            }
        }
    }

    /// Like [`pos_of`](Self::pos_of) for bidirectional text laid out with base direction
    /// `base`: the cursor before logical byte `byte_index` is at the left edge of its glyph in
    /// an LTR run and at the right edge in an RTL run; the end of a line is at the line's
    /// right end (LTR base) or left end (RTL base). [`TextAlign::Auto`] follows the base.
    #[must_use]
    pub fn pos_of_bidi(&self, byte_index: usize, align: TextAlign, area_w: i32, base: TextDir) -> Point {
        let i = floor_boundary(self.text, byte_index);
        let k = self.line_of(i);
        let line = self.lines().nth(k).unwrap_or_default();
        let base = resolve_dir(base, &self.text[line.range.clone()]);
        let line_x = self.line_x(&line, area_w, align.resolve(base));
        let y = k as i32 * self.line_height();
        let mut x = None;
        self.walk_visual(&line, base, &mut |g| {
            if g.byte == i {
                x = Some(if g.rtl { g.x + g.adv } else { g.x });
                return true;
            }
            false
        });
        let x = x.unwrap_or(if base == TextDir::Rtl { 0 } else { line.width });
        Point::new(line_x + x, y)
    }

    /// Like [`char_at`](Self::char_at) for bidirectional text: the logical byte index of the
    /// cursor position closest to `p` (inside an RTL glyph, the left half is after the
    /// character and the right half before it).
    #[must_use]
    pub fn char_at_bidi(&self, p: Point, align: TextAlign, area_w: i32, base: TextDir) -> usize {
        let lh = self.line_height();
        let idx = if p.y <= 0 || lh <= 0 {
            0
        } else {
            (p.y / lh) as usize
        };
        let mut last = Line::default();
        let mut line = None;
        for (k, l) in self.lines().enumerate() {
            if k == idx {
                line = Some(l);
                break;
            }
            last = l;
        }
        let line = line.unwrap_or(last);
        let base = resolve_dir(base, &self.text[line.range.clone()]);
        let x = p.x - self.line_x(&line, area_w, align.resolve(base));
        let mut hit = None;
        self.walk_visual(&line, base, &mut |g| {
            if x < g.x + (g.adv.max(0) + 1) / 2 {
                hit = Some(if g.rtl { g.byte + g.len } else { g.byte });
                return true;
            }
            if x < g.x + g.adv.max(1) {
                hit = Some(if g.rtl { g.byte } else { g.byte + g.len });
                return true;
            }
            false
        });
        hit.unwrap_or(if base == TextDir::Rtl {
            line.range.start
        } else {
            line.range.end
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn shown(line: &str, base: TextDir) -> String {
        visual_runs(line, base)
            .visual_chars(line)
            .map(|(_, c)| c)
            .collect()
    }

    #[test]
    fn ltr_text_is_one_run() {
        let v = visual_runs("hello world", TextDir::Ltr);
        assert_eq!(
            v.runs(),
            [BidiRun {
                range: 0..11,
                rtl: false
            }]
        );
        assert!(!v.is_rtl());
        assert!(visual_runs("", TextDir::Rtl).runs().is_empty());
    }

    #[test]
    fn bidi_hebrew_with_numbers_order() {
        // Numbers keep their order inside RTL text and sit visually left of the word.
        assert_eq!(shown("שלום 123", TextDir::Rtl), "123 םולש");
        assert_eq!(shown("שלום 123", TextDir::Auto), "123 םולש");
        // In an LTR paragraph the Hebrew word is reversed in place.
        // (numbers following Hebrew belong to its run, as the UBA specifies).
        assert_eq!(shown("abc שלום 12", TextDir::Ltr), "abc 12 םולש");
    }

    #[test]
    fn bidi_mixed_arabic_english_runs() {
        let line = "مرحبا Twine!";
        let v = visual_runs(line, TextDir::Rtl);
        // Visual order in an RTL paragraph: "!" (neutral, RTL context) then "Twine", then Arabic.
        let runs: alloc::vec::Vec<_> = v.runs().iter().map(|r| (&line[r.range.clone()], r.rtl)).collect();
        assert_eq!(runs, [("!", true), ("Twine", false), ("مرحبا ", true)]);
        assert_eq!(shown(line, TextDir::Rtl), "!Twine ابحرم");
    }

    #[test]
    fn brackets_mirror_in_rtl_runs() {
        // Logical "(שלום)" in RTL: drawn reversed with mirrored brackets, so it still reads (…).
        assert_eq!(shown("(שלום)", TextDir::Rtl), "(םולש)");
        assert_eq!(mirror('['), ']');
        assert_eq!(mirror('a'), 'a');
    }

    #[test]
    fn auto_base_dir_detection() {
        assert_eq!(detect_base_dir("שלום world"), TextDir::Rtl);
        assert_eq!(detect_base_dir("  مرحبا"), TextDir::Rtl);
        assert_eq!(detect_base_dir("hello שלום"), TextDir::Ltr);
        assert_eq!(detect_base_dir("!? 42"), TextDir::Ltr);
        assert_eq!(resolve_dir(TextDir::Rtl, "abc"), TextDir::Rtl);
        assert!(has_rtl("a ש") && !has_rtl("abc é ü"));
    }
}
