//! The browsing pages: Albums, Artists and Playlists — grids of generated artwork.
//!
//! The catalogue never changes, so these pages are built once as plain rows of tiles in a
//! scrolling column (no keyed lists needed); the only live binding per tile is "is this
//! playing", which tints the title. Covers are canvases, so a tile is a few nodes and its
//! artwork is drawn on demand as it scrolls into view.
//!
//! Clicking an album or a playlist plays it from the top; clicking an artist opens the Songs
//! page searched for them — the same filtered, sortable table instead of a second list.

use twine::prelude::*;

use crate::art::{Cover, cover};
use crate::catalog::catalog;
use crate::theme::{BODY, DISPLAY, PRESSED, RAISED, S1, S2, S3, S4, S5, TITLE};
use crate::ui::{App, Page, caption, pill, text};
use crate::util::summary;

/// The page frame: a title, a subtitle and a scrolling body.
fn frame(title: &'static str, subtitle: String, body: impl View + 'static) -> Flex {
    column((
        column((
            label(title).font(DISPLAY).text_color(design::ON_SURFACE),
            caption(subtitle),
        ))
        .gap(2)
        .padding_each(Insets::new(S5, S5, S5, 0)),
        scroll_view(Axis::Vertical, body)
            .fill_width()
            .flex_grow(1)
            .padding_each(Insets::new(S5, S4, S5, S5)),
    ))
    .fill()
    .gap(0)
    .scrollable(false)
}

/// `items` in rows of `per_row` tiles.
fn rows(items: Vec<Flex>, per_row: usize, gap: i32) -> Flex {
    let mut out = Vec::new();
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let chunk: Vec<Flex> = it.by_ref().take(per_row).collect();
        out.push(row(chunk).gap(gap).scrollable(false));
    }
    column(out).fill_width().gap(S5).scrollable(false)
}

/// The tile behaviour shared by every grid: press feedback, focus back to the shortcuts.
fn tile(app: App, body: Flex) -> Flex {
    body.radius(10)
        .padding(S2)
        .scrollable(false)
        .on_state(State::PRESSED, |s| s.bg(PRESSED))
        .on_press(move || app.focus.release())
}

/// Album tiles: 4 per row, 148 px covers.
pub fn albums(app: App) -> impl View {
    let lib = catalog();
    let tiles = lib
        .albums
        .iter()
        .map(|al| {
            let id = al.id;
            let playing = move || lib.track(app.player.current.get()).album == id;
            tile(
                app,
                column((
                    cover(Cover::Album(id)).size(148, 148),
                    text(al.title)
                        .width(148)
                        .margin_each(Insets::new(0, S2, 0, 0))
                        .text_color(move || {
                            if playing() {
                                design::PRIMARY
                            } else {
                                design::ON_SURFACE
                            }
                        }),
                    caption(format!("{} • {}", lib.artist(al.artist).name, al.year)).width(148),
                ))
                .gap(2),
            )
            .on_click(move || app.player.play_list(&lib.album(id).tracks, 0))
        })
        .collect();
    frame(
        "Albums",
        format!("{} albums", lib.albums.len()),
        rows(tiles, 4, S3),
    )
}

/// Artist tiles: 5 per row, round portraits from their latest cover.
pub fn artists(app: App) -> impl View {
    let lib = catalog();
    let mut order: Vec<usize> = (0..lib.artists.len()).collect();
    order.sort_by_key(|&i| lib.artists[i].name);
    let tiles = order
        .into_iter()
        .map(|i| {
            let ar = &lib.artists[i];
            let latest = *ar.albums.last().unwrap_or(&0);
            let songs: usize = ar.albums.iter().map(|&a| lib.album(a).tracks.len()).sum();
            let name = ar.name;
            tile(
                app,
                column((
                    cover(Cover::Portrait(latest)).size(112, 112),
                    text(name)
                        .width(120)
                        .text_align(TextAlign::Center)
                        .margin_each(Insets::new(0, S2, 0, 0)),
                    caption(format!("{} • {songs} songs", ar.genre.name()))
                        .width(120)
                        .text_align(TextAlign::Center),
                ))
                .gap(2)
                .align_items(CrossAlign::Center),
            )
            .on_click(move || {
                app.query.set(name.to_string());
                app.open(Page::Songs);
            })
        })
        .collect();
    frame(
        "Artists",
        format!("{} artists", lib.artists.len()),
        rows(tiles, 5, S1),
    )
}

/// Playlist cards: 2 per row.
pub fn playlists(app: App) -> impl View {
    let lib = catalog();
    let cards = lib
        .playlists
        .iter()
        .enumerate()
        .map(|(i, pl)| {
            let secs = lib.total_secs(&pl.tracks);
            row((
                cover(Cover::Playlist(pl.hue)).size(104, 104),
                column((
                    label(pl.name).font(TITLE).text_color(design::ON_SURFACE),
                    label(pl.blurb)
                        .font(BODY)
                        .text_color(design::ON_SURFACE_MUTED)
                        .width(196)
                        .long_mode(LongMode::Wrap),
                    caption(format!("{} songs, {}", pl.tracks.len(), summary(secs))),
                    pill(app, format!("{}  Play", Symbol::Play), true, move || {
                        app.player.play_list(&catalog().playlists[i].tracks, 0);
                    })
                    .margin_each(Insets::new(0, S1, 0, 0)),
                ))
                .gap(S1),
            ))
            .width(344)
            .padding(S3)
            .gap(S4)
            .radius(14)
            .bg(RAISED)
            .scrollable(false)
        })
        .collect();
    frame(
        "Playlists",
        format!("{} playlists, curated for you", lib.playlists.len()),
        rows(cards, 2, S4),
    )
}
