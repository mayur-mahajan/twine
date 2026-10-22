//! Shift amounts of the glyph readers (R5.S09): a font's bit depth decides every shift amount of
//! the bitmap readers, so a depth other than 1, 2, 4 or 8 is refused before any bitmap is read
//! (its glyphs are missing). CI also runs this file with `--release`, where an out-of-range
//! shift would silently mask the amount instead of panicking.
//!
//! req: SAF-032

use twine_text::encode::{BitWriter, rle_decode, rle_encode};
use twine_text::{
    BitmapFont, BitmapFormat, Cmap, CmapKind, GlyphCache, GlyphDsc, GlyphIdOfs, GlyphProvider, Kern,
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
fn font(bpp: u8, format: BitmapFormat) -> BitmapFont {
    BitmapFont {
        bpp,
        bitmap: &[0xFF; 4],
        glyphs: &GLYPHS,
        cmaps: &CMAPS,
        kern: Kern::None,
        kern_scale: 16,
        format,
    }
}

#[test]
fn every_legal_depth_reads_its_bits() {
    for bpp in [1, 2, 4, 8] {
        let f = font(bpp, BitmapFormat::Plain);
        let info = f.glyph_info('A', None).expect("legal depth");
        let mut out = [0u8; 2];
        assert!(f.render_a8(&info, &mut out, &mut GlyphCache::new(0)), "bpp {bpp}");
        assert_eq!(out, [255, 255], "bpp {bpp}");
    }
}

/// Debug builds assert on an invalid depth (a font is static data: a build error); release
/// builds treat the glyphs as missing.
#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "invalid bitmap font bpp"))]
fn an_illegal_depth_is_refused_before_any_bitmap_read() {
    for bpp in [0, 3, 5, 7, 9, 16, 32, 255] {
        for format in [
            BitmapFormat::Plain,
            BitmapFormat::Compressed,
            BitmapFormat::CompressedNoPrefilter,
        ] {
            let f = font(bpp, format);
            assert_eq!(f.glyph_info('A', None), None, "bpp {bpp}");
            let info = font(4, format).glyph_info('A', None).unwrap();
            let mut out = [0u8; 2];
            assert!(
                !f.render_a8(&info, &mut out, &mut GlyphCache::new(0)),
                "bpp {bpp}"
            );
        }
    }
}

#[test]
fn rle_tools_refuse_illegal_depths() {
    let bytes = rle_encode(&[1, 1, 0, 1], 1);
    assert_eq!(rle_decode(&bytes, 1, 4), [1, 1, 0, 1]);
    for bpp in [0, 3, 9, 200] {
        assert_eq!(rle_decode(&[0xFF; 8], bpp, 5), [0; 5], "bpp {bpp}");
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
