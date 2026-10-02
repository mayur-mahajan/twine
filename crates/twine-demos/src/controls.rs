//! Every basic control on one scrolling screen: cards of 148 px that wrap into two columns on
//! a 320 px wide display (one column on 240 px).
//!
//! - **Level**: a slider, a bar (animated) and an arc bound to one `level` signal, with a
//!   percentage label — dragging any of them moves the others.
//! - **Enabled**: a switch, a checkbox, an LED and an image button bound to one `enabled`
//!   signal.
//! - **Busy**: the spinner and an image animation (4 frames).
//! - **Chart**: a polyline from a static array.
//!
//! Every control is in the default focus group: the keypad (Tab, arrows, Enter) and the
//! encoder operate all of them. [`app_static`] leaves out the spinner and the animation (it
//! is idle whenever nobody touches it).

use twine::prelude::*;

use crate::assets::{frame0::FRAME0, frame1::FRAME1, frame2::FRAME2, frame3::FRAME3};
use crate::assets::{power_off::POWER_OFF, power_on::POWER_ON};

/// The animation frames.
static FRAMES: [ImageSource; 4] = [
    ImageSource::Static(&FRAME0),
    ImageSource::Static(&FRAME1),
    ImageSource::Static(&FRAME2),
    ImageSource::Static(&FRAME3),
];

/// The polyline of the chart card.
static CHART: [Point; 8] = [
    Point::new(0, 40),
    Point::new(18, 22),
    Point::new(36, 30),
    Point::new(54, 8),
    Point::new(72, 18),
    Point::new(90, 4),
    Point::new(108, 26),
    Point::new(126, 12),
];

/// The width of a card.
const CARD_W: i32 = 148;

/// A card with a muted title (the theme's `ON_SURFACE_MUTED`) above its content.
fn titled_card(title: &'static str, content: impl ViewSeq) -> impl View {
    card((label(title).text_color(design::ON_SURFACE_MUTED), content))
        .size(CARD_W, Length::Content)
        .padding(8)
        .gap(6)
}

/// The controls demo with the running spinner and image animation.
///
/// ```
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(320, 240).mount(twine_demos::controls::app);
/// t.advance(twine::core::Duration::ms(100));
/// assert_eq!(t.find(by_id("level")).text(), "40%");
/// ```
pub fn app(cx: Scope) -> impl View {
    build(cx, true)
}

/// The controls demo without the spinner and the image animation (idle between inputs).
pub fn app_static(cx: Scope) -> impl View {
    build(cx, false)
}

fn build(cx: Scope, animated: bool) -> impl View {
    let level = cx.signal(40i32);
    let enabled = cx.signal(false);

    let level_card = titled_card(
        "Level",
        (
            row((
                slider(level).range(0..=100).flex_grow(1).test_id("slider"),
                label(text!("{}%", level.get())).width(40).test_id("level"),
            ))
            .fill_width()
            .gap(14)
            .padding_y(6)
            .align_items(CrossAlign::Center),
            bar(level).animated(Duration::ms(300)).fill_width().test_id("bar"),
        ),
    );
    let arc_card = titled_card(
        "Dial",
        stack((
            arc(level)
                .range(0..=100)
                .size(90, 90)
                .focusable(true)
                .test_id("arc"),
            label(text!("{}", level.get())),
        ))
        .size(Length::pct(100), 94)
        .bg_opacity(Opa::TRANSP)
        .border_width(0)
        .padding(0),
    );
    let enabled_card = titled_card(
        "Enabled",
        (
            row((
                switch(enabled).test_id("switch"),
                led(enabled).size(16, 16),
                image_button(ImageSource::Static(&POWER_OFF), ImageSource::Static(&POWER_OFF))
                    .checked_images(ImageSource::Static(&POWER_ON), ImageSource::Static(&POWER_ON))
                    .checkable(true)
                    .checked(enabled)
                    .focusable(true)
                    .test_id("power"),
            ))
            .fill_width()
            .justify(MainAlign::SpaceBetween)
            .align_items(CrossAlign::Center),
            checkbox("Enabled", enabled).test_id("check"),
        ),
    );
    let busy_card = animated.then(|| {
        titled_card(
            "Busy",
            row((
                spinner().size(44, 44),
                animimg(&FRAMES, Duration::ms(800)).test_id("anim"),
            ))
            .gap(24)
            .align_items(CrossAlign::Center),
        )
    });
    let chart_card = titled_card("Chart", line_static(&CHART).width(2).rounded(true));

    scroll_view(
        Axis::Vertical,
        flex(
            FlexDirection::Row,
            (level_card, arc_card, enabled_card, busy_card, chart_card),
        )
        .wrap(true)
        .fill_width()
        .gap(8)
        .justify(MainAlign::Center),
    )
    .fill()
    .padding(8)
}
