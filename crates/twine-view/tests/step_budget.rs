//! `Ui::update_budgeted` (R3.S04): a frame rendered a few chunks per update is completed over
//! several updates (each returning `Wake::Now` until it is), with exactly the pixels of the
//! same frame rendered by one update — also when the UI changes between the partial steps.

use std::cell::Cell;
use std::rc::Rc;

use twine_core::ColorFormat;
use twine_hal::DisplayInfo;
use twine_testing::{MemoryDisplay, MockClock};
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 48;
/// Rows per chunk: a full frame is `H / ROWS` = 12 chunks.
const ROWS: u16 = 4;

fn app(text: Signal<&'static str>) -> impl FnOnce(Scope) -> AnyView {
    move |_cx| {
        column((label(move || text.get()), button(label("Press")), bar(40)))
            .gap(4)
            .padding(4)
            .bg(Color::hex(0x1E_88_E5))
            .size(Length::Pct(100), Length::Pct(100))
            .into_any()
    }
}

fn make_ui(clock: &MockClock, double: bool, text: &'static str) -> (Ui, Signal<&'static str>) {
    let spec = if double {
        BufferSpec::PartialDouble { rows: ROWS }
    } else {
        BufferSpec::PartialSingle { rows: ROWS }
    };
    let slot = Rc::new(Cell::new(None));
    let s = slot.clone();
    let ui = Ui::builder(MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(spec))
        .theme(DefaultTheme::light())
        .clock(clock.clone())
        .build(move |cx| {
            let text = cx.signal(text);
            s.set(Some(text));
            app(text)(cx)
        });
    (ui, slot.get().unwrap())
}

fn pixels(ui: &Ui) -> Vec<u8> {
    ui.engine()
        .driver::<MemoryDisplay>(ui.display())
        .unwrap()
        .framebuffer()
        .to_vec()
}

/// Updates with `budget` until the UI is idle, moving the clock to every deadline; returns
/// the number of updates that returned `Wake::Now`.
fn settle(ui: &mut Ui, clock: &MockClock, budget: StepBudget) -> usize {
    let mut partial = 0;
    for _ in 0..1000 {
        match ui.update_budgeted(budget) {
            Wake::Now => partial += 1,
            Wake::At(t) => clock.set(t),
            Wake::Idle | Wake::IdleFor(_) => return partial,
        }
    }
    panic!("the UI never settled");
}

#[test]
fn budgeted_frame_has_the_pixels_of_one_update() {
    for double in [false, true] {
        for chunks in [1u16, 3, 5] {
            let clock = MockClock::new();
            let (mut reference, _) = make_ui(&clock, double, "Twine");
            assert_eq!(settle(&mut reference, &clock, StepBudget::UNLIMITED), 0);

            let clock = MockClock::new();
            let (mut budgeted, _) = make_ui(&clock, double, "Twine");
            let partial = settle(&mut budgeted, &clock, StepBudget::chunks(chunks));
            // 12 chunks, `chunks` per update: the last update finishes the frame.
            let expected = usize::from((H / ROWS).div_ceil(chunks)) - 1;
            assert_eq!(partial, expected, "double={double} chunks={chunks}");
            assert!(
                pixels(&budgeted) == pixels(&reference),
                "double={double} chunks={chunks}: pixels differ"
            );
        }
    }
}

#[test]
fn every_partial_update_renders_at_most_the_budget() {
    let clock = MockClock::new();
    let (mut ui, _) = make_ui(&clock, false, "Twine");
    let d = ui.display();
    let flushes = |ui: &Ui| ui.engine().driver::<MemoryDisplay>(d).unwrap().flushes().len();
    let mut seen = 0;
    while ui.update_budgeted(StepBudget::chunks(2)) == Wake::Now {
        let now = flushes(&ui);
        assert_eq!(now - seen, 2, "two chunks per update");
        seen = now;
    }
    assert_eq!(flushes(&ui), usize::from(H / ROWS));
}

#[test]
fn change_between_partial_updates_is_drawn_like_a_fresh_frame() {
    let clock = MockClock::new();
    let (mut ui, text) = make_ui(&clock, false, "first");
    // Half of the first frame, then the label changes (in a part already drawn).
    for _ in 0..6 {
        assert_eq!(ui.update_budgeted(StepBudget::chunks(1)), Wake::Now);
    }
    text.set("second, longer");
    settle(&mut ui, &clock, StepBudget::chunks(1));

    let clock = MockClock::new();
    let (mut fresh, _) = make_ui(&clock, false, "second, longer");
    settle(&mut fresh, &clock, StepBudget::UNLIMITED);
    assert!(
        pixels(&ui) == pixels(&fresh),
        "stale pixels after a mid-frame change"
    );
}

#[test]
fn unbudgeted_update_finishes_a_budgeted_frame() {
    let clock = MockClock::new();
    let (mut reference, _) = make_ui(&clock, true, "Twine");
    settle(&mut reference, &clock, StepBudget::UNLIMITED);

    let clock = MockClock::new();
    let (mut ui, _) = make_ui(&clock, true, "Twine");
    assert_eq!(ui.update_budgeted(StepBudget::chunks(1)), Wake::Now);
    assert_ne!(ui.update(), Wake::Now, "an unlimited update completes the frame");
    assert!(pixels(&ui) == pixels(&reference), "pixels differ");
}
