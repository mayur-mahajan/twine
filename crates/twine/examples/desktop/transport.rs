//! The player bar along the bottom of the window: the current track on the left, the
//! transport and seek bar in the middle, level meters, the queue toggle and volume on the
//! right.
//!
//! **Bindings.** Everything here reads the [`Player`](crate::player::Player) store. The seek
//! bar is the busiest: its value and the two time labels follow `position`, which the frame
//! clock advances every 16 ms; the labels only re-render when the displayed second changes
//! (text bindings compare before they draw). The seek slider shows a copy of the position
//! (`seek`, updated by an effect) and reports drags through `on_change`, so dragging seeks
//! and playback keeps moving the knob without the two fighting.

use twine::prelude::*;

use crate::art::{Cover, cover};
use crate::catalog::catalog;
use crate::player::Repeat;
use crate::theme::{CAPTION, LINE, S1, S2, S3, S4, SIDEBAR, palette};
use crate::ui::{App, caption, icon_button, slim_slider, text};
use crate::util::clock;
use crate::viz::meters;

/// The bar's height.
pub const HEIGHT: i32 = 84;
/// Width of the left and right sections (equal, so the transport is centred in the window).
const SIDE: i32 = 280;

pub fn bar(cx: Scope, app: App) -> impl View {
    let lib = catalog();
    let queue_open = app.queue_open;
    let player = app.player;
    let track = move || lib.track(player.current.get());

    let now = row((
        cover(move || Cover::Album(track().album))
            .size(56, 56)
            .shadow_width(16)
            .shadow_opacity(Opa::P40)
            .shadow_color(Color::BLACK),
        column((
            text(text!("{}", track().title)).fill_width().test_id("bar-title"),
            caption(text!("{}", lib.artist_of(track().id))).fill_width(),
        ))
        .flex_grow(1)
        .gap(3)
        .scrollable(false),
    ))
    .width(SIDE)
    .gap(S3)
    .align_items(CrossAlign::Center)
    .scrollable(false);

    // The seek bar's own copy of the position (see the module docs).
    let seek = cx.signal(0);
    cx.effect(move || seek.set(player.position.get() as i32));
    let len_ms = move || track().secs as i32 * 1000;
    let secs = cx.memo(move || player.position.get() / 1000);

    let play_button = row(label(text!(
        "{}",
        if player.playing.get() {
            Symbol::Pause
        } else {
            Symbol::Play
        }
    ))
    .font(&fonts::MONTSERRAT_18))
    .size(40, 40)
    .radius(Radius::Circle)
    .bg(palette::TEXT)
    .text_color(palette::BG)
    .justify(MainAlign::Center)
    .align_items(CrossAlign::Center)
    .scrollable(false)
    .on_state(State::PRESSED, |s| {
        s.bg(palette::AMBER).transform_scale(Scale::pct(94))
    })
    .on_press(move || app.focus.release())
    .on_click(move || player.toggle())
    .test_id("play");

    let repeat_glyph = move || match player.repeat.get() {
        Repeat::One => format!("{}1", Symbol::Loop),
        _ => Symbol::Loop.to_string(),
    };

    let transport = column((
        row((
            icon_button(app, Symbol::Shuffle.to_string(), 34, player.shuffle, move || {
                player.toggle_shuffle();
            })
            .test_id("shuffle"),
            icon_button(app, Symbol::Prev.to_string(), 34, false, move || player.prev()).test_id("prev"),
            play_button,
            icon_button(app, Symbol::Next.to_string(), 34, false, move || player.next()).test_id("next"),
            icon_button(
                app,
                text!("{}", repeat_glyph()),
                34,
                move || player.repeat.get() != Repeat::Off,
                move || player.cycle_repeat(),
            )
            .test_id("repeat"),
        ))
        .gap(S3)
        .align_items(CrossAlign::Center)
        .scrollable(false),
        row((
            label(text!("{}", clock(secs.get())))
                .font(CAPTION)
                .text_color(design::ON_SURFACE_MUTED)
                .width(40)
                .text_align(TextAlign::Right),
            slim_slider(seek, 4)
                .range(move || 0..=len_ms())
                .height(4)
                .flex_grow(1)
                .on_change(move |v| player.seek(v as u32))
                .test_id("seek"),
            label(text!("-{}", clock(track().secs.saturating_sub(secs.get()))))
                .font(CAPTION)
                .text_color(design::ON_SURFACE_MUTED)
                .width(40),
        ))
        .fill_width()
        .gap(S3)
        .align_items(CrossAlign::Center)
        .scrollable(false),
    ))
    .flex_grow(1)
    .gap(S2)
    .align_items(CrossAlign::Center)
    .scrollable(false);

    let volume_glyph = move || {
        let v = if player.muted.get() {
            0
        } else {
            player.volume.get()
        };
        match v {
            0 => Symbol::Mute,
            1..=50 => Symbol::VolumeMid,
            _ => Symbol::VolumeMax,
        }
    };
    let right = row((
        meters(player.levels).size(95, 14).test_id("meters"),
        icon_button(app, Symbol::List.to_string(), 32, queue_open, move || {
            queue_open.update(|o| *o = !*o);
        })
        .test_id("queue-toggle"),
        icon_button(app, text!("{}", volume_glyph()), 32, false, move || {
            player.muted.update(|m| *m = !*m);
        })
        .test_id("mute"),
        slim_slider(player.volume, 4)
            .range(0..=100)
            .size(84, 4)
            .on_change(move |_| player.muted.set(false))
            .test_id("volume"),
    ))
    .width(SIDE)
    .gap(S1)
    .justify(MainAlign::End)
    .align_items(CrossAlign::Center)
    .scrollable(false);

    row((now, transport, right))
        .fill_width()
        .height(HEIGHT)
        .padding_x(S4)
        .gap(S4)
        .bg(SIDEBAR)
        .border(1, LINE)
        .border_side(BorderSide::TOP)
        .align_items(CrossAlign::Center)
        .scrollable(false)
}
