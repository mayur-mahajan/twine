//! The sidebar: the brand, navigation between the library's pages and the player's pages,
//! and the playlists for one-click listening.
//!
//! Each entry is a row bound to the `page` signal: its highlight (a raised background, the
//! accent icon, the bright label) is a `CHECKED` state computed from that signal, so moving
//! between pages touches two rows' state and nothing else. The playlists at the bottom are
//! static (the catalogue does not change), so they are a plain `Vec` of rows.

use twine::prelude::*;
use twine::render::GradKind;

use crate::canvas::canvas;
use crate::catalog::catalog;
use crate::paint::{gradient, line, rrect_grad};
use crate::theme::{BODY, PRESSED, RAISED, S2, S3, S4, SIDEBAR, TITLE, palette};
use crate::ui::{App, Glyph, Icon, Page, icon, overline};

/// The sidebar's width.
pub const WIDTH: i32 = 208;
/// How many playlists the sidebar lists (the Playlists page has them all).
const SIDEBAR_PLAYLISTS: usize = 4;

pub fn sidebar(app: App) -> impl View {
    let lib = catalog();
    column((
        brand().margin_each(Insets::new(S2, S2, 0, S4)),
        overline("LIBRARY").margin_each(Insets::new(S3, 0, 0, S2)),
        nav(app, Icon::Songs, "Songs", Page::Songs),
        nav(app, Icon::Albums, "Albums", Page::Albums),
        nav(app, Icon::Artists, "Artists", Page::Artists),
        nav(app, Icon::Playlists, "Playlists", Page::Playlists),
        overline("PLAYER").margin_each(Insets::new(S3, S4, 0, S2)),
        nav(app, Icon::NowPlaying, "Now Playing", Page::NowPlaying),
        nav(app, Icon::Equalizer, "Equalizer", Page::Equalizer),
        overline("PLAYLISTS").margin_each(Insets::new(S3, S4, 0, S2)),
        column(
            lib.playlists
                .iter()
                .take(SIDEBAR_PLAYLISTS)
                .enumerate()
                .map(|(i, pl)| playlist_row(app, i, pl.name))
                .collect::<Vec<_>>(),
        )
        .fill_width()
        .gap(0),
    ))
    .width(WIDTH)
    .fill_height()
    .padding(S3)
    .gap(2)
    .bg(SIDEBAR)
    .scrollable(false)
}

/// The app mark (a coral-to-amber tile with a waveform) and name.
fn brand() -> Flex {
    row((
        canvas((), |(), cx| {
            let a = cx.coords();
            let p = cx.painter();
            let g = gradient(
                GradKind::Linear {
                    start: Point::new(0, a.height()),
                    end: Point::new(a.width(), 0),
                },
                palette::ACCENT,
                palette::AMBER,
            );
            rrect_grad(p, a, 8, &g);
            // A twine of three strands: short to tall to short.
            let c = a.center();
            for (i, h) in [4, 8, 5].into_iter().enumerate() {
                let x = c.x - 6 + i as i32 * 6;
                line(
                    p,
                    Point::new(x, c.y - h),
                    Point::new(x, c.y + h),
                    3,
                    Color::hex(0x1A_0D_0B),
                    Opa::COVER,
                );
            }
        })
        .size(30, 30),
        label("Twine Player").font(TITLE).text_color(design::ON_SURFACE),
    ))
    .gap(S3)
    .align_items(CrossAlign::Center)
}

/// One navigation entry.
fn nav(app: App, kind: Icon, name: &'static str, page: Page) -> Flex {
    let on = move || app.page.get() == page;
    row((
        icon(move || Glyph {
            icon: kind,
            active: on(),
        }),
        label(name).font(BODY),
    ))
    .fill_width()
    .height(36)
    .padding_x(S3)
    .gap(S3)
    .radius(8)
    .align_items(CrossAlign::Center)
    .text_color(design::ON_SURFACE_MUTED)
    .scrollable(false)
    .state(State::CHECKED, on)
    .on_state(State::CHECKED, |s| s.bg(RAISED).text_color(design::ON_SURFACE))
    .on_state(State::PRESSED, |s| s.bg(PRESSED))
    .on_click(move || app.open(page))
    .test_id(name)
}

/// A playlist entry: plays it from the top.
fn playlist_row(app: App, index: usize, name: &'static str) -> Flex {
    row(label(name)
        .font(BODY)
        .long_mode(LongMode::Dots)
        .max_lines(1)
        .flex_grow(1))
    .fill_width()
    .height(30)
    .padding_x(S3)
    .radius(8)
    .align_items(CrossAlign::Center)
    .text_color(design::ON_SURFACE_MUTED)
    .scrollable(false)
    .on_state(State::PRESSED, |s| s.bg(PRESSED).text_color(design::ON_SURFACE))
    .on_press(move || app.focus.release())
    .on_click(move || {
        app.player.play_list(&catalog().playlists[index].tracks, 0);
    })
}
