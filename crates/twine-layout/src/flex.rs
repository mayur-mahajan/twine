//! Flex layout (LVGL `lv_flex.c`).
//!
//! Faithful to LVGL 9:
//! - Items are the children without `HIDDEN`, `IGNORE_LAYOUT` and `FLOATING`; reverse flows
//!   take them from the last child to the first, and place them from the main-axis start.
//! - Tracks: items are added while `Σ main sizes (+ margins) + gaps ≤ content main size`
//!   (wrapping flows only); the first item of a track is always taken, `FLEX_IN_NEW_TRACK`
//!   forces a break. A content-sized main axis never wraps.
//! - Grow items count with their min size when building tracks and share the remaining space
//!   `free × grow / Σ grow`, rounded to the closest integer in child order with the rest
//!   carried to the next item (the sum is exact); items clamped by min/max are frozen and the
//!   distribution is repeated for the others. Their main size ignores `Width`/`Height`.
//! - Main/track placement: `Start`, `End`, `Center` (`free / 2`), `SpaceEvenly`
//!   (`free / (n + 1)` before every item), `SpaceAround` (`free / n` between, half before),
//!   `SpaceBetween` (`free / (n − 1)` between); with ≤ 1 item `SpaceEvenly`/`SpaceAround`
//!   center. Gaps (`PadColumn`/`PadRow`) are added on top. Negative free space is not
//!   special-cased (as in LVGL).
//! - Cross placement per item within its track: `Start`, `Center`, `End` (the `Space*` values
//!   act as `Start`); margins are respected.
//! - Right-to-left rows are placed from the right; right-to-left column flows place their
//!   tracks from the right.

use twine_core::{Rect, Size};
use twine_style::{FlexAlign, FlexFlow, PropId};

use crate::layout::{Item, LayoutScratch};
use crate::position::abs_extent_all;
use crate::size::{raw_size, resolve_node, translate};
use crate::tree::{Axis, LayoutFlags, LayoutTree, end, margins, place, start, style_enum, sum};

/// One flex track (line) of items.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Track {
    /// First item index (absolute index into the scratch items).
    start: usize,
    /// One past the last item index.
    end: usize,
    /// Main size used for placement (the full content size if the track has grow items).
    main: i32,
    /// Σ main sizes with margins of the non-grow items, plus the gaps.
    fix: i32,
    /// Σ min sizes of the grow items.
    grow_min: i32,
    /// Largest cross size with margins.
    cross: i32,
    count: i32,
    grow_count: i32,
}

/// The container's flex settings.
struct Flex {
    main: Axis,
    wrap: bool,
    rev: bool,
    main_place: FlexAlign,
    cross_place: FlexAlign,
    track_place: FlexAlign,
    item_gap: i32,
    track_gap: i32,
}

impl Flex {
    fn of<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> Self {
        let flow = style_enum::<T, FlexFlow>(t, id, PropId::FlexFlow);
        let row = !flow.is_column();
        let (pad_col, pad_row) = (
            t.style_i32(id, PropId::PadColumn),
            t.style_i32(id, PropId::PadRow),
        );
        Flex {
            main: if row { Axis::X } else { Axis::Y },
            wrap: flow.is_wrap(),
            rev: flow.is_reverse(),
            main_place: style_enum::<T, FlexAlign>(t, id, PropId::FlexMainPlace),
            cross_place: style_enum::<T, FlexAlign>(t, id, PropId::FlexCrossPlace),
            track_place: style_enum::<T, FlexAlign>(t, id, PropId::FlexTrackPlace),
            item_gap: if row { pad_col } else { pad_row },
            track_gap: if row { pad_row } else { pad_col },
        }
    }

    fn row(&self) -> bool {
        self.main == Axis::X
    }
}

/// `FlexGrow` of a node.
pub(crate) fn grow_of<T: LayoutTree + ?Sized>(t: &T, id: T::Id) -> u8 {
    t.style_prop(id, PropId::FlexGrow)
        .get::<u8>()
        .unwrap_or_else(|| t.style_i32(id, PropId::FlexGrow).clamp(0, 255) as u8)
}

/// LVGL `div_round_closest` (in `i64`, so `free × grow` cannot overflow).
fn div_round_closest(a: i64, b: i64) -> i64 {
    (a + b / 2) / b
}

/// Size of an item on `axis` including margins; `measure` applies the content-size exclusion.
fn extent_m<I>(it: &Item<I>, axis: Axis, measure: bool) -> i32 {
    if measure && it.ignore[axis.index()] {
        0
    } else {
        axis.of(it.size) + sum(it.margin, axis)
    }
}

