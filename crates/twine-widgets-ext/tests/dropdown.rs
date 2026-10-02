//! `Dropdown`: LVGL's defaults, the option list on the top layer (open, close, positioning
//! and flipping), pointer / keypad / encoder control, events, idempotent setters, the optional
//! fade animation, cleanup and the theme's look.

mod common;

use std::rc::Rc;

use common::{Mode, center, class, count_events, get, harness, harness_with_group, has_state, values, with};
use twine_core::{Color, Duration, Point, Size};
use twine_engine::{EventCode, Key, NodeId, State, ThemeCx, ThemeHook, WidgetClass};
use twine_image::ImageSource;
use twine_style::{Align, Part, PropId, Selector, Side, StyleBuf};
use twine_testing::{EngineHarness, capture_logs};
use twine_text::Font;
use twine_theme::{DefaultTheme, Palette};
use twine_widgets_ext::dropdown::{self, DROPDOWN_CLASS, DROPDOWN_LIST_CLASS, Dropdown};

const CITIES: &str = "Berlin\nLondon\nMadrid\nParis\nRome\nVienna";

/// A 320 × 240 screen with a dropdown of [`CITIES`] aligned by `align`.
fn scene(mode: Mode, align: Align) -> (EngineHarness, NodeId) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let e = h.engine_mut();
    // A second focusable node, so that the encoder can leave edit mode.
    let other = twine_widgets::button::create(e, screen).unwrap();
    e.set_size(other, 10, 10);
    e.align(other, Align::BottomRight, 0, 0);
    let d = dropdown::create(e, screen).unwrap();
    e.align(d, align, 0, if align == Align::TopMid { 10 } else { 0 });
    with(&mut h, d, |w: &mut Dropdown, cx| w.set_options_static(cx, CITIES));
    h.run_until_idle();
    (h, d)
}

fn list_of(h: &EngineHarness, d: NodeId) -> Option<NodeId> {
    get::<Dropdown>(h, d)
        .list()
        .filter(|l| h.engine().tree().contains(*l))
}

fn selected(h: &EngineHarness, d: NodeId) -> u16 {
    get::<Dropdown>(h, d).selected()
}

fn shown(h: &EngineHarness, d: NodeId) -> String {
    let mut s = String::new();
    get::<Dropdown>(h, d).selected_str(&mut s);
    s
}

/// The center of option `i` in the open list (the list's `Main` font and line space).
fn option_point(h: &EngineHarness, list: NodeId, i: i32) -> Point {
    let e = h.engine();
    let t = twine_engine::MeasureCx::new(e, list).text_dsc(Part::Main);
    let unit = i32::from(t.font.line_height) + t.line_space;
    let c = e.content_area(list);
    let y = c.y0 - e.scroll_offset(list).y + i * unit + i32::from(t.font.line_height) / 2;
    Point::new(c.x0 + c.width() / 2, y)
}

#[test]
fn dd_defaults() {
    let mut h = harness(320, 240, Mode::Light);
    let screen = h.screen();
    let d = dropdown::create(h.engine_mut(), screen).unwrap();
    h.run_until_idle();
    assert_eq!(class(&h, d), "dropdown");
    assert_eq!(DROPDOWN_CLASS.editable, twine_engine::Editable::True);
    assert_eq!(DROPDOWN_CLASS.group_def, twine_engine::GroupDef::True);
    assert!(
        !DROPDOWN_LIST_CLASS
            .default_flags
            .contains(twine_engine::ObjFlags::CLICK_FOCUSABLE)
    );
    let w = get::<Dropdown>(&h, d);
    assert_eq!(w.options(), dropdown::DROPDOWN_DEFAULT_OPTIONS);
    assert_eq!((w.option_count(), w.selected()), (3, 0));
    assert_eq!(w.dir(), Side::Bottom);
    assert_eq!(w.symbol(), Some(&dropdown::DROPDOWN_DEFAULT_SYMBOL));
    assert!(w.selected_highlight());
    assert_eq!(w.text(), None);
    assert!(!w.is_open(h.engine()));
    // LVGL: `LV_DPI_DEF` wide, content high (one line + the theme's small padding + border).
    let c = h.engine().coords(d);
    assert_eq!(c.width(), 130);
    let e = h.engine();
    let pad =
        e.style_i32(d, Part::Main, PropId::PaddingTop) + e.style_i32(d, Part::Main, PropId::PaddingBottom);
    let border = 2 * e.style_i32(d, Part::Main, PropId::BorderWidth);
    let line = i32::from(e.style_font(d, Part::Main).line_height);
    assert_eq!(c.height(), line + pad + border);
    assert_eq!(shown(&h, d), "Option 1");
    h.assert_idle();
}

