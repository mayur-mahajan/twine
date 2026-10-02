//! `style!` macro tests (integration tests: the macro is used from another crate, as by users).

use twine_core::{Color, Opa, Scale};
use twine_style::{Length, PropId, Radius, Style, StyleProp, StyleValue, style};

fn ids(s: &Style) -> Vec<PropId> {
    s.props().iter().map(StyleProp::id).collect()
}

fn values(s: &Style) -> Vec<StyleValue> {
    s.props().iter().map(StyleProp::value).collect()
}

#[test]
fn macro_basic_props() {
    static S: Style = style! { bg_color: Color::RED, bg_opacity: Opa::COVER, radius: 8, clip_corner: true };
    static EMPTY: Style = style! {};
    assert_eq!(
        ids(&S),
        [
            PropId::BgColor,
            PropId::BgOpacity,
            PropId::Radius,
            PropId::ClipCorner
        ]
    );
    assert_eq!(S.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
    assert_eq!(S.get(PropId::Radius), Some(StyleValue::Length(Length::Px(8))));
    assert_eq!(S.get(PropId::ClipCorner), Some(StyleValue::Bool(true)));
    assert!(EMPTY.is_empty());
}

#[test]
fn macro_shorthands_expand_to_expected_props() {
    static S: Style = style! {
        padding: 1,
        padding_x: 2,
        padding_y: 3,
        gap: 4,
        margin: 5,
        margin_x: 6,
        margin_y: 7,
        size: (100, Length::pct(50)),
        transform_scale: Scale::from_raw_256(300),
        border: (2, Color::BLUE),
    };
    use PropId as P;
    assert_eq!(
        ids(&S),
        [
            P::PaddingTop,
            P::PaddingBottom,
            P::PaddingLeft,
            P::PaddingRight, // pad_all
            P::PaddingLeft,
            P::PaddingRight, // pad_hor
            P::PaddingTop,
            P::PaddingBottom, // pad_ver
            P::RowGap,
            P::ColumnGap, // pad_gap
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
            P::BorderColor,
            P::BorderOpacity, // border
        ]
    );
    let v = values(&S);
    assert_eq!(&v[..4], &[StyleValue::Length(Length::Px(1)); 4]);
    assert_eq!(
        &v[18..20],
        &[
            StyleValue::Length(Length::Px(100)),
            StyleValue::Length(Length::Pct(50))
        ]
    );
    assert_eq!(&v[20..22], &[StyleValue::Scale(Scale::from_raw_256(300)); 2]);
    assert_eq!(
        &v[22..],
        &[
            StyleValue::Length(Length::Px(2)),
            StyleValue::Color(Color::BLUE),
            StyleValue::Opa(Opa::COVER)
        ]
    );
    // Later shorthands override earlier ones.
    assert_eq!(S.get(P::PaddingLeft), Some(StyleValue::Length(Length::Px(2))));
    assert_eq!(S.get(P::PaddingTop), Some(StyleValue::Length(Length::Px(3))));
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
        transform_scale_y: Scale::pct(50),
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
        Some(StyleValue::Scale(Scale::from_raw_256(128)))
    );
}

#[test]
fn macro_dp_lengths_and_radius() {
    static S: Style = style! {
        padding: Length::dp(8),
        gap: 4,
        radius: Radius::Circle,
        border_width: Length::dp(1),
    };
    static R: Style = style! { radius: 6 };
    assert_eq!(
        S.get(PropId::PaddingLeft),
        Some(StyleValue::Length(Length::Dp(8)))
    );
    assert_eq!(S.get(PropId::RowGap), Some(StyleValue::Length(Length::Px(4))));
    assert_eq!(
        S.get(PropId::Radius).and_then(StyleValue::get::<Radius>),
        Some(Radius::Circle)
    );
    assert_eq!(
        S.get(PropId::BorderWidth),
        Some(StyleValue::Length(Length::Dp(1)))
    );
    assert_eq!(
        R.get(PropId::Radius).and_then(StyleValue::get::<Radius>),
        Some(Radius::Px(6))
    );
}

#[test]
fn macro_trailing_comma() {
    static A: Style = style! { radius: 1, };
    static B: Style = style! { radius: 1 };
    static C: Style = style! { padding: 2, };
    static D: Style = style! { size: (1, 2,), border: (1, Color::RED,) };
    assert_eq!(values(&A), values(&B));
    assert_eq!(C.props().len(), 4);
    assert_eq!(D.props().len(), 5);
}

#[test]
fn macro_duplicate_last_wins() {
    static S: Style = style! { radius: 1, bg_color: Color::RED, radius: 2 };
    assert_eq!(S.props().len(), 3, "the static keeps both entries");
    assert_eq!(S.get(PropId::Radius), Some(StyleValue::Length(Length::Px(2))));
}

