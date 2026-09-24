//! Modifiers (`ViewExt`), containers and widget views.
//!
//! Checklist — every modifier row of the view API table and its test:
//! - Size/pos (`width`, `height`, `size`, `min_*`, `max_*`, `x`, `y`, `pos`, `align`,
//!   `align_to`, `translate`, `offset`): `style_modifiers_resolve`, `align_to_follows_ref`,
//!   `modifier_idempotency_all`.
//! - Spacing (`padding*`, `margin*`, `gap`, `row_gap`, `column_gap`): `style_modifiers_resolve`.
//! - Background (`bg`, `bg_color`, `bg_opa`, `bg_grad`, `bg_image`): `style_modifiers_resolve`.
//! - Border/outline (`border*`, `outline`, `radius`): `style_modifiers_resolve`.
//! - Shadow (`shadow`, `shadow_*`): `style_modifiers_resolve`.
//! - Text (`font`, `text_color`, `text_opa`, `text_align`, `letter_space`, `line_space`,
//!   `text_decor`): `style_modifiers_resolve`.
//! - Visual (`opacity`, `transform_*`, `blend_mode`, `clip_corner`, `recolor`):
//!   `style_modifiers_resolve`.
//! - Flags (`hidden`, `disabled`, `clickable`, `checkable`, `scrollable`, `scroll_*`,
//!   `scrollbar`, `focusable`, `floating`, `ignore_layout`, `event_bubble`,
//!   `overflow_visible`): `flag_modifiers_apply`.
//! - Style (`style`, `style_for`, `style_ref`, `class_style`): `style_list_modifiers`.
//! - Layout as child (`flex_grow`, `grid_cell`, `grid_cell_align`): `grid_places_cells`,
//!   `spacer_grows`, `style_modifiers_resolve`.
//! - Events (`on_click` … `on_event`): `event_modifiers_fire`.
//! - Identity (`test_id`, `node_ref`, `group`): `identity_modifiers`.
//! - Transition (`transition`): `style_modifiers_resolve`.
//! - Dynamic values of every kind: `dynamic_modifier_updates`, `modifier_idempotency_all`.

use std::cell::RefCell;
use std::rc::Rc;

use twine_core::{Angle, Duration, Insets, Point, Rect, Scale};
use twine_engine::{EventCode, EventParam, NodeId, ObjFlags};
use twine_image::ImageSource;
use twine_render::Gradient;
use twine_style::StyleValue;
use twine_testing::{TestUi, by_id, by_text};
use twine_theme::DefaultTheme;
use twine_view::prelude::*;
use twine_widgets::label::Label;

static GRAD: Gradient = Gradient::new(
    twine_render::GradKind::Ver,
    &[
        twine_render::GradStop::new(Color::RED, 0),
        twine_render::GradStop::new(Color::BLUE, 255),
    ],
);
const MONT20: &twine_text::Font = &twine_assets::fonts::MONTSERRAT_20;
static BG_IMG: ImageSource = ImageSource::Symbol("x");
static SMOOTH: TransitionDsc = TransitionDsc::new(&[PropId::BgColor], Duration::ms(100), Easing::Linear);
static STYLE_A: Style = style! { radius: 5 };
static STYLE_B: Style = style! { bg_color: Color::GREEN };

/// Mounts `v` as the only view (with id "n").
fn mount(v: impl FnOnce(Scope) -> AnyView) -> TestUi {
    let mut t = TestUi::new(200, 150).mount(v);
    t.run_until_idle();
    t
}

fn node(t: &TestUi) -> NodeId {
    t.find(by_id("n")).id()
}

fn prop(t: &TestUi, p: PropId) -> StyleValue {
    t.engine().style_prop(node(t), Part::Main, p)
}