#[test]
fn dd_theme_styles() {
    let (mut h, d) = scene(Mode::Light, Align::Center);
    let e = h.engine();
    // `card` + `pad_small`: white, grey border, 10 px (dpx) padding at 130 dpi.
    assert_eq!(e.style_color(d, Part::Main, PropId::BgColor), Color::WHITE);
    assert_eq!(
        e.style_i32(d, Part::Main, PropId::PaddingLeft),
        twine_style::dpx(10, 130)
    );
    with(&mut h, d, |w: &mut Dropdown, cx| w.open(cx));
    let l = list_of(&h, d).unwrap();
    let e = h.engine();
    assert_eq!(
        e.style_i32(l, Part::Main, PropId::LineSpacing),
        twine_style::dpx(20, 130)
    );
    assert_eq!(
        e.style_i32(l, Part::Main, PropId::MaxHeight),
        260,
        "LV_DPI_DEF * 2"
    );
    assert_eq!(
        e.style_i32(l, Part::Main, PropId::AnimDuration),
        0,
        "no animation (LVGL)"
    );
    let checked = e.rect_dsc_for_state(l, Part::Selected, State::CHECKED, twine_core::Opa::COVER);
    assert_eq!(checked.base.bg_color, Palette::Blue.main());
    let normal = e.rect_dsc_for_state(l, Part::Selected, State::DEFAULT, twine_core::Opa::COVER);
    assert_eq!(normal.base.bg_color, Color::WHITE);
}

#[test]
fn dd_open_creates_list_on_top_layer() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let ready = count_events(&mut h, d, EventCode::Ready);
    let nodes = h.engine().tree().len();
    h.tap(center(&h, d));
    let l = list_of(&h, d).expect("open");
    let top = h.engine().top_layer(h.display()).unwrap();
    assert_eq!(h.engine().tree().parent(l), Some(top));
    assert_eq!(class(&h, l), "dropdown_list");
    assert_eq!(h.engine().tree().len(), nodes + 1);
    assert!(has_state(&h, d, State::CHECKED), "the dropdown looks open");
    assert_eq!(ready.get(), 1);
    // Below the dropdown, as wide as it, as tall as the options up to the screen's bottom
    // edge (LVGL: `LV_VER_RES - y2 - 1`).
    h.run_until_idle();
    let (dc, lc) = (h.engine().coords(d), h.engine().coords(l));
    assert_eq!(lc.y0, dc.y1, "{dc:?} {lc:?}");
    assert_eq!((lc.x0, lc.width()), (dc.x0, dc.width()));
    let t = twine_engine::MeasureCx::new(h.engine(), l).text_dsc(Part::Main);
    let text_h = 6 * i32::from(t.font.line_height) + 5 * t.line_space;
    let e = h.engine();
    let space = e.style_i32(l, Part::Main, PropId::PaddingTop)
        + e.style_i32(l, Part::Main, PropId::PaddingBottom)
        + 2 * e.style_i32(l, Part::Main, PropId::BorderWidth);
    assert_eq!(lc.height(), (text_h + space).min(260).min(240 - dc.y1));
    // The options scroll inside it.
    assert!(h.engine().scroll_bottom(l) > 0);
    h.assert_idle();
}

