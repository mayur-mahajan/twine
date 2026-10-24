//! Arabic and Persian contextual shaping (feature `arabic-shaping`): a port of LVGL's
//! `src/misc/lv_text_ap.c` (MIT; table and algorithm from LVGL commit
//! `c329a2db3497b6f69c0ed16b8a4254114cea11c2`, the last change of that file on `master`).
//!
//! Arabic letters change shape depending on whether they join the previous and/or the next
//! letter. [`shape_into`] replaces each letter of the Arabic block (U+0622…U+06F9) by its
//! isolated, initial, medial or final form from the Presentation Forms blocks
//! (U+FB50…U+FDFF, U+FE70…U+FEFF), and lam + alef by the lam-alef ligatures. Harakat
//! (U+064B…U+0652) are kept and do not break joining. Everything else passes through.
//!
//! Shape the **logical** text first, then reorder it for display (feature `bidi`). The fonts
//! must contain the presentation forms (e.g. `DEJAVU_16_PERSIAN_HEBREW` in `twine-assets`).

use alloc::string::String;

/// Code point of `ApChar::offset` 0 (LVGL `LV_AP_ALPHABET_BASE_CODE`).
const BASE: u32 = 0x0622;

/// One letter of the shaping table (LVGL `ap_chars_map_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ApChar {
    /// Letter code point minus U+0622.
    pub offset: u8,
    /// Final form (the other forms are relative to it).
    pub end: u16,
    /// Initial form offset from `end`.
    pub beginning: i8,
    /// Medial form offset from `end`.
    pub middle: i8,
    /// Isolated form offset from `end`.
    pub isolated: i8,
    /// Whether the letter joins the previous letter.
    pub joins_previous: bool,
    /// Whether the letter joins the next letter.
    pub joins_next: bool,
}

const fn ap(offset: u8, end: u16, beginning: i8, middle: i8, isolated: i8, prev: u8, next: u8) -> ApChar {
    ApChar {
        offset,
        end,
        beginning,
        middle,
        isolated,
        joins_previous: prev != 0,
        joins_next: next != 0,
    }
}

