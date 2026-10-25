//! The display's text gamma (R6.S05): `Engine::set_text_gamma` replaces the fonts' own
//! coverage → opacity curves for every text drawn on that display, and redraws it.

use twine_core::{ColorFormat, Instant, Opa};
use twine_engine::{DisplayId, Engine, EngineConfig, NodeId, Obj};
use twine_hal::DisplayInfo;
use twine_style::Part;
use twine_text::gamma;

const CHUNK: usize = 64 * 2 * 8;

fn display(e: &mut Engine) -> (DisplayId, NodeId) {
    let d = e
        .add_chunked_display(DisplayInfo::new(64, 32, ColorFormat::Rgb565), CHUNK, None)
        .unwrap();
    let text = e.create(e.active_screen(d).unwrap(), Box::new(Obj)).unwrap();
    (d, text)
}

/// Renders every pending frame; returns whether anything was drawn.
fn render(e: &mut Engine, t: u64) -> bool {
    let mut buf = vec![0u8; CHUNK];
    if e.refresh_begin(Instant::from_millis(t)).is_none() {
        return false;
    }
    while e.render_chunk(&mut buf).is_some() {}
    e.refresh_end();
    true
}

#[test]
fn each_display_draws_with_its_own_text_gamma() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let (a, text_a) = display(&mut e);
    let (b, text_b) = display(&mut e);
    e.set_text_gamma(a, Some(&gamma::LINEAR));
    assert!(core::ptr::eq(
        e.text_gamma(a).unwrap(),
        core::ptr::from_ref(&gamma::LINEAR)
    ));
    assert_eq!(e.text_gamma(b), None);
    let on_a = e.text_dsc(text_a, Part::Main, Opa::COVER).gamma;
    assert!(on_a.is_some_and(|g| core::ptr::eq(g, core::ptr::from_ref(&gamma::LINEAR))));
    assert!(
        e.text_dsc(text_b, Part::Main, Opa::COVER).gamma.is_none(),
        "the fonts' own"
    );
    // Back to the fonts' own.
    e.set_text_gamma(a, None);
    assert!(e.text_dsc(text_a, Part::Main, Opa::COVER).gamma.is_none());
}

#[test]
fn changing_the_text_gamma_redraws_the_display() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let (d, _) = display(&mut e);
    assert!(render(&mut e, 0), "first frame");
    assert!(!render(&mut e, 20), "nothing changed");
    e.set_text_gamma(d, Some(&gamma::LINEAR));
    assert!(render(&mut e, 40), "a new gamma redraws the text");
    e.set_text_gamma(d, Some(&gamma::LINEAR));
    assert!(!render(&mut e, 60), "the same gamma again changes nothing");
    e.set_text_gamma(d, None);
    assert!(render(&mut e, 80));
}

#[test]
fn an_unknown_display_is_ignored() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let (d, _) = display(&mut e);
    let other = {
        let mut e2 = Engine::new(EngineConfig::default()).unwrap();
        display(&mut e2);
        display(&mut e2).0
    };
    e.set_text_gamma(other, Some(&gamma::LINEAR));
    assert_eq!(e.text_gamma(other), None);
    assert_eq!(e.text_gamma(d), None);
}
