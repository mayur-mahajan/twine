//! `cargo xtask sim todo_list`: the store pattern (`twine_demos::todos`, guide
//! `twine::guide::app_state`). The list's state is one `Copy` store of signals and memos
//! provided as context; the header and the rows read it from the context and change it through
//! its action methods. A background thread plays a companion app: every 5 s it sends a command
//! through `REMOTE` (the ports pattern), which calls the same actions.
//!
//! The list is keyed: adding redraws only the new row and the counter, toggling restyles one
//! row, moving up moves one row (`RUST_LOG=twine::view=debug` logs
//! `for_each reconcile: … moved=1`).

use std::time::Duration as StdDuration;

use twine::prelude::{Channel, lvgl};
use twine_demos::todos::{Ports, Remote, app};
use twine_sim::SimConfig;

/// Commands from the companion app (on a device: a radio or USB task).
static REMOTE: Channel<Remote, 4> = Channel::new();

fn main() {
    std::thread::spawn(|| {
        for cmd in [Remote::Add("Sent from the phone"), Remote::ClearDone]
            .into_iter()
            .cycle()
        {
            std::thread::sleep(StdDuration::from_secs(5));
            let _ = REMOTE.try_send(cmd); // full: the UI is behind; drop it
        }
    });
    let ports = Ports { remote: &REMOTE };
    twine_sim::run(
        SimConfig::new(320, 240)
            .title("todo list")
            .scale(2)
            .theme(&lvgl::THEME),
        move |cx| app(cx, ports),
    );
}
