//! The Songs page: the whole library as a sortable table with incremental search.
//!
//! **Data flow.** The search text (`App::query`, bound two-way to the search field) and the
//! sort order feed one memo, `visible`, that filters the catalogue and sorts the matching
//! ids — about 500 string comparisons, done once per keystroke or header click, never per
//! frame. The table is a [`virtual_list`] over those ids: only the ~12 visible rows exist,
//! scrolling disposes and builds rows at the edges, and a new search result keeps the rows
//! whose ids are still visible (moving them if needed). A row looks its track up in the
//! static catalogue, so its item is a bare `u32`; its small cover is a canvas drawn once
//! when the row appears.
//!
//! **Per-row reactivity.** A row's only live bindings are "is this the current track"
//! (accent title, the animated playing icon instead of the number) and "is this the
//! selected row". The playing icon follows the visualizer every frame, so it is built only
//! inside the current row (`when`): the other rows never wake up during playback.
//!
//! **Input.** Click selects a row, double-click plays it with the rest of the visible list
//! queued after it — like a desktop music app.

use std::cmp::Ordering;

use twine::prelude::*;

use crate::art::{Cover, cover};
use crate::catalog::catalog;
use crate::theme::{BODY, DISPLAY, FAINT, LINE, PRESSED, RAISED, S2, S3, S5};
use crate::ui::{App, Glyph, Icon, caption, icon, icon_button, overline, text};
use crate::util::{clock, summary};
use crate::viz::{icon_bars, playing_icon};

/// A sortable column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    /// Album order (the library's natural order).
    Number,
    Title,
    Artist,
    Album,
    Time,
}

/// Column widths; they add up to the table's inner width (752 − 2 × 24 px padding,
/// minus four 8 px gaps).
const W_NUM: i32 = 40;
const W_TITLE: i32 = 262;
const W_ARTIST: i32 = 150;
const W_ALBUM: i32 = 168;
const W_TIME: i32 = 52;
/// Row height.
const ROW_H: i32 = 40;

/// Whether `haystack` contains every word of `query` (already lower case).
fn matches(haystack: &str, query: &str) -> bool {
    query.split_whitespace().all(|w| haystack.contains(w))
}

/// Orders two tracks by `col` (ties: library order, so sorting is stable and predictable).
fn compare(col: Column, a: u32, b: u32) -> Ordering {
    let lib = catalog();
    let (ta, tb) = (lib.track(a), lib.track(b));
    let primary = match col {
        Column::Number => Ordering::Equal,
        Column::Title => ta.title.to_lowercase().cmp(&tb.title.to_lowercase()),
        Column::Artist => lib.artist_of(a).cmp(lib.artist_of(b)),
        Column::Album => lib.album_of(a).cmp(lib.album_of(b)),
        Column::Time => ta.secs.cmp(&tb.secs),
    };
    primary.then(a.cmp(&b))
}

pub fn page(cx: Scope, app: App) -> impl View {
    let sort = cx.signal((Column::Number, true));
    let selected = cx.signal(None::<u32>);
    let visible = cx.memo(move || {
        let q = app.query.with(|q| q.trim().to_lowercase());
        let (col, asc) = sort.get();
        let mut ids: Vec<u32> = catalog()
            .tracks
            .iter()
            .filter(|t| q.is_empty() || matches(&t.haystack, &q))
            .map(|t| t.id)
            .collect();
        ids.sort_by(|&a, &b| {
            let o = compare(col, a, b);
            if asc { o } else { o.reverse() }
        });
        ids
    });
    let count = cx.memo(move || visible.with(|v| (v.len(), catalog().total_secs(v))));

    // Plays `id` with the rest of the visible list after it.
    let play = move |id: u32| {
        visible.with_untracked(|v| {
            if let Some(i) = v.iter().position(|&t| t == id) {
                app.player.play_list(v, i);
            }
        });
    };

    let header = row((
        column((
            label("Songs").font(DISPLAY).text_color(design::ON_SURFACE),
            caption(text!("{} songs, {}", count.get().0, summary(count.get().1))).test_id("song-count"),
        ))
        .gap(2),
        spacer(),
        search_box(app),
    ))
    .fill_width()
    .align_items(CrossAlign::End);

    let columns = row((
        sort_header(app, sort, Column::Number, "#", W_NUM),
        sort_header(app, sort, Column::Title, "TITLE", W_TITLE),
        sort_header(app, sort, Column::Artist, "ARTIST", W_ARTIST),
        sort_header(app, sort, Column::Album, "ALBUM", W_ALBUM),
        sort_header(app, sort, Column::Time, "TIME", W_TIME),
    ))
    .fill_width()
    .height(28)
    .gap(S2)
    .scrollable(false)
    .align_items(CrossAlign::Center)
    .border(1, LINE)
    .border_side(BorderSide::BOTTOM);

    let table = virtual_list(
        visible,
        |id| *id,
        ROW_H,
        move |_cx, id| song_row(app, id.get_untracked(), selected, play),
    )
    .fill_width()
    .flex_grow(1)
    // A slim, quiet scrollbar.
    .part(Part::Scrollbar, |s| s.width(4).radius(Radius::Circle).bg(FAINT))
    .test_id("songs");

    let empty = when(
        move || visible.with(Vec::is_empty),
        move |_| {
            column((
                icon(Glyph {
                    icon: Icon::Search,
                    active: false,
                })
                .size(40, 40),
                label(text!("No results for \"{}\"", app.query.get().trim()))
                    .font(BODY)
                    .text_color(design::ON_SURFACE_MUTED),
                caption("Check the spelling, or try an artist or album name."),
            ))
            .fill_width()
            .gap(S2)
            .padding_y(48)
            .align_items(CrossAlign::Center)
        },
    );

    column((header, columns, empty, table))
        .fill()
        .padding_each(Insets::new(S5, S5, S5, 0))
        .gap(S3)
        .scrollable(false)
}

