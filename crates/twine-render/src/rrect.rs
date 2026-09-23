//! Row structure of anti-aliased rounded rectangles, shared by backgrounds, borders, outlines,
//! arcs, shadows and radius masks.

use twine_core::Rect;

use crate::circle::{CircleRow, Quarter};

/// Effective radius of a rounded rectangle: `radius` clamped to `[0, min(w, h) / 2]`.
#[inline]
#[must_use]
pub(crate) fn eff_radius(area: Rect, radius: i32) -> i32 {
    radius.clamp(0, area.width().min(area.height()) / 2)
}

/// The coverage of one row of a rounded rectangle:
/// 0 outside `[aa_l0, aa_r1)`, 255 inside `[full_l, full_r)`, anti-aliased in between.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RowSpan<'c> {
    pub aa_l0: i32,
    pub full_l: i32,
    pub full_r: i32,
    pub aa_r1: i32,
    x0: i32,
    x1: i32,
    row: CircleRow<'c>,
}

impl<'c> RowSpan<'c> {
    /// An empty row.
    pub const EMPTY: RowSpan<'static> = RowSpan {
        aa_l0: 0,
        full_l: 0,
        full_r: 0,
        aa_r1: 0,
        x0: 0,
        x1: 0,
        row: CircleRow::FULL,
    };

    /// Row `y` of the rounded rectangle `area` whose (effective) radius is `q.radius()`.
    #[inline]
    pub fn new(area: Rect, q: &Quarter<'c>, y: i32) -> RowSpan<'c> {
        if y < area.y0 || y >= area.y1 || area.is_empty() {
            return RowSpan::EMPTY;
        }
        let r = q.radius();
        let py = (y - area.y0).min(area.y1 - 1 - y);
        let (x0, x1) = (area.x0, area.x1);
        if py >= r {
            return RowSpan {
                aa_l0: x0,
                full_l: x0,
                full_r: x1,
                aa_r1: x1,
                x0,
                x1,
                row: CircleRow::FULL,
            };
        }
        let row = q.row(py);
        RowSpan {
            aa_l0: x0 + row.aa_start,
            full_l: x0 + row.full_from,
            full_r: x1 - row.full_from,
            aa_r1: x1 - row.aa_start,
            x0,
            x1,
            row,
        }
    }

    /// Whether the row covers nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.aa_l0 >= self.aa_r1
    }

    /// Coverage at `x`.
    #[inline(always)]
    pub fn cov(&self, x: i32) -> u8 {
        if x < self.aa_l0 || x >= self.aa_r1 {
            0
        } else if x >= self.full_l && x < self.full_r {
            255
        } else if x < self.full_l {
            self.row.cov(x - self.x0)
        } else {
            self.row.cov(self.x1 - 1 - x)
        }
    }

    /// Classification of `x`: 0 = none, 1 = anti-aliased, 2 = full.
    #[inline]
    pub fn class(&self, x: i32) -> u8 {
        if x < self.aa_l0 || x >= self.aa_r1 {
            0
        } else if x >= self.full_l && x < self.full_r {
            2
        } else {
            1
        }
    }
}
