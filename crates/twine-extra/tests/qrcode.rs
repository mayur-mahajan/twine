//! QR code encoder and drawing: cross-checked with an independent implementation (`rxing`, a `ZXing`
//! port) and decoded back from rendered pixels.
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

use rxing::qrcode::common::ErrorCorrectionLevel;
use rxing::{BarcodeFormat, RXingResultMetadataType, RXingResultMetadataValue};
use twine_core::{Color, ColorFormat, Rect};
use twine_extra::qrcode::{Ecc, QrError, QrLayout, QrMatrix, QrStyle, draw_qr, draw_qr_placeholder};
use twine_testing::{RenderHarness, assert_render_snapshot};

/// Decodes a QR code from 8-bit luminance, returning the text and the error correction level.
fn decode_luma(luma: Vec<u8>, w: u32, h: u32) -> (String, String) {
    let r =
        rxing::helpers::detect_in_luma(luma, w, h, Some(BarcodeFormat::QR_CODE)).expect("QR code decodes");
    let ecl = match r
        .getRXingResultMetadata()
        .get(&RXingResultMetadataType::ERROR_CORRECTION_LEVEL)
    {
        Some(RXingResultMetadataValue::ErrorCorrectionLevel(s)) => s.clone(),
        other => panic!("no error correction level in {other:?}"),
    };
    (r.getText().to_owned(), ecl)
}

/// The harness pixels as luminance (the red channel: images are gray or black/white).
fn harness_luma(h: &RenderHarness) -> Vec<u8> {
    h.rgb888().chunks_exact(3).map(|p| p[0]).collect()
}

/// The matrix as luminance, `scale` px per module and a 4-module quiet zone.
fn matrix_luma(qr: &QrMatrix, scale: u32) -> (Vec<u8>, u32) {
    let n = u32::from(qr.size());
    let side = (n + 8) * scale;
    let mut luma = vec![255u8; (side * side) as usize];
    for py in 0..side {
        for px in 0..side {
            let (mx, my) = (i64::from(px / scale) - 4, i64::from(py / scale) - 4);
            if (0..i64::from(n)).contains(&mx)
                && (0..i64::from(n)).contains(&my)
                && qr.get(mx as u8, my as u8)
            {
                luma[(py * side + px) as usize] = 0;
            }
        }
    }
    (luma, side)
}

fn ecc_letter(e: Ecc) -> &'static str {
    match e {
        Ecc::Low => "L",
        Ecc::Medium => "M",
        Ecc::Quartile => "Q",
        Ecc::High => "H",
    }
}

#[test]
fn encode_hello_world_version1_matches_reference() {
    let qr = QrMatrix::encode(b"HELLO WORLD", Ecc::Quartile).unwrap();
    assert_eq!((qr.version(), qr.size(), qr.ecc()), (1, 21, Ecc::Quartile));
    // Mask pattern from the format information: bits 12..10 at (2..=4, 8), XOR the format mask
    // 0x5412 (0b101 in those bits). Mask selection penalties differ slightly between
    // implementations, so the reference encoder is given the same mask; everything else (mode,
    // data codewords, Reed-Solomon, placement, format bits) must then match module for module.
    let raw = u8::from(qr.get(2, 8)) << 2 | u8::from(qr.get(3, 8)) << 1 | u8::from(qr.get(4, 8));
    let mask = raw ^ 0b101;
    let hints = rxing::EncodeHints {
        QrMaskPattern: Some(mask.to_string()),
        ..Default::default()
    };
    let reference = rxing::qrcode::encoder::qrcode_encoder::encode_with_hints(
        "HELLO WORLD",
        ErrorCorrectionLevel::Q,
        &hints,
    )
    .unwrap();
    assert_eq!(reference.getMaskPattern(), i32::from(mask));
    let m = reference.getMatrix().as_ref().unwrap();
    assert_eq!((m.getWidth(), m.getHeight()), (21, 21));
    let mut diff = 0;
    for y in 0..21u8 {
        for x in 0..21u8 {
            if qr.get(x, y) != (m.get(u32::from(x), u32::from(y)) == 1) {
                diff += 1;
            }
        }
    }
    assert_eq!(
        diff, 0,
        "module matrix differs from the reference encoder in {diff} modules"
    );
}

#[test]
fn encoded_codes_decode_with_independent_decoder() {
    let long = "Twine ".repeat(160);
    let v40 = "a".repeat(2953);
    let cases: [(&str, Ecc); 8] = [
        ("HELLO WORLD", Ecc::Quartile),
        ("0123456789012345678901234567890123456789", Ecc::Low),
        ("https://github.com/twine-gui?q=qr&v=1", Ecc::Medium),
        ("Grüße, 你好, привет", Ecc::High),
        ("", Ecc::Medium),
        (&long, Ecc::Quartile),
        (&v40, Ecc::Low),
        ("DIGITS AND CAPS 42 $%*+-./:", Ecc::High),
    ];
    for (text, ecc) in cases {
        let qr = QrMatrix::encode(text.as_bytes(), ecc).unwrap();
        assert!(qr.ecc() >= ecc);
        if text.is_empty() {
            continue; // ZXing rejects empty payloads when decoding
        }
        let (luma, side) = matrix_luma(&qr, 3);
        let (decoded, ecl) = decode_luma(luma, side, side);
        assert_eq!(decoded, text, "version {}", qr.version());
        assert_eq!(ecl, ecc_letter(qr.ecc()), "{text:.20}");
    }
}

