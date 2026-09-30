//! R1.S04: one style vocabulary. The view modifiers, the `style!` keys and the `StyleBuf`
//! builders are generated from the same property and shorthand tables of `twine-style`; these
//! tests walk the tables (through the same callback macros) and check that every property and
//! every shorthand exists in all three APIs under its table name and sets the same properties.

use std::collections::BTreeSet;
use twine_core::Fraction;
use twine_style::Radius;

use twine_anim::{AnimTemplate, Easing};
use twine_core::{Angle, Color, Duration, Insets, Opa, Point, Scale};
use twine_engine::NodeId;
use twine_image::ImageSource;
use twine_render::{BlendMode, BorderSide, GradKind, GradStop, Gradient, ShadowDsc};
use twine_style::{
    Align, BaseDir, BlurQuality, ColorFilter, CrossAlign, FlexFlow, GradDir, GridAlign, GridSpan, GridTrack,
    ImageColorkey, LayoutKind, Length, MainAlign, PROP_ALIASES, PROP_COUNT, PROP_META, Part, PropId,
    PropValue, SHORTHANDS, Style, StyleBuf, StyleProp, StyleValue, TextLeadingTrim, TransitionDsc,
};
use twine_testing::{TestUi, by_id};
use twine_text::{Font, TextAlign, TextDecor};
use twine_view::prelude::{AnyView, IntoAnyView, Scope, ViewExt, label};

/// A constant, non-default sample of every payload type (so a set property is observable).
trait Sample: Copy {
    const S: Self;
}

macro_rules! samples {
    ($($t:ty = $v:expr;)*) => { $(impl Sample for $t { const S: Self = $v; })* };
}

static GRAD: Gradient = Gradient::new(
    GradKind::Ver,
    &[
        GradStop::new(Color::RED, Fraction::ZERO),
        GradStop::new(Color::BLUE, Fraction::ONE),
    ],
);
static IMG: ImageSource = ImageSource::Symbol("s");
static KEY: ImageColorkey = ImageColorkey {
    low: Color::BLACK,
    high: Color::WHITE,
};
static FILTER: ColorFilter = ColorFilter::SHADE;
static ANIM: AnimTemplate = AnimTemplate::new(Duration::ms(5), Easing::Linear);
static TRANS: TransitionDsc = TransitionDsc::new(&[PropId::BgColor], Duration::ms(5), Easing::Linear);
static TRACKS: [GridTrack; 2] = [GridTrack::Px(10), GridTrack::Fr(1)];

samples! {
    i32 = 7;
    u32 = 9;
    u8 = 3;
    u16 = 300;
    bool = true;
    Length = Length::Px(11);
    Color = Color::RED;
    Opa = Opa::P50;
    Angle = Angle::deci_deg(150);
    Scale = Scale::from_raw_256(300);
    Radius = Radius::Px(13);
    Fraction = Fraction::from_raw(77);
    twine_style::DurationMs = twine_style::DurationMs::from_duration(Duration::ms(123));
    Align = Align::Center;
    GradDir = GradDir::Hor;
    BorderSide = BorderSide::BOTTOM;
    BlurQuality = BlurQuality::Speed;
    TextDecor = TextDecor::UNDERLINE;
    TextAlign = TextAlign::Center;
    TextLeadingTrim = TextLeadingTrim::CapitalBaseline;
    BlendMode = BlendMode::Additive;
    LayoutKind = LayoutKind::Grid;
    BaseDir = BaseDir::Rtl;
    FlexFlow = FlexFlow::COLUMN.wrap(true);
    MainAlign = MainAlign::SpaceAround;
    CrossAlign = CrossAlign::Center;
    GridSpan = GridSpan::new(2, 3);
    GridAlign = GridAlign::End;
    &'static Gradient = &GRAD;
    &'static ImageSource = &IMG;
    &'static ImageColorkey = &KEY;
    &'static Font = &twine_assets::fonts::MONTSERRAT_20;
    &'static ColorFilter = &FILTER;
    &'static AnimTemplate = &ANIM;
    &'static TransitionDsc = &TRANS;
    &'static [GridTrack] = &TRACKS;
    Point = Point::new(4, 5);
    Insets = Insets::new(1, 2, 3, 4);
    ShadowDsc = ShadowDsc { width: 6, ofs_x: 1, ofs_y: 2, spread: 3, color: Color::BLUE, opa: Opa::P30 };
}

/// The sample value of a property, as the style value it should resolve to.
fn sample_value<T: Sample + PropValue>() -> StyleValue {
    T::S.into_value()
}

fn mount(v: impl FnOnce(Scope) -> AnyView + 'static) -> (TestUi, NodeId) {
    let mut t = TestUi::new(120, 80).mount(v);
    t.run_until_idle();
    let n = t.find(by_id("n")).id();
    (t, n)
}

/// One row per property: `(id, key, StyleBuf value, style! value, view value)`.
type PropCheck = (
    PropId,
    &'static str,
    Option<StyleValue>,
    Option<StyleValue>,
    StyleValue,
);

