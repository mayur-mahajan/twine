//! Grid layout (LVGL `lv_grid.c`).
//!
//! Faithful to LVGL 9:
//! - Templates come from [`LayoutTree::grid_tracks`] (by default the `GridColumnTracks` /
//!   `GridRowTracks` styles); gaps `ColumnGap` / `RowGap`.
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
pub(crate) fn templates<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> Option<(&[GridTrack], &[GridTrack])> {
    let cols = t.grid_tracks(id, Axis::X)?;
    let rows = t.grid_tracks(id, Axis::Y)?;
    (!cols.is_empty() && !rows.is_empty()).then_some((cols, rows))
}

/// Column, column span, row, row span of a grid item (unclamped).
pub(crate) fn cell_of<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> [i32; 4] {
    [
        t.style_i32(id, PropId::GridCellColumn),
        t.style_i32(id, PropId::GridCellColumnSpan),
        t.style_i32(id, PropId::GridCellRow),
        t.style_i32(id, PropId::GridCellRowSpan),
    ]
}

/// Position and span of a grid item's cell on `axis` (from [`cell_of`]'s order).
const fn cell_axis(cell: [i32; 4], axis: Axis) -> (i32, i32) {
    let [col, col_span, row, row_span] = cell;
    match axis {
        Axis::X => (col, col_span),
        Axis::Y => (row, row_span),
    }
}

