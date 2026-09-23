//! The logging macros with the `log` backend (`cargo test -p twine-core --features log`).
#![cfg(all(feature = "log", not(feature = "defmt")))]

use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};
use twine_core::log::{debug, error, info, trace, warn};

/// `(level, target, message)` of every record.
static RECORDS: Mutex<Vec<(Level, String, String)>> = Mutex::new(Vec::new());

struct Capture;

impl Log for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn log(&self, r: &Record<'_>) {
        println!("captured: [{}] {}: {}", r.level(), r.target(), r.args());
        RECORDS
            .lock()
            .unwrap()
            .push((r.level(), r.target().to_string(), r.args().to_string()));
    }
    fn flush(&self) {}
}

static LOGGER: Capture = Capture;

#[test]
fn forwards_level_target_and_message() {
    log::set_logger(&LOGGER).expect("only this test installs a logger");
    log::set_max_level(LevelFilter::Trace);
    assert_eq!(twine_core::log::BACKEND, "log");

    let id = 42;
    warn!(target: "twine::engine", "node {} not found", id);
    info!("plain {}", "info");
    trace!(target: "twine::refresh", "t");
    debug!("d {:?}", Some(1));
    error!(target: "twine::driver", "e {} {}", 1, 2);

    let got = RECORDS.lock().unwrap().clone();
    assert_eq!(
        got,
        vec![
            (Level::Warn, "twine::engine".into(), "node 42 not found".into()),
            (Level::Info, "twine".into(), "plain info".into()),
            (Level::Trace, "twine::refresh".into(), "t".into()),
            (Level::Debug, "twine".into(), "d Some(1)".into()),
            (Level::Error, "twine::driver".into(), "e 1 2".into()),
        ]
    );
}
