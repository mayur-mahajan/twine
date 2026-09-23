//! Kerning and underline metrics read with `ttf-parser`: fontdue only reads the legacy `kern`
//! table, while most modern fonts (Montserrat included) store kerning in `GPOS` pair
//! adjustments, like LVGL's font converter reads them.

use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
use ttf_parser::{Face, GlyphId, Tag};

/// Horizontal pair kerning of a face, in font units.
pub struct Kerning<'a> {
    face: Face<'a>,
    lookups: Vec<u16>,
}

impl<'a> Kerning<'a> {
    /// Parses `data`; `None` if it is not a font ttf-parser understands.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let mut lookups = Vec::new();
        if let Some(gpos) = face.tables().gpos {
            for f in gpos.features {
                if f.tag == Tag::from_bytes(b"kern") {
                    lookups.extend(f.lookup_indices);
                }
            }
        }
        lookups.sort_unstable();
        lookups.dedup();
        Some(Self { face, lookups })
    }

    /// Units per em.
    #[must_use]
    pub fn units_per_em(&self) -> u16 {
        self.face.units_per_em()
    }

    /// `post` underline position and thickness in font units.
    #[must_use]
    pub fn underline(&self) -> Option<(i16, i16)> {
        self.face.underline_metrics().map(|m| (m.position, m.thickness))
    }

    /// Whether the face has any pair kerning (GPOS `kern` feature or a `kern` table).
    #[must_use]
    pub fn has_kerning(&self) -> bool {
        !self.lookups.is_empty() || self.face.tables().kern.is_some()
    }

    fn gid(&self, c: char) -> Option<GlyphId> {
        self.face.glyph_index(c)
    }

    /// Kerning between two characters in font units (GPOS `kern` lookups, else the `kern`
    /// table).
    #[must_use]
    pub fn kern(&self, left: char, right: char) -> i32 {
        let (Some(l), Some(r)) = (self.gid(left), self.gid(right)) else {
            return 0;
        };
        if self.lookups.is_empty() {
            return self.kern_table(l, r);
        }
        let Some(gpos) = self.face.tables().gpos else {
            return 0;
        };
        let mut total = 0;
        for &li in &self.lookups {
            let Some(lookup) = gpos.lookups.get(li) else {
                continue;
            };
            for sub in lookup.subtables.into_iter::<PositioningSubtable<'_>>() {
                let PositioningSubtable::Pair(pair) = sub else {
                    continue;
                };
                let v = match pair {
                    PairAdjustment::Format1 { coverage, sets } => coverage
                        .get(l)
                        .and_then(|i| sets.get(i))
                        .and_then(|set| set.get(r))
                        .map(|(a, _)| a.x_advance),
                    PairAdjustment::Format2 {
                        coverage,
                        classes,
                        matrix,
                    } => {
                        if coverage.contains(l) {
                            matrix
                                .get((classes.0.get(l), classes.1.get(r)))
                                .map(|(a, _)| a.x_advance)
                        } else {
                            None
                        }
                    }
                };
                if let Some(v) = v {
                    // The first applicable subtable of a lookup wins.
                    total += i32::from(v);
                    break;
                }
            }
        }
        total
    }

    fn kern_table(&self, l: GlyphId, r: GlyphId) -> i32 {
        let Some(kern) = self.face.tables().kern else {
            return 0;
        };
        kern.subtables
            .into_iter()
            .filter(|s| s.horizontal && !s.variable)
            .find_map(|s| s.glyphs_kerning(l, r))
            .map_or(0, i32::from)
    }
}
