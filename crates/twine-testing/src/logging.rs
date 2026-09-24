//! Test logging: [`init_test_logging`] and [`capture_logs`].

use std::cell::RefCell;
use std::sync::OnceLock;

/// One log record captured by [`capture_logs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedLog {
    /// The level.
    pub level: log::Level,
    /// The target (`"twine::image"`…).
    pub target: String,
    /// The formatted message.
    pub message: String,
}

thread_local! {
    /// Records of the running `capture_logs` calls of this thread (innermost last).
    static CAPTURE: RefCell<Vec<Vec<CapturedLog>>> = const { RefCell::new(Vec::new()) };
}

/// Forwards to `env_logger` (filter from `RUST_LOG`) and records every record while a
/// [`capture_logs`] call runs on the same thread.
struct TestLogger {
    inner: env_logger::Logger,
}

impl log::Log for TestLogger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        capturing() || self.inner.enabled(m)
    }

    fn log(&self, r: &log::Record<'_>) {
        if capturing() {
            let rec = CapturedLog {
                level: r.level(),
                target: r.target().to_owned(),
                message: r.args().to_string(),
            };
            let _ = CAPTURE.try_with(|c| {
                if let Some(v) = c.borrow_mut().last_mut() {
                    v.push(rec);
                }
            });
        }
        if self.inner.matches(r) {
            self.inner.log(r);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

fn capturing() -> bool {
    CAPTURE.try_with(|c| !c.borrow().is_empty()).unwrap_or(false)
}

static LOGGER: OnceLock<TestLogger> = OnceLock::new();

/// Installs the test logger once per process (a no-op if another logger is installed).
fn install() {
    let l = LOGGER.get_or_init(|| TestLogger {
        inner: env_logger::Builder::from_default_env().is_test(true).build(),
    });
    if log::set_logger(l).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
}

/// Initialises logging for tests: `env_logger` output (captured by the test harness, filter
/// from `RUST_LOG`, e.g. `RUST_LOG=twine=debug`) plus the [`capture_logs`] hook. Safe to call
/// from every test: only the first call has an effect.
///
/// ```
/// twine_testing::init_test_logging();
/// twine_testing::init_test_logging(); // idempotent
/// ```
pub fn init_test_logging() {
    install();
}

/// Runs `f` and returns its result with every log record it emitted on this thread (all
/// levels, whatever `RUST_LOG` says). Installs the test logger if needed.
///
/// ```
/// use twine_testing::capture_logs;
/// let ((), logs) = capture_logs(|| log::warn!(target: "twine::demo", "careful: {}", 3));
/// assert_eq!(logs.len(), 1);
/// assert_eq!(logs[0].level, log::Level::Warn);
/// assert_eq!(logs[0].message, "careful: 3");
/// ```
pub fn capture_logs<R>(f: impl FnOnce() -> R) -> (R, Vec<CapturedLog>) {
    install();
    CAPTURE.with(|c| c.borrow_mut().push(Vec::new()));
    let r = f();
    let logs = CAPTURE.with(|c| c.borrow_mut().pop()).unwrap_or_default();
    (r, logs)
}