/// LVGL's `ap_chars_map` (same order; the end-of-list sentinel omitted).
///
/// Two entries differ from LVGL, whose unused form offsets alias other characters (the
/// lookup also maps every form back to its letter): ة has no medial form (LVGL's `2` named
/// U+FE96, the final teh), and ۰ has no contextual forms (LVGL's `-1` / `2` named U+06EF and
/// U+06F2, so the Persian digit two was drawn as zero). Every code point of the table belongs
/// to exactly one entry (checked by a unit test).
pub static AP_CHARS_MAP: [ApChar; 52] = [
    ap(0, 0xFE81, 0, 0, 0, 0, 0),     // آ
    ap(1, 0xFE84, -1, 0, -1, 1, 0),   // أ
    ap(2, 0xFE86, -1, 0, -1, 1, 0),   // ؤ
    ap(3, 0xFE88, -1, 0, -1, 1, 0),   // إ
    ap(4, 0xFE8A, 1, 2, -1, 1, 1),    // ئ
    ap(5, 0xFE8E, -1, 0, -1, 1, 0),   // ا
    ap(6, 0xFE90, 1, 2, -1, 1, 1),    // ب
    ap(92, 0xFB57, 1, 2, -1, 1, 1),   // پ
    ap(8, 0xFE96, 1, 2, -1, 1, 1),    // ت
    ap(9, 0xFE9A, 1, 2, -1, 1, 1),    // ث
    ap(10, 0xFE9E, 1, 2, -1, 1, 1),   // ج
    ap(100, 0xFB7B, 1, 2, -1, 1, 1),  // چ
    ap(11, 0xFEA2, 1, 2, -1, 1, 1),   // ح
    ap(12, 0xFEA6, 1, 2, -1, 1, 1),   // خ
    ap(13, 0xFEAA, -1, 0, -1, 1, 0),  // د
    ap(14, 0xFEAC, -1, 0, -1, 1, 0),  // ذ
    ap(15, 0xFEAE, -1, 0, -1, 1, 0),  // ر
    ap(16, 0xFEB0, -1, 0, -1, 1, 0),  // ز
    ap(118, 0xFB8B, -1, 0, -1, 1, 0), // ژ
    ap(17, 0xFEB2, 1, 2, -1, 1, 1),   // س
    ap(18, 0xFEB6, 1, 2, -1, 1, 1),   // ش
    ap(19, 0xFEBA, 1, 2, -1, 1, 1),   // ص
    ap(20, 0xFEBE, 1, 2, -1, 1, 1),   // ض
    ap(21, 0xFEC2, 1, 2, -1, 1, 1),   // ط
    ap(22, 0xFEC6, 1, 2, -1, 1, 1),   // ظ
    ap(23, 0xFECA, 1, 2, -1, 1, 1),   // ع
    ap(24, 0xFECE, 1, 2, -1, 1, 1),   // غ
    ap(30, 0x0640, 0, 0, 0, 1, 1),    // ـ (tatweel)
    ap(31, 0xFED2, 1, 2, -1, 1, 1),   // ف
    ap(32, 0xFED6, 1, 2, -1, 1, 1),   // ق
    ap(135, 0xFB8F, 1, 2, -1, 1, 1),  // ک
    ap(33, 0xFEDA, 1, 2, -1, 1, 1),   // ك
    ap(141, 0xFB93, 1, 2, -1, 1, 1),  // گ
    ap(34, 0xFEDE, 1, 2, -1, 1, 1),   // ل
    ap(35, 0xFEE2, 1, 2, -1, 1, 1),   // م
    ap(36, 0xFEE6, 1, 2, -1, 1, 1),   // ن
    ap(38, 0xFEEE, -1, 0, -1, 1, 0),  // و
    ap(37, 0xFEEA, 1, 2, -1, 1, 1),   // ه
    ap(39, 0xFEF0, 0, 0, -1, 1, 0),   // ى
    ap(40, 0xFEF2, 1, 2, -1, 1, 1),   // ي
    ap(170, 0xFBFD, 1, 2, -1, 1, 1),  // ی
    ap(7, 0xFE94, -1, 0, -1, 1, 0),   // ة (LVGL: medial 2)
    ap(206, 0x06F0, 0, 0, 0, 0, 0),   // ۰ (LVGL: initial -1, medial 2)
    ap(207, 0x06F1, 0, 0, 0, 0, 0),   // ۱
    ap(208, 0x06F2, 0, 0, 0, 0, 0),   // ۲
    ap(209, 0x06F3, 0, 0, 0, 0, 0),   // ۳
    ap(210, 0x06F4, 0, 0, 0, 0, 0),   // ۴
    ap(211, 0x06F5, 0, 0, 0, 0, 0),   // ۵
    ap(212, 0x06F6, 0, 0, 0, 0, 0),   // ۶
    ap(213, 0x06F7, 0, 0, 0, 0, 0),   // ۷
    ap(214, 0x06F8, 0, 0, 0, 0, 0),   // ۸
    ap(215, 0x06F9, 0, 0, 0, 0, 0),   // ۹
];

impl ApChar {
    /// The base letter's code point (`BASE + offset`).
    const fn base(self) -> u32 {
        // SAFE-ARITH: `BASE` is U+0622 and `offset` a `u8`.
        BASE.wrapping_add(self.offset as u32)
    }

    /// The form `end + ofs`.
    const fn form(self, ofs: i8) -> u32 {
        // SAFE-ARITH: a `u16` plus an `i8` fits in `i32`.
        (self.end as i32).wrapping_add(ofs as i32) as u32
    }
}

/// The entry of [`AP_CHARS_MAP`] for `c`: the base letter or any of its presentation forms
/// (LVGL `lv_ap_get_char_index`).
fn entry_of(c: char) -> Option<&'static ApChar> {
    let c = u32::from(c);
    if c < BASE {
        return None;
    }
    AP_CHARS_MAP.iter().find(|a| {
        c == a.base()
            || c == u32::from(a.end)
            || c == a.form(a.beginning)
            || c == a.form(a.middle)
            || c == a.form(a.isolated)
    })
}

/// Harakat (short vowel marks, U+064B…U+0652): kept in place, transparent for joining.
#[must_use]
pub const fn is_arabic_vowel(c: char) -> bool {
    matches!(c as u32, 0x064B..=0x0652)
}

