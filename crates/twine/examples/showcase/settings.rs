//! The settings tab: switch the theme mode at run time (every widget re-styles itself
//! through its design elements) and turn the tab view's slide animation on or off.

use twine::prelude::*;

pub fn page(cx: Scope, animated: Signal<bool>) -> impl View {
    let theme = use_theme(cx);
    let dark = cx.signal(false);
    cx.effect(move || {
        theme.set_mode(if dark.get() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
    });

    let setting = |title: &'static str, hint: &'static str, on: Signal<bool>, id: &'static str| {
        row((
            column((label(title), label(hint).text_color(design::ON_SURFACE_MUTED))).gap(2),
            switch(on).test_id(id),
        ))
        .fill_width()
        .justify(MainAlign::SpaceBetween)
        .align_items(CrossAlign::Center)
    };

    column((
        card((
            setting("Dark mode", "Re-styles every widget at once", dark, "dark"),
            setting("Slide between tabs", "Animate tab changes", animated, "slide"),
        ))
        .padding(12)
        .gap(14)
        .fill_width(),
        label("Twine: a Rust GUI library for microcontrollers and desktop hosts.")
            .text_color(design::ON_SURFACE_MUTED)
            .long_mode(LongMode::Wrap)
            .fill_width(),
    ))
    .gap(10)
    .padding(10)
    .fill()
}
