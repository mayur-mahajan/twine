//! Grid layout (LVGL `lv_grid.c`).
//!
//! Faithful to LVGL 9:
//! - Templates are `GridColumnDscArray` / `GridRowDscArray`; gaps `PadColumn` / `PadRow`.
//! - Track sizes: `Px(n)` fixed; `Content` = the largest size (without margins) of the
//!   single-span items placed in that track, 0 if none (items spanning several tracks never
//!   grow `Content` tracks); `Fr(k)` share `free = content − gaps − Σ other tracks` (at least
//!   0), `free × k / Σ k` rounded to the closest integer in track order with the rest carried
//!   to the next `Fr` track, so the sum is exact. On a content-sized axis `Fr` tracks behave
//!   like `Content` and the tracks are packed at the start.
//! - Track alignment (`GridColumnAlign`/`GridRowAlign`): `Start`, `Center`, `End`, and the
//!   `Space*` modes which replace the gap by the distributed free space (one track: centered);
//!   `Stretch` acts as `Start`.
//! - Cells: position and span are clamped into the template with a warning (negative position
//!   → 0, beyond the end → last track, span < 1 → 1, span past the end → shortened).
//! - Cell alignment (`GridCellXAlign`/`GridCellYAlign`): `Start`, `Center`, `End`,
//!   `Stretch` (the item takes the cell size minus its margins, overriding its width/height
//!   and, as in LVGL, its min/max); the `Space*` values act as `Start`. An item with a
//!   right-to-left base direction swaps `Start` and `End` horizontally.
//! - Right-to-left containers mirror the columns (column 0 on the right).

use twine_core::log::warn;
use twine_core::{Rect, Size};
use twine_style::{GridAlign, GridTrack, PropId};

use crate::layout::{Item, LayoutScratch};
use crate::position::abs_extent_all;
use crate::size::{raw_size, resolve_node, translate};
use crate::tree::{self, Axis, LayoutTree, is_rtl, margins, place, style_enum, sum};

/// The column and row templates, if both are set and non-empty.
pub(crate) fn templates<T: LayoutTree + ?Sized>(
    t: &T,
    id: T::Id,
) -> Option<(&'static [GridTrack], &'static [GridTrack])> {
    let cols = t.style_prop(id, PropId::GridColumnDscArray).as_grid_tracks()?;
    let rows = t.style_prop(id, PropId::GridRowDscArray).as_grid_tracks()?;
    (!cols.is_empty() && !rows.is_empty()).then_some((cols, rows))
}

/// Column, column span, row, row span of a grid item (unclamped).
pub(crate) fn cell_of<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> [i32; 4] {
    [
        t.style_i32(id, PropId::GridCellColumnPos),
        t.style_i32(id, PropId::GridCellColumnSpan),
        t.style_i32(id, PropId::GridCellRowPos),
        t.style_i32(id, PropId::GridCellRowSpan),
    ]
}

/// Track sizes of one axis (LVGL `calc_cols`/`calc_rows`) into `out`. `auto`: the container
/// is content-sized on this axis (`Fr` behaves as `Content`, percentage items excluded).
fn track_sizes<I>(
    items: &[Item<I>],
    templ: &[GridTrack],
    axis: Axis,
    avail: i32,
    gap: i32,
    auto: bool,
    out: &mut [i32],
) {
    let a = axis.index();
    let (pos_i, span_i) = match axis {
        Axis::X => (0, 1),
        Axis::Y => (2, 3),
    };
    let mut fr_total: i64 = 0;
    let mut fixed = 0i32;
    for (i, (track, o)) in templ.iter().zip(out.iter_mut()).enumerate() {
        match *track {
            GridTrack::Px(n) => {
                *o = n;
                fixed += n;
            }
            GridTrack::Fr(k) if !auto => {
                *o = 0;
                fr_total += i64::from(k);
            }
            GridTrack::Content | GridTrack::Fr(_) => {
                let size = items
                    .iter()
                    .filter(|it| it.in_flow && it.cell[span_i] == 1 && it.cell[pos_i] == i as i32)
                    .filter(|it| !(auto && it.ignore[a]))
                    .map(|it| axis.of(it.size))
                    .max()
                    .unwrap_or(i32::MIN);
                *o = size.max(0);
                fixed += *o;
            }
        }
    }
    if fr_total == 0 {
        return;
    }
    let mut free = i64::from((avail - gap * (templ.len() as i32 - 1) - fixed).max(0));
    for (track, o) in templ.iter().zip(out.iter_mut()) {
        if let GridTrack::Fr(k) = *track {
            let k = i64::from(k);
            if k == 0 {
                continue;
            }
            let v = (free * k + fr_total / 2) / fr_total;
            *o = v as i32;
            fr_total -= k;
            free -= v;
        }
    }
}

