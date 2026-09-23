//! Test logging: [`init_test_logging`].

/// Initialises `env_logger` for tests (output captured by the test harness, filter from
/// `RUST_LOG`, e.g. `RUST_LOG=twine=debug`). Safe to call from every test: only the first
/// call has an effect.
///
/// ```
/// twine_testing::init_test_logging();
/// twine_testing::init_test_logging(); // idempotent
/// ```
pub fn init_test_logging() {
    let _ = env_logger::builder().is_test(true).try_init();
}
