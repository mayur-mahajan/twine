//! `Window`: the header (title with dots, buttons) and the scrolling content, the header
//! height from the DPI, and the theme's look.

mod common;

use std::rc::Rc;

use common::{Mode, center, class, count_events, get, harness_with_group, with};
use twine_core::{ColorFormat, Duration};
use twine_engine::{BufferMode, EventCode, NodeId};
use twine_hal::DisplayInfo;
use twine_image::ImageSource;
use twine_style::{Part, PropId};
use twine_testing::EngineHarness;
use twine_testing::MemoryDisplay;
use twine_text::{LongMode, symbols};
use twine_widgets::label::Label;
use twine_widgets_ext::window::{self, Window};

fn scene(mode: Mode) -> (EngineHarness, NodeId, NodeId, NodeId) {
    let mut h = harness_with_group(320, 240, mode);
    let screen = h.screen();
    let w = window::create(h.engine_mut(), screen).unwrap();
    let (title, btn) = with(&mut h, w, |win: &mut Window, cx| {
        let t = win.add_title(cx, "A window with a rather long title").unwrap();
        win.add_button(cx, ImageSource::Symbol(symbols::LEFT), 40);
        let b = win
            .add_button(cx, ImageSource::Symbol(symbols::CLOSE), 60)
            .unwrap();
        (t, b)
    });
    let content = get::<Window>(&h, w).content();
    for i in 0..12 {
        let l = twine_widgets::label::create(h.engine_mut(), content).unwrap();
        let text: &'static str = Box::leak(format!("Line {i} of the window's content").into_boxed_str());
        h.engine_mut()
            .with_widget_mut(l, |x: &mut Label, cx| x.set_text_static(cx, text));
        h.engine_mut().set_y(l, i * 24);
    }
    h.run_until_idle();
    (h, w, title, btn)
}

#[test]
fn window_structure_header_content() {
    let (h, w, _, _) = scene(Mode::Light);
    let win = get::<Window>(&h, w);
    let e = h.engine();
    assert_eq!(class(&h, w), "win");
    assert_eq!(class(&h, win.header()), "win_header");
    assert_eq!(class(&h, win.content()), "win_content");
    assert_eq!(e.coords(w).size(), twine_core::Size::new(320, 240));
    let (hc, cc) = (e.coords(win.header()), e.coords(win.content()));
    assert_eq!(hc.width(), 320);
    assert_eq!(cc.y0, hc.y1);
    assert_eq!(cc.y1, 240, "the content takes the rest");
}

#[test]
fn window_header_height_follows_dpi() {
    // LVGL `lv_win_constructor`: the header is `lv_display_get_dpi / 2` high.
    let mut h = EngineHarness::new(320, 240).theme(Rc::new(twine_theme::DefaultTheme::light()));
    let mut info = DisplayInfo::new(320, 240, ColorFormat::Rgb565);
    info.dpi = 260;
    let buf: &'static mut [u8] = Box::leak(vec![0u8; 320 * 2 * 20].into_boxed_slice());
    let e = h.engine_mut();
    let d260 = e
        .add_display(MemoryDisplay::new(info), BufferMode::partial_single(buf))
        .unwrap();
    let s130 = h.screen();
    let e = h.engine_mut();
    let s260 = e.active_screen(d260).unwrap();
    let w130 = window::create(e, s130).unwrap();
    let w260 = window::create(e, s260).unwrap();
    h.run_until_idle();
    for (w, dpi) in [(w130, 130), (w260, 260)] {
        let header = get::<Window>(&h, w).header();
        assert_eq!(h.engine().coords(header).height(), dpi / 2, "{dpi} dpi");
        assert_eq!(window::default_header_height(dpi as u16), dpi / 2);
    }
}

#[test]
fn window_title_ellipsis() {
    let (h, _, title, _) = scene(Mode::Light);
    let l = get::<Label>(&h, title);
    assert_eq!(l.long_mode(), LongMode::Dots);
    assert!(l.dots_end().is_some(), "cut with dots");
    let line = i32::from(h.engine().style_font(title, Part::Main).line_height);
    assert_eq!(h.engine().coords(title).height(), line, "one line");
    assert_eq!(h.engine().style_i32(title, Part::Main, PropId::FlexGrow), 1);
}

#[test]
fn window_content_scrolls() {
    let (mut h, w, _, _) = scene(Mode::Light);
    let c = get::<Window>(&h, w).content();
    assert!(h.engine().scroll_bottom(c) > 0);
    let p = center(&h, c);
    h.drag(p, twine_core::Point::new(p.x, p.y - 80), Duration::ms(200));
    h.run_until_idle();
    assert!(h.engine().scroll_offset(c).y > 0);
    h.assert_idle();
}

#[test]
fn window_header_button_click() {
    let (mut h, w, _, btn) = scene(Mode::Light);
    let clicks = count_events(&mut h, btn, EventCode::Clicked);
    let header = get::<Window>(&h, w).header();
    let e = h.engine();
    assert_eq!(e.coords(btn).width(), 60);
    assert_eq!(e.coords(btn).height(), e.content_area(header).height());
    h.tap(center(&h, btn));
    assert_eq!(clicks.get(), 1);
    // The header height can be changed (idempotent).
    with(&mut h, w, |win: &mut Window, cx| win.set_header_height(cx, 50));
    h.run_until_idle();
    assert_eq!(h.engine().coords(header).height(), 50);
    with(&mut h, w, |win: &mut Window, cx| win.set_header_height(cx, 50));
    h.assert_idle();
}

#[test]
fn window_theme_styles() {
    let (h, w, _, _) = scene(Mode::Light);
    let win = get::<Window>(&h, w);
    let e = h.engine();
    // Header: grey with `pad_tiny`; content: the screen look with `pad_normal`.
    assert_eq!(
        e.style_color(win.header(), Part::Main, PropId::BgColor),
        twine_theme::default::colors::LIGHT_GREY
    );
    assert_eq!(
        e.style_i32(win.header(), Part::Main, PropId::PadLeft),
        twine_theme::dpx(2, 130)
    );
    assert_eq!(
        e.style_color(win.content(), Part::Main, PropId::BgColor),
        twine_theme::default::colors::LIGHT_SCR
    );
    assert_eq!(
        e.style_i32(win.content(), Part::Main, PropId::PadLeft),
        twine_theme::dpx(16, 130)
    );
}

#[test]
fn snapshot_window() {
    for m in Mode::ALL {
        let (mut h, _, _, _) = scene(m);
        h.assert_snapshot(&format!("window_{}", m.suffix()));
    }
}