#[test]
fn dd_close_deletes_list() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let cancel = count_events(&mut h, d, EventCode::Cancel);
    let nodes = h.engine().tree().len();
    h.tap(center(&h, d));
    let l = list_of(&h, d).unwrap();
    // A second click on the dropdown closes it.
    h.tap(center(&h, d));
    assert!(!h.engine().tree().contains(l));
    assert_eq!(get::<Dropdown>(&h, d).list(), None);
    assert!(!has_state(&h, d, State::CHECKED));
    assert_eq!(h.engine().tree().len(), nodes);
    assert_eq!(cancel.get(), 1);
    assert_eq!(
        h.engine().outside_press_count(),
        0,
        "the watch went with the list"
    );
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_list_positions_below_or_flips_up() {
    // Near the bottom: more room above, so the list opens upwards.
    let (mut h, d) = scene(Mode::Light, Align::BottomMid);
    with(&mut h, d, |w: &mut Dropdown, cx| w.open(cx));
    h.run_until_idle();
    let (dc, lc) = (h.engine().coords(d), h.engine().coords(list_of(&h, d).unwrap()));
    assert_eq!(lc.y1, dc.y0, "above the dropdown");
    with(&mut h, d, |w: &mut Dropdown, cx| w.close(cx));
    // Asking for the top near the top of the screen flips it down.
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_dir(cx, Side::Top);
        w.open(cx);
    });
    h.run_until_idle();
    let (dc, lc) = (h.engine().coords(d), h.engine().coords(list_of(&h, d).unwrap()));
    assert_eq!(lc.y0, dc.y1, "below");
    // Left and right: beside it, top-aligned, as wide as the options.
    for (dir, left) in [(Side::Left, true), (Side::Right, false)] {
        let (mut h, d) = scene(Mode::Light, Align::Center);
        with(&mut h, d, |w: &mut Dropdown, cx| {
            w.set_dir(cx, dir);
            w.open(cx);
        });
        h.run_until_idle();
        let (dc, lc) = (h.engine().coords(d), h.engine().coords(list_of(&h, d).unwrap()));
        if left {
            assert_eq!(lc.x1, dc.x0);
        } else {
            assert_eq!(lc.x0, dc.x1);
        }
        assert!(lc.y1 <= 240, "kept on the screen: {lc:?}");
    }
    // The list never exceeds the space to the screen edge: a long list below a low dropdown.
    let (mut h, d) = scene(Mode::Light, Align::Center);
    let many: String = (0..40)
        .map(|i| format!("Item {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_options(cx, &many);
        w.open(cx);
    });
    h.run_until_idle();
    let lc = h.engine().coords(list_of(&h, d).unwrap());
    assert!(lc.height() <= 260, "max height: {lc:?}");
    assert!(lc.y1 <= 240 && lc.y0 >= 0, "{lc:?}");
}

