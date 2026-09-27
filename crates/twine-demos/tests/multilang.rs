//! The multilang demo: every language renders without warnings (no missing glyph, the file
//! avatar found, no cache thrashing) and looks right (one snapshot per language).

use twine_demos::multilang::{self, CODES};
use twine_testing::{TestUi, by_id, capture_logs};

#[test]
fn multilang_smoke() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(480, 320).mount(multilang::app);
        t.run_until_idle();
        for (i, code) in CODES.iter().enumerate() {
            if i > 0 {
                t.find(by_id("next")).click();
                t.run_until_idle();
            }
            // Every card of the list, scrolled into view once per language.
            let n = t.engine().tree().len();
            assert!(n > 10, "{code}: {n} nodes");
        }
        // Back to English: the header follows.
        t.find(by_id("next")).click();
        t.run_until_idle();
        assert_eq!(t.find(by_id("title")).text(), "Twine speaks your language");
        t.assert_idle();
    });
    let bad: Vec<_> = logs
        .iter()
        .filter(|l| l.level <= log::Level::Warn && l.target.starts_with("twine"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn multilang_cards_scroll_without_warnings() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(480, 320).mount(multilang::app);
        t.run_until_idle();
        // Scroll the list to the end: the RTL and CJK cards are built and drawn.
        for _ in 0..4 {
            t.drag(
                twine::core::Point::new(240, 280),
                twine::core::Point::new(240, 120),
                twine::core::Duration::ms(200),
            );
            t.run_until_idle();
        }
        assert!(t.find(by_id("fa")).is_visible());
    });
    let bad: Vec<_> = logs
        .iter()
        .filter(|l| l.level <= log::Level::Warn && l.target.starts_with("twine"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn multilang_snapshots_per_language() {
    let mut t = TestUi::new(480, 320).mount(multilang::app);
    t.run_until_idle();
    for (i, code) in CODES.iter().enumerate() {
        if i > 0 {
            t.find(by_id("next")).click();
            t.run_until_idle();
        }
        t.assert_snapshot(&format!("multilang_{code}"));
    }
}

#[test]
fn multilang_language_list_draws_every_script() {
    let ((), logs) = capture_logs(|| {
        let mut t = TestUi::new(480, 320).mount(multilang::app);
        t.run_until_idle();
        let dd = t.find(by_id("language"));
        let c = dd.coords();
        dd.click();
        t.run_until_idle();
        // The list opens below the dropdown; scroll it to its end so every name is drawn.
        let x = c.center().x;
        for _ in 0..4 {
            t.drag(
                twine::core::Point::new(x, 300),
                twine::core::Point::new(x, c.y1 + 10),
                twine::core::Duration::ms(200),
            );
            t.run_until_idle();
        }
    });
    let bad: Vec<_> = logs
        .iter()
        .filter(|l| l.level <= log::Level::Warn && l.target.starts_with("twine"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}
