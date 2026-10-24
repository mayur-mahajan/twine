//! Shift amounts of the glyph readers (R5.S09, R6.S02): a font's bit depth decides every shift
//! amount of the bitmap readers, so it is a typed [`Bpp`] — 1, 2, 4 or 8, validated where it is
//! parsed — and a font cannot hold any other depth. CI also runs this file with `--release`,
//! where an out-of-range shift would silently mask the amount instead of panicking.
//!
//! req: SAF-032

use twine_text::encode::{BitWriter, rle_decode, rle_encode};
use twine_text::{
    BitmapFont, BitmapFormat, Bpp, Cmap, CmapKind, GlyphCache, GlyphDsc, GlyphIdOfs, GlyphProvider, Kern,
    Polarity,
};

static GLYPHS: [GlyphDsc; 2] = [
    GlyphDsc {
        bitmap_index: 0,
        adv_w: 0,
        box_w: 0,
        box_h: 0,
        ofs_x: 0,
        ofs_y: 0,
    },
    // `A`: 2 × 1.
    GlyphDsc {
        bitmap_index: 0,
        adv_w: 48,
        box_w: 2,
        box_h: 1,
        ofs_x: 0,
        ofs_y: 0,
    },
];

static CMAPS: [Cmap; 1] = [Cmap {
    range_start: 'A' as u32,
    range_length: 1,
    glyph_id_start: 1,
    unicode_list: &[],
    glyph_id_ofs_list: GlyphIdOfs::None,
    kind: CmapKind::Format0Tiny,
}];

/// A one-glyph font of depth `bpp` whose bitmap is all ones.
fn font(bpp: Bpp, format: BitmapFormat) -> BitmapFont {
    BitmapFont {
        bpp,
        bitmap: &[0xFF; 4],
        glyphs: &GLYPHS,
        cmaps: &CMAPS,
        kern: Kern::None,
        kern_scale: 16,
        format,
        gamma: &twine_text::gamma::LINEAR,
    }
}

#[test]
fn every_legal_depth_reads_its_bits() {
    for bpp in Bpp::ALL {
        for format in [
            BitmapFormat::Plain,
            BitmapFormat::Compressed,
            BitmapFormat::CompressedNoPrefilter,
        ] {
            let f = font(bpp, format);
            let info = f.glyph_info('A', None).expect("legal depth");
            assert_eq!(info.bpp, Some(bpp));
            let mut out = [0u8; 2];
            // All-ones data decodes to full coverage in the plain format; the compressed
            // formats read the same bits as literals and repeats, never past the data.
            assert!(
                f.render_a8(&info, Polarity::DarkOnLight, &mut out, &mut GlyphCache::new(0)),
                "{bpp:?} {format:?}"
            );
            if format == BitmapFormat::Plain {
                assert_eq!(out, [255, 255], "{bpp:?}");
            }
        }
    }
}

/// A depth other than 1, 2, 4 or 8 cannot be constructed, so no font can hold one: the check
/// happens once, where a generator parses the depth.
#[test]
fn an_illegal_depth_is_refused_where_it_is_parsed() {
    for bits in 0..=u8::MAX {
        let depth = Bpp::new(bits);
        assert_eq!(depth.is_some(), matches!(bits, 1 | 2 | 4 | 8), "{bits}");
        assert_eq!(depth.map(Bpp::bits), depth.is_some().then_some(bits));
    }
}

#[test]
fn rle_tools_round_trip_every_depth() {
    for bpp in Bpp::ALL {
        let values = [bpp.max_value(), bpp.max_value(), 0, 1];
        let bytes = rle_encode(&values, bpp);
        assert_eq!(rle_decode(&bytes, bpp, 4), values, "{bpp:?}");
    }
}

#[test]
fn bit_writer_writes_the_bits_above_a_byte_as_zeros() {
    let mut w = BitWriter::new();
    w.write(0xFF, 12);
    w.write(0b1, 1);
    assert_eq!(w.bit_len(), 13);
    assert_eq!(w.finish(), [0x0F, 0xF8]);
}
