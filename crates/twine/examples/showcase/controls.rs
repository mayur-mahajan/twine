//! The controls tab: one `level` signal shared by a slider, a bar, an arc and a label — move
//! any of them and the others follow (two-way bindings, no glue code) — plus a switch with a
//! LED, a dropdown and a roller.

use twine::prelude::*;

static MODES: [&str; 4] = ["Eco", "Comfort", "Boost", "Away"];
static FAN: [&str; 5] = ["Off", "Low", "Medium", "High", "Auto"];

pub fn page(cx: Scope) -> impl View {
    let level = cx.signal(40i32);
    let power = cx.signal(true);
    let mode = cx.signal(1usize);
    let fan = cx.signal(2usize);

    let levels = card((
        row((
            slider(level)
                .range(0..=100)
                .flex_grow(1)
                .disabled(move || !power.get()),
            label(text!("{}%", level.get())).width(44).test_id("level"),
        ))
        .fill_width()
        .gap(12)
        .align_items(CrossAlign::Center),
        bar(level).animated(Duration::ms(300)).fill_width(),
        row((
            switch(power).test_id("power"),
            led(power).size(14, 14),
            label(text!("{}", if power.get() { "On" } else { "Off" })),
        ))
        .gap(10)
        .align_items(CrossAlign::Center),
        dropdown(&MODES, mode).fill_width().test_id("mode"),
    ))
    .padding(10)
    .gap(12)
    .width(Length::pct(58));

    let selection = card((
        label("Fan").text_color(design::ON_SURFACE_MUTED),
        roller(&FAN, fan).visible_rows(3).fill_width(),
    ))
    .padding(10)
    .gap(6)
    .flex_grow(1);

    row((levels, selection))
        .gap(10)
        .padding(10)
        .align_items(CrossAlign::Start)
        .fill()
}
