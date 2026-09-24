//! `style!` macro tests (integration tests: the macro is used from another crate, as by users).

use twine_core::{Color, Opa, Scale};
use twine_style::{Length, PropId, Style, StyleProp, StyleValue, style};

fn ids(s: &Style) -> Vec<PropId> {
    s.props().iter().map(StyleProp::id).collect()
}

fn values(s: &Style) -> Vec<StyleValue> {
    s.props().iter().map(StyleProp::value).collect()
}

#[test]
fn macro_basic_props() {
    static S: Style = style! { bg_color: Color::RED, bg_opa: Opa::COVER, radius: 8, clip_corner: true };
    static EMPTY: Style = style! {};
    assert_eq!(
        ids(&S),
        [PropId::BgColor, PropId::BgOpa, PropId::Radius, PropId::ClipCorner]
    );
    assert_eq!(S.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
    assert_eq!(S.get(PropId::Radius), Some(StyleValue::Int(8)));
    assert_eq!(S.get(PropId::ClipCorner), Some(StyleValue::Bool(true)));
    assert!(EMPTY.is_empty());
}

#[test]
fn macro_shorthands_expand_to_expected_props() {
    static S: Style = style! {
        pad_all: 1,
        pad_hor: 2,
        pad_ver: 3,
        pad_gap: 4,
        margin_all: 5,
        margin_hor: 6,
        margin_ver: 7,
        size: (100, Length::pct(50)),
        transform_scale: 300,
        border: (2, Color::BLUE),
    };
    use PropId as P;
    assert_eq!(
        ids(&S),
        [
            P::PadTop,
            P::PadBottom,
            P::PadLeft,
            P::PadRight, // pad_all
            P::PadLeft,
            P::PadRight, // pad_hor
            P::PadTop,
            P::PadBottom, // pad_ver
            P::PadRow,
            P::PadColumn, // pad_gap
            P::MarginTop,
            P::MarginBottom,
            P::MarginLeft,
            P::MarginRight, // margin_all
            P::MarginLeft,
            P::MarginRight, // margin_hor
            P::MarginTop,
            P::MarginBottom, // margin_ver
            P::Width,
            P::Height, // size
            P::TransformScaleX,
            P::TransformScaleY, // transform_scale
            P::BorderWidth,
            P::BorderColor, // border
        ]
    );
    let v = values(&S);
    assert_eq!(&v[..4], &[StyleValue::Int(1); 4]);
    assert_eq!(
        &v[18..20],
        &[
            StyleValue::Length(Length::Px(100)),
            StyleValue::Length(Length::Pct(50))
        ]
    );
    assert_eq!(&v[20..22], &[StyleValue::Scale(Scale(300)); 2]);
    assert_eq!(&v[22..], &[StyleValue::Int(2), StyleValue::Color(Color::BLUE)]);
    // Later shorthands override earlier ones.
    assert_eq!(S.get(P::PadLeft), Some(StyleValue::Int(2)));
    assert_eq!(S.get(P::PadTop), Some(StyleValue::Int(3)));
}

#[test]
fn macro_length_literal_and_pct() {
    const W: i32 = 42;
    static S: Style = style! {
        width: 100,
        height: Length::pct(25),
        x: -3,
        y: W,
        min_width: Length::Content,
        translate_x: Length::pct(10),
        transform_pivot_x: 5,
        transform_scale_x: Scale::ONE,
        transform_scale_y: 128,
    };
    assert_eq!(S.get(PropId::Width), Some(StyleValue::Length(Length::Px(100))));
    assert_eq!(S.get(PropId::Height), Some(StyleValue::Length(Length::Pct(25))));
    assert_eq!(S.get(PropId::X), Some(StyleValue::Length(Length::Px(-3))));
    assert_eq!(S.get(PropId::Y), Some(StyleValue::Length(Length::Px(42))));
    assert_eq!(S.get(PropId::MinWidth), Some(StyleValue::Length(Length::Content)));
    assert_eq!(
        S.get(PropId::TranslateX),
        Some(StyleValue::Length(Length::Pct(10)))
    );
    assert_eq!(
        S.get(PropId::TransformPivotX),
        Some(StyleValue::Length(Length::Px(5)))
    );
    assert_eq!(
        S.get(PropId::TransformScaleX),
        Some(StyleValue::Scale(Scale::ONE))
    );
    assert_eq!(
        S.get(PropId::TransformScaleY),
        Some(StyleValue::Scale(Scale(128)))
    );
}

#[test]
fn macro_trailing_comma() {
    static A: Style = style! { radius: 1, };
    static B: Style = style! { radius: 1 };
    static C: Style = style! { pad_all: 2, };
    static D: Style = style! { size: (1, 2,), border: (1, Color::RED,) };
    assert_eq!(values(&A), values(&B));
    assert_eq!(C.props().len(), 4);
    assert_eq!(D.props().len(), 4);
}

#[test]
fn macro_duplicate_last_wins() {
    static S: Style = style! { radius: 1, bg_color: Color::RED, radius: 2 };
    assert_eq!(S.props().len(), 3, "the static keeps both entries");
    assert_eq!(S.get(PropId::Radius), Some(StyleValue::Int(2)));
}

#[test]
fn macro_covers_every_property() {
    // Every snake name of the table is a `style!` key (spot check across all groups; the
    // mapping is generated from the same table as `PropId`).
    static S: Style = style! {
        grid_cell_y_align: twine_style::GridAlign::End,
        text_leading_trim: twine_style::TextLeadingTrim::Capital,
        rotary_sensitivity: 512u32,
        flex_grow: 2u8,
        anim_duration: 300u32,
        drop_shadow_quality: twine_style::BlurQuality::Speed,
        base_dir: twine_style::BaseDir::Rtl,
    };
    assert_eq!(S.props().len(), 7);
    for id in PropId::ALL {
        assert!(!id.meta().snake_name.is_empty());
    }
}
