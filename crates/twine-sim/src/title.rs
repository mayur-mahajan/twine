//! [`set_title`]: changing the window title from inside an app (e.g. to show the current
//! configuration of an interactive example).

use std::cell::RefCell;

thread_local! {
    /// The title requested by the app, applied by the window on its next tick.
    static PENDING: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Sets the simulator window title (applied on the window's next tick; headless runs only log
/// it). Call it from the app, e.g. in a [`SimConfig::on_raw_key`](crate::SimConfig::on_raw_key)
/// hook. The title is also logged at `info` level (`twine::sim`).
///
/// ```
/// twine_sim::set_title("flex_layout — Row, Start");
/// ```
pub fn set_title(title: &str) {
    log::info!(target: "twine::sim", "title: {title}");
    PENDING.with(|p| *p.borrow_mut() = Some(title.to_owned()));
}

/// Takes the title requested since the last call.
pub(crate) fn take_pending() -> Option<String> {
    PENDING.with(|p| p.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_is_taken_once() {
        super::set_title("a");
        super::set_title("b");
        assert_eq!(super::take_pending().as_deref(), Some("b"));
        assert_eq!(super::take_pending(), None);
    }
}
