//! The logging macros with the default (`none`) backend: they compile with every supported
//! syntax and never evaluate their arguments.
#![cfg(not(any(feature = "log", feature = "defmt")))]

use twine_core::log::{BACKEND, debug, error, info, trace, warn};

#[test]
fn backend_is_none() {
    assert_eq!(BACKEND, "none");
    assert!(!twine_core::log::__log_enabled());
}

#[test]
fn all_macros_compile_with_and_without_target() {
    let (a, b, c) = (1, "two", 3.5_f32);
    trace!("zero args");
    trace!("one {}", a);
    trace!(target: "twine::core", "two {} {}", a, b);
    trace!(target: "twine::core", "three {} {} {:?}", a, b, c);
    debug!("zero args");
    debug!("one {}", a);
    debug!(target: "twine::engine", "two {} {}", a, b);
    debug!(target: "twine::engine", "three {} {} {:?}", a, b, c);
    info!("zero args");
    info!("one {}", a);
    info!(target: "twine::view", "two {} {}", a, b);
    info!(target: "twine::view", "three {} {} {:?}", a, b, c);
    warn!("zero args");
    warn!("one {}", a);
    warn!(target: "twine::render", "two {} {}", a, b);
    warn!(target: "twine::render", "three {} {} {:?}", a, b, c);
    error!("zero args");
    error!("one {}", a);
    error!(target: "twine::driver", "two {} {}", a, b);
    error!(target: "twine::driver", "three {} {} {:?}", a, b, c);
}

#[test]
fn arguments_are_not_evaluated() {
    let mut called = false;
    debug!("{}", {
        called = true;
        1
    });
    assert!(!called);

    let mut count = 0;
    let mut side_effect = || {
        count += 1;
        count
    };
    warn!(target: "twine::core", "{} {}", side_effect(), side_effect());
    error!("{}", side_effect());
    assert_eq!(count, 0);
}
