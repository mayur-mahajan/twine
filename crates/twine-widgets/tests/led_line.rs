//! `Led` and `Line`: LVGL defaults, brightness color mixing, toggling, point storage, content
//! size, y inversion, bounding-box invalidation, and snapshots.

mod common;

use common::{Mode, get, harness, with};
use twine_core::{Color, Duration, Fraction, Opa, Point, Rect, Size};
use twine_engine::ObjFlags;
use twine_style::{Align, Part, PropId, Selector, StyleProp};
use twine_testing::EngineHarness;
use twine_widgets::led::{self, LED_BRIGHT_MAX, LED_BRIGHT_MIN, LED_CLASS, LED_DEFAULT_COLOR, Led};
use twine_widgets::line::{self, LINE_CLASS, Line, LinePoints};

#[test]
fn led_defaults() {
    let mut h = harness(60, 60, Mode::Light);
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let n = h.engine().tree().node(l).unwrap();
    assert_eq!(n.class().name, "led");
    assert_eq!(LED_CLASS.parts, &[Part::Main]);
    // LVGL `width_def = height_def = LV_DPI_DEF / 5`.
    assert_eq!(h.engine().coords(l).size(), Size::new(26, 26));
    let w = get::<Led>(&h, l);
    assert_eq!((w.color(), w.brightness()), (LED_DEFAULT_COLOR, LED_BRIGHT_MAX));
    assert!(w.is_on());
    // The theme's `led` style: white, circle, glow.
    let e = h.engine();
    assert_eq!(e.style_color(l, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(e.style_i32(l, Part::Main, PropId::ShadowWidth), 12);
    h.assert_idle();
}

#[test]
fn led_setters_same_value_no_invalidate() {
    let mut h = harness(60, 60, Mode::Light);
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    with(&mut h, l, |w: &mut Led, cx| {
        w.set_color(cx, LED_DEFAULT_COLOR);
        w.set_brightness(cx, Fraction::from_raw(255));
        w.on(cx);
    });
    assert!(h.engine().invalidation_log().is_empty());
    with(&mut h, l, |w: &mut Led, cx| {
        w.set_brightness(cx, Fraction::from_raw(10));
    }); // clamps to 80
    h.run_until_idle();
    with(&mut h, l, |w: &mut Led, cx| {
        w.set_brightness(cx, Fraction::from_raw(0));
    });
    assert!(h.engine().invalidation_log().is_empty());
    h.assert_idle();
}

#[test]
fn led_brightness_mixes_color() {
    let mut h = EngineHarness::new(40, 40).no_theme();
    let screen = h.screen();
    let e = h.engine_mut();
    let l = led::create(e, screen).unwrap();
    e.align(l, Align::Center, 0, 0);
    // A plain white square: the drawn color is the LED color darkened by the brightness.
    e.set_local_prop(l, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    e.set_local_prop(l, Selector::MAIN, StyleProp::BgColor(Color::WHITE.into()));
    let red = Color::new(255, 0, 0);
    with(&mut h, l, |w: &mut Led, cx| {
        w.set_color(cx, red);
        w.set_brightness(cx, Fraction::from_raw(160));
    });
    h.run_until_idle();
    let px = h.pixel(20, 20);
    let expected = Color::mix(red, Color::BLACK, Opa::from_raw(160));
    assert!(
        (i32::from(px.r) - i32::from(expected.r)).abs() <= 8,
        "{px:?} vs {expected:?}"
    );
    assert!(px.g < 8 && px.b < 8);
}

#[test]
fn led_toggle() {
    let mut h = harness(60, 60, Mode::Light);
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    with(&mut h, l, |w: &mut Led, cx| w.toggle(cx));
    assert_eq!(get::<Led>(&h, l).brightness(), LED_BRIGHT_MIN);
    with(&mut h, l, |w: &mut Led, cx| w.toggle(cx));
    assert_eq!(get::<Led>(&h, l).brightness(), LED_BRIGHT_MAX);
    // LVGL: halfway (167) counts as off, above it as on.
    with(&mut h, l, |w: &mut Led, cx| {
        w.set_brightness(cx, Fraction::from_raw(167));
        w.toggle(cx);
    });
    assert_eq!(get::<Led>(&h, l).brightness(), LED_BRIGHT_MAX);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn led_off_uses_min_brightness() {
    let mut h = harness(60, 60, Mode::Light);
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    with(&mut h, l, |w: &mut Led, cx| w.off(cx));
    let w = get::<Led>(&h, l);
    assert_eq!(w.brightness(), LED_BRIGHT_MIN);
    assert!(!w.is_on());
}

#[test]
fn snapshot_led_on_off() {
    for m in Mode::ALL {
        let mut h = harness(120, 60, m);
        let screen = h.screen();
        let e = h.engine_mut();
        let on = led::create(e, screen).unwrap();
        let off = led::create(e, screen).unwrap();
        let red = led::create(e, screen).unwrap();
        e.align(on, Align::LeftMid, 12, 0);
        e.align(off, Align::Center, 0, 0);
        e.align(red, Align::RightMid, -12, 0);
        with(&mut h, off, |w: &mut Led, cx| w.off(cx));
        with(&mut h, red, |w: &mut Led, cx| {
            w.set_color(cx, Color::new(0xF4, 0x43, 0x36));
            w.set_brightness(cx, Fraction::from_raw(200));
        });
        h.run_until_idle();
        h.assert_snapshot(&format!("led_on_off_{}", m.suffix()));
    }
}

static ZIGZAG: [Point; 5] = [
    Point::new(0, 30),
    Point::new(25, 0),
    Point::new(50, 30),
    Point::new(75, 0),
    Point::new(100, 30),
];

fn line_scene(mode: Mode) -> (EngineHarness, twine_engine::NodeId) {
    let mut h = harness(140, 60, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    let l = line::create(e, screen).unwrap();
    e.align(l, Align::Center, 0, 0);
    with(&mut h, l, |w: &mut Line, cx| w.set_points_static(cx, &ZIGZAG));
    h.run_until_idle();
    (h, l)
}

#[test]
fn line_defaults() {
    let (mut h, l) = line_scene(Mode::Light);
    let n = h.engine().tree().node(l).unwrap();
    assert_eq!(n.class().name, "line");
    assert_eq!(LINE_CLASS.parts, &[Part::Main]);
    assert!(
        !n.flags().contains(ObjFlags::CLICKABLE),
        "LVGL: lines are not clickable"
    );
    // The theme: 1 px in the text color.
    assert_eq!(h.engine().style_i32(l, Part::Main, PropId::LineWidth), 1);
    assert!(matches!(
        get::<Line>(&h, l).points_storage(),
        Some(LinePoints::Static(_))
    ));
    assert!(!get::<Line>(&h, l).y_invert());
    h.assert_idle();
}

#[test]
fn line_content_size() {
    let (mut h, l) = line_scene(Mode::Light);
    // LVGL `GET_SELF_SIZE`: the largest x and y.
    assert_eq!(h.engine().coords(l).size(), Size::new(100, 30));
    with(&mut h, l, |w: &mut Line, cx| {
        w.set_points(cx, &[Point::new(5, 5), Point::new(60, 44)]);
    });
    h.run_until_idle();
    assert_eq!(h.engine().coords(l).size(), Size::new(60, 44));
    // The extra draw size is the line width.
    h.engine_mut()
        .set_local_prop(l, Selector::MAIN, StyleProp::LineWidth(6));
    h.run_until_idle();
    assert_eq!(h.engine().tree().node(l).unwrap().ext_draw(), 6);
}

#[test]
fn line_same_points_no_invalidate() {
    let (mut h, l) = line_scene(Mode::Light);
    with(&mut h, l, |w: &mut Line, cx| {
        w.set_points_static(cx, &ZIGZAG);
        w.set_points(cx, &ZIGZAG); // equal content: nothing
        w.set_y_invert(cx, false);
    });
    assert!(h.engine().invalidation_log().is_empty());
    // Owned storage reuses its capacity.
    with(&mut h, l, |w: &mut Line, cx| w.set_points(cx, &ZIGZAG[..4]));
    with(&mut h, l, |w: &mut Line, cx| w.set_points(cx, &ZIGZAG[..3]));
    assert!(matches!(get::<Line>(&h, l).points_storage(), Some(LinePoints::Owned(v)) if v.capacity() >= 4));
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn line_invalidates_old_and_new_bbox() {
    let (mut h, l) = line_scene(Mode::Light);
    let c = h.engine().coords(l);
    // Move the middle point: the bounding boxes (and the content size) stay the same.
    let mut moved = ZIGZAG;
    moved[2] = Point::new(50, 20);
    with(&mut h, l, |w: &mut Line, cx| w.set_points(cx, &moved));
    let log = h.engine().invalidation_log().to_vec();
    // Done-when: the union of the old and the new bounding box plus half the line width.
    let bbox = Rect::new(c.x0, c.y0, c.x0 + 101, c.y0 + 31).expand(1);
    assert!(!log.is_empty());
    for (r, _) in &log {
        assert!(bbox.contains_rect(r), "{r} outside {bbox}");
    }
    let dirty = h.engine().tree().node(l).unwrap().layout_dirty();
    assert_eq!(
        dirty,
        twine_engine::LayoutDirty::empty(),
        "same content size: no layout"
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn line_y_invert() {
    let (mut h, l) = line_scene(Mode::Light);
    h.engine_mut()
        .set_local_prop(l, Selector::MAIN, StyleProp::LineWidth(2));
    with(&mut h, l, |w: &mut Line, cx| w.set_y_invert(cx, true));
    h.run_until_idle();
    let c = h.engine().coords(l);
    let text = twine_theme::default::colors::LIGHT_TEXT;
    // The first point (0, 30) is now at the top-left, (25, 0) at the bottom.
    let dark = |p: Color| p.r < 128 && p.r.abs_diff(text.r) < 60;
    assert!(dark(h.pixel((c.x0 + 1) as u32, (c.y0 + 1) as u32)));
    assert!(dark(h.pixel((c.x0 + 25) as u32, (c.y1 - 1) as u32)));
}

#[test]
fn snapshot_line() {
    for m in Mode::ALL {
        let (mut h, l) = line_scene(m);
        let e = h.engine_mut();
        e.set_local_prop(l, Selector::MAIN, StyleProp::LineWidth(3));
        e.set_local_prop(l, Selector::MAIN, StyleProp::LineDashWidth(6));
        e.set_local_prop(l, Selector::MAIN, StyleProp::LineDashGap(4));
        h.run_until_idle();
        h.assert_snapshot(&format!("line_polyline_dashed_{}", m.suffix()));
        let (mut h, l) = line_scene(m);
        let e = h.engine_mut();
        e.set_local_prop(l, Selector::MAIN, StyleProp::LineWidth(10));
        e.set_local_prop(l, Selector::MAIN, StyleProp::LineRounded(true));
        e.set_local_prop(
            l,
            Selector::MAIN,
            StyleProp::LineColor(Color::new(0x21, 0x96, 0xF3).into()),
        );
        h.run_until_idle();
        h.advance(Duration::ms(1));
        h.assert_snapshot(&format!("line_rounded_thick_{}", m.suffix()));
    }
}

#[test]
fn led_takes_theme_primary_color() {
    use twine_style::design;
    use twine_theme::{DefaultTheme, Palette};
    let theme = DefaultTheme::builder()
        .primary(Palette::Teal)
        .secondary(Palette::Amber)
        .build();
    let mut h = EngineHarness::new(60, 60).theme(theme);
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    assert_eq!(get::<Led>(&h, l).color(), Palette::Teal.main());
    assert_eq!(get::<Led>(&h, l).color_value(), design::PRIMARY.into());
    // Without a theme: LVGL's default blue.
    let mut h = EngineHarness::new(60, 60).no_theme();
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    assert_eq!(get::<Led>(&h, l).color(), LED_DEFAULT_COLOR);
    assert_eq!(LED_DEFAULT_COLOR, Palette::Blue.main());
}

/// The LED's color is a design element resolved on creation and on every theme mode switch
/// (not captured once): it follows Light, Dark, Night and High contrast.
#[test]
fn led_color_follows_every_theme_mode() {
    use std::rc::Rc;
    use twine_core::Size;
    use twine_engine::ThemeHook;
    use twine_style::{ThemeMode, design};
    use twine_theme::DefaultTheme;
    let theme = Rc::new(DefaultTheme::light());
    let mut h = EngineHarness::new(60, 60).theme(theme.clone());
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    let alarm = led::create(h.engine_mut(), screen).unwrap();
    let fixed = led::create(h.engine_mut(), screen).unwrap();
    h.engine_mut()
        .with_widget_mut(alarm, |w: &mut Led, cx| w.set_color(cx, design::DANGER));
    h.engine_mut()
        .with_widget_mut(fixed, |w: &mut Led, cx| w.set_color(cx, Color::GREEN));
    h.run_until_idle();
    let d = h.engine().default_display().unwrap();
    let mut seen = Vec::new();
    for mode in [
        ThemeMode::Dark,
        ThemeMode::Night,
        ThemeMode::HighContrast,
        ThemeMode::Light,
    ] {
        h.engine_mut().set_theme_mode(d, mode);
        h.run_until_idle();
        let table = theme.design(mode, 130, Size::new(60, 60)).unwrap();
        assert_eq!(
            get::<Led>(&h, l).color(),
            table.get(design::PRIMARY).unwrap(),
            "{mode:?}"
        );
        assert_eq!(
            get::<Led>(&h, alarm).color(),
            table.get(design::DANGER).unwrap(),
            "{mode:?}"
        );
        assert_eq!(get::<Led>(&h, fixed).color(), Color::GREEN, "{mode:?}");
        seen.push(get::<Led>(&h, l).color());
    }
    // Night and high contrast have their own accents.
    assert_ne!(seen[1], seen[3]);
    assert_ne!(seen[2], seen[3]);
}

/// The LED's pixels (not only its resolved color) follow a theme mode switch in the update
/// that applies it: the partial redraw after the switch shows what a full redraw shows, and
/// the pixels change with the mode's primary color. Guards against stale pixels when a color
/// resolved outside the style system is re-resolved (`set_theme_mode` invalidates the whole
/// display, then sends `StyleChanged`, before the next render).
#[test]
fn regression_led_pixels_follow_theme_mode_in_one_update() {
    use twine_style::ThemeMode;
    use twine_theme::DefaultTheme;
    let mut h = EngineHarness::new(60, 60).theme(DefaultTheme::light());
    let screen = h.screen();
    let l = led::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    let c = h.engine().coords(l).center();
    let (x, y) = (c.x as u32, c.y as u32);
    let d = h.engine().default_display().unwrap();
    let mut seen = vec![h.pixel(x, y)];
    for mode in [ThemeMode::HighContrast, ThemeMode::Night, ThemeMode::Light] {
        h.engine_mut().set_theme_mode(d, mode);
        h.run_until_idle();
        let partial = h.pixel(x, y);
        h.render_full();
        assert_eq!(partial, h.pixel(x, y), "{mode:?}: stale LED pixels");
        assert_ne!(Some(&partial), seen.last(), "{mode:?}: LED did not change");
        seen.push(partial);
    }
    assert_eq!(seen[0], seen[3], "back to light");
}
