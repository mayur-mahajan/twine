//! R2.S02: a design element the theme does not define resolves to the property's default,
//! warns once per element kind and never panics. In its own test binary: the "warned" flags
//! are process-wide.

use twine_style::StyleValue;
use twine_style::design::{ColorElement, LengthElement};
use twine_testing::{TestUi, by_id, capture_logs};
use twine_view::prelude::*;

#[test]
fn missing_element_falls_back_to_the_default_and_warns_once() {
    // Application elements nobody gave a value.
    const MISSING: ColorElement = ColorElement::custom(40);
    const MISSING_TOO: ColorElement = ColorElement::custom(41);
    const NO_SPACE: LengthElement = LengthElement::custom(3);
    let (t, logs) = capture_logs(|| {
        let mut t = TestUi::new(100, 100).theme(DefaultTheme::light()).mount(|_| {
            column((
                container(())
                    .size(10, 10)
                    .border_color(MISSING)
                    .padding(NO_SPACE)
                    .test_id("a"),
                container(()).size(10, 10).border_color(MISSING_TOO).test_id("b"),
            ))
        });
        t.run_until_idle();
        t
    });
    let e = t.engine();
    let (a, b) = (t.find(by_id("a")).id(), t.find(by_id("b")).id());
    // `border_color`'s default is black, `padding_top`'s 0.
    assert_eq!(
        e.style_prop(a, Part::Main, PropId::BorderColor),
        StyleValue::Color(Color::BLACK)
    );
    assert_eq!(
        e.style_prop(b, Part::Main, PropId::BorderColor),
        StyleValue::Color(Color::BLACK)
    );
    assert_eq!(e.style_i32(a, Part::Main, PropId::PaddingTop), 0);
    let warnings: Vec<_> = logs
        .iter()
        .filter(|l| l.target == "twine::style" && l.message.contains("is not defined by the theme"))
        .collect();
    // Once per kind (color, length), although resolved many times.
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings.iter().any(|l| l.message.contains("color element #60")),
        "{warnings:?}"
    );

    // Without any theme, standard elements fall back the same way (no new warning).
    let (mut t, logs) = capture_logs(|| {
        TestUi::new(100, 100)
            .no_theme()
            .mount(|_| container(()).size(10, 10).bg(design::SURFACE).test_id("c"))
    });
    t.run_until_idle();
    let c = t.find(by_id("c")).id();
    assert_eq!(
        t.engine().style_prop(c, Part::Main, PropId::BgColor),
        PropId::BgColor.meta().default
    );
    assert!(
        !logs.iter().any(|l| l.message.contains("is not defined")),
        "{logs:?}"
    );
}