/// LVGL `grid_align`: positions of the tracks (relative to the content start) from their
/// sizes; returns the total grid size.
fn grid_align(
    cont: i32,
    auto: bool,
    align: GridAlign,
    gap: i32,
    sizes: &[i32],
    pos: &mut [i32],
    reverse: bool,
) -> i32 {
    let n = sizes.len() as i32;
    let mut gap = gap;
    if auto {
        pos[0] = 0;
    } else {
        let mut align = align;
        if matches!(
            align,
            GridAlign::SpaceAround | GridAlign::SpaceBetween | GridAlign::SpaceEvenly
        ) {
            gap = 0;
            if n == 1 {
                align = GridAlign::Center;
            }
        }
        let grid = sizes.iter().sum::<i32>() + gap * (n - 1);
        pos[0] = match align {
            GridAlign::Start | GridAlign::Stretch => 0,
            GridAlign::Center => (cont - grid) / 2,
            GridAlign::End => cont - grid,
            GridAlign::SpaceBetween => {
                gap = (cont - grid) / (n - 1);
                0
            }
            GridAlign::SpaceAround => {
                gap = (cont - grid) / n;
                gap / 2
            }
            GridAlign::SpaceEvenly => {
                gap = (cont - grid) / (n + 1);
                gap
            }
        };
    }
    for i in 1..sizes.len() {
        pos[i] = pos[i - 1] + sizes[i - 1] + gap;
    }
    let last = sizes.len() - 1;
    let total = pos[last] + sizes[last] - pos[0];
    if reverse {
        for (p, s) in pos.iter_mut().zip(sizes) {
            *p = cont - *p - s;
        }
    }
    total
}

/// Content extent of a grid container on `axis`: Σ tracks + gaps (tracks sized as on a
/// content-sized axis), or the extent of the non-grid children if larger.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extent<T: LayoutTree + ?Sized>(
    t: &T,
    s: &mut LayoutScratch<T::Id>,
    id: T::Id,
    frame: usize,
    end: usize,
    axis: Axis,
    avail: Size,
    sized: [bool; 2],
    rtl: bool,
) -> Option<i32> {
    let (cols, rows) = templates(t, id)?;
    let (templ, gap) = match axis {
        Axis::X => (cols, t.style_i32(id, PropId::PadColumn)),
        Axis::Y => (rows, t.style_i32(id, PropId::PadRow)),
    };
    let base = s.ints.len();
    s.ints.resize(base + templ.len(), 0);
    track_sizes(
        &s.items[frame..end],
        templ,
        axis,
        0,
        gap,
        true,
        &mut s.ints[base..],
    );
    let total = s.ints[base..].iter().sum::<i32>() + gap * (templ.len() as i32 - 1);
    s.ints.truncate(base);
    let others = abs_extent_all(t, &s.items[frame..end], axis, avail, sized, rtl);
    Some(others.map_or(total, |o| o.max(total)))
}

/// Clamps a cell position/span into `n` tracks like LVGL `item_repos`, warning on changes.
fn clamp_cell(pos: i32, span: i32, n: i32, what: &str) -> (i32, i32) {
    let mut span = span;
    let mut pos = pos;
    if span <= 0 {
        warn!(target: "twine::layout", "grid {} span was {}, setting it to 1", what, span);
        span = 1;
    }
    if pos < 0 {
        warn!(target: "twine::layout", "grid {} position was {}, setting it to 0", what, pos);
        pos = 0;
    }
    if pos >= n {
        warn!(target: "twine::layout", "grid {} position was {}, setting it to {}", what, pos, n - 1);
        pos = n - 1;
    }
    if pos + span > n {
        span = n - pos;
        warn!(target: "twine::layout", "grid {} span is too large, limiting it to {}", what, span);
    }
    (pos, span)
}

/// Offset of an item in its cell along one axis and whether it is stretched.
fn cell_offset(align: GridAlign, cell: i32, size: i32, m_start: i32, m_end: i32) -> i32 {
    match align {
        GridAlign::Center => (cell - size) / 2 + (m_start - m_end) / 2,
        GridAlign::End => cell - size - m_end,
        _ => m_start,
    }
}