/// The search field: an icon, the text and a clear button, in a raised pill. Esc clears it,
/// Enter returns the keyboard to the shortcuts (the results stay).
fn search_box(app: App) -> Flex {
    let field = textarea(app.query)
        .one_line(true)
        .placeholder("Search your library")
        .font(BODY)
        .size(264, 36)
        .bg(RAISED)
        .border(1, RAISED)
        .radius(10)
        .padding_each(Insets::new(36, 9, 30, 9))
        .text_color(design::ON_SURFACE)
        .on_state(State::FOCUSED, |s| s.border(1, design::PRIMARY))
        .on_ready(move || app.focus.release())
        .on_key(move |k| {
            if k == Key::Esc {
                app.query.set(String::new());
                app.focus.release();
            }
        })
        .node_ref(app.focus.search)
        .test_id("search");
    row((
        field,
        icon(Glyph {
            icon: Icon::Search,
            active: false,
        })
        .size(16, 16)
        .pos(12, 10),
        icon_button(app, Symbol::Close.to_string(), 24, false, move || {
            app.query.set(String::new());
        })
        .pos(234, 6)
        .hidden(move || app.query.with(String::is_empty))
        .text_color(FAINT),
    ))
    .layout(Layout::none())
    .size(264, 36)
}

/// A column header: click to sort by it, again to reverse. The sorted column shows an arrow.
fn sort_header(app: App, sort: Signal<(Column, bool)>, col: Column, name: &'static str, width: i32) -> Flex {
    let arrow = move || match sort.get() {
        (c, true) if c == col => format!(" {}", Symbol::Up),
        (c, false) if c == col => format!(" {}", Symbol::Down),
        _ => String::new(),
    };
    let row_view = row((
        overline(name),
        label(text!("{}", arrow()))
            .font(&fonts::MONTSERRAT_8)
            .text_color(design::PRIMARY),
    ))
    .width(width)
    .fill_height()
    .align_items(CrossAlign::Center)
    .scrollable(false)
    .on_press(move || app.focus.release())
    .on_click(move || {
        sort.update(|(c, asc)| {
            if *c == col {
                *asc = !*asc;
            } else {
                (*c, *asc) = (col, true);
            }
        });
    });
    if col == Column::Time {
        row_view.justify(MainAlign::End)
    } else {
        row_view
    }
}

/// One track row.
fn song_row(app: App, id: u32, selected: Signal<Option<u32>>, play: impl Fn(u32) + Copy + 'static) -> Flex {
    let lib = catalog();
    let t = lib.track(id);
    let current = move || app.player.current.get() == id;
    let number = row((
        when(
            move || !current(),
            move |_| label(t.number.to_string()).font(BODY).text_color(FAINT),
        ),
        when(current, move |_| {
            playing_icon(move || {
                if app.player.playing.get() {
                    icon_bars(&app.player.spectrum.get())
                } else {
                    [250, 450, 300]
                }
            })
            .size(16, 14)
        }),
    ))
    .width(W_NUM)
    .align_items(CrossAlign::Center)
    .scrollable(false);
    row((
        number,
        row((
            cover(Cover::Album(t.album)).size(28, 28),
            text(t.title.as_str()).flex_grow(1).text_color(move || {
                if current() {
                    design::PRIMARY
                } else {
                    design::ON_SURFACE
                }
            }),
        ))
        .width(W_TITLE)
        .gap(S3)
        .align_items(CrossAlign::Center)
        .scrollable(false),
        text(lib.artist_of(id))
            .width(W_ARTIST)
            .text_color(design::ON_SURFACE_MUTED),
        text(lib.album_of(id))
            .width(W_ALBUM)
            .text_color(design::ON_SURFACE_MUTED),
        text(clock(t.secs))
            .width(W_TIME)
            .text_align(TextAlign::Right)
            .text_color(design::ON_SURFACE_MUTED),
    ))
    .fill_width()
    .height(ROW_H)
    .gap(S2)
    .radius(6)
    .align_items(CrossAlign::Center)
    .scrollable(false)
    .state(State::CHECKED, move || selected.get() == Some(id))
    .on_state(State::CHECKED, |s| s.bg(RAISED))
    .on_state(State::PRESSED, |s| s.bg(PRESSED))
    .on_press(move || app.focus.release())
    .on_click(move || selected.set(Some(id)))
    .on_code(EventCode::DoubleClicked, move || play(id))
}