/// The lam-alef ligature for lam (at `cur`) followed by an alef variant (at `next`)
/// (LVGL `lv_text_lam_alef`): isolated form; +1 gives the final form.
fn lam_alef(cur: ApChar, next: Option<&ApChar>) -> Option<u32> {
    if cur.offset != 34 {
        return None;
    }
    match next?.base() {
        0x0622 => Some(0xFEF5),
        0x0623 => Some(0xFEF7),
        0x0625 => Some(0xFEF9),
        0x0627 => Some(0xFEFB),
        _ => None,
    }
}

/// Whether `text` contains a character the shaper changes (a quick check to skip shaping).
#[must_use]
pub fn needs_shaping(text: &str) -> bool {
    text.chars().any(|c| matches!(u32::from(c), 0x0622..=0x06F9))
}

/// Appends `src` with Arabic/Persian letters replaced by their contextual presentation forms
/// to `out` (LVGL `lv_text_ap_proc`). Non-Arabic text is copied unchanged; `out` is not
/// cleared, so a reused `String` needs no new allocation once it has grown.
///
/// ```
/// use twine_text::shape_into;
///
/// let mut out = String::new();
/// shape_into("سلام", &mut out); // salam: seen (initial), lam-alef (final), meem (isolated)
/// assert_eq!(out, "\u{FEB3}\u{FEFC}\u{FEE1}");
/// ```
// NOTE(P22.S08): label/spangroup/textarea shape their text into a widget-owned scratch
// `String` when the text changes, then draw it with bidi reordering.
pub fn shape_into(src: &str, out: &mut String) {
    let mut chars = src.chars().peekable();
    let mut prev: Option<&ApChar> = None;
    let mut first = true;
    while let Some(c) = chars.next() {
        let is_first = core::mem::replace(&mut first, false);
        if is_arabic_vowel(c) {
            // Vowels keep the joining state of the letter before them.
            out.push(c);
            continue;
        }
        let Some(cur) = entry_of(c) else {
            out.push(c);
            prev = None;
            continue;
        };
        // The next letter, skipping one vowel (as LVGL).
        let mut ahead = chars.clone();
        let mut n = ahead.next();
        if n.is_some_and(is_arabic_vowel) {
            n = ahead.next();
        }
        let next = n.and_then(entry_of);
        let joins_prev = !is_first && prev.is_some_and(|p| p.joins_next);
        let joins_next = next.is_some_and(|n| n.joins_previous);
        if let Some(lig) = lam_alef(*cur, next) {
            // SAFE-ARITH: the ligatures are below U+FEFC.
            let lig = if joins_prev { lig.wrapping_add(1) } else { lig };
            out.push(char::from_u32(lig).unwrap_or(c));
            // The ligature replaces lam and the alef; a vowel between them follows the
            // ligature. (LVGL skips exactly one character here, which would duplicate the
            // alef after a vowel.)
            let vowel = chars.next_if(|&v| is_arabic_vowel(v));
            chars.next();
            if let Some(v) = vowel {
                out.push(v);
            }
            prev = None;
            continue;
        }
        let a = cur;
        let form = match (joins_prev, joins_next) {
            (true, true) => a.form(a.middle),
            (false, true) => a.form(a.beginning),
            (true, false) => u32::from(a.end),
            (false, false) => a.form(a.isolated),
        };
        out.push(char::from_u32(form).unwrap_or(c));
        prev = Some(cur);
    }
}

