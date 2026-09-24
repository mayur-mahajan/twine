//! Code 128 encoder and drawing: known vectors, code set selection, and decoding of rendered
//! symbols with an independent decoder (`rxing`, a `ZXing` port).
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use rxing::BarcodeFormat;
use twine_core::{Color, ColorFormat, Rect};
use twine_extra::barcode::{BarcodeStyle, Code128, Code128Error, Orientation, draw_barcode};
use twine_testing::{RenderHarness, assert_render_snapshot};

const START_B: u8 = 104;
const START_C: u8 = 105;
const CODE_C: u8 = 99;
const CODE_B: u8 = 100;
const STOP: u8 = 106;

fn decode(luma: Vec<u8>, w: u32, h: u32) -> String {
    rxing::helpers::detect_in_luma(luma, w, h, Some(BarcodeFormat::CODE_128))
        .expect("barcode decodes")
        .getText()
        .to_owned()
}

/// Renders `code` horizontally at `scale` px per module, 24 px tall, and returns the luminance.
fn render(code: &Code128, scale: u8) -> (RenderHarness, u32, u32) {
    let style = BarcodeStyle {
        scale,
        ..BarcodeStyle::default()
    };
    let w = style.length_px(code) as u16;
    let mut h = RenderHarness::new(w, 24, ColorFormat::L8);
    let area = h.area();
    h.paint(|p| draw_barcode(p, area, code, &style));
    (h, u32::from(w), 24)
}

fn luma(h: &RenderHarness) -> Vec<u8> {
    h.rgb888().chunks_exact(3).map(|p| p[0]).collect()
}

#[test]
fn code128_checksum_known_vector() {
    // The "Wikipedia" example: Start B, W i k i p e d i a, check 88, stop.
    let code = Code128::encode("Wikipedia").unwrap();
    assert_eq!(
        code.symbols(),
        &[START_B, 55, 73, 75, 73, 80, 69, 68, 73, 65, 88, STOP]
    );
    assert_eq!(code.check_symbol(), 88);
    assert_eq!(code.module_count(), 11 * 11 + 13);
    // Bar/space widths of the start (211214), 'W' (value 55: 311321) and the stop (2331112).
    let widths: Vec<u8> = code.widths().collect();
    assert_eq!(&widths[..12], &[2, 1, 1, 2, 1, 4, 3, 1, 1, 3, 2, 1]);
    assert_eq!(&widths[widths.len() - 7..], &[2, 3, 3, 1, 1, 1, 2]);
}

#[test]
fn numeric_run_uses_code_c() {
    // Only digits: start C, two digits per symbol.
    let code = Code128::encode("123456").unwrap();
    assert_eq!(&code.symbols()[..4], &[START_C, 12, 34, 56]);
    assert_eq!(code.symbols().len(), 6);
    // A run of 4 digits inside text switches to C and back.
    let code = Code128::encode("AB1234cd").unwrap();
    assert_eq!(
        &code.symbols()[..8],
        &[START_B, 33, 34, CODE_C, 12, 34, CODE_B, 67]
    );
    // Runs of 3 digits stay in B (C would not be shorter).
    let code = Code128::encode("A123").unwrap();
    assert_eq!(&code.symbols()[..5], &[START_B, 33, 17, 18, 19]);
    // Exactly two digits: start C.
    assert_eq!(Code128::encode("42").unwrap().symbols()[..2], [START_C, 42]);
}

#[test]
fn invalid_char_rejected() {
    // NOTE(P25.S02): the `Barcode` widget logs a warning and keeps its previous symbol.
    assert_eq!(
        Code128::encode("price: 5€"),
        Err(Code128Error::InvalidChar { index: 8, ch: '€' })
    );
    assert_eq!(Code128::encode(""), Err(Code128Error::Empty));
    assert_eq!(
        Code128::encode("é").unwrap_err().to_string(),
        "character 'é' at byte 0 cannot be encoded in Code 128"
    );
}