#[test]
fn macro_covers_every_property() {
    // Every snake name of the table is a `style!` key (spot check across all groups; the
    // mapping is generated from the same table as `PropId`).
    static S: Style = style! {
        grid_cell_y_align: twine_style::GridAlign::End,
        text_leading_trim: twine_style::TextLeadingTrim::Capital,
        rotary_sensitivity: Scale::pct(200),
        flex_grow: 2,
        anim_duration: twine_core::Duration::ms(300),
        drop_shadow_quality: twine_style::BlurQuality::Speed,
        base_dir: twine_style::BaseDir::Rtl,
    };
    assert_eq!(S.props().len(), 7);
    for id in PropId::ALL {
        assert!(!id.snake_name().is_empty());
    }
}

#[test]
fn macro_new_shorthands_are_const() {
    use twine_core::{Insets, Point};
    use twine_render::ShadowDsc;
    use twine_style::{GridAlign, GridSpan};
    const SHADOW: ShadowDsc = ShadowDsc {
        width: 4,
        ofs_x: 1,
        ofs_y: 2,
        spread: 0,
        color: Color::BLACK,
        opa: Opa::P30,
    };
    // `const` (not only `static`): every shorthand stays a constant expression in flash.
    const S: Style = style! {
        bg: Color::RED,
        pos: (1, Length::pct(2)),
        translate: (3, 4),
        offset: Point::new(5, 6),
        padding_each: Insets::new(1, 2, 3, 4),
        margin_each: Insets::new(5, 6, 7, 8),
        outline: (2, Color::BLUE, 3),
        shadow: SHADOW,
        shadow_offset: (7, 8),
        transform_pivot: Point::new(9, 10),
        grid_col: GridSpan::new(1, 2),
        grid_row: GridSpan::range(3..7),
        grid_align: (GridAlign::Center, GridAlign::End),
        radius: (4),
    };
    static STATIC: Style = S;
    use PropId as P;
    assert_eq!(STATIC.get(P::BgOpacity), Some(StyleValue::Opa(Opa::COVER)));
    assert_eq!(STATIC.get(P::Y), Some(StyleValue::Length(Length::Pct(2))));
    // `offset` comes after `translate`: the later entry wins.
    assert_eq!(STATIC.get(P::TranslateX), Some(StyleValue::Length(Length::Px(5))));
    assert_eq!(STATIC.get(P::PaddingTop), Some(StyleValue::Length(Length::Px(2))));
    assert_eq!(
        STATIC.get(P::MarginRight),
        Some(StyleValue::Length(Length::Px(7)))
    );
    assert_eq!(STATIC.get(P::OutlineOpacity), Some(StyleValue::Opa(Opa::COVER)));
    assert_eq!(STATIC.get(P::ShadowOpacity), Some(StyleValue::Opa(Opa::P30)));
    // `shadow_offset` comes after `shadow`.
    assert_eq!(STATIC.get(P::ShadowOffsetY), Some(StyleValue::Int(8)));
    assert_eq!(
        STATIC.get(P::TransformPivotY),
        Some(StyleValue::Length(Length::Px(10)))
    );
    assert_eq!(STATIC.get(P::GridCellRowSpan), Some(StyleValue::Int(4)));
    assert_eq!(
        STATIC.get(P::GridCellYAlign),
        Some(StyleValue::from(GridAlign::End))
    );
    assert_eq!(STATIC.get(P::Radius), Some(StyleValue::Length(Length::Px(4))));
}

#[test]
fn macro_and_builder_agree_on_every_shorthand() {
    use twine_style::{SHORTHANDS, StyleBuf};
    // The shorthand list is generated; spot-check that `StyleBuf` and `style!` expand one the
    // same way (the full check over the table is `twine-view/tests/style_vocabulary.rs`).
    static S: Style = style! { padding: 3, gap: 2, border: (1, Color::RED) };
    let b = StyleBuf::new().padding(3).gap(2).border(1, Color::RED);
    for p in S.props() {
        assert_eq!(b.get(p.id()), Some(p.value()), "{:?}", p.id());
    }
    assert_eq!(b.len(), S.props().len());
    assert!(SHORTHANDS.iter().any(|s| s.name == "padding_x"));
}

// ---- Composition: `..BASE` spreads (R2.S05) ---------------------------------------------

static BUTTON: Style = style! { bg: Color::WHITE, radius: 8, padding: 12 };

