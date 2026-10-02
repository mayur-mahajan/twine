//! `cargo xtask sim when_demo`: `when` switches between two panels. The checkable button
//! decides which one exists; the other one's scope (its signals, bindings and timer) is
//! disposed and its nodes deleted. The rest of the screen is never rebuilt.

use twine::prelude::*;
use twine_sim::SimConfig;

fn clock_panel(cx: Scope) -> impl View {
    let seconds = cx.signal(0u32);
    cx.interval(Duration::ms(1000), move || seconds.update(|s| *s += 1));
    container(label(text!("Panel A: {} s", seconds.get()))).width(Length::pct(100))
}

fn info_panel(_cx: Scope) -> impl View {
    container(column((label("Panel B"), label("nothing ticks here")))).width(Length::pct(100))
}

fn app(cx: Scope) -> impl View {
    let show_a = cx.signal(true);
    column((
        button(label("Panel A")).checkable(true).checked(show_a),
        when(move || show_a.get(), clock_panel).otherwise(info_panel),
        label("(this label is never rebuilt)"),
    ))
    .gap(10)
    .padding(10)
    .size(Length::pct(100), Length::pct(100))
}

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("when").scale(2), app);
}
