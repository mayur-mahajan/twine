//! The counter: a label counting clicks and a reset button enabled only when there is
//! something to reset.
//!
//! Clicking changes exactly the label's text (and the reset button's state): one binding
//! runs, one small area is redrawn, and the UI is idle again right after.

use twine::prelude::*;

/// The counter application.
///
/// ```
/// use twine_testing::{TestUi, by_id, by_text};
///
/// let mut t = TestUi::new(320, 240).mount(twine_demos::counter::app);
/// t.find(by_text("Click me")).click();
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("count")).text(), "Clicked 1 times");
/// ```
pub fn app(cx: Scope) -> impl View {
    let count = cx.signal(0u32);

    column((
        label(text!("Clicked {} times", count.get()))
            .font(&fonts::MONTSERRAT_20)
            .test_id("count"),
        button(label("Click me")).on_click(move || count.update(|c| *c += 1)),
        button(label("Reset"))
            .disabled(move || count.get() == 0)
            .on_click(move || count.set(0)),
    ))
    .gap(12)
    .padding(16)
    .align_items(FlexAlign::Center)
    .size(Length::Pct(100), Length::Pct(100))
}
