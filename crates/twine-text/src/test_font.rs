//! A hand-built bitmap font for unit tests.
//!
//! | glyph id | char | box | adv (1/16 px) |
//! |---------:|------|-----|---------------|
//! | 1 | `A` | 3×2 | 7 |
//! | 2 | `B` | 2×2 | 8 |
//! | 3 | `C` | 1×1 | 24 |
//! | 4 | `°` | 2×2 | 32 |
//! | 5 | `•` | 1×1 | 32 |
//! | 6 | ` ` | 0×0 | 64 |
//!
//! Kerning pairs: `A`→`B` −16, `B`→`A` +8 (scale 16).

use crate::bitmap_font::{BitmapFont, BitmapFormat, Cmap, CmapKind, GlyphDsc, GlyphIdOfs, Kern, KernPairIds};
use crate::font::{Font, Subpx};

static BITMAP: [u8; 8] = [
    // A: 3×2 values [0, 15, 0, 8, 15, 1]
    0x0F, 0x08, 0xF1, //
    // B: 2×2 values [15, 15, 15, 15]
    0xFF, 0xFF, //
    // C: 1×1 value 15 (+pad)
    0xF0, //
    // °: 2×2 [15, 0, 0, 15]
    0xF0, 0x0F,
];

static GLYPHS: [GlyphDsc; 7] = [
    GlyphDsc {
        bitmap_index: 0,
        adv_w: 0,
        box_w: 0,
        box_h: 0,
        ofs_x: 0,
        ofs_y: 0,
    },
    GlyphDsc {
        bitmap_index: 0,
        adv_w: 7,
        box_w: 3,
        box_h: 2,
        ofs_x: 0,
        ofs_y: 0,
    },
    GlyphDsc {
        bitmap_index: 3,
        adv_w: 8,
        box_w: 2,
        box_h: 2,
        ofs_x: 0,
        ofs_y: 0,
    },
    GlyphDsc {
        bitmap_index: 5,
        adv_w: 24,
        box_w: 1,
        box_h: 1,
        ofs_x: 0,
        ofs_y: 0,
    },
    GlyphDsc {
        bitmap_index: 6,
        adv_w: 32,
        box_w: 2,
        box_h: 2,
        ofs_x: 0,
        ofs_y: 4,
    },
    GlyphDsc {
        bitmap_index: 5,
        adv_w: 32,
        box_w: 1,
        box_h: 1,
        ofs_x: 0,
        ofs_y: 2,
    },
    GlyphDsc {
        bitmap_index: 0,
        adv_w: 64,
        box_w: 0,
        box_h: 0,
        ofs_x: 0,
        ofs_y: 0,
    },
];

static SPARSE: [u16; 2] = [0, 0x2022 - 0xB0];

static CMAPS: [Cmap; 3] = [
    Cmap {
        range_start: 0x41,
        range_length: 3,
        glyph_id_start: 1,
        unicode_list: &[],
        glyph_id_ofs_list: GlyphIdOfs::None,
        kind: CmapKind::Format0Tiny,
    },
    Cmap {
        range_start: 0xB0,
        range_length: 0x2022 - 0xB0 + 1,
        glyph_id_start: 4,
        unicode_list: &SPARSE,
        glyph_id_ofs_list: GlyphIdOfs::None,
        kind: CmapKind::SparseTiny,
    },
    Cmap {
        range_start: 0x20,
        range_length: 1,
        glyph_id_start: 6,
        unicode_list: &[],
        glyph_id_ofs_list: GlyphIdOfs::None,
        kind: CmapKind::Format0Tiny,
    },
];

static KERN_PAIRS: [[u8; 2]; 2] = [[1, 2], [2, 1]];
static KERN_VALUES: [i8; 2] = [-16, 8];

/// The test font's glyph data.
pub(crate) static TEST_BITMAP: BitmapFont = BitmapFont {
    bpp: 4,
    bitmap: &BITMAP,
    glyphs: &GLYPHS,
    cmaps: &CMAPS,
    kern: Kern::Pairs {
        glyph_ids: KernPairIds::U8(&KERN_PAIRS),
        values: &KERN_VALUES,
    },
    kern_scale: 16,
    format: BitmapFormat::Plain,
};

/// The test font: line height 10, baseline 2 px above the bottom.
pub(crate) static TEST_FONT: Font = Font {
    line_height: 10,
    base_line: 2,
    underline_position: -1,
    underline_thickness: 1,
    provider: &TEST_BITMAP,
    fallback: None,
    subpx: Subpx::None,
};