#[test]
fn ecc_is_boosted_while_the_version_stays() {
    // 5 bytes fit version 1 even at level H.
    let qr = QrMatrix::encode(b"twine", Ecc::Low).unwrap();
    assert_eq!((qr.version(), qr.ecc()), (1, Ecc::High));
}

#[test]
fn data_too_long_is_an_error() {
    let data = vec![b'x'; 1300];
    assert!(QrMatrix::encode(&data, Ecc::Low).is_ok());
    assert_eq!(
        QrMatrix::encode(&data, Ecc::High),
        Err(QrError::DataTooLong { len: 1300 })
    );
    assert_eq!(
        QrMatrix::encode(&[b'1'; 7090], Ecc::Low),
        Err(QrError::DataTooLong { len: 7090 })
    );
    assert_eq!(
        QrError::DataTooLong { len: 3 }.to_string(),
        "data of 3 bytes does not fit in a QR code at this error correction level"
    );
}

fn draw(
    w: u16,
    h: u16,
    format: ColorFormat,
    qr: &QrMatrix,
    style: QrStyle,
) -> (RenderHarness, Option<QrLayout>) {
    let mut harness = RenderHarness::new(w, h, format);
    let area = harness.area();
    let layout = harness.paint(|p| draw_qr(p, area, qr, &style));
    (harness, layout)
}

#[test]
fn drawn_code_decodes() {
    let text = "https://example.com/twine/qr?id=1234567890";
    let qr = QrMatrix::encode(text.as_bytes(), Ecc::Medium).unwrap();
    for (w, h) in [(160, 120), (97, 97), (58, 70)] {
        let (harness, layout) = draw(w, h, ColorFormat::L8, &qr, QrStyle::default());
        let layout = layout.unwrap();
        assert_eq!(layout.scale, i32::from(w.min(h)) / (i32::from(qr.size()) + 4));
        let (decoded, _) = decode_luma(harness_luma(&harness), u32::from(w), u32::from(h));
        assert_eq!(decoded, text, "{w}x{h}");
    }
}

#[test]
fn drawn_pixels_follow_the_matrix() {
    let qr = QrMatrix::encode(b"pixels", Ecc::Low).unwrap();
    let style = QrStyle {
        dark: Color::hex(0x10_20_30),
        light: Color::hex(0xF0_E0_D0),
        quiet_zone: 1,
    };
    let (harness, layout) = draw(100, 80, ColorFormat::Rgb888, &qr, style);
    let layout = layout.unwrap();
    let rgb = harness.rgb888();
    for py in 0..80 {
        for px in 0..100 {
            let (mx, my) = (
                (px - layout.origin.x).div_euclid(layout.scale),
                (py - layout.origin.y).div_euclid(layout.scale),
            );
            let n = i32::from(qr.size());
            let dark = (0..n).contains(&mx) && (0..n).contains(&my) && qr.get(mx as u8, my as u8);
            let c = if dark { style.dark } else { style.light };
            let i = (py * 100 + px) as usize * 3;
            assert_eq!(&rgb[i..i + 3], &[c.r, c.g, c.b], "pixel ({px}, {py})");
        }
    }
}

#[test]
fn chunked_drawing_is_identical() {
    let qr = QrMatrix::encode(b"chunks of rows", Ecc::Quartile).unwrap();
    let (full, _) = draw(90, 90, ColorFormat::Rgb565, &qr, QrStyle::default());
    let mut chunked = RenderHarness::new(90, 90, ColorFormat::Rgb565);
    let area = chunked.area();
    chunked.paint_chunked(7, |p| {
        draw_qr(p, area, &qr, &QrStyle::default());
    });
    assert!(full.rgb888() == chunked.rgb888());
}

#[test]
fn area_too_small_draws_placeholder() {
    let qr = QrMatrix::encode(b"small", Ecc::Low).unwrap();
    let (harness, layout) = draw(24, 24, ColorFormat::Rgb565, &qr, QrStyle::default());
    assert_eq!(layout, None);
    let mut expected = RenderHarness::new(24, 24, ColorFormat::Rgb565);
    let area = expected.area();
    expected.paint(|p| draw_qr_placeholder(p, area, &QrStyle::default()));
    assert!(harness.rgb888() == expected.rgb888());
}

#[test]
fn qr_snapshot() {
    let qr = QrMatrix::encode("https://example.com/twine".as_bytes(), Ecc::Medium).unwrap();
    let style = QrStyle {
        dark: Color::hex(0x1E_3A_8A),
        light: Color::hex(0xF8_FA_FC),
        quiet_zone: 2,
    };
    let mut h = RenderHarness::new(200, 120, ColorFormat::Rgb565);
    h.paint(|p| {
        draw_qr(p, Rect::from_xywh(0, 0, 120, 120), &qr, &style);
        draw_qr(p, Rect::from_xywh(125, 10, 70, 50), &qr, &QrStyle::default());
        draw_qr_placeholder(p, Rect::from_xywh(125, 70, 40, 40), &style);
    });
    assert_render_snapshot!(h, "qr_snapshot");
}
