//! Bidirectional text: hit testing and drawing order (feature `bidi`).
#![allow(clippy::unreadable_literal, clippy::borrow_as_ptr)] // colors; font identity checks

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::Point;
use twine_text::{TextAlign, TextDir, TextLayout};

#[test]
fn rtl_cursor_moves_visually_left_as_index_grows() {
    // Hebrew is missing in Montserrat: every letter is a placeholder of equal width.
    let text = "שלום";
    let l = TextLayout::new(text, &MONTSERRAT_14);
    let xs: Vec<i32> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .map(|i| l.pos_of_bidi(i, TextAlign::Auto, 200, TextDir::Rtl).x)
        .collect();
    assert!(xs.windows(2).all(|w| w[1] < w[0]), "{xs:?}");
    assert_eq!(*xs.first().unwrap(), 200, "logical start at the right edge");
}

#[test]
fn bidi_hit_test_roundtrips() {
    for (text, base) in [
        ("abc שלום 12", TextDir::Ltr),
        ("שלום abc!", TextDir::Rtl),
        ("hello", TextDir::Rtl),
        ("مرحبا Twine", TextDir::Auto),
    ] {
        let l = TextLayout::new(text, &MONTSERRAT_14);
        for (i, _) in text.char_indices() {
            let p = l.pos_of_bidi(i, TextAlign::Auto, 300, base);
            let j = l.char_at_bidi(Point::new(p.x, p.y + 2), TextAlign::Auto, 300, base);
            // A boundary between runs has two visual positions; the one found maps back to
            // the same logical index or to the other side of the same boundary glyph.
            let back = l.pos_of_bidi(j, TextAlign::Auto, 300, base);
            assert_eq!(back.x, p.x, "{text:?} index {i} → {j}");
        }
    }
}

#[test]
fn ltr_text_matches_plain_hit_testing() {
    let text = "Hello world";
    let l = TextLayout::new(text, &MONTSERRAT_14);
    for i in 0..=text.len() {
        assert_eq!(
            l.pos_of_bidi(i, TextAlign::Left, 200, TextDir::Ltr),
            l.pos_of(i, TextAlign::Left, 200)
        );
    }
}