/// A track count as `i32` (saturating; templates are short lists).
fn count_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Entry `i` of the track table in the scratch integers (`0` past its end: never, the table
/// is sized for every index the callers compute).
fn track_at(v: &[i32], i: usize) -> i32 {
    v.get(i).copied().unwrap_or(0)
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
    // Sizes saturate (style values are any `i32`).
    let mut fr_total: i64 = 0;
    let mut fixed = 0i32;
    for (i, (track, o)) in templ.iter().zip(out.iter_mut()).enumerate() {
        match *track {
            GridTrack::Px(n) => {
                *o = n;
                fixed = fixed.saturating_add(n);
            }
            GridTrack::Fr(k) if !auto => {
                *o = 0;
                // SAFE-ARITH: at most `templ.len()` weights below 2^8 each.
                fr_total = fr_total.wrapping_add(i64::from(k));
            }
            GridTrack::Content | GridTrack::Fr(_) => {
                let size = items
                    .iter()
                    .filter(|it| {
                        let (pos, span) = cell_axis(it.cell, axis);
                        it.in_flow && span == 1 && usize::try_from(pos) == Ok(i)
                    })
                    .filter(|it| !(auto && axis.pick(it.ignore)))
                    .map(|it| axis.of(it.size))
                    .max()
                    .unwrap_or(i32::MIN);
                *o = size.max(0);
                fixed = fixed.saturating_add(*o);
            }
        }
    }
    if fr_total == 0 {
        return;
    }
    let gaps = gap.saturating_mul(count_i32(templ.len()).saturating_sub(1));
    let mut free = i64::from(avail.saturating_sub(gaps).saturating_sub(fixed).max(0));
    for (track, o) in templ.iter().zip(out.iter_mut()) {
        if let GridTrack::Fr(k) = *track {
            let k = i64::from(k);
            if k == 0 {
                continue;
            }
            // `fr_total` is the sum of the weights of the `Fr` tracks not placed yet (all of
            // them are counted when `!auto`; `auto` leaves it 0 and returned above), which
            // includes this track's `k ≥ 1`: the division's fallback is never taken.
            // SAFE-ARITH: `0 ≤ free < 2^31` and `k < 2^8`, so the sum is below 2^40; `v ≤ free`
            // and `k ≤ fr_total`.
            let v = free
                .wrapping_mul(k)
                .wrapping_add(fr_total / 2)
                .checked_div(fr_total)
                .unwrap_or(0);
            *o = v as i32;
            fr_total = fr_total.wrapping_sub(k);
            free = free.wrapping_sub(v);
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
    let n = count_i32(sizes.len());
    let mut gap = gap;
    // Coordinates saturate (style values are any `i32`). `n ≥ 1` (`templates` refuses empty
    // track lists), so the divisors below are at least 1 and the `checked_div` fallbacks are
    // never taken.
    let first = if auto {
        0
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
        let grid = sizes
            .iter()
            .fold(0i32, |acc, s| acc.saturating_add(*s))
            .saturating_add(gap.saturating_mul(n.saturating_sub(1)));
        let free = cont.saturating_sub(grid);
        match align {
            GridAlign::Start | GridAlign::Stretch => 0,
            GridAlign::Center => free / 2,
            GridAlign::End => free,
            GridAlign::SpaceBetween => {
                // `n ≥ 2`: `n == 1` was turned into `Center` above.
                gap = free.checked_div(n.saturating_sub(1)).unwrap_or(0);
                0
            }
            GridAlign::SpaceAround => {
                gap = free.checked_div(n).unwrap_or(0);
                gap / 2
            }
            GridAlign::SpaceEvenly => {
                gap = free.checked_div(n.saturating_add(1)).unwrap_or(0);
                gap
            }
        }
    };
    // Track `i` starts after track `i − 1` and the gap; the total runs from the first track's
    // start to the last one's end.
    let mut next = first;
    let mut last_end = first;
    for (p, &size) in pos.iter_mut().zip(sizes) {
        *p = next;
        last_end = next.saturating_add(size);
        next = last_end.saturating_add(gap);
    }
    let total = last_end.saturating_sub(first);
    if reverse {
        for (p, s) in pos.iter_mut().zip(sizes) {
            *p = cont.saturating_sub(*p).saturating_sub(*s);
        }
    }
    total
}

/// Content extent of a grid container on `axis`: Σ tracks + gaps (tracks sized as on a
/// content-sized axis), or the extent of the non-grid children if larger.
#[allow(clippy::too_many_arguments)]
// Out of line: kept off the frames of the recursive layout walk (`arrange` / `resolve_node`),
// whose stack use per tree level is measured (`docs/safety/profile.md` § 10).
#[inline(never)]
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
        Axis::X => (cols, t.style_i32(id, PropId::ColumnGap)),
        Axis::Y => (rows, t.style_i32(id, PropId::RowGap)),
    };
    let base = s.ints.len();
    // SAFE-ARITH: two in-memory lengths (each below `isize::MAX`).
    s.ints.resize(base.wrapping_add(templ.len()), 0);
    // SAFE-INDEX: the caller's frame `frame..end` lies inside the scratch items and `base` is
    // the old length of `ints`; the fallbacks are never taken.
    let items = s.items.get(frame..end).unwrap_or_default();
    let sizes = s.ints.get_mut(base..).unwrap_or_default();
    track_sizes(items, templ, axis, 0, gap, true, sizes);
    let total = sizes
        .iter()
        .fold(0i32, |acc, s| acc.saturating_add(*s))
        .saturating_add(gap.saturating_mul(count_i32(templ.len()).saturating_sub(1)));
    s.ints.truncate(base);
    let others = abs_extent_all(t, items, axis, avail, sized, rtl);
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
    // `n ≥ 1` (`templates` refuses empty track lists).
    let last = n.saturating_sub(1);
    if pos >= n {
        warn!(target: "twine::layout", "grid {} position was {}, setting it to {}", what, pos, last);
        pos = last;
    }
    if pos.saturating_add(span) > n {
        // SAFE-ARITH: `0 ≤ pos < n` here.
        span = n.wrapping_sub(pos);
        warn!(target: "twine::layout", "grid {} span is too large, limiting it to {}", what, span);
    }
    (pos, span)
}

/// Offset of an item in its cell along one axis and whether it is stretched.
fn cell_offset(align: GridAlign, cell: i32, size: i32, m_start: i32, m_end: i32) -> i32 {
    match align {
        GridAlign::Center => {
            (cell.saturating_sub(size) / 2).saturating_add(m_start.saturating_sub(m_end) / 2)
        }
        GridAlign::End => cell.saturating_sub(size).saturating_sub(m_end),
        _ => m_start,
    }
}

/// Positions the grid items among `s.items[frame..end]` in the content area `content`.
#[allow(clippy::too_many_arguments)]
// Out of line: kept off the frames of the recursive layout walk (`arrange` / `resolve_node`),
// whose stack use per tree level is measured (`docs/safety/profile.md` § 10).
#[inline(never)]
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
    let [sized_x, sized_y] = sized;
    // SAFE-INDEX: the caller's frame `frame..end` lies inside the scratch items; the `get`
    // fallbacks are never taken.
    for i in frame..end {
        let Some(&Item { in_flow, id: c, .. }) = s.items.get(i) else {
            break;
        };
        if !in_flow {
            continue;
        }
        let r = resolve_node(&*t, s, c, csize, [None; 2]);
        if let Some(it) = s.items.get_mut(i) {
            it.set_resolved(r);
            it.margin = margins(&*t, c);
            it.cell = cell_of(&*t, c);
            let [pct_x, pct_y] = r.pct;
            it.ignore = [sized_x && pct_x, sized_y && pct_y];
        }
    }

    let (nc, nr) = (cols.len(), rows.len());
    let (col_gap, row_gap) = (
        t.style_i32(id, PropId::ColumnGap),
        t.style_i32(id, PropId::RowGap),
    );
    let base = s.ints.len();
    // The track table `[col widths | col x | row heights | row y]` after `base`.
    // SAFE-ARITH: in-memory lengths (each below `isize::MAX / 4`): the sums cannot overflow.
    let (cw, cx, rh, ry) = (
        base,
        base.wrapping_add(nc),
        base.wrapping_add(nc).wrapping_add(nc),
        base.wrapping_add(nc).wrapping_add(nc).wrapping_add(nr),
    );
    s.ints.resize(ry.wrapping_add(nr), 0);
    {
        // SAFE-INDEX: the table was sized for the four parts just above; the fallback (no
        // tracks sized) is never taken.
        let table = s.ints.get_mut(base..).unwrap_or_default();
        let (cw, rest) = table.split_at_mut_checked(nc).unwrap_or_default();
        let (cx, rest) = rest.split_at_mut_checked(nc).unwrap_or_default();
        let (rh, ry) = rest.split_at_mut_checked(nr).unwrap_or_default();
        let items = s.items.get(frame..end).unwrap_or_default();
        track_sizes(items, cols, Axis::X, csize.w, col_gap, sized_x, cw);
        track_sizes(items, rows, Axis::Y, csize.h, row_gap, sized_y, rh);
        let col_align = style_enum::<T, GridAlign>(&*t, id, PropId::GridColumnAlign);
        let row_align = style_enum::<T, GridAlign>(&*t, id, PropId::GridRowAlign);
        grid_align(csize.w, sized_x, col_align, col_gap, cw, cx, rtl);
        grid_align(csize.h, sized_y, row_align, row_gap, rh, ry, false);
    }

    for j in frame..end {
        let Some(&it) = s.items.get(j) else { break };
        if !it.in_flow {
            continue;
        }
        let [col, col_span, row, row_span] = it.cell;
        let (cp, cs) = clamp_cell(col, col_span, count_i32(nc), "column");
        let (rp, rs) = clamp_cell(row, row_span, count_i32(nr), "row");
        let (cp, cs, rp, rs) = (cp as usize, cs as usize, rp as usize, rs as usize);
        // SAFE-ARITH: `clamp_cell` gives `0 ≤ pos < n` and `1 ≤ span ≤ n − pos`, so the last
        // spanned track `pos + span − 1 < n` and every index below lies inside its part of the
        // table (SAFE-INDEX: `track_at`'s fallback is never taken).
        let (c_last, r_last) = (
            cp.wrapping_add(cs).wrapping_sub(1),
            rp.wrapping_add(rs).wrapping_sub(1),
        );
        let v = &s.ints;
        let at = |part: usize, i: usize| track_at(v, part.wrapping_add(i));
        let (col_x, col_w) = if rtl && cs > 1 {
            let x1 = at(cx, c_last);
            (x1, at(cx, cp).saturating_add(at(cw, cp)).saturating_sub(x1))
        } else {
            let x1 = at(cx, cp);
            (
                x1,
                at(cx, c_last).saturating_add(at(cw, c_last)).saturating_sub(x1),
            )
        };
        let row_y = at(ry, rp);
        let row_h = at(ry, r_last)
            .saturating_add(at(rh, r_last))
            .saturating_sub(row_y);

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
        let (stretch_x, stretch_y) = (x_align == GridAlign::Stretch, y_align == GridAlign::Stretch);
        let mut size = it.size;
        if stretch_x || stretch_y {
            let ovr = [
                stretch_x.then(|| col_w.saturating_sub(sum(m, Axis::X))),
                stretch_y.then(|| row_h.saturating_sub(sum(m, Axis::Y))),
            ];
            size = resolve_node(&*t, s, it.id, csize, ovr).size;
            if let Some(it) = s.items.get_mut(j) {
                it.size = size;
                it.ovr = [stretch_x, stretch_y];
            }
        }
        let x = col_x.saturating_add(cell_offset(
            x_align,
            col_w,
            size.w,
            tree::start(m, Axis::X),
            tree::end(m, Axis::X),
        ));
        let y = row_y.saturating_add(cell_offset(
            y_align,
            row_h,
            size.h,
            tree::start(m, Axis::Y),
            tree::end(m, Axis::Y),
        ));
        let (tx, ty) = translate(&*t, it.id, size);
        let x0 = content.x0.saturating_add(x).saturating_add(tx);
        let y0 = content.y0.saturating_add(y).saturating_add(ty);
        place(t, s, it.id, Rect::from_xywh(x0, y0, size.w, size.h));
        if let Some(it) = s.items.get_mut(j) {
            it.done = true;
        }
    }
    s.ints.truncate(base);
}