#[test]
fn dd_click_option_selects_and_closes() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let changes = values(&mut h, d);
    h.tap(center(&h, d));
    h.run_until_idle();
    let l = list_of(&h, d).unwrap();
    let p = option_point(&h, l, 3);
    // Pressing highlights the option.
    h.press(p);
    assert_eq!(get::<Dropdown>(&h, d).pressed_option(), Some(3));
    h.release();
    assert!(!h.engine().tree().contains(l), "closed");
    assert_eq!(selected(&h, d), 3);
    assert_eq!(shown(&h, d), "Paris");
    assert_eq!(*changes.borrow(), vec![3]);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_click_outside_closes_without_change() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let changes = values(&mut h, d);
    with(&mut h, d, |w: &mut Dropdown, cx| w.set_selected(cx, 1));
    h.tap(center(&h, d));
    let l = list_of(&h, d).unwrap();
    h.tap(Point::new(5, 235));
    assert!(!h.engine().tree().contains(l));
    assert_eq!(selected(&h, d), 1);
    assert!(changes.borrow().is_empty());
    assert_eq!(h.engine().outside_press_count(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_keypad_esc_restores() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let changes = values(&mut h, d);
    let _ = h.keypad_input();
    h.engine_mut().focus(d);
    h.key(Key::Down); // opens
    assert!(list_of(&h, d).is_some());
    h.key(Key::Down);
    h.key(Key::Down);
    assert_eq!(selected(&h, d), 2, "the highlight moves");
    assert_eq!(shown(&h, d), "Berlin", "the button keeps the confirmed option");
    h.key(Key::Esc);
    assert!(list_of(&h, d).is_none());
    assert_eq!(selected(&h, d), 0, "restored");
    assert!(changes.borrow().is_empty());
    // Enter confirms.
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Enter);
    assert!(list_of(&h, d).is_none());
    assert_eq!(selected(&h, d), 1);
    assert_eq!(*changes.borrow(), vec![1]);
    // Up stops at the first option.
    h.key(Key::Up);
    h.key(Key::Up);
    h.key(Key::Up);
    assert_eq!(selected(&h, d), 0);
    h.key(Key::Esc);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_encoder_flow() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let changes = values(&mut h, d);
    let g = h.engine().group_of(d).unwrap();
    let _ = h.encoder_input();
    h.engine_mut().focus(d);
    // A click enters edit mode, which opens the list.
    h.encoder_click();
    assert!(h.engine().group_editing(g));
    assert!(list_of(&h, d).is_some());
    h.encoder(2);
    assert_eq!(selected(&h, d), 2);
    // A click confirms, closes and leaves edit mode.
    h.encoder_click();
    assert!(list_of(&h, d).is_none());
    assert!(!h.engine().group_editing(g));
    assert_eq!(*changes.borrow(), vec![2]);
    assert_eq!(shown(&h, d), "Madrid");
    // Turning in navigate mode moves the focus away; the list stays closed.
    h.encoder(1);
    assert!(list_of(&h, d).is_none());
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_list_inherits_text_style_of_dropdown_ancestors() {
    use twine_assets::fonts::{MONTSERRAT_14, MONTSERRAT_20};
    use twine_style::{BaseDir, StyleProp};
    let mut h = harness(320, 240, Mode::Light);
    let screen = h.screen();
    let e = h.engine_mut();
    let cont = e.create(screen, Box::new(twine_engine::Obj)).unwrap();
    e.set_size(cont, 320, 240);
    e.set_local_prop(cont, Selector::MAIN, StyleProp::Font((&MONTSERRAT_20).into()));
    e.set_local_prop(cont, Selector::MAIN, StyleProp::BaseDir(BaseDir::Rtl));
    let d = dropdown::create(e, cont).unwrap();
    with(&mut h, d, |w: &mut Dropdown, cx| w.set_options_static(cx, CITIES));
    h.run_until_idle();
    h.tap(center(&h, d));
    let l = list_of(&h, d).expect("open");
    let top = h.engine().top_layer(h.display()).unwrap();
    assert_eq!(h.engine().tree().parent(l), Some(top), "still on the top layer");
    assert_eq!(h.engine().tree().style_parent(l), Some(d));
    let font = h.engine().style_font(l, Part::Main);
    assert!(
        core::ptr::eq(font, &raw const MONTSERRAT_20),
        "the list uses the container's font"
    );
    assert_eq!(
        h.engine()
            .style_prop(l, Part::Main, PropId::BaseDir)
            .get::<BaseDir>(),
        Some(BaseDir::Rtl)
    );
    // A change while open reaches the list.
    h.run_until_idle();
    h.engine_mut()
        .set_local_prop(cont, Selector::MAIN, StyleProp::Font((&MONTSERRAT_14).into()));
    assert!(core::ptr::eq(
        h.engine().style_font(l, Part::Main),
        &raw const MONTSERRAT_14
    ));
    h.run_until_idle();
    h.engine().tree().check_invariants().unwrap();
}

#[test]
fn dd_rotary_opens_and_moves() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    h.engine_mut()
        .send_event(d, EventCode::Rotary, twine_engine::EventParam::Rotary(1));
    assert!(list_of(&h, d).is_some());
    h.engine_mut()
        .send_event(d, EventCode::Rotary, twine_engine::EventParam::Rotary(10));
    assert_eq!(selected(&h, d), 5, "clamped to the last option");
}

