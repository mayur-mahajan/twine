//! The `core_widgets` showcase: labels in every long mode, buttons (normal, checkable,
//! disabled) and images (static, QOI, recolored, symbol, rotating), styled by the default
//! theme. Built imperatively, LVGL-style.

use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_48};
use twine_core::{Angle, Color, Duration, Opa};
use twine_engine::{
    Anim, AnimProp, Engine, EngineConfig, EventCode, EventFilter, EventResult, NodeId, Repeat, State,
};
use twine_image::ImageSource;
use twine_style::{Align, FlexFlow, LayoutKind, Length, Part, Selector, StyleProp};
use twine_text::LongMode;
use twine_theme::Palette;
use twine_widgets::prelude::*;

use crate::assets::logo_rgb565a8::LOGO_RGB565A8;

/// The logo encoded as QOI (decoded by the engine's decoder registry into its image cache).
pub static LOGO_QOI: &[u8] = include_bytes!("../../assets/images/twine_logo.qoi");

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
    e.set_local_prop(c, Selector::MAIN, StyleProp::Height(h));
    e.set_layout(c, LayoutKind::Flex);
    e.set_flex_flow(c, flow);
    set(e, c, &[StyleProp::PadRow(6), StyleProp::PadColumn(12)]);
    c
}

/// A full-width label with `mode`.
fn label_in(e: &mut Engine, parent: NodeId, text: &'static str, mode: LongMode) -> NodeId {
    let l = label::create_with(e, parent, text).expect("parent exists");
    e.set_local_prop(l, Selector::MAIN, StyleProp::Width(Length::Pct(100)));
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
    e.set_local_prop(b, Selector::MAIN, StyleProp::Width(Length::Pct(100)));
    let l = label::create_with(e, b, text).expect("button exists");
    e.align(l, Align::Center, 0, 0);
    e.add_event_handler(b, EventFilter::Code(EventCode::Clicked), move |cx, _| {
        let checked = cx
            .engine()
            .tree()
            .node(cx.node())
            .is_some_and(|n| n.state().contains(State::CHECKED));
        twine_core::info!(target: "twine::example", "{} clicked (checked: {})", text, checked);
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

/// Nodes of the showcase that tests and scripts address.
#[derive(Clone, Copy, Debug)]
pub struct CoreWidgets {
    /// The scrolling label.
    pub scroll_label: NodeId,
    /// The checkable button.
    pub checkable: NodeId,
    /// The rotating image.
    pub rotating: NodeId,
}

/// Builds the showcase on the active screen of the default display.
///
/// # Panics
/// If the engine has no display.
pub fn build(e: &mut Engine) -> CoreWidgets {
    let d = e.default_display().expect("a display");
    let screen = e.active_screen(d).expect("an active screen");
    e.set_layout(screen, LayoutKind::Flex);
    e.set_flex_flow(screen, FlexFlow::RowWrap);
    set(
        e,
        screen,
        &[
            StyleProp::PadLeft(PAD),
            StyleProp::PadTop(PAD),
            StyleProp::PadRight(PAD),
            StyleProp::PadBottom(PAD),
        ],
    );

    // Labels in every long mode.
    let labels = card(e, screen, LABELS_W, Length::Px(TOP_H), FlexFlow::Column);
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
    let scroll_label = label_in(
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
    e.set_local_prop(sel, part, StyleProp::BgColor(Palette::Blue.main()));
    e.set_local_prop(sel, part, StyleProp::TextColor(Color::WHITE));
    e.with_widget_mut(sel, |w: &mut Label, cx| w.set_selection(cx, 11, 18));

    // Buttons: normal, checkable, disabled.
    let rest = i32::from(W) - 2 * PAD - LABELS_W - 10;
    let buttons = card(e, screen, rest, Length::Px(TOP_H), FlexFlow::Column);
    set(e, buttons, &[StyleProp::PadRow(12)]);
    button_in(e, buttons, "Normal");
    let checkable = button_in(e, buttons, "Checkable");
    e.with_widget_mut(checkable, |w: &mut Button, cx| w.set_checkable(cx, true));
    let disabled = button_in(e, buttons, "Disabled");
    e.add_state(disabled, State::DISABLED);

    // Images: static RGB565A8, QOI, recolored, a symbol, rotating with anti-aliasing.
    let images = card(e, screen, i32::from(W) - 2 * PAD, Length::Content, FlexFlow::Row);
    e.set_flex_align(
        images,
        twine_style::FlexAlign::SpaceEvenly,
        twine_style::FlexAlign::Center,
        twine_style::FlexAlign::Center,
    );
    image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    image_in(e, images, ImageSource::Encoded(LOGO_QOI));
    let recolored = image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    set(
        e,
        recolored,
        &[
            StyleProp::ImageRecolor(Palette::DeepOrange.main()),
            StyleProp::ImageRecolorOpa(Opa::P60),
        ],
    );
    let symbol = image_in(e, images, ImageSource::Symbol(twine_text::symbols::OK));
    set(
        e,
        symbol,
        &[
            StyleProp::TextFont(&MONTSERRAT_48),
            StyleProp::TextColor(Palette::Green.main()),
        ],
    );
    let rotating = image_in(e, images, ImageSource::Static(&LOGO_RGB565A8));
    e.with_widget_mut(rotating, |w: &mut Image, cx| {
        w.set_antialias(cx, true);
        w.set_rotation(cx, Angle(0));
    });
    e.anim_start(
        rotating,
        AnimProp::Value,
        Anim::new(0, 3600)
            .duration(Duration::secs(6))
            .repeat(Repeat::Infinite),
    );
    CoreWidgets {
        scroll_label,
        checkable,
        rotating,
    }
}
