//! The Lottie demo: plays without warnings, the controls work, pausing everything makes the
//! UI idle.

use twine::core::Duration;
use twine_lottie::widget::Lottie;
use twine_testing::{TestUi, by_id, capture_logs};

fn frame(t: &TestUi, id: &'static str) -> u32 {
    let n = t.find(by_id(id)).id();
    t.engine().widget::<Lottie>(n).unwrap().current_frame()
}

#[test]
fn lottie_demo_smoke() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(480, 320)
            .app_config(twine_demos::config())
            .mount(twine_demos::lottie::app);
        t.advance(Duration::ms(500));
        for name in ["Loader", "Check", "Heart"] {
            assert!(frame(&t, name) >= 13, "{name} plays");
        }
        t.assert_panel_snapshot("lottie_demo_500ms");
        // Pause all three: idle.
        for id in ["play0", "play1", "play2"] {
            t.find(by_id(id)).click();
        }
        t.run_until_idle();
        t.assert_idle();
        // Scrub the heart: the animation seeks.
        let s = t.find(by_id("scrub2")).coords();
        t.tap(twine::core::Point::new(s.x1 - 2, (s.y0 + s.y1) / 2));
        t.run_until_idle();
        assert!(
            frame(&t, "Heart") >= 55,
            "scrubbed to the end: {}",
            frame(&t, "Heart")
        );
        // Loop off, play: finishes once and the button shows "play" again.
        t.find(by_id("loop0")).click();
        t.find(by_id("play0")).click();
        t.advance(Duration::ms(2500));
        assert_eq!(frame(&t, "Loader"), 59);
        t.run_until_idle();
        t.assert_idle();
    });
    let bad: Vec<_> = logs
        .iter()
        .filter(|l| l.level <= log::Level::Warn && l.target.starts_with("twine"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}