macro_rules! check_props {
    (
        []
        $(
            $(#[doc = $doc:literal])*
            $name:ident ( $key:ident ) : $kind:ident [$($ty:tt)+] [ $($flag:ident)* ] $default:ident
                [ $($alias:literal)* ];
        )*
    ) => {
        /// Sets every property through its `StyleBuf` builder, its `style!` key and its view
        /// modifier (all on one label), and reports what each API stored.
        fn check_every_prop() -> Vec<(PropCheck, StyleValue)> {
            // One view with every property modifier.
            let (t, n) = mount(|_| {
                let v = label("x").test_id("n");
                $( let v = v.$key(<twine_style::__prop_ty!($($ty)+) as Sample>::S); )*
                v.into_any()
            });
            let e = t.engine();
            vec![$(
                {
                    static S: Style = twine_style::style! { $key: <twine_style::__prop_ty!($($ty)+) as Sample>::S };
                    let want = sample_value::<twine_style::__prop_ty!($($ty)+)>();
                    let buf = StyleBuf::new().$key(<twine_style::__prop_ty!($($ty)+) as Sample>::S);
                    (
                        (
                            PropId::$name,
                            stringify!($key),
                            buf.get(PropId::$name),
                            S.get(PropId::$name),
                            e.style_prop(n, Part::Main, PropId::$name),
                        ),
                        want,
                    )
                },
            )*]
        }
    };
}

twine_style::__prop_table!(check_props);

/// One row per shorthand: `(name, StyleBuf props, style! props, view values)`.
type ShorthandCheck = (
    &'static str,
    Vec<StyleProp>,
    Vec<StyleProp>,
    Vec<(PropId, StyleValue)>,
);

macro_rules! check_shorthands {
    (
        []
        $(
            $(#[doc = $doc:literal])*
            $name:ident (
                $(
                    $p:ident : $pk:ident $(<$g:ident>)? [$($pty:tt)+]
                        => $( $var:ident $( ( $($sel:tt)+ ) )? $( = $c:ident )? ),+
                );+
            ) [ $($alias:literal)* ] { $(#[doc = $ex:literal])* };
        )*
    ) => {
        /// Applies every shorthand through `StyleBuf`, `style!` and the view modifier.
        fn check_every_shorthand() -> Vec<ShorthandCheck> {
            vec![$(
                {
                    static S: Style = twine_style::style! {
                        $name: ($(<twine_style::__prop_ty!($($pty)+) as Sample>::S),+)
                    };
                    let buf = StyleBuf::new().$name($(<twine_style::__prop_ty!($($pty)+) as Sample>::S),+);
                    let (t, n) = mount(|_| {
                        label("x")
                            .test_id("n")
                            .$name($(<twine_style::__prop_ty!($($pty)+) as Sample>::S),+)
                            .into_any()
                    });
                    let view = buf
                        .iter()
                        .map(|p| (p.id(), t.engine().style_prop(n, Part::Main, p.id())))
                        .collect();
                    (stringify!($name), buf.iter().copied().collect(), S.props().to_vec(), view)
                },
            )*]
        }
    };
}

twine_style::__shorthand_table!(check_shorthands);

fn ids(props: &[StyleProp]) -> Vec<PropId> {
    props.iter().map(StyleProp::id).collect()
}

#[test]
fn every_table_property_has_modifier_builder_and_key() {
    let rows = check_every_prop();
    assert_eq!(rows.len(), PROP_COUNT, "the table has every property once");
    let mut seen = BTreeSet::new();
    for ((id, key, buf, style, view), want) in rows {
        assert!(seen.insert(id), "{id:?} twice");
        assert_eq!(key, id.meta().snake_name, "{id:?}: key");
        assert_ne!(
            want,
            id.meta().default,
            "{id:?}: the sample must differ from the default"
        );
        assert_eq!(buf, Some(want), "StyleBuf::{key}");
        assert_eq!(style, Some(want), "style! {{ {key}: .. }}");
        assert_eq!(view, want, "ViewExt::{key}");
    }
    assert_eq!(seen.len(), PROP_COUNT);
}

#[test]
fn every_shorthand_has_modifier_builder_and_key() {
    let rows = check_every_shorthand();
    assert_eq!(rows.len(), SHORTHANDS.len());
    for ((name, buf, style, view), meta) in rows.into_iter().zip(SHORTHANDS) {
        assert_eq!(name, meta.name);
        // `style!` keeps every entry in order; `StyleBuf` keeps one per property.
        assert_eq!(ids(&style), meta.props, "style! {{ {name}: .. }}");
        let unique: BTreeSet<PropId> = meta.props.iter().copied().collect();
        assert_eq!(
            ids(&buf).into_iter().collect::<BTreeSet<_>>(),
            unique,
            "StyleBuf::{name}"
        );
        for p in &buf {
            let s = style
                .iter()
                .rev()
                .find(|q| q.id() == p.id())
                .map(StyleProp::value);
            assert_eq!(
                s,
                Some(p.value()),
                "{name}: style! and StyleBuf differ on {:?}",
                p.id()
            );
        }
        for (id, v) in view {
            let b = buf.iter().find(|q| q.id() == id).map(StyleProp::value);
            assert_eq!(Some(v), b, "{name}: view and StyleBuf differ on {id:?}");
        }
    }
}

#[test]
fn names_are_unique_and_old_names_are_only_aliases() {
    let keys: BTreeSet<&str> = PROP_META.iter().map(|m| m.snake_name).collect();
    assert_eq!(keys.len(), PROP_COUNT, "property keys are unique");
    let shorthands: BTreeSet<&str> = SHORTHANDS.iter().map(|s| s.name).collect();
    assert_eq!(shorthands.len(), SHORTHANDS.len(), "shorthand names are unique");
    assert!(keys.is_disjoint(&shorthands), "a shorthand shadows a property");
    for (m, aliases) in PROP_META.iter().zip(PROP_ALIASES.iter()) {
        assert!(
            !aliases.is_empty(),
            "{}: every property names its LVGL constant",
            m.name
        );
        for a in *aliases {
            assert!(
                !keys.contains(a) && !shorthands.contains(a),
                "{}: alias {a} is a live name",
                m.name
            );
        }
    }
    for s in SHORTHANDS {
        for a in s.aliases {
            assert!(
                !keys.contains(a) && !shorthands.contains(a),
                "{}: alias {a} is a live name",
                s.name
            );
        }
    }
}
