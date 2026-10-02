//! The `core_widgets` showcase: labels in every long mode, buttons (normal, checkable,
//! disabled) and images (static, QOI, recolored, symbol, rotating), styled by the default
//! theme. Built imperatively, LVGL-style.

use twine::assets::fonts::{MONTSERRAT_14, MONTSERRAT_48};
use twine::core::{Angle, Color, Duration, Opa};
use twine::engine::{
    Anim, AnimProp, Engine, EngineConfig, EventCode, EventFilter, EventResult, NodeId, Repeat, State,
};
use twine::image::ImageSource;
use twine::style::{Align, FlexFlow, LayoutKind, Length, Part, Selector, StyleProp};
use twine::text::LongMode;
use twine::theme::Palette;
use twine::widgets::prelude::*;

#[path = "../assets/logo_rgb565a8.rs"]
mod logo_rgb565a8;

use logo_rgb565a8::LOGO_RGB565A8;

/// The logo encoded as QOI (decoded by the engine's decoder registry into its image cache).
pub static LOGO_QOI: &[u8] = include_bytes!("../../../../assets/images/twine_logo.qoi");

/// Screen size of the showcase.
pub const W: u16 = 480;
/// Screen height.
pub const H: u16 = 320;

/// Screen padding.
const PAD: i32 = 8;
/// Width of the label card (the button card takes the rest of the top row).
const LABELS_W: i32 = 280;
/// Height of the top row.
const TOP_H: i32 = 196;

/// Engine configuration of the showcase: the image cache holds the decoded QOI logo.
#[must_use]
pub fn engine_config() -> EngineConfig {
    EngineConfig {
        image_cache_bytes: 64 * 1024,
        ..EngineConfig::default()
    }
}

fn set(e: &mut Engine, id: NodeId, props: &[StyleProp]) {
    for p in props {
        e.set_local_prop(id, Selector::MAIN, *p);
    }
}

/// A card (container) laid out as a flex column (or row) with a smaller gap.
fn card(e: &mut Engine, parent: NodeId, w: i32, h: Length, flow: FlexFlow) -> NodeId {
    let c = container::create(e, parent).expect("parent exists");
    e.set_width(c, w);
    e.set_local_prop(c, Selector::MAIN, StyleProp::Height(h.into()));
    e.set_layout(c, LayoutKind::Flex);
    e.set_flex_flow(c, flow);
    set(
        e,
        c,
        &[
            StyleProp::RowGap(Length::Px(6).into()),
            StyleProp::ColumnGap(Length::Px(12).into()),
        ],
    );
    c
}

/// A full-width label with `mode`.
fn label_in(e: &mut Engine, parent: NodeId, text: &'static str, mode: LongMode) -> NodeId {
    let l = label::create_with(e, parent, text).expect("parent exists");
    e.set_local_prop(l, Selector::MAIN, StyleProp::Width(Length::Pct(100).into()));
    e.with_widget_mut(l, |w: &mut Label, cx| w.set_long_mode(cx, mode));
    if mode != LongMode::Wrap {
        let h = i32::from(MONTSERRAT_14.line_height);
        e.set_height(l, h);
    }
    l
}

/// A full-width button with a centered label; clicks are logged.
fn button_in(e: &mut Engine, parent: NodeId, text: &'static str) -> NodeId {
    let b = button::create(e, parent).expect("parent exists");
    e.set_local_prop(b, Selector::MAIN, StyleProp::Width(Length::Pct(100).into()));
    let l = label::create_with(e, b, text).expect("button exists");
    e.align(l, Align::Center, 0, 0);
    e.add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        let checked = cx
            .engine()
            .tree()
            .node(cx.node())
            .is_some_and(|n| n.state().contains(State::CHECKED));
        twine::core::info!(target: "twine::example", "{} clicked (checked: {})", text, checked);
        EventResult::Continue
    });
    b
}

/// An image showing `src`.
fn image_in(e: &mut Engine, parent: NodeId, src: ImageSource) -> NodeId {
    let i = image::create(e, parent).expect("parent exists");
    e.with_widget_mut(i, |w: &mut Image, cx| w.set_src(cx, src));
    i
}

