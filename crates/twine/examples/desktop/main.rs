//! `cargo xtask sim desktop` (or `cargo run -p twine --example desktop`): **Twine Player**, a
//! desktop music player built with Twine — a 960 × 600 window, an ordinary program on Linux,
//! macOS or Windows.
//!
//! | Area | What it does |
//! |------|--------------|
//! | Sidebar | Songs, Albums, Artists, Playlists; Now Playing and Equalizer; one-click playlists |
//! | Songs | ~500 tracks in a virtualized table: incremental search, sortable columns, double-click to play |
//! | Albums, Artists, Playlists | grids of generated cover art |
//! | Now Playing | the cover and details, a 32-band visualizer (bars or radial) with falling peak caps |
//! | Equalizer | 10 bands (31 Hz–16 kHz), eased presets, the live frequency-response curve |
//! | Player bar | transport, seek, shuffle, repeat, stereo level meters, volume, the Up Next queue |
//!
//! Keys: Space plays / pauses, ← / → seek 5 s, ↑ / ↓ change the volume, F (or Ctrl/Cmd+F)
//! searches, Esc leaves the search (or closes the queue), V switches the visualizer, Q opens
//! the queue, N / P skip, M mutes.
//!
//! **No real audio.** Playback is simulated: a frame clock advances the position, and a
//! synthetic, beat-synced signal per track (see `audio`) feeds the visualizer and the meters,
//! reshaped by the equalizer. Everything is seeded and driven by the engine's clock, so
//! headless scripted runs are deterministic.
//!
//! **Structure.** One `Copy` store, [`App`](ui::App), holds the player, the equalizer and the
//! navigation signals and is handed to every screen. Screens read the signals they show and
//! call store methods to act; nothing else is shared. The main area keeps every page built and
//! shows one at a time (with a short fade), so switching pages keeps scroll positions and
//! costs no rebuild.
//!
//! | Module | Role |
//! |--------|------|
//! | `catalog` | the generated library (static) |
//! | `player`, `eq`, `audio`, `viz` | playback state, equalizer math, simulated sound, visualizer ballistics |
//! | `canvas`, `paint`, `art` | the custom-drawn widget, drawing helpers, generated cover art |
//! | `sidebar`, `library`, `browse`, `now_playing`, `equalizer`, `queue`, `transport` | the screens |
//! | `theme`, `ui`, `util` | palette and type scale, shared components and focus, helpers |

// `App` bundles ~30 signal handles (a few hundred bytes of ids, no data) and is copied into
// the closures that use it — Twine's store pattern; passing it by reference would only add
// dereferences.
#![allow(clippy::large_types_passed_by_value)]

mod art;
mod audio;
mod browse;
mod canvas;
mod catalog;
mod eq;
mod equalizer;
mod library;
mod now_playing;
mod paint;
mod player;
mod queue;
mod sidebar;
mod theme;
mod transport;
mod ui;
mod util;
mod viz;

use twine::core::ColorFormat;
use twine::prelude::*;
use twine_sim::SimConfig;

use catalog::catalog;
use eq::Equalizer;
use player::Player;
use ui::{App, Focus, Page};
use viz::VizMode;

/// The window's size.
const WIDTH: u16 = 960;
const HEIGHT: u16 = 600;
/// The album and track the player opens with ("Nightdrive 1987", second track), and where
/// in it — as if restoring the last session.
const START_ALBUM: u32 = 18;
const START_TRACK: usize = 1;
const START_MS: u32 = 41_000;

/// Seek step of ← / →.
const SEEK_MS: i32 = 5000;
/// Volume step of ↑ / ↓.
const VOLUME_STEP: i32 = 5;