#[test]
fn dd_delete_while_open_cleans_up() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let nodes = h.engine().tree().len();
    h.tap(center(&h, d));
    let l = list_of(&h, d).unwrap();
    h.engine_mut().delete(d).unwrap();
    assert!(!h.engine().tree().contains(l));
    assert_eq!(h.engine().tree().len(), nodes - 1);
    assert_eq!(h.engine().outside_press_count(), 0);
    h.engine().tree().check_invariants().unwrap();
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_options_same_no_invalidate() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_options(cx, "a\nb\nc");
        w.set_selected(cx, 2);
    });
    h.run_until_idle();
    // The same values again: nothing to redraw, the selection kept.
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_options(cx, "a\nb\nc");
        w.set_selected(cx, 2);
        w.set_dir(cx, Side::Bottom);
        w.set_symbol(cx, Some(dropdown::DROPDOWN_DEFAULT_SYMBOL));
        w.set_text(cx, None);
        w.set_selected_highlight(cx, true);
    });
    assert_eq!(selected(&h, d), 2);
    h.assert_idle();
    let s: &'static str = "x\ny";
    with(&mut h, d, |w: &mut Dropdown, cx| w.set_options_static(cx, s));
    h.run_until_idle();
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_options_static(cx, s);
        w.set_text_static(cx, None);
    });
    h.assert_idle();
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_text(cx, Some("Pick"));
    });
    h.run_until_idle();
    with(&mut h, d, |w: &mut Dropdown, cx| w.set_text(cx, Some("Pick")));
    h.assert_idle();
}

#[test]
fn dd_option_editing() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_selected(cx, 1);
        w.add_option(cx, "Amsterdam", 0);
        w.add_option(cx, "Zurich", u32::MAX);
    });
    let w = get::<Dropdown>(&h, d);
    assert_eq!(w.option_count(), 8);
    assert_eq!(w.option_index("Zurich"), Some(7));
    assert_eq!(w.option_index("Nowhere"), None);
    assert_eq!(w.selected(), 1, "the index is kept");
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_selected(cx, 200);
    });
    assert_eq!(selected(&h, d), 7, "clamped");
    with(&mut h, d, |w: &mut Dropdown, cx| w.clear_options(cx));
    assert_eq!(get::<Dropdown>(&h, d).option_count(), 0);
    assert_eq!(shown(&h, d), "");
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.add_option(cx, "bad\noption", 0);
    });
    assert_eq!(get::<Dropdown>(&h, d).option_count(), 0, "rejected");
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_static_text_and_symbol() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_text_static(cx, Some("Menu"));
        w.set_symbol(cx, None);
    });
    assert_eq!(get::<Dropdown>(&h, d).text(), Some("Menu"));
    assert_eq!(get::<Dropdown>(&h, d).symbol(), None);
    // Selecting an option does not change the fixed text.
    h.tap(center(&h, d));
    h.run_until_idle();
    let l = list_of(&h, d).unwrap();
    h.tap(option_point(&h, l, 4));
    assert_eq!(selected(&h, d), 4);
    assert_eq!(get::<Dropdown>(&h, d).text(), Some("Menu"));
    h.run_until_idle();
}

#[test]
fn dd_open_close_100_times_leaves_no_nodes() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    h.tap(center(&h, d));
    h.tap(center(&h, d));
    h.run_until_idle();
    let nodes = h.engine().tree().len();
    for i in 0..100 {
        h.tap(center(&h, d));
        assert!(list_of(&h, d).is_some(), "open #{i}");
        h.tap(center(&h, d));
        assert!(list_of(&h, d).is_none(), "closed #{i}");
    }
    assert_eq!(h.engine().tree().len(), nodes);
    assert_eq!(h.engine().outside_press_count(), 0);
    h.run_until_idle();
    h.assert_idle();
}

#[test]
fn dd_setters_log() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let ((), logs) = capture_logs(|| {
        with(&mut h, d, |w: &mut Dropdown, cx| w.set_options(cx, "p\nq"));
    });
    assert!(
        logs.iter().any(|l| l.target == "twine::engine"
            && l.message.contains("dropdown#")
            && l.message.contains("set_options")),
        "{logs:?}"
    );
}

/// The default theme plus a fade on the dropdown list.
struct FadingTheme(DefaultTheme, Rc<StyleBuf>);