#[test]
#[allow(clippy::too_many_lines)]
fn style_modifiers_resolve() {
    type Case = (
        &'static str,
        fn(WidgetView<Label>) -> WidgetView<Label>,
        &'static [(PropId, StyleValue)],
    );
    let px = |v| StyleValue::Length(Length::Px(v));
    let _ = px;
    let cases: &[Case] = &[
        (
            "width",
            |v| v.width(40),
            &[(PropId::Width, StyleValue::Length(Length::Px(40)))],
        ),
        (
            "height",
            |v| v.height(Length::pct(50)),
            &[(PropId::Height, StyleValue::Length(Length::Pct(50)))],
        ),
        (
            "size",
            |v| v.size(30, 20),
            &[
                (PropId::Width, StyleValue::Length(Length::Px(30))),
                (PropId::Height, StyleValue::Length(Length::Px(20))),
            ],
        ),
        (
            "min_width",
            |v| v.min_width(10),
            &[(PropId::MinWidth, StyleValue::Length(Length::Px(10)))],
        ),
        (
            "max_width",
            |v| v.max_width(90),
            &[(PropId::MaxWidth, StyleValue::Length(Length::Px(90)))],
        ),
        (
            "min_height",
            |v| v.min_height(11),
            &[(PropId::MinHeight, StyleValue::Length(Length::Px(11)))],
        ),
        (
            "max_height",
            |v| v.max_height(91),
            &[(PropId::MaxHeight, StyleValue::Length(Length::Px(91)))],
        ),
        (
            "pos",
            |v| v.pos(3, 4),
            &[
                (PropId::X, StyleValue::Length(Length::Px(3))),
                (PropId::Y, StyleValue::Length(Length::Px(4))),
            ],
        ),
        ("x", |v| v.x(7), &[(PropId::X, StyleValue::Length(Length::Px(7)))]),
        ("y", |v| v.y(8), &[(PropId::Y, StyleValue::Length(Length::Px(8)))]),
        (
            "align",
            |v| v.align(Align::Center),
            &[(PropId::Align, StyleValue::Enum(Align::Center as u8))],
        ),
        (
            "translate",
            |v| v.translate(2, 3),
            &[
                (PropId::TranslateX, StyleValue::Length(Length::Px(2))),
                (PropId::TranslateY, StyleValue::Length(Length::Px(3))),
            ],
        ),
        (
            "offset",
            |v| v.offset(Point::new(5, 6)),
            &[
                (PropId::TranslateX, StyleValue::Length(Length::Px(5))),
                (PropId::TranslateY, StyleValue::Length(Length::Px(6))),
            ],
        ),
        (
            "padding",
            |v| v.padding(4),
            &[
                (PropId::PadTop, StyleValue::Int(4)),
                (PropId::PadBottom, StyleValue::Int(4)),
                (PropId::PadLeft, StyleValue::Int(4)),
                (PropId::PadRight, StyleValue::Int(4)),
            ],
        ),
        (
            "padding_hor",
            |v| v.padding_hor(5),
            &[
                (PropId::PadLeft, StyleValue::Int(5)),
                (PropId::PadRight, StyleValue::Int(5)),
            ],
        ),
        (
            "padding_ver",
            |v| v.padding_ver(6),
            &[
                (PropId::PadTop, StyleValue::Int(6)),
                (PropId::PadBottom, StyleValue::Int(6)),
            ],
        ),
        (
            "padding_each",
            |v| v.padding_each(Insets::new(1, 2, 3, 4)),
            &[
                (PropId::PadLeft, StyleValue::Int(1)),
                (PropId::PadTop, StyleValue::Int(2)),
                (PropId::PadRight, StyleValue::Int(3)),
                (PropId::PadBottom, StyleValue::Int(4)),
            ],
        ),
        (
            "margin",
            |v| v.margin(2),
            &[
                (PropId::MarginTop, StyleValue::Int(2)),
                (PropId::MarginRight, StyleValue::Int(2)),
            ],
        ),
        (
            "margin_each",
            |v| v.margin_each(Insets::new(1, 2, 3, 4)),
            &[
                (PropId::MarginLeft, StyleValue::Int(1)),
                (PropId::MarginBottom, StyleValue::Int(4)),
            ],
        ),
        (
            "gap",
            |v| v.gap(9),
            &[
                (PropId::PadRow, StyleValue::Int(9)),
                (PropId::PadColumn, StyleValue::Int(9)),
            ],
        ),
        (
            "row_gap",
            |v| v.row_gap(3),
            &[(PropId::PadRow, StyleValue::Int(3))],
        ),
        (
            "column_gap",
            |v| v.column_gap(4),
            &[(PropId::PadColumn, StyleValue::Int(4))],
        ),
        (
            "bg",
            |v| v.bg(Color::RED),
            &[
                (PropId::BgColor, StyleValue::Color(Color::RED)),
                (PropId::BgOpa, StyleValue::Opa(Opa::COVER)),
            ],
        ),
        (
            "bg_color",
            |v| v.bg_color(Color::BLUE),
            &[(PropId::BgColor, StyleValue::Color(Color::BLUE))],
        ),
        (
            "bg_opa",
            |v| v.bg_opa(Opa::P50),
            &[(PropId::BgOpa, StyleValue::Opa(Opa::P50))],
        ),
        ("bg_grad", |v| v.bg_grad(&GRAD), &[]),
        ("bg_image", |v| v.bg_image(&BG_IMG), &[]),
        (
            "border",
            |v| v.border(2, Color::RED),
            &[
                (PropId::BorderWidth, StyleValue::Int(2)),
                (PropId::BorderColor, StyleValue::Color(Color::RED)),
            ],
        ),
        (
            "border_width",
            |v| v.border_width(3),
            &[(PropId::BorderWidth, StyleValue::Int(3))],
        ),
        (
            "border_color",
            |v| v.border_color(Color::GREEN),
            &[(PropId::BorderColor, StyleValue::Color(Color::GREEN))],
        ),
        (
            "border_opa",
            |v| v.border_opa(Opa::P30),
            &[(PropId::BorderOpa, StyleValue::Opa(Opa::P30))],
        ),
        ("border_side", |v| v.border_side(BorderSide::BOTTOM), &[]),
        (
            "outline",
            |v| v.outline(2, Color::BLUE, 3),
            &[
                (PropId::OutlineWidth, StyleValue::Int(2)),
                (PropId::OutlinePad, StyleValue::Int(3)),
            ],
        ),
        ("radius", |v| v.radius(6), &[(PropId::Radius, StyleValue::Int(6))]),
        (
            "shadow",
            |v| {
                v.shadow(ShadowDsc {
                    width: 5,
                    ofs_x: 1,
                    ofs_y: 2,
                    spread: 3,
                    color: Color::RED,
                    opa: Opa::P50,
                })
            },
            &[
                (PropId::ShadowWidth, StyleValue::Int(5)),
                (PropId::ShadowOffsetX, StyleValue::Int(1)),
                (PropId::ShadowOffsetY, StyleValue::Int(2)),
                (PropId::ShadowSpread, StyleValue::Int(3)),
                (PropId::ShadowOpa, StyleValue::Opa(Opa::P50)),
            ],
        ),
        (
            "shadow_width",
            |v| v.shadow_width(7),
            &[(PropId::ShadowWidth, StyleValue::Int(7))],
        ),
        (
            "shadow_offset",
            |v| v.shadow_offset(1, 2),
            &[
                (PropId::ShadowOffsetX, StyleValue::Int(1)),
                (PropId::ShadowOffsetY, StyleValue::Int(2)),
            ],
        ),
        (
            "shadow_spread",
            |v| v.shadow_spread(2),
            &[(PropId::ShadowSpread, StyleValue::Int(2))],
        ),
        (
            "shadow_color",
            |v| v.shadow_color(Color::RED),
            &[(PropId::ShadowColor, StyleValue::Color(Color::RED))],
        ),
        (
            "shadow_opa",
            |v| v.shadow_opa(Opa::P30),
            &[(PropId::ShadowOpa, StyleValue::Opa(Opa::P30))],
        ),
        (
            "font",
            |v| v.font(MONT20),
            &[(PropId::TextFont, StyleValue::Font(MONT20))],
        ),
        (
            "text_color",
            |v| v.text_color(Color::RED),
            &[(PropId::TextColor, StyleValue::Color(Color::RED))],
        ),
        (
            "text_opa",
            |v| v.text_opa(Opa::P50),
            &[(PropId::TextOpa, StyleValue::Opa(Opa::P50))],
        ),
        (
            "text_align",
            |v| v.text_align(TextAlign::Center),
            &[(PropId::TextAlign, StyleValue::Enum(TextAlign::Center as u8))],
        ),
        (
            "letter_space",
            |v| v.letter_space(2),
            &[(PropId::TextLetterSpace, StyleValue::Int(2))],
        ),
        (
            "line_space",
            |v| v.line_space(3),
            &[(PropId::TextLineSpace, StyleValue::Int(3))],
        ),
        ("text_decor", |v| v.text_decor(TextDecor::UNDERLINE), &[]),
        (
            "opacity",
            |v| v.opacity(Opa::P50),
            &[(PropId::OpaLayered, StyleValue::Opa(Opa::P50))],
        ),
        (
            "transform_rotation",
            |v| v.transform_rotation(Angle::deg(10)),
            &[(PropId::TransformRotation, StyleValue::Angle(Angle(100)))],
        ),
        (
            "transform_scale",
            |v| v.transform_scale(Scale(300)),
            &[(PropId::TransformScaleX, StyleValue::Scale(Scale(300)))],
        ),
        (
            "transform_pivot",
            |v| v.transform_pivot(Point::new(3, 4)),
            &[(PropId::TransformPivotX, StyleValue::Length(Length::Px(3)))],
        ),
        ("blend_mode", |v| v.blend_mode(BlendMode::Additive), &[]),
        (
            "clip_corner",
            |v| v.clip_corner(true),
            &[(PropId::ClipCorner, StyleValue::Bool(true))],
        ),
        (
            "recolor",
            |v| v.recolor(Color::RED, Opa::P50),
            &[
                (PropId::Recolor, StyleValue::Color(Color::RED)),
                (PropId::RecolorOpa, StyleValue::Opa(Opa::P50)),
            ],
        ),
        (
            "flex_grow",
            |v| v.flex_grow(2),
            &[(PropId::FlexGrow, StyleValue::Int(2))],
        ),
        (
            "grid_cell_align",
            |v| v.grid_cell_align(GridAlign::Center, GridAlign::End),
            &[
                (PropId::GridCellXAlign, StyleValue::Enum(GridAlign::Center as u8)),
                (PropId::GridCellYAlign, StyleValue::Enum(GridAlign::End as u8)),
            ],
        ),
        ("transition", |v| v.transition(&SMOOTH), &[]),
    ];
    for (name, apply, want) in cases {
        let apply = *apply;
        let t = mount(move |_| apply(label("x").test_id("n")).into_any());
        for (p, v) in *want {
            assert_eq!(prop(&t, *p), *v, "{name}: {p:?}");
        }
        // Every modifier sets at least one local property.
        let e = t.engine();
        let local = e
            .tree()
            .node(node(&t))
            .unwrap()
            .styles()
            .entries()
            .iter()
            .any(|s| s.kind == twine_style::EntryKind::Local);
        assert!(local, "{name} set no local property");
    }
}