fn app(cx: Scope) -> impl View {
    let album = &catalog().album(START_ALBUM).tracks;
    let player = Player::new(cx, album.clone(), album[START_TRACK], START_MS);
    let eq = Equalizer::new(cx);
    let app = App {
        player,
        eq,
        page: cx.signal(Page::Songs),
        query: cx.signal(String::new()),
        viz: cx.signal(VizMode::Bars),
        queue_open: cx.signal(false),
        focus: Focus::new(cx),
    };
    player.run(cx, eq);
    player.playing.set(true);

    let main_height = i32::from(HEIGHT) - transport::HEIGHT;
    let pages = row((
        screen(cx, app, Page::Songs, library::page(cx, app)),
        screen(cx, app, Page::Albums, browse::albums(app)),
        screen(cx, app, Page::Artists, browse::artists(app)),
        screen(cx, app, Page::Playlists, browse::playlists(app)),
        screen(cx, app, Page::NowPlaying, now_playing::page(app)),
        screen(cx, app, Page::Equalizer, equalizer::page(cx, app)),
    ))
    .layout(Layout::none())
    .padding(0)
    .flex_grow(1)
    .fill_height()
    .bg(design::BACKGROUND)
    .scrollable(false);

    column((
        row((sidebar::sidebar(app), pages))
            .fill_width()
            .height(main_height)
            .gap(0)
            .scrollable(false),
        transport::bar(cx, app),
        queue::panel(cx, app, main_height),
    ))
    .size(i32::from(WIDTH), i32::from(HEIGHT))
    .gap(0)
    .bg(design::BACKGROUND)
    .scrollable(false)
    // The window owns the keyboard shortcuts: it is the focused node whenever the search
    // field is not (see `ui::Focus`).
    .focusable(true)
    .on_state(State::FOCUS_KEY, |s| s.outline_width(0))
    .on_key(move |k| shortcut(app, k))
    .node_ref(app.focus.root)
}

/// A page of the main area: shown while it is the current page, fading and rising into
/// place when it becomes current.
fn screen(cx: Scope, app: App, page: Page, view: impl View + 'static) -> Flex {
    let shown = move || app.page.get() == page;
    let t = cx.tween(
        move || if shown() { 1000 } else { 0 },
        AnimSpec::new(Duration::ms(220)).ease_out(),
    );
    row(view)
        .fill()
        .scrollable(false)
        .hidden(move || !shown())
        .opacity(move || Opa::from_raw((t.get() * 255 / 1000) as u8))
        .translate_y(move || (1000 - t.get()) * 10 / 1000)
}

/// The window's keyboard shortcuts.
fn shortcut(app: App, key: Key) {
    let p = app.player;
    match key {
        Key::Char(' ') => p.toggle(),
        Key::Left => p.seek_by(-SEEK_MS),
        Key::Right => p.seek_by(SEEK_MS),
        Key::Up => p.nudge_volume(VOLUME_STEP),
        Key::Down => p.nudge_volume(-VOLUME_STEP),
        // The simulator's keyboard reports no modifiers, so Ctrl+F and Cmd+F arrive as `f`:
        // with the shortcuts focused, `f` (or `/`) always means "search".
        Key::Char('f' | 'F' | '/') => {
            app.open(Page::Songs);
            app.focus.search();
        }
        Key::Char('v' | 'V') => app.viz.update(|m| {
            *m = if *m == VizMode::Bars {
                VizMode::Radial
            } else {
                VizMode::Bars
            };
        }),
        Key::Char('q' | 'Q') => app.queue_open.update(|o| *o = !*o),
        Key::Char('n' | 'N') => p.next(),
        Key::Char('p' | 'P') => p.prev(),
        Key::Char('m' | 'M') => p.muted.update(|m| *m = !*m),
        Key::Esc => app.queue_open.set(false),
        _ => {}
    }
}

fn main() {
    twine_sim::run(
        SimConfig::new(WIDTH, HEIGHT)
            .title("Twine Player")
            .scale(1)
            .format(ColorFormat::Xrgb8888)
            .theme(theme::theme()),
        app,
    );
}
