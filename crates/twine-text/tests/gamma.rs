//! Text gamma (R6.S03): `draw_text` picks the font's coverage → opacity curve once per run from
//! the text and background colors.

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, ColorFormat, Rect};
use twine_render::{DrawBuf, Painter, RenderCaches};
use twine_text::{GlyphCache, Polarity, TextDsc, draw_text, gamma};

const W: i32 = 40;
const H: i32 = 20;

/// Draws "l" with `dsc` over an L8 buffer filled with `bg` (black or white) and returns the
/// opacity of each pixel (`|pixel − bg|`).
fn opacity(dsc: &TextDsc, bg: u8) -> Vec<u8> {
    let mut caches = RenderCaches::default();
    let mut glyphs = GlyphCache::default();
    let mut px = vec![bg; (W * H) as usize];
    let area = Rect::from_xywh(0, 0, W, H);
    let buf = DrawBuf::new_packed(&mut px, ColorFormat::L8, area).unwrap();
    let mut p = Painter::new(buf, &mut caches);
    draw_text(&mut p, area, "l", dsc, &mut glyphs);
    drop(p);
    px.iter().map(|&v| v.abs_diff(bg)).collect()
}

/// The opacities the font's ramp of `polarity` gives the glyph "l" (sorted, without zeros).
fn ramp_values(polarity: Polarity) -> Vec<u8> {
    let (f, info) = MONTSERRAT_14.glyph('l', None).unwrap();
    let mut out = vec![0u8; usize::from(info.box_w) * usize::from(info.box_h)];
    assert!(
        f.provider
            .render_a8(&info, polarity, &mut out, &mut GlyphCache::new(0))
    );
    let mut v: Vec<u8> = out.into_iter().filter(|&a| a > 0).collect();
    v.sort_unstable();
    v
}

fn drawn(dsc: &TextDsc, bg: u8) -> Vec<u8> {
    let mut v: Vec<u8> = opacity(dsc, bg).into_iter().filter(|&a| a > 0).collect();
    v.sort_unstable();
    v
}

#[test]
fn built_in_fonts_carry_the_default_curves() {
    let (dark, light) = (
        ramp_values(Polarity::DarkOnLight),
        ramp_values(Polarity::LightOnDark),
    );
    // Every partially covered pixel is more opaque light-on-dark.
    assert_ne!(dark, light);
    assert!(dark.iter().zip(&light).all(|(d, l)| d <= l));
    let t = gamma::DEFAULT.ramp(Polarity::LightOnDark).table4();
    assert!(light.iter().all(|v| t.contains(v)), "{light:?}");
}

#[test]
fn draw_text_picks_the_polarity_of_the_run() {
    // White text over black: light on dark, decided from the text color alone.
    let mut d = TextDsc::new(&MONTSERRAT_14);
    d.color = Color::WHITE;
    assert_eq!(drawn(&d, 0), ramp_values(Polarity::LightOnDark));
    // Black text over white: dark on light.
    d.color = Color::BLACK;
    assert_eq!(drawn(&d, 255), ramp_values(Polarity::DarkOnLight));
    // A known background decides: mid-grey text over black is light on dark.
    d.color = Color::hex(0x00_75_75_75);
    d.background = Some(Color::BLACK);
    let grey = opacity(&d, 0);
    d.background = None;
    assert_ne!(
        grey,
        opacity(&d, 0),
        "without the background, grey text counts as dark"
    );
}

#[test]
fn selection_uses_its_own_polarity() {
    let mut d = TextDsc::new(&MONTSERRAT_14);
    d.color = Color::BLACK;
    d.sel_start = Some(0);
    d.sel_end = Some(1);
    d.sel_color = Color::WHITE;
    d.sel_bg_color = Color::BLACK;
    // The selection background is black, the selected "l" white: light on dark.
    assert_eq!(drawn(&d, 0), ramp_values(Polarity::LightOnDark));
}