/// [`shape_into`] into a new `String`.
#[must_use]
pub fn shape(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    shape_into(src, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cps(s: &str) -> alloc::vec::Vec<u32> {
        s.chars().map(u32::from).collect()
    }

    #[test]
    fn shape_salam() {
        assert_eq!(cps(&shape("سلام")), [0xFEB3, 0xFEFC, 0xFEE1]);
    }

    #[test]
    fn shape_lam_alef_ligature() {
        assert_eq!(cps(&shape("لا")), [0xFEFB], "isolated lam-alef");
        assert_eq!(
            cps(&shape("بلا")),
            [0xFE91, 0xFEFC],
            "final after a joining letter"
        );
        assert_eq!(cps(&shape("لأ")), [0xFEF7]);
        assert_eq!(cps(&shape("لإ")), [0xFEF9]);
        assert_eq!(cps(&shape("لآ")), [0xFEF5]);
    }

    #[test]
    fn persian_gaf_forms() {
        // گ isolated, initial, medial, final.
        assert_eq!(cps(&shape("گ")), [0xFB92]);
        assert_eq!(cps(&shape("گب")), [0xFB94, 0xFE90]);
        assert_eq!(cps(&shape("بگب")), [0xFE91, 0xFB95, 0xFE90]);
        assert_eq!(cps(&shape("بگ")), [0xFE91, 0xFB93]);
        // Other Persian letters: پ چ ژ ک ی.
        assert_eq!(cps(&shape("پچ")), [0xFB58, 0xFB7B]);
        assert_eq!(cps(&shape("ژ")), [0xFB8A]);
        assert_eq!(cps(&shape("کی")), [0xFB90, 0xFBFD]);
    }

    #[test]
    fn diacritics_do_not_break_joining() {
        // ب + fatha + ت: still initial + final, the vowel stays in place.
        assert_eq!(cps(&shape("بَت")), [0xFE91, 0x064E, 0xFE96]);
        assert_eq!(cps(&shape("بت")), [0xFE91, 0xFE96]);
    }

    #[test]
    fn non_arabic_unchanged() {
        for s in ["", "Hello, world!", "שלום 123", "中文", "ab\ncd"] {
            assert_eq!(shape(s), s);
            assert!(!needs_shaping(s));
        }
        // Mixed: only the Arabic word changes; a space breaks joining.
        assert_eq!(cps(&shape("a بب b")), [0x61, 0x20, 0xFE91, 0xFE90, 0x20, 0x62]);
        assert!(needs_shaping("a بب"));
    }

    #[test]
    fn regression_persian_digits_pass_through_unchanged() {
        // LVGL's table gave ۰ a "medial form" of U+06F2, so ۲ was looked up as ۰ and drawn as
        // ۰ (a localized "۱٬۲۳۴" showed "۱٬۰۳۴").
        let digits = "۰۱۲۳۴۵۶۷۸۹";
        assert_eq!(shape(digits), digits);
        assert_eq!(shape("۱٬۲۳۴٬۵۶۷٫۸۹"), "۱٬۲۳۴٬۵۶۷٫۸۹");
        assert_eq!(cps(&shape("ب۲")), [0xFE8F, 0x06F2], "a digit does not join");
        // U+06EF (LVGL's "initial form" of ۰) is not a digit and passes through.
        assert_eq!(shape("\u{6EF}"), "\u{6EF}");
    }

    #[test]
    fn regression_teh_marbuta_has_no_medial_alias() {
        // LVGL's ة "medial form" was U+FE96, the final teh: no entry may claim another's form.
        assert_eq!(cps(&shape("بة")), [0xFE91, 0xFE94]);
        assert_eq!(cps(&shape("ة")), [0xFE93]);
    }

    #[test]
    fn every_code_point_of_the_table_belongs_to_one_entry() {
        for a in &AP_CHARS_MAP {
            let forms = [
                a.base(),
                u32::from(a.end),
                a.form(a.beginning),
                a.form(a.middle),
                a.form(a.isolated),
            ];
            for cp in forms {
                let c = char::from_u32(cp).unwrap();
                assert!(
                    core::ptr::eq(entry_of(c).unwrap(), a),
                    "U+{cp:04X} of the entry for U+{:04X} resolves to U+{:04X}",
                    a.base(),
                    entry_of(c).unwrap().base()
                );
            }
        }
    }

    #[test]
    fn lam_vowel_alef_keeps_the_vowel_once() {
        assert_eq!(cps(&shape("لَا")), [0xFEFB, 0x064E]);
    }

    #[test]
    fn table_matches_unicode_forms() {
        // Spot checks against the Unicode presentation forms: isolated / initial / medial.
        let beh = entry_of('ب').unwrap();
        assert_eq!(
            [
                beh.form(beh.isolated),
                beh.form(beh.beginning),
                beh.form(beh.middle),
                u32::from(beh.end)
            ],
            [0xFE8F, 0xFE91, 0xFE92, 0xFE90]
        );
        let yeh = entry_of('ی').unwrap();
        assert_eq!(yeh.form(yeh.isolated), 0xFBFC);
        assert!(
            core::ptr::eq(entry_of('\u{FE92}').unwrap(), entry_of('ب').unwrap()),
            "presentation forms map back"
        );
    }
}
