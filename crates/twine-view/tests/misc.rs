//! Theme mode switching from a view, screen replacement and label text selection.

use twine_core::Point;
use twine_engine::EventCode;
use twine_style::{Part, PropId};
use twine_testing::{TestUi, by_id, by_text};
use twine_view::prelude::*;
use twine_widgets::label::Label;

#[test]
fn use_theme_switches_the_display_theme_mode() {
    let mut t = TestUi::new(200, 100)
        .mount(|cx| button(label("dark")).on_click(move || use_theme(cx).set_mode(ThemeMode::Dark)));
    t.run_until_idle();
    let screen = t
        .engine()
        .active_screen(t.engine().default_display().unwrap())
        .unwrap();
    let light = t.engine().style_color(screen, Part::Main, PropId::BgColor);
    t.find(by_text("dark")).click();
    t.run_until_idle();
    let dark = t.engine().style_color(screen, Part::Main, PropId::BgColor);
    assert_ne!(light, dark);
    t.assert_idle();
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    First,
    Second,
}

fn page(cx: Scope, p: Page) -> AnyView {
    match p {
        Page::First => {
            let nav = use_navigator::<Page>(cx).unwrap();
            button(label("first"))
                .on_click(move || nav.replace(Page::Second, ScreenAnim::FadeIn(Duration::ms(100))))
                .into_any()
        }
        Page::Second => label("second").test_id("second").into_any(),
    }
}

#[test]
fn navigator_replace_keeps_depth() {
    let mut t = TestUi::new(200, 100).mount(|cx| navigator(cx, Page::First, page));
    t.run_until_idle();
    t.find(by_text("first")).click();
    t.run_until_idle();
    let nav = use_navigator::<Page>(t.root_scope()).unwrap();
    assert_eq!(nav.depth(), 1);
    assert_eq!(nav.stack(), [Page::Second]);
    assert_eq!(t.find(by_id("second")).text(), "second");
    assert!(t.find_all(by_text("first")).is_empty());
    assert_eq!(nav.pop(ScreenAnim::None), NavOutcome::AtRoot);
}

#[test]
fn selectable_label_selects_by_dragging() {
    let mut t = TestUi::new(240, 60).mount(|_| label("select me please").selectable(true).test_id("l"));
    t.run_until_idle();
    let c = t.find(by_id("l")).coords();
    t.press(Point::new(c.x0 + 2, c.center().y));
    t.harness_mut().move_to(Point::new(c.x1 - 2, c.center().y));
    t.release();
    t.run_until_idle();
    let id = t.find(by_id("l")).id();
    let sel = t.engine().widget::<Label>(id).unwrap().selection();
    let (a, b) = sel.expect("a selection");
    assert!(b > a + 5, "{sel:?}");
    let _ = EventCode::Pressing;
}