#[test]
fn rendered_symbols_decode_with_independent_decoder() {
    let long = "Twine GUI 0123456789 ".repeat(4);
    for text in [
        "Wikipedia",
        "123456",
        "1234567",
        "AB1234cd",
        "HELLO\tWORLD\r\n",
        "a\tb",
        "~!@#$%^&*()_+{}|:\"<>?`-=[]\\;',./",
        "x",
        long.as_str(),
    ] {
        let code = Code128::encode(text).unwrap();
        for scale in [1, 2] {
            let (h, w, hh) = render(&code, scale);
            assert_eq!(decode(luma(&h), w, hh), text, "scale {scale}");
        }
    }
}

#[test]
fn vertical_symbol_is_the_horizontal_one_rotated() {
    let code = Code128::encode("Vertical 42").unwrap();
    let (hor, w, h) = render(&code, 2);
    let style = BarcodeStyle {
        scale: 2,
        orientation: Orientation::Vertical,
        ..BarcodeStyle::default()
    };
    let mut ver = RenderHarness::new(h as u16, w as u16, ColorFormat::L8);
    let area = ver.area();
    ver.paint(|p| draw_barcode(p, area, &code, &style));
    // Transpose back: pixel (x, y) of the horizontal symbol is (y, x) of the vertical one.
    let (hl, vl) = (luma(&hor), luma(&ver));
    for y in 0..h {
        for x in 0..w {
            assert_eq!(hl[(y * w + x) as usize], vl[(x * h + y) as usize], "({x}, {y})");
        }
    }
}

#[test]
fn symbol_is_centred_and_chunked_drawing_identical() {
    let code = Code128::encode("centre").unwrap();
    let style = BarcodeStyle::default();
    let len = style.length_px(&code);
    let mut full = RenderHarness::new(len as u16 + 40, 30, ColorFormat::Rgb565);
    let area = full.area();
    full.paint(|p| draw_barcode(p, area, &code, &style));
    // First bar after 20 px of margin plus the 10-module quiet zone.
    let rgb = full.rgb888();
    let first_dark = (0..area.width() as usize).find(|&x| rgb[x * 3] == 0).unwrap();
    assert_eq!(first_dark, 30);
    let mut vertical_chunks = RenderHarness::new(30, len as u16, ColorFormat::Rgb565);
    let mut vertical_full = RenderHarness::new(30, len as u16, ColorFormat::Rgb565);
    let ver = BarcodeStyle {
        orientation: Orientation::Vertical,
        ..style
    };
    let varea = vertical_full.area();
    vertical_full.paint(|p| draw_barcode(p, varea, &code, &ver));
    vertical_chunks.paint_chunked(5, |p| draw_barcode(p, varea, &code, &ver));
    assert!(vertical_full.rgb888() == vertical_chunks.rgb888());
}

#[test]
fn symbol_longer_than_area_stays_inside() {
    let code = Code128::encode("does not fit").unwrap();
    let mut h = RenderHarness::new(100, 10, ColorFormat::Rgb565);
    h.paint(|p| draw_barcode(p, Rect::from_xywh(10, 2, 80, 6), &code, &BarcodeStyle::default()));
    let rgb = h.rgb888();
    for y in 0..10 {
        for x in 0..100 {
            if !(10..90).contains(&x) || !(2..8).contains(&y) {
                assert_eq!(rgb[(y * 100 + x) * 3], 255, "({x}, {y})");
            }
        }
    }
}

#[test]
fn barcode_snapshot_hor_and_ver() {
    let code = Code128::encode("Twine 2026").unwrap();
    let hor = BarcodeStyle {
        scale: 2,
        dark: Color::hex(0x11_18_27),
        light: Color::hex(0xFE_F3_C7),
        ..BarcodeStyle::default()
    };
    let ver = BarcodeStyle {
        orientation: Orientation::Vertical,
        ..BarcodeStyle::default()
    };
    let len = hor.length_px(&code);
    let mut h = RenderHarness::new(len as u16, 220, ColorFormat::Rgb565);
    h.paint(|p| {
        draw_barcode(p, Rect::from_xywh(0, 0, len, 50), &code, &hor);
        draw_barcode(p, Rect::from_xywh(20, 60, 40, 155), &code, &ver);
    });
    assert_render_snapshot!(h, "barcode_hor_and_ver");
}
