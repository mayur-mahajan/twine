//! The Now Playing page: the cover and details of the current track and the next two in
//! the queue, next to the large visualizer with its Bars / Radial switch and a legend of the
//! keyboard shortcuts.
//!
//! The visualizer is one canvas bound to a closure that combines the player's spectrum
//! (written every frame by the player clock), the chosen mode and the album's hue; the
//! canvas redraws when that combined value changes, i.e. once per frame while music plays
//! and not at all once the bars have settled after a pause. Clicking it switches the mode.

use twine::prelude::*;

use crate::art::{Cover, cover};
use crate::catalog::catalog;
use crate::player::Entry;
use crate::theme::{BODY, CAPTION, HEADLINE, LINE, PRESSED, RAISED, S1, S2, S3, S4, S5};
use crate::ui::{App, caption, overline, text};
use crate::viz::{Frame, VizMode, visualizer};

pub fn page(app: App) -> impl View {
    let lib = catalog();
    let track = move || lib.track(app.player.current.get());
    let details = column((
        cover(move || Cover::Album(track().album)).size(236, 236),
        label(text!("{}", track().title))
            .font(HEADLINE)
            .text_color(design::ON_SURFACE)
            .width(236)
            .long_mode(LongMode::Dots)
            .max_lines(1)
            .margin_each(Insets::new(0, S4, 0, 0))
            .test_id("np-title"),
        label(text!("{}", lib.artist_of(track().id)))
            .font(BODY)
            .text_color(design::PRIMARY),
        caption(text!(
            "{} • {}",
            lib.album_of(track().id),
            lib.album(track().album).year
        ))
        .width(236),
        row((
            chip(move || lib.genre_of(track().id).name().to_string()),
            chip(move || format!("{} BPM", track().bpm)),
            chip(move || format!("Track {}", track().number)),
        ))
        .gap(S2)
        .margin_each(Insets::new(0, S2, 0, 0)),
        overline("UP NEXT").margin_each(Insets::new(0, S4, 0, S1)),
        column(for_each(
            move || {
                app.player
                    .queue
                    .with(|q| q.iter().take(2).copied().collect::<Vec<Entry>>())
            },
            |e| e.key,
            move |_cx, e| up_next(app, e.get_untracked()),
        ))
        .fill_width()
        .gap(2)
        .scrollable(false),
    ))
    .width(236)
    .gap(S1)
    .scrollable(false);

    let frame = move || Frame {
        spectrum: app.player.spectrum.get(),
        mode: app.viz.get(),
        hue: lib.album(track().album).hue,
    };
    let stage = column((
        row((
            overline("VISUALIZER"),
            spacer(),
            segment(app, "Bars", VizMode::Bars),
            segment(app, "Radial", VizMode::Radial),
        ))
        .fill_width()
        .gap(S1)
        .align_items(CrossAlign::Center),
        visualizer(frame)
            .fill_width()
            .flex_grow(1)
            .clickable(true)
            .on_click(move || {
                app.viz.update(|m| {
                    *m = match m {
                        VizMode::Bars => VizMode::Radial,
                        VizMode::Radial => VizMode::Bars,
                    }
                });
            })
            .test_id("visualizer"),
        shortcuts_legend(),
    ))
    .flex_grow(1)
    .fill_height()
    .gap(S3)
    .scrollable(false);

    row((details, stage)).fill().padding(S5).gap(S5).scrollable(false)
}

/// A queued track: cover, title and artist; click to play it now.
fn up_next(app: App, e: Entry) -> Flex {
    let lib = catalog();
    let t = lib.track(e.track);
    row((
        cover(Cover::Album(t.album)).size(32, 32),
        column((
            text(t.title.as_str()).fill_width(),
            caption(lib.artist_of(t.id)).fill_width(),
        ))
        .flex_grow(1)
        .gap(0)
        .scrollable(false),
    ))
    .fill_width()
    .height(40)
    .gap(S3)
    .padding_x(S1)
    .radius(8)
    .align_items(CrossAlign::Center)
    .scrollable(false)
    .on_state(State::PRESSED, |s| s.bg(PRESSED))
    .on_press(move || app.focus.release())
    .on_click(move || app.player.jump_to(e.key))
}

/// A small rounded tag.
fn chip<M>(t: impl IntoText<M>) -> Flex {
    row(label(t).font(CAPTION).text_color(design::ON_SURFACE_MUTED))
        .padding_x(S2)
        .height(22)
        .radius(Radius::Circle)
        .border(1, LINE)
        .align_items(CrossAlign::Center)
        .scrollable(false)
}

/// One option of the visualizer's mode switch.
fn segment(app: App, name: &'static str, mode: VizMode) -> Flex {
    row(label(name).font(CAPTION))
        .padding_x(S3)
        .height(26)
        .radius(Radius::Circle)
        .align_items(CrossAlign::Center)
        .text_color(design::ON_SURFACE_MUTED)
        .scrollable(false)
        .state(State::CHECKED, move || app.viz.get() == mode)
        .on_state(State::CHECKED, |s| s.bg(RAISED).text_color(design::ON_SURFACE))
        .on_state(State::PRESSED, |s| s.bg(PRESSED))
        .on_press(move || app.focus.release())
        .on_click(move || app.viz.set(mode))
        .test_id(name)
}

/// The keyboard shortcuts, as key caps and what they do.
fn shortcuts_legend() -> Flex {
    let keys: [(String, &str); 4] = [
        ("Space".into(), "Play / pause"),
        (format!("{} {}", Symbol::Left, Symbol::Right), "Seek"),
        (format!("{} {}", Symbol::Up, Symbol::Down), "Volume"),
        ("V".into(), "Visualizer"),
    ];
    row(keys
        .into_iter()
        .map(|(cap, what)| {
            row((
                row(label(cap)
                    .font(&fonts::MONTSERRAT_10)
                    .text_color(design::ON_SURFACE))
                .padding_x(6)
                .height(18)
                .radius(4)
                .bg(RAISED)
                .border(1, LINE)
                .align_items(CrossAlign::Center)
                .scrollable(false),
                caption(what),
            ))
            .gap(6)
            .align_items(CrossAlign::Center)
            .scrollable(false)
        })
        .collect::<Vec<_>>())
    .fill_width()
    .gap(S4)
    .scrollable(false)
}
