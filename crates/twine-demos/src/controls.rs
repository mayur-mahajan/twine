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

/// A card: the theme's container look with a title and its content in a column.
fn card(title: &'static str, content: impl ViewSeq) -> impl View {
    container((label(title).text_color(Color::hex(0x75_75_75)), content))
        .op(|cx, n| {
            let e = cx.engine();
            e.set_local_prop(n, Selector::MAIN, StyleProp::Layout(LayoutKind::Flex));
            e.set_local_prop(n, Selector::MAIN, StyleProp::FlexFlow(FlexFlow::Column));
        })
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

    let level_card = card(
        "Level",
        (
            row((
                slider(level).range(0..=100).flex_grow(1).test_id("slider"),
                label(text!("{}%", level.get())).width(40).test_id("level"),
            ))
            .width(Length::pct(100))
            .gap(14)
            .padding_ver(6)
            .align_items(FlexAlign::Center),
            bar(level)
                .animated(Duration::ms(300))
                .width(Length::pct(100))
                .test_id("bar"),
        ),
    );
    let arc_card = card(
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
        .bg_opa(Opa::TRANSP)
        .border_width(0)
        .padding(0),
    );
    let enabled_card = card(
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
            .width(Length::pct(100))
            .justify(FlexAlign::SpaceBetween)
            .align_items(FlexAlign::Center),
            checkbox("Enabled", enabled).test_id("check"),
        ),
    );
    let busy_card = animated.then(|| {
        card(
            "Busy",
            row((
                spinner().size(44, 44),
                animimg(&FRAMES, Duration::ms(800)).test_id("anim"),
            ))
            .gap(24)
            .align_items(FlexAlign::Center),
        )
    });
    let chart_card = card("Chart", line_static(&CHART).width(2).rounded(true));

    scroll_view(
        Dir::VER,
        flex(
            FlexFlow::RowWrap,
            (level_card, arc_card, enabled_card, busy_card, chart_card),
        )
        .width(Length::pct(100))
        .gap(8)
        .justify(FlexAlign::Center),
    )
    .size(Length::pct(100), Length::pct(100))
    .padding(8)
}