/// LVGL `find_track_end`: the track starting at `first` (absolute index) in `items[..end]`.
fn find_track<I>(
    items: &[Item<I>],
    first: usize,
    f: &Flex,
    wrap: bool,
    max_main: i32,
    measure: bool,
) -> Track {
    let mut tr = Track {
        start: first,
        ..Track::default()
    };
    let mut j = first;
    let mut first_item = true;
    while j < items.len() {
        let it = &items[j];
        if !it.in_flow {
            j += 1;
            continue;
        }
        if !first_item && it.flags.contains(LayoutFlags::FLEX_IN_NEW_TRACK) {
            break;
        }
        let gap = if first_item { 0 } else { f.item_gap };
        if it.grow > 0 {
            let min = it.min[f.main.index()];
            if wrap && !first_item && tr.fix + tr.grow_min + min + gap > max_main {
                break;
            }
            tr.grow_min += min;
            tr.fix += gap;
            tr.grow_count += 1;
        } else {
            let req = extent_m(it, f.main, measure) + gap;
            if wrap && !first_item && tr.fix + tr.grow_min + req > max_main {
                break;
            }
            tr.fix += req;
        }
        first_item = false;
        tr.cross = tr.cross.max(extent_m(it, f.main.other(), measure));
        tr.count += 1;
        j += 1;
    }
    tr.end = j;
    tr.main = if tr.grow_count > 0 { max_main } else { tr.fix };
    tr
}

/// LVGL `place_content`: moves `start` and sets the extra `gap` for a placement mode.
fn place_content(place: FlexAlign, max: i32, content: i32, n: i32, start: &mut i32, gap: &mut i32) {
    let place = if n <= 1 && matches!(place, FlexAlign::SpaceAround | FlexAlign::SpaceEvenly) {
        FlexAlign::Center
    } else {
        place
    };
    match place {
        FlexAlign::Center => {
            *gap = 0;
            *start += (max - content) / 2;
        }
        FlexAlign::End => {
            *gap = 0;
            *start += max - content;
        }
        FlexAlign::SpaceBetween => {
            if n > 1 {
                *gap = (max - content) / (n - 1);
            }
        }
        FlexAlign::SpaceAround => {
            *gap += (max - content) / n;
            *start += *gap / 2;
        }
        FlexAlign::SpaceEvenly => {
            *gap = (max - content) / (n + 1);
            *start += *gap;
        }
        FlexAlign::Start => *gap = 0,
    }
}

/// Content extent of a flex container on `axis` (LVGL `calc_min_size` for the main axis:
/// the longest track without wrapping, other children ignored; the cross axis: the sum of the
/// tracks and gaps, or the extent of the non-flex children if larger).
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
    let f = Flex::of(t, id);
    let items = &mut s.items[frame..end];
    if f.rev {
        items.reverse();
    }
    let mut i = 0;
    if axis == f.main {
        let mut req = 0;
        loop {
            let tr = find_track(items, i, &f, false, 0, true);
            if tr.count == 0 {
                break;
            }
            req = req.max(tr.fix + tr.grow_min);
            i = tr.end;
        }
        return Some(req);
    }
    let wrap = f.wrap && !sized[f.main.index()];
    let max_main = f.main.of(avail);
    let mut total: Option<i32> = None;
    loop {
        let tr = find_track(items, i, &f, wrap, max_main, true);
        if tr.count == 0 {
            break;
        }
        total = Some(total.map_or(tr.cross, |v| v + f.track_gap + tr.cross));
        i = tr.end;
    }
    let others = abs_extent_all(t, items, axis, avail, sized, rtl);
    match (total, others) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// Grow distribution of one track (LVGL `children_repos`, first loop).
fn distribute_grow<I>(items: &mut [Item<I>], tr: &Track, main: Axis) {
    let a = main.index();
    let is_grow = |it: &Item<I>| it.in_flow && it.grow > 0;
    for it in items.iter_mut().filter(|it| is_grow(it)) {
        it.clamped = false;
    }
    let mut again = true;
    while again {
        again = false;
        let mut sum_grow: i64 = 0;
        let mut free = i64::from(tr.main - tr.fix);
        for it in items.iter().filter(|it| is_grow(it)) {
            if it.clamped {
                free -= i64::from(it.final_main);
            } else {
                sum_grow += i64::from(it.grow);
            }
        }
        for it in items.iter_mut().filter(|it| is_grow(it)) {
            if it.clamped {
                continue;
            }
            let g = i64::from(it.grow);
            let size =
                div_round_closest(free * g, sum_grow).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            let c = it.min[a].max(size.min(it.max[a]));
            if c != size {
                it.clamped = true;
                again = true;
            }
            it.final_main = c;
            sum_grow -= g;
            free -= i64::from(c);
        }
    }
}