impl ThemeHook for FadingTheme {
    fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass) {
        self.0.apply(cx, class);
        if class.is(&twine_widgets_ext::dropdown::DROPDOWN_LIST_CLASS) {
            cx.add_style(Selector::MAIN, self.1.clone());
        }
    }
    fn font_normal(&self) -> &'static Font {
        self.0.font_normal()
    }
    fn name(&self) -> &'static str {
        "fading"
    }
    // A wrapping theme forwards the modes and the design elements of the theme it extends.
    fn mode(&self) -> twine_style::ThemeMode {
        ThemeHook::mode(&self.0)
    }
    fn modes(&self) -> &'static [twine_style::ThemeMode] {
        self.0.modes()
    }
    fn design(
        &self,
        mode: twine_style::ThemeMode,
        dpi: u16,
        resolution: twine_core::Size,
    ) -> Option<Rc<twine_style::design::ElementTable>> {
        self.0.design(mode, dpi, resolution)
    }
}

#[test]
fn dd_open_close_animations() {
    // Without an anim duration (the themes): opened and closed at once.
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let nodes = h.engine().tree().len();
    with(&mut h, d, |w: &mut Dropdown, cx| w.open(cx));
    assert_eq!(h.engine().anim_count(), 0);
    with(&mut h, d, |w: &mut Dropdown, cx| w.close(cx));
    assert_eq!(h.engine().tree().len(), nodes);
    // With 200 ms: fade in, fade out, then deleted.
    let theme = FadingTheme(
        DefaultTheme::light(),
        Rc::new(StyleBuf::new().anim_duration(twine_core::Duration::ms(200))),
    );
    let mut h = EngineHarness::new(320, 240).theme(theme);
    let screen = h.screen();
    let d = dropdown::create(h.engine_mut(), screen).unwrap();
    h.engine_mut().align(d, Align::TopMid, 0, 10);
    h.run_until_idle();
    let nodes = h.engine().tree().len();
    with(&mut h, d, |w: &mut Dropdown, cx| w.open(cx));
    let l = list_of(&h, d).unwrap();
    h.advance(Duration::ms(100));
    let opa = h.engine().style_opa(l, Part::Main, PropId::PartOpacity).raw();
    assert!(opa > 0 && opa < 255, "fading in: {opa}");
    h.run_until_idle();
    assert_eq!(
        h.engine().style_opa(l, Part::Main, PropId::PartOpacity).raw(),
        255
    );
    with(&mut h, d, |w: &mut Dropdown, cx| w.close(cx));
    assert!(
        !get::<Dropdown>(&h, d).is_open(h.engine()),
        "closed at once for the widget"
    );
    assert!(h.engine().tree().contains(l), "still fading out");
    h.advance(Duration::ms(100));
    assert!(h.engine().tree().contains(l));
    h.run_until_idle();
    assert!(!h.engine().tree().contains(l), "deleted after the fade");
    assert_eq!(h.engine().tree().len(), nodes);
    // Reopening during a fade-out drops the old list at once.
    with(&mut h, d, |w: &mut Dropdown, cx| w.open(cx));
    h.run_until_idle();
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.close(cx);
        w.open(cx);
    });
    h.run_until_idle();
    assert_eq!(h.engine().tree().len(), nodes + 1);
    h.assert_idle();
}

#[test]
fn dd_image_symbol_size() {
    let (mut h, d) = scene(Mode::Light, Align::TopMid);
    let before = h.engine().coords(d).height();
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_symbol(cx, Some(ImageSource::symbol(twine_text::Symbol::Ok)));
    });
    h.run_until_idle();
    assert_eq!(h.engine().coords(d).height(), before);
    let _ = Size::ZERO;
}

#[test]
fn snapshot_dropdown() {
    for m in Mode::ALL {
        let (mut h, d) = scene(m, Align::TopMid);
        with(&mut h, d, |w: &mut Dropdown, cx| w.set_selected(cx, 2));
        h.run_until_idle();
        h.assert_snapshot(&format!("dd_closed_{}", m.suffix()));
        h.tap(center(&h, d));
        h.run_until_idle();
        h.assert_snapshot(&format!("dd_open_{}", m.suffix()));
    }
    let (mut h, d) = scene(Mode::Light, Align::BottomMid);
    with(&mut h, d, |w: &mut Dropdown, cx| {
        w.set_dir(cx, Side::Top);
        w.open(cx);
    });
    h.run_until_idle();
    h.assert_snapshot("dd_open_up");
}
