//! The Up Next panel: a sheet that slides in from the right over the current page, with the
//! track playing now and the queue, which can be reordered, trimmed and jumped into.
//!
//! **Motion.** The panel is always built; a tween eases its horizontal offset between "in"
//! and "out" (with an ease-out curve, so it decelerates into place), and it is hidden once it
//! has fully left, so a closed panel costs nothing to draw.
//!
//! **Keyed updates.** The queue is a [`virtual_list`] keyed by each entry's own key (not the
//! track: a song may be queued twice). Moving an entry up or down, removing it or skipping
//! to it changes the `queue` signal; the list compares keys and moves the existing rows
//! (their widgets and bindings intact) instead of rebuilding them. Only the visible rows
//! exist, so a 500-song queue (playing the whole library) costs the same as a short one.

use twine::prelude::*;

use crate::art::{Cover, cover};
use crate::catalog::catalog;
use crate::player::Entry;
use crate::theme::{BODY, LINE, PRESSED, S1, S2, S3, S4, TITLE, palette};
use crate::ui::{App, caption, icon_button, overline, pill, text};
use crate::util::summary;
use crate::viz::{icon_bars, playing_icon};

/// The panel's width.
pub const WIDTH: i32 = 324;
/// Height of a queue row.
const ROW_H: i32 = 54;

pub fn panel(cx: Scope, app: App, height: i32) -> impl View {
    let lib = catalog();
    let open = app.queue_open;
    let player = app.player;
    // 0 = in place, WIDTH + shadow = out of the window.
    let out = WIDTH + 40;
    let offset = cx.tween(
        move || if open.get() { 0 } else { out },
        AnimSpec::new(Duration::ms(280)).ease_out(),
    );
    let summary_text = cx.memo(move || {
        player.queue.with(|q| {
            let secs: u32 = q.iter().map(|e| lib.track(e.track).secs).sum();
            format!("{} songs, {}", q.len(), summary(secs))
        })
    });

    let header = row((
        column((
            label("Up Next").font(TITLE).text_color(design::ON_SURFACE),
            caption(text!("{}", summary_text.get())),
        ))
        .gap(2),
        spacer(),
        pill(app, "Clear", false, move || player.clear_queue())
            .height(28)
            .disabled(move || player.queue.with(Vec::is_empty)),
        icon_button(app, Symbol::Close.to_string(), 30, false, move || open.set(false))
            .test_id("queue-close"),
    ))
    .fill_width()
    .gap(S2)
    .align_items(CrossAlign::Center);

    let current = row((
        cover(move || Cover::Album(lib.track(player.current.get()).album)).size(44, 44),
        column((
            text(text!("{}", lib.track(player.current.get()).title))
                .text_color(design::PRIMARY)
                .fill_width(),
            caption(text!("{}", lib.artist_of(player.current.get()))).fill_width(),
        ))
        .flex_grow(1)
        .gap(2),
        playing_icon(move || {
            if player.playing.get() {
                icon_bars(&player.spectrum.get())
            } else {
                [250, 450, 300]
            }
        })
        .size(18, 16),
    ))
    .fill_width()
    .gap(S3)
    .padding(S2)
    .radius(10)
    .bg(design::SURFACE_VARIANT)
    .align_items(CrossAlign::Center)
    .scrollable(false);

    let list = virtual_list(
        player.queue,
        |e: &Entry| e.key,
        ROW_H,
        move |_cx, e| queue_row(app, e.get_untracked()),
    )
    .fill_width()
    .flex_grow(1)
    .test_id("queue");

    let empty = when(
        move || player.queue.with(Vec::is_empty),
        |_| caption("Nothing queued. Double-click a song to start a queue.").padding_y(S4),
    );

    column((
        header,
        overline("NOW PLAYING").margin_each(Insets::new(0, S2, 0, 0)),
        current,
        overline("UP NEXT").margin_each(Insets::new(0, S2, 0, 0)),
        empty,
        list,
    ))
    .width(WIDTH)
    .height(height)
    .padding(S4)
    .gap(S2)
    .bg(design::SURFACE)
    .border(1, LINE)
    .border_side(BorderSide::LEFT)
    .shadow_width(40)
    .shadow_opacity(Opa::P60)
    .shadow_color(Color::BLACK)
    .floating(true)
    .align(Align::TopRight)
    .translate_x(offset)
    .hidden(move || !open.get() && offset.get() >= out)
    .scrollable(false)
    .test_id("queue-panel")
}

/// One queued track: cover, title and artist; move up, move down and remove on the right.
/// Clicking it plays it (skipping what is before it).
fn queue_row(app: App, e: Entry) -> Flex {
    let lib = catalog();
    let t = lib.track(e.track);
    let player = app.player;
    row((
        cover(Cover::Album(t.album)).size(38, 38),
        column((
            text(t.title.as_str()).font(BODY).fill_width(),
            caption(lib.artist_of(t.id)).fill_width(),
        ))
        .flex_grow(1)
        .gap(2)
        .scrollable(false),
        icon_button(app, Symbol::Up.to_string(), 26, false, move || {
            player.shift(e.key, -1);
        }),
        icon_button(app, Symbol::Down.to_string(), 26, false, move || {
            player.shift(e.key, 1);
        }),
        icon_button(app, Symbol::Close.to_string(), 26, false, move || {
            player.remove(e.key);
        })
        .text_color(palette::FAINT),
    ))
    .fill_width()
    .height(ROW_H - S1)
    .gap(S1)
    .padding_x(S1)
    .radius(8)
    .align_items(CrossAlign::Center)
    .scrollable(false)
    .on_state(State::PRESSED, |s| s.bg(PRESSED))
    .on_press(move || app.focus.release())
    .on_click(move || player.jump_to(e.key))
}
