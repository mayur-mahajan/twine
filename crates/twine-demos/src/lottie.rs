//! Lottie animations (LVGL's `lv_lottie` example idea): three samples in a row — a loader, a
//! check mark and a beating heart — each with a play/pause button, a loop switch and a frame
//! scrubber bound two-way to the animation's frame.
//!
//! A paused or finished animation leaves the UI idle; a playing one re-renders its own buffer
//! only when the frame number changes (30 fps samples: 30 redraws per second, whatever the
//! display refresh rate).

use twine::prelude::*;
use twine_lottie::view::lottie;

/// The samples: `(name, JSON)`.
pub static SAMPLES: [(&str, &[u8]); 3] = [
    ("Loader", include_bytes!("../../../assets/lottie/loader.json")),
    ("Check", include_bytes!("../../../assets/lottie/check.json")),
    ("Heart", include_bytes!("../../../assets/lottie/heart.json")),
];

/// Frames of every sample (60 frames at 30 fps).
const FRAMES: i32 = 60;

/// One sample with its controls.
fn card(cx: Scope, i: usize) -> impl View {
    let (name, data) = SAMPLES[i];
    let playing = cx.signal(true);
    let looping = cx.signal(true);
    let frame = cx.signal(0u32);
    let scrub = cx.signal(0i32);
    // The scrubber and the frame follow each other (`set_if_changed` ends the round trip).
    cx.effect(move || scrub.set_if_changed(i32::try_from(frame.get()).unwrap_or(0)));
    cx.effect(move || frame.set_if_changed(u32::try_from(scrub.get()).unwrap_or(0)));
    container(
        column((
            label(name),
            lottie(data, 100, 100)
                .playing(playing)
                .looping(looping)
                .frame(frame)
                .on_complete(move || playing.set(false))
                .test_id(name),
            row((
                button(label(text!(
                    "{}",
                    if playing.get() {
                        symbols::PAUSE
                    } else {
                        symbols::PLAY
                    }
                )))
                .on_click(move || playing.update(|p| *p = !*p))
                .test_id(["play0", "play1", "play2"][i]),
                switch(looping).test_id(["loop0", "loop1", "loop2"][i]),
            ))
            .gap(8)
            .align_items(FlexAlign::Center),
            slider(scrub)
                .range(0..=FRAMES - 1)
                .width(Length::pct(100))
                .test_id(["scrub0", "scrub1", "scrub2"][i]),
        ))
        .gap(6)
        .align_items(FlexAlign::Center)
        .width(Length::pct(100)),
    )
    .width(148)
    .height(Length::Content)
    .padding(6)
}

/// The Lottie demo.
///
/// ```
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(480, 320).mount(twine_demos::lottie::app);
/// t.advance(twine::core::Duration::ms(100));
/// assert!(t.find(by_id("Loader")).coords().width() == 100);
/// ```
pub fn app(cx: Scope) -> impl View {
    row((card(cx, 0), card(cx, 1), card(cx, 2)))
        .gap(8)
        .padding(8)
        .size(Length::pct(100), Length::pct(100))
        .align_items(FlexAlign::Start)
}