#[test]
fn dynamic_modifier_updates() {
    let mut t = mount(|cx| {
        let w = cx.signal(20);
        let c = cx.signal(Color::RED);
        cx.provide((w, c));
        label("x").width(w).bg(c).test_id("n").into_any()
    });
    assert_eq!(t.find(by_id("n")).coords().width(), 20);
    let (w, c) = t.root_scope().expect_context::<(Signal<i32>, Signal<Color>)>();
    w.set(33);
    c.set(Color::BLUE);
    t.run_until_idle();
    assert_eq!(t.find(by_id("n")).coords().width(), 33);
    assert_eq!(prop(&t, PropId::BgColor), StyleValue::Color(Color::BLUE));
}

/// A view whose modifier reads `tick` but always produces the same value.
type Idem = (&'static str, fn(Signal<u32>) -> AnyView);

#[test]
fn modifier_idempotency_all() {
    let cases: &[Idem] = &[
        ("width", |s| {
            label("x")
                .width(move || {
                    s.get();
                    30
                })
                .into_any()
        }),
        ("size", |s| {
            label("x")
                .size(
                    move || {
                        s.get();
                        30
                    },
                    move || {
                        s.get();
                        20
                    },
                )
                .into_any()
        }),
        ("pos", |s| {
            label("x")
                .pos(
                    move || {
                        s.get();
                        3
                    },
                    4,
                )
                .into_any()
        }),
        ("align", |s| {
            label("x")
                .align(move || {
                    s.get();
                    Align::Center
                })
                .into_any()
        }),
        ("translate", |s| {
            label("x")
                .translate(
                    move || {
                        s.get();
                        2
                    },
                    0,
                )
                .into_any()
        }),
        ("offset", |s| {
            label("x")
                .offset(move || {
                    s.get();
                    Point::new(1, 1)
                })
                .into_any()
        }),
        ("padding", |s| {
            label("x")
                .padding(move || {
                    s.get();
                    4
                })
                .into_any()
        }),
        ("gap", |s| {
            column(label("x"))
                .gap(move || {
                    s.get();
                    4
                })
                .into_any()
        }),
        ("margin", |s| {
            label("x")
                .margin(move || {
                    s.get();
                    2
                })
                .into_any()
        }),
        ("bg", |s| {
            label("x")
                .bg(move || {
                    s.get();
                    Color::RED
                })
                .into_any()
        }),
        ("bg_opa", |s| {
            label("x")
                .bg_opa(move || {
                    s.get();
                    Opa::P50
                })
                .into_any()
        }),
        ("border", |s| {
            label("x")
                .border(
                    move || {
                        s.get();
                        2
                    },
                    Color::RED,
                )
                .into_any()
        }),
        ("outline", |s| {
            label("x")
                .outline(
                    move || {
                        s.get();
                        2
                    },
                    Color::RED,
                    1,
                )
                .into_any()
        }),
        ("radius", |s| {
            label("x")
                .radius(move || {
                    s.get();
                    4
                })
                .into_any()
        }),
        ("shadow_width", |s| {
            label("x")
                .shadow_width(move || {
                    s.get();
                    4
                })
                .into_any()
        }),
        ("text_color", |s| {
            label("x")
                .text_color(move || {
                    s.get();
                    Color::RED
                })
                .into_any()
        }),
        ("opacity", |s| {
            label("x")
                .opacity(move || {
                    s.get();
                    Opa::P50
                })
                .into_any()
        }),
        ("transform_scale", |s| {
            label("x")
                .transform_scale(move || {
                    s.get();
                    Scale(300)
                })
                .into_any()
        }),
        ("recolor", |s| {
            label("x")
                .recolor(
                    move || {
                        s.get();
                        Color::RED
                    },
                    Opa::P50,
                )
                .into_any()
        }),
        ("hidden", |s| {
            label("x")
                .hidden(move || {
                    s.get();
                    false
                })
                .into_any()
        }),
        ("disabled", |s| {
            button(label("x"))
                .disabled(move || {
                    s.get();
                    true
                })
                .into_any()
        }),
        ("clickable", |s| {
            label("x")
                .clickable(move || {
                    s.get();
                    true
                })
                .into_any()
        }),
        ("scroll_dir", |s| {
            column(label("x"))
                .scroll_dir(move || {
                    s.get();
                    Dir::VER
                })
                .into_any()
        }),
        ("scrollbar", |s| {
            column(label("x"))
                .scrollbar(move || {
                    s.get();
                    ScrollbarMode::Off
                })
                .into_any()
        }),
        ("focusable", |s| {
            label("x")
                .focusable(move || {
                    s.get();
                    true
                })
                .into_any()
        }),
        ("flex_grow", |s| {
            label("x")
                .flex_grow(move || {
                    s.get();
                    1
                })
                .into_any()
        }),
        ("text", |s| {
            label(text!("same {}", {
                s.get();
                1
            }))
            .into_any()
        }),
        ("long_mode", |s| {
            label("x")
                .long_mode(move || {
                    s.get();
                    LongMode::Dots
                })
                .into_any()
        }),
        ("image", |s| {
            image(move || {
                s.get();
                ImageSource::Symbol("\u{f00c}")
            })
            .into_any()
        }),
        ("image_rotation", |s| {
            image(ImageSource::Symbol("a"))
                .rotation(move || {
                    s.get();
                    Angle::deg(5)
                })
                .into_any()
        }),
    ];
    for (name, f) in cases {
        let f = *f;
        let mut t = TestUi::new(200, 150).mount(move |cx| {
            let tick = cx.signal(0u32);
            cx.provide(tick);
            f(tick)
        });
        t.run_until_idle();
        let tick = t.root_scope().expect_context::<Signal<u32>>();
        let runs = twine_reactive::debug_stats().effect_runs;
        tick.set(1);
        t.update();
        assert!(
            twine_reactive::debug_stats().effect_runs > runs,
            "{name}: no binding ran"
        );
        assert!(
            t.invalidations().is_empty(),
            "{name}: {:?}",
            t.invalidations().to_vec()
        );
        assert!(t.flushes().is_empty(), "{name}: redrew");
        t.assert_idle();
    }
}

#[test]
fn flag_modifiers_apply() {
    type Case = (&'static str, fn() -> AnyView, ObjFlags, bool);
    let cases: &[Case] = &[
        (
            "hidden",
            || label("x").hidden(true).test_id("n").into_any(),
            ObjFlags::HIDDEN,
            true,
        ),
        (
            "clickable",
            || label("x").clickable(true).test_id("n").into_any(),
            ObjFlags::CLICKABLE,
            true,
        ),
        (
            "checkable",
            || label("x").checkable(true).test_id("n").into_any(),
            ObjFlags::CHECKABLE,
            true,
        ),
        (
            "scrollable",
            || label("x").scrollable(false).test_id("n").into_any(),
            ObjFlags::SCROLLABLE,
            false,
        ),
        (
            "scroll_momentum",
            || label("x").scroll_momentum(false).test_id("n").into_any(),
            ObjFlags::SCROLL_MOMENTUM,
            false,
        ),
        (
            "scroll_elastic",
            || label("x").scroll_elastic(false).test_id("n").into_any(),
            ObjFlags::SCROLL_ELASTIC,
            false,
        ),
        (
            "scroll_one",
            || label("x").scroll_one(true).test_id("n").into_any(),
            ObjFlags::SCROLL_ONE,
            true,
        ),
        (
            "scroll_on_focus",
            || label("x").scroll_on_focus(false).test_id("n").into_any(),
            ObjFlags::SCROLL_ON_FOCUS,
            false,
        ),
        (
            "focusable",
            || label("x").focusable(true).test_id("n").into_any(),
            ObjFlags::CLICK_FOCUSABLE,
            true,
        ),
        (
            "floating",
            || label("x").floating(true).test_id("n").into_any(),
            ObjFlags::FLOATING,
            true,
        ),
        (
            "ignore_layout",
            || label("x").ignore_layout(true).test_id("n").into_any(),
            ObjFlags::IGNORE_LAYOUT,
            true,
        ),
        (
            "event_bubble",
            || label("x").event_bubble(true).test_id("n").into_any(),
            ObjFlags::EVENT_BUBBLE,
            true,
        ),
        (
            "overflow_visible",
            || label("x").overflow_visible(true).test_id("n").into_any(),
            ObjFlags::OVERFLOW_VISIBLE,
            true,
        ),
    ];
    for (name, f, flag, on) in cases {
        let f = *f;
        let t = mount(move |_| f());
        assert_eq!(t.engine().has_flag(node(&t), *flag), *on, "{name}");
    }
    let t = mount(|_| button(label("x")).disabled(true).test_id("n").into_any());
    assert!(t.find(by_id("n")).state().contains(State::DISABLED));
    let t = mount(|_| {
        column(label("x"))
            .scroll_dir(Dir::HOR)
            .scrollbar(ScrollbarMode::On)
            .scroll_snap_x(ScrollSnap::Center)
            .scroll_snap_y(ScrollSnap::Start)
            .test_id("n")
            .into_any()
    });
    let e = t.engine();
    let n = node(&t);
    assert_eq!(e.scroll_dir(n), Dir::HOR);
    assert_eq!(e.scrollbar_mode(n), ScrollbarMode::On);
    assert_eq!(e.scroll_snap_x(n), ScrollSnap::Center);
    assert_eq!(e.scroll_snap_y(n), ScrollSnap::Start);
    drop(e);
    let t = mount(|_| label("x").focusable(true).test_id("n").into_any());
    assert!(
        t.engine().group_of(node(&t)).is_some(),
        "focusable joins the default group"
    );
}

#[test]
fn style_list_modifiers() {
    let t = mount(|_| {
        label("x")
            .style(&STYLE_A)
            .style_for(Selector::state(State::PRESSED), &STYLE_B)
            .style_ref(Selector::MAIN, StyleRef::Static(&STYLE_B))
            .class_style(Selector::MAIN, &STYLE_A)
            .test_id("n")
            .into_any()
    });
    assert_eq!(prop(&t, PropId::Radius), StyleValue::Int(5));
    assert_eq!(prop(&t, PropId::BgColor), StyleValue::Color(Color::GREEN));
    let e = t.engine();
    let kinds: Vec<_> = e
        .tree()
        .node(node(&t))
        .unwrap()
        .styles()
        .entries()
        .iter()
        .map(|s| s.kind)
        .collect();
    assert!(kinds.contains(&twine_style::EntryKind::Theme));
    assert!(kinds.contains(&twine_style::EntryKind::Normal));
}

#[test]
fn event_modifiers_fire() {
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let l = log.clone();
    let mut t = mount(move |_| {
        let push = move |s: &'static str| {
            let l = l.clone();
            move || l.borrow_mut().push(s)
        };
        let (g, k, sc, cu) = (push("gesture"), push("key"), push("scroll"), push("custom"));
        label("x")
            .clickable(true)
            .on_click(push("click"))
            .on_press(push("press"))
            .on_release(push("release"))
            .on_long_press(push("long"))
            .on_long_press_repeat(push("repeat"))
            .on_focus(push("focus"))
            .on_defocus(push("defocus"))
            .on_value_changed(push("value"))
            .on_gesture(move |_d| g())
            .on_key(move |_k| k())
            .on_scroll(move |_| sc())
            .on_event(EventCode::Custom(3), move |_cx| cu())
            .test_id("n")
            .into_any()
    });
    let n = node(&t);
    let e = t.engine_mut();
    for (code, param) in [
        (EventCode::Clicked, EventParam::None),
        (EventCode::Pressed, EventParam::None),
        (EventCode::Released, EventParam::None),
        (EventCode::LongPressed, EventParam::None),
        (EventCode::LongPressedRepeat, EventParam::None),
        (EventCode::Focused, EventParam::None),
        (EventCode::Defocused, EventParam::None),
        (EventCode::ValueChanged, EventParam::None),
        (EventCode::Gesture, EventParam::Dir(Dir::LEFT)),
        (EventCode::Key, EventParam::Key(Key::Enter)),
        (EventCode::Scroll, EventParam::None),
        (EventCode::Custom(3), EventParam::None),
    ] {
        e.send_event(n, code, param);
    }
    let got = log.borrow().clone();
    for want in [
        "click", "press", "release", "long", "repeat", "focus", "defocus", "value", "gesture", "key",
        "scroll", "custom",
    ] {
        assert!(got.contains(&want), "{want} missing in {got:?}");
    }
}

#[test]
fn identity_modifiers() {
    let r: Rc<RefCell<Option<NodeRef<Label>>>> = Rc::default();
    let r2 = r.clone();
    let t = mount(move |cx| {
        let nr = cx.node_ref::<Label>();
        *r2.borrow_mut() = Some(nr);
        label("x").test_id("n").node_ref(nr).into_any()
    });
    let nr = r.borrow().unwrap();
    assert_eq!(nr.get(), Some(node(&t)));
    let g = t.engine().default_group().unwrap();
    let t = mount(move |_| label("x").group(g).test_id("n").into_any());
    assert!(t.engine().group_of(node(&t)).is_some());
}

#[test]
fn align_to_follows_ref() {
    let t = mount(|cx| {
        let base = cx.node_ref::<Label>();
        container((
            label("base").pos(40, 30).node_ref(base).test_id("base"),
            label("next")
                .align_to(base, Align::OutBottomLeft, 0, 5)
                .test_id("n"),
        ))
        .size(Length::pct(100), Length::pct(100))
        .into_any()
    });
    let b = t.find(by_id("base")).coords();
    let n = t.find(by_id("n")).coords();
    assert_eq!((n.x0, n.y0), (b.x0, b.y1 + 5));
}

#[test]
fn column_children_stacked() {
    let t = mount(|_| {
        column((label("a").test_id("a"), label("b").test_id("b")))
            .gap(7)
            .into_any()
    });
    let a = t.find(by_id("a")).coords();
    let b = t.find(by_id("b")).coords();
    assert_eq!(b.y0, a.y1 + 7);
    assert_eq!(a.x0, b.x0);
    let t = mount(|_| {
        row((label("a").test_id("a"), label("b").test_id("b")))
            .gap(5)
            .into_any()
    });
    let a = t.find(by_id("a")).coords();
    let b = t.find(by_id("b")).coords();
    assert_eq!(b.x0, a.x1 + 5);
}

static COLS: [GridTrack; 2] = [GridTrack::Px(50), GridTrack::Px(60)];
static ROWS: [GridTrack; 2] = [GridTrack::Px(20), GridTrack::Px(30)];

#[test]
fn grid_places_cells() {
    let t = mount(|_| {
        grid(
            &COLS,
            &ROWS,
            (
                label("a").grid_cell(0, 1, 0, 1).test_id("a"),
                label("b").grid_cell(1, 1, 1, 1).test_id("b"),
            ),
        )
        .gap(0)
        .test_id("g")
        .into_any()
    });
    let g = t.find(by_id("g")).coords();
    let a = t.find(by_id("a")).coords();
    let b = t.find(by_id("b")).coords();
    assert_eq!((a.x0 - g.x0, a.y0 - g.y0), (0, 0));
    assert_eq!((b.x0 - g.x0, b.y0 - g.y0), (50, 20));
}

#[test]
fn stack_centers() {
    let t = mount(|_| {
        stack((label("a").test_id("a"), label("bbbbbb").test_id("b")))
            .size(120, 80)
            .test_id("s")
            .into_any()
    });
    let s = t.engine().content_area(t.find(by_id("s")).id());
    for id in ["a", "b"] {
        let c = t.find(by_id(id)).coords();
        assert!((c.center().x - s.center().x).abs() <= 1, "{id}");
        assert!((c.center().y - s.center().y).abs() <= 1, "{id}");
    }
}

#[test]
fn spacer_grows() {
    let t = mount(|_| {
        row((label("a").test_id("a"), spacer(), label("b").test_id("b")))
            .width(180)
            .test_id("r")
            .into_any()
    });
    let r = t.find(by_id("r")).coords();
    let b = t.find(by_id("b")).coords();
    assert_eq!(b.x1, r.x1, "the spacer pushes b to the end");
}

#[test]
fn scroll_view_scrolls_on_drag() {
    let mut t = mount(|_| {
        scroll_view(
            Dir::VER,
            (0..20)
                .map(|i| label(format!("row {i}")).height(20))
                .collect::<Vec<_>>(),
        )
        .size(150, 100)
        .test_id("n")
        .into_any()
    });
    let n = node(&t);
    assert_eq!(t.engine().scroll_offset(n).y, 0);
    t.drag(Point::new(50, 80), Point::new(50, 20), Duration::ms(300));
    t.run_until_idle();
    assert!(
        t.engine().scroll_offset(n).y > 20,
        "{:?}",
        t.engine().scroll_offset(n)
    );
}

fn column_row_scene(_cx: Scope) -> AnyView {
    column((
        label("Column / row"),
        row((button(label("One")), button(label("Two")), button(label("Three")))).gap(6),
        row((label("left"), spacer(), label("right"))).width(Length::pct(100)),
        container(label("card")).width(120),
    ))
    .gap(8)
    .padding(8)
    .size(Length::pct(100), Length::pct(100))
    .into_any()
}

#[test]
fn view_column_row_light() {
    let mut t = TestUi::new(240, 160).mount(column_row_scene);
    t.run_until_idle();
    t.assert_snapshot("view_column_row_light");
}

static GRID_COLS: [GridTrack; 3] = [GridTrack::Fr(1), GridTrack::Fr(1), GridTrack::Fr(1)];
static GRID_ROWS: [GridTrack; 2] = [GridTrack::Fr(1), GridTrack::Fr(1)];

#[test]
fn view_grid_dark() {
    let mut t = TestUi::new(240, 160)
        .theme(Rc::new(DefaultTheme::dark()))
        .mount(|_| {
            let cells: Vec<_> = (0..6)
                .map(|i| {
                    button(label(["A", "B", "C", "D", "E", "F"][i]))
                        .grid_cell((i % 3) as i32, 1, (i / 3) as i32, 1)
                        .grid_cell_align(GridAlign::Stretch, GridAlign::Stretch)
                })
                .collect();
            grid(&GRID_COLS, &GRID_ROWS, cells)
                .gap(6)
                .padding(6)
                .size(Length::pct(100), Length::pct(100))
        });
    t.run_until_idle();
    t.assert_snapshot("view_grid_dark");
    let _ = (by_text("A"), Rect::ZERO);
}