/// Builds the showcase on the active screen of the default display.
///
/// # Panics
/// If the engine has no display.
pub fn build(e: &mut Engine) {
    let d = e.default_display().expect("a display");
    let screen = e.active_screen(d).expect("an active screen");
    e.set_layout(screen, LayoutKind::Flex);
    e.set_flex_flow(screen, FlexFlow::ROW.wrap(true));
    set(
        e,
        screen,
        &[
            StyleProp::PaddingLeft(Length::Px(PAD).into()),
            StyleProp::PaddingTop(Length::Px(PAD).into()),
            StyleProp::PaddingRight(Length::Px(PAD).into()),
            StyleProp::PaddingBottom(Length::Px(PAD).into()),
        ],
    );

    // Labels in every long mode.
    let labels = card(e, screen, LABELS_W, Length::Px(TOP_H), FlexFlow::COLUMN);
    label_in(
        e,
        labels,
        "Wrap: a long text wraps onto the next line when it does not fit.",
        LongMode::Wrap,
    );
    label_in(
        e,
        labels,
        "Dots: this text is far too long for one line",
        LongMode::Dots,
    );
    label_in(
        e,
        labels,
        "Scroll: back and forth at 40 px per second",
        LongMode::Scroll,
    );
    label_in(
        e,
        labels,
        "Circular: an endless ticker of text going round",
        LongMode::ScrollCircular,
    );
    label_in(
        e,
        labels,
        "Clip: this one is simply cut at the edge",
        LongMode::Clip,
    );
    let sel = label_in(e, labels, "Selection: part of this text", LongMode::Wrap);
    let part = Selector::part(Part::Selected);
    e.set_local_prop(sel, part, StyleProp::BgColor(Palette::Blue.main().into()));
    e.set_local_prop(sel, part, StyleProp::TextColor(Color::WHITE.into()));
    e.with_widget_mut(sel, |w: &mut Label, cx| w.set_selection(cx, 11, 18));

    // Buttons: normal, checkable, disabled.
    let rest = i32::from(W) - 2 * PAD - LABELS_W - 10;
    let buttons = card(e, screen, rest, Length::Px(TOP_H), FlexFlow::COLUMN);
    set(e, buttons, &[StyleProp::RowGap(Length::Px(12).into())]);
    button_in(e, buttons, "Normal");
    let checkable = button_in(e, buttons, "Checkable");
    e.with_widget_mut(checkable, |w: &mut Button, cx| w.set_checkable(cx, true));
    let disabled = button_in(e, buttons, "Disabled");
    e.add_state(disabled, State::DISABLED);

    // Images: static RGB565A8, QOI, recolored, a symbol, rotating with anti-aliasing.
    let images = card(e, screen, i32::from(W) - 2 * PAD, Length::Content, FlexFlow::ROW);
    e.set_flex_align(
        images,
        twine::style::MainAlign::SpaceEvenly,
        twine::style::CrossAlign::Center,
        twine::style::MainAlign::Center,
    );
    image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    image_in(e, images, ImageSource::Encoded(LOGO_QOI));
    let recolored = image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    set(
        e,
        recolored,
        &[
            StyleProp::ImageRecolor(Palette::DeepOrange.main().into()),
            StyleProp::ImageRecolorOpacity(Opa::P60.into()),
        ],
    );
    let symbol = image_in(e, images, ImageSource::symbol(twine::text::Symbol::Ok));
    set(
        e,
        symbol,
        &[
            StyleProp::Font((&MONTSERRAT_48).into()),
            StyleProp::TextColor(Palette::Green.main().into()),
        ],
    );
    let rotating = image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    e.with_widget_mut(rotating, |w: &mut Image, cx| {
        w.set_antialias(cx, true);
        w.set_rotation(cx, Angle::deci_deg(0));
    });
    e.anim_start(
        rotating,
        AnimProp::Value,
        Anim::new(0, 3600)
            .duration(Duration::secs(6))
            .repeat(Repeat::Forever),
    );
}