/// Positions the grid items among `s.items[frame..end]` in the content area `content`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn arrange<T: LayoutTree + ?Sized>(
    t: &mut T,
    s: &mut LayoutScratch<T::Id>,
    id: T::Id,
    frame: usize,
    end: usize,
    content: Rect,
    sized: [bool; 2],
    rtl: bool,
) {
    let Some((cols, rows)) = templates(&*t, id) else {
        return;
    };
    let csize = raw_size(content);
    for i in frame..end {
        if !s.items[i].in_flow {
            continue;
        }
        let c = s.items[i].id;
        let r = resolve_node(&*t, s, c, csize, [None; 2]);
        let it = &mut s.items[i];
        it.set_resolved(r);
        it.margin = margins(&*t, c);
        it.cell = cell_of(&*t, c);
        it.ignore = [sized[0] && r.pct[0], sized[1] && r.pct[1]];
    }

    let (nc, nr) = (cols.len(), rows.len());
    let (col_gap, row_gap) = (
        t.style_i32(id, PropId::PadColumn),
        t.style_i32(id, PropId::PadRow),
    );
    let base = s.ints.len();
    s.ints.resize(base + 2 * (nc + nr), 0);
    {
        let (cw, rest) = s.ints[base..].split_at_mut(nc);
        let (cx, rest) = rest.split_at_mut(nc);
        let (rh, ry) = rest.split_at_mut(nr);
        let items = &s.items[frame..end];
        track_sizes(items, cols, Axis::X, csize.w, col_gap, sized[0], cw);
        track_sizes(items, rows, Axis::Y, csize.h, row_gap, sized[1], rh);
        let col_align = style_enum::<T, GridAlign>(&*t, id, PropId::GridColumnAlign);
        let row_align = style_enum::<T, GridAlign>(&*t, id, PropId::GridRowAlign);
        grid_align(csize.w, sized[0], col_align, col_gap, cw, cx, rtl);
        grid_align(csize.h, sized[1], row_align, row_gap, rh, ry, false);
    }
    let (cw, cx, rh, ry) = (base, base + nc, base + 2 * nc, base + 2 * nc + nr);

    for j in frame..end {
        let it = s.items[j];
        if !it.in_flow {
            continue;
        }
        let (cp, cs) = clamp_cell(it.cell[0], it.cell[1], nc as i32, "column");
        let (rp, rs) = clamp_cell(it.cell[2], it.cell[3], nr as i32, "row");
        let (cp, cs, rp, rs) = (cp as usize, cs as usize, rp as usize, rs as usize);
        let v = &s.ints;
        let (col_x, col_w) = if rtl && cs > 1 {
            let x1 = v[cx + cp + cs - 1];
            (x1, v[cx + cp] + v[cw + cp] - x1)
        } else {
            let x1 = v[cx + cp];
            (x1, v[cx + cp + cs - 1] + v[cw + cp + cs - 1] - x1)
        };
        let row_y = v[ry + rp];
        let row_h = v[ry + rp + rs - 1] + v[rh + rp + rs - 1] - row_y;

        let mut x_align = style_enum::<T, GridAlign>(&*t, it.id, PropId::GridCellXAlign);
        let y_align = style_enum::<T, GridAlign>(&*t, it.id, PropId::GridCellYAlign);
        if is_rtl(&*t, it.id) {
            x_align = match x_align {
                GridAlign::Start => GridAlign::End,
                GridAlign::End => GridAlign::Start,
                other => other,
            };
        }
        let m = it.margin;
        let stretch = [x_align == GridAlign::Stretch, y_align == GridAlign::Stretch];
        let mut size = it.size;
        if stretch[0] || stretch[1] {
            let ovr = [
                stretch[0].then(|| col_w - sum(m, Axis::X)),
                stretch[1].then(|| row_h - sum(m, Axis::Y)),
            ];
            size = resolve_node(&*t, s, it.id, csize, ovr).size;
            s.items[j].size = size;
            s.items[j].ovr = stretch;
        }
        let x = col_x
            + cell_offset(
                x_align,
                col_w,
                size.w,
                tree::start(m, Axis::X),
                tree::end(m, Axis::X),
            );
        let y = row_y
            + cell_offset(
                y_align,
                row_h,
                size.h,
                tree::start(m, Axis::Y),
                tree::end(m, Axis::Y),
            );
        let (tx, ty) = translate(&*t, it.id, size);
        place(
            t,
            it.id,
            Rect::from_xywh(content.x0 + x + tx, content.y0 + y + ty, size.w, size.h),
        );
        s.items[j].done = true;
    }
    s.ints.truncate(base);
}
