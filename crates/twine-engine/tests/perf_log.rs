//! The periodic `twine::perf` log line (captured with a test logger).

use std::sync::Mutex;

use twine_core::Duration;
use twine_engine::{EngineConfig, InvalidateReason};
use twine_testing::EngineHarness;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.target() == "twine::perf"
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            LINES.lock().unwrap().push(r.args().to_string());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Capture = Capture;

#[test]
fn perf_log_emitted_every_period() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    let cfg = EngineConfig {
        perf_log_period: Duration::secs(2),
        ..EngineConfig::default()
    };
    let mut h = EngineHarness::new(64, 32).no_theme().config(cfg);
    let d = h.display();
    for _ in 0..=(5_000 / 16) {
        let area = twine_core::Rect::from_xywh(0, 0, 8, 8);
        h.engine_mut().invalidate_area(d, area, InvalidateReason::Anim);
        h.advance(Duration::ms(16));
    }
    let lines = LINES.lock().unwrap().clone();
    // About 5 s: lines at 2 s and 4 s after the first frame.
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("fps=6") && lines[0].contains(" cpu=0%"),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains("px/frame=64"), "{}", lines[0]);
}