/// Positions the flex items among `s.items[frame..end]` in the content area `content`.
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
    let f = Flex::of(&*t, id);
    let (main, cross) = (f.main, f.main.other());
    let csize = raw_size(content);
    let (max_main, max_cross) = (main.of(csize), cross.of(csize));
    let wrap = f.wrap && !sized[main.index()];
    if f.rev {
        s.items[frame..end].reverse();
    }

    for i in frame..end {
        if !s.items[i].in_flow {
            continue;
        }
        let c = s.items[i].id;
        let r = resolve_node(&*t, s, c, csize, [None; 2]);
        let it = &mut s.items[i];
        it.set_resolved(r);
        it.margin = margins(&*t, c);
        it.grow = grow_of(&*t, c);
    }

    // Build the tracks, grow their items and measure their final cross sizes.
    let tframe = s.tracks.len();
    let mut i = frame;
    loop {
        let mut tr = find_track(&s.items[..end], i, &f, wrap, max_main, false);
        if tr.count == 0 {
            break;
        }
        if tr.grow_count > 0 {
            distribute_grow(&mut s.items[tr.start..tr.end], &tr, main);
            for j in tr.start..tr.end {
                let it = s.items[j];
                if !it.in_flow || it.grow == 0 {
                    continue;
                }
                let mut ovr = [None; 2];
                ovr[main.index()] = Some(it.final_main);
                let r = resolve_node(&*t, s, it.id, csize, ovr);
                s.items[j].size = r.size;
                s.items[j].ovr[main.index()] = true;
            }
            tr.cross = s.items[tr.start..tr.end]
                .iter()
                .filter(|it| it.in_flow)
                .map(|it| extent_m(it, cross, false))
                .max()
                .unwrap_or(0);
        }
        i = tr.end;
        s.tracks.push(tr);
    }
    let ntracks = (s.tracks.len() - tframe) as i32;

    // Content-sized cross axis: tracks are packed at the start.
    let mut track_place = if sized[cross.index()] {
        FlexAlign::Start
    } else {
        f.track_place
    };
    let rtl_col = rtl && !f.row();
    if rtl_col {
        track_place = match track_place {
            FlexAlign::Start => FlexAlign::End,
            FlexAlign::End => FlexAlign::Start,
            other => other,
        };
    }
    let total =
        s.tracks[tframe..].iter().map(|tr| tr.cross).sum::<i32>() + f.track_gap * (ntracks - 1).max(0);
    let mut cross_pos = 0;
    let mut track_extra = 0;
    place_content(
        track_place,
        max_cross,
        total,
        ntracks,
        &mut cross_pos,
        &mut track_extra,
    );
    if rtl_col {
        cross_pos += total;
    }
    for k in tframe..s.tracks.len() {
        let tr = s.tracks[k];
        if rtl_col {
            cross_pos -= tr.cross;
        }
        place_track(t, s, &f, &tr, content, cross_pos, max_main, rtl);
        if rtl_col {
            cross_pos -= track_extra + f.track_gap;
        } else {
            cross_pos += tr.cross + track_extra + f.track_gap;
        }
    }
    s.tracks.truncate(tframe);
}

/// Positions the items of one track (LVGL `children_repos`, second part).
#[allow(clippy::too_many_arguments)]
fn place_track<T: LayoutTree + ?Sized>(
    t: &mut T,
    s: &mut LayoutScratch<T::Id>,
    f: &Flex,
    tr: &Track,
    content: Rect,
    cross_pos: i32,
    max_main: i32,
    rtl: bool,
) {
    let (main, cross) = (f.main, f.main.other());
    let mut main_pos = 0;
    let mut extra = 0;
    place_content(
        f.main_place,
        max_main,
        tr.main,
        tr.count,
        &mut main_pos,
        &mut extra,
    );
    let rtl_row = rtl && f.row();
    if rtl_row {
        main_pos = max_main - main_pos;
    }
    for j in tr.start..tr.end {
        let it = s.items[j];
        if !it.in_flow {
            continue;
        }
        let (m, size) = (it.margin, it.size);
        let cross_off = match f.cross_place {
            FlexAlign::Center => {
                (((tr.cross + 1) & !1) - cross.of(size)) / 2 + (start(m, cross) - end(m, cross)) / 2
            }
            FlexAlign::End => tr.cross - cross.of(size) - end(m, cross),
            _ => start(m, cross),
        };
        if rtl_row {
            main_pos -= main.of(size);
        }
        let (tx, ty) = translate(&*t, it.id, size);
        let mpos = main_pos + start(m, main);
        let (x, y) = if f.row() {
            (mpos, cross_pos + cross_off)
        } else {
            (cross_pos + cross_off, mpos)
        };
        place(
            t,
            it.id,
            Rect::from_xywh(content.x0 + x + tx, content.y0 + y + ty, size.w, size.h),
        );
        s.items[j].done = true;
        if rtl_row {
            main_pos -= f.item_gap + extra;
        } else {
            main_pos += main.of(size) + f.item_gap + extra + sum(m, main);
        }
    }
}
