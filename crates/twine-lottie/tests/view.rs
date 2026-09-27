//! The `lottie` view: play/pause and loop bindings, a two-way frame model (a scrubber),
//! `on_complete` once per non-looping playback.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::Duration;
use twine_lottie::view::lottie;
use twine_lottie::widget::Lottie;
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

static LOADER: &[u8] = include_bytes!("data/loader.json");

struct Handles {
    playing: Signal<bool>,
    looping: Signal<bool>,
    frame: Signal<u32>,
    completed: Rc<Cell<u32>>,
}

/// The signals of the mounted view: playing, looping, frame.
type Signals = (Signal<bool>, Signal<bool>, Signal<u32>);

fn ui() -> (TestUi, Handles) {
    let slot: Rc<Cell<Option<Signals>>> = Rc::default();
    let completed = Rc::new(Cell::new(0));
    let (s, c) = (slot.clone(), completed.clone());
    let mut t = TestUi::new(120, 120).mount(move |cx| {
        let (playing, looping, frame) = (cx.signal(false), cx.signal(true), cx.signal(0u32));
        s.set(Some((playing, looping, frame)));
        lottie(LOADER, 100, 100)
            .playing(playing)
            .looping(looping)
            .frame(frame)
            .on_complete(move || c.set(c.get() + 1))
            .test_id("anim")
    });
    t.run_until_idle();
    let (playing, looping, frame) = slot.get().unwrap();
    (
        t,
        Handles {
            playing,
            looping,
            frame,
            completed,
        },
    )
}

fn widget_frame(t: &TestUi) -> u32 {
    let id = t.find(by_id("anim")).id();
    t.engine().widget::<Lottie>(id).unwrap().current_frame()
}

#[test]
fn lottie_view_play_pause_binding() {
    let (mut t, h) = ui();
    assert_eq!(widget_frame(&t), 0);
    h.playing.set(true);
    t.advance(Duration::ms(500));
    let f = widget_frame(&t);
    assert!((13..=16).contains(&f), "{f}");
    assert_eq!(h.frame.get_untracked(), f, "the frame model follows playback");
    h.playing.set(false);
    t.run_until_idle();
    t.assert_idle();
    assert_eq!(widget_frame(&t), f);
}

#[test]
fn scrubber_seeks_frame() {
    let (mut t, h) = ui();
    h.frame.set(42);
    t.run_until_idle();
    assert_eq!(widget_frame(&t), 42);
    h.frame.set(7);
    t.run_until_idle();
    assert_eq!(widget_frame(&t), 7);
}

#[test]
fn on_complete_fires_once_non_looping() {
    let (mut t, h) = ui();
    h.looping.set(false);
    h.playing.set(true);
    t.advance(Duration::ms(2500));
    assert_eq!(h.completed.get(), 1);
    assert_eq!(widget_frame(&t), 59);
    t.advance(Duration::ms(1000));
    assert_eq!(h.completed.get(), 1, "once");
    t.run_until_idle();
    t.assert_idle();
}