#[test]
fn spread_single_overrides_in_place() {
    static DANGER: Style = style! { ..BUTTON, bg_color: Color::RED, width: 60 };
    // The overridden property keeps its position; new ones are appended; no duplicates.
    assert_eq!(
        ids(&DANGER),
        [
            PropId::BgColor,
            PropId::BgOpacity,
            PropId::Radius,
            PropId::PaddingTop,
            PropId::PaddingBottom,
            PropId::PaddingLeft,
            PropId::PaddingRight,
            PropId::Width
        ]
    );
    assert_eq!(DANGER.get(PropId::BgColor), Some(StyleValue::Color(Color::RED)));
    assert_eq!(
        DANGER.get(PropId::Radius),
        Some(StyleValue::Length(Length::Px(8)))
    );
    assert_eq!(BUTTON.get(PropId::BgColor), Some(StyleValue::Color(Color::WHITE)));
    assert_eq!(DANGER.has_group(), BUTTON.has_group() | PropId::Width.group_bit());
}

#[test]
fn spread_order_matters_and_later_wins() {
    static A: Style = style! { radius: 1, width: 10 };
    static B: Style = style! { radius: 2, height: 20 };
    static AB: Style = style! { ..A, ..B };
    static BA: Style = style! { ..B, ..A };
    // A key before a spread is overridden by the spread; one after it wins.
    static KEY_FIRST: Style = style! { radius: 9, ..A };
    static KEY_LAST: Style = style! { ..A, radius: 9 };
    static MIXED: Style = style! { width: 1, ..A, radius: 3, ..B, height: 4, };
    let r = |s: &Style| s.get(PropId::Radius);
    assert_eq!(r(&AB), Some(StyleValue::Length(Length::Px(2))));
    assert_eq!(r(&BA), Some(StyleValue::Length(Length::Px(1))));
    assert_eq!(r(&KEY_FIRST), Some(StyleValue::Length(Length::Px(1))));
    assert_eq!(r(&KEY_LAST), Some(StyleValue::Length(Length::Px(9))));
    assert_eq!(ids(&AB), [PropId::Radius, PropId::Width, PropId::Height]);
    assert_eq!(ids(&MIXED), [PropId::Width, PropId::Radius, PropId::Height]);
    assert_eq!(
        values(&MIXED),
        [
            StyleValue::Length(Length::Px(10)),
            StyleValue::Length(Length::Px(2)),
            StyleValue::Length(Length::Px(4))
        ]
    );
}

#[test]
fn spread_nested_and_of_consts_and_references() {
    const BASE: Style = style! { radius: 4, bg_opacity: Opa::P50 };
    static MID: Style = style! { ..BASE, radius: 6 };
    static LEAF: Style = style! { ..&MID, bg_opacity: Opa::COVER };
    static ONLY: Style = style! { ..LEAF };
    assert_eq!(LEAF.get(PropId::Radius), Some(StyleValue::Length(Length::Px(6))));
    assert_eq!(LEAF.get(PropId::BgOpacity), Some(StyleValue::Opa(Opa::COVER)));
    assert_eq!(ids(&ONLY), ids(&LEAF));
    assert_eq!(values(&ONLY), values(&LEAF));
}

#[test]
fn spread_collapses_duplicates_of_a_plain_base() {
    // A plain `style!` keeps its entries as written (later wins at lookup); a spread stores
    // each property once.
    static PLAIN: Style = style! { padding: 1, padding_top: 2 };
    static MERGED: Style = style! { ..PLAIN };
    assert_eq!(PLAIN.props().len(), 5);
    assert_eq!(MERGED.props().len(), 4);
    assert_eq!(
        MERGED.get(PropId::PaddingTop),
        Some(StyleValue::Length(Length::Px(2)))
    );
    assert_eq!(MERGED.get(PropId::PaddingLeft), PLAIN.get(PropId::PaddingLeft));
}

// Compile-time evaluation: the composition is a constant, so it cannot allocate or run code.
const COMPOSED: Style = style! { ..BUTTON, radius: Radius::Circle, transition: &SMOOTH };
static SMOOTH: twine_style::Transition = twine_style::Transition::of(
    twine_style::Props::of(PropId::BgColor),
    twine_core::Duration::ms(100),
);
const _: () = assert!(COMPOSED.props().len() == BUTTON.props().len() + 1);

#[test]
fn spread_is_const_and_keeps_static_references() {
    static FROM_CONST: Style = COMPOSED;
    // Pointers to other statics (fonts, transitions, …) survive the compile-time merge.
    static WITH_FONT: Style = style! { ..COMPOSED, font: &twine_text::EMPTY_FONT };
    static FONTED: Style = style! { ..WITH_FONT, padding: 0 };
    static CIRCLE: Style = style! { radius: Radius::Circle };
    assert_eq!(FROM_CONST.get(PropId::Radius), CIRCLE.get(PropId::Radius));
    assert!(matches!(
        FONTED.get(PropId::Transition),
        Some(StyleValue::Transition(twine_style::TransitionRef::Static(t))) if core::ptr::eq(t, &raw const SMOOTH)
    ));
    assert!(matches!(
        FONTED.get(PropId::Font),
        Some(StyleValue::Font(f)) if core::ptr::eq(f, &raw const twine_text::EMPTY_FONT)
    ));
}
