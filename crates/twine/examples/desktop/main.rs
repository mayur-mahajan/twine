//! `cargo xtask sim desktop` (or `cargo run -p twine --example desktop`): Twine as the UI of
//! an ordinary desktop program on Linux, macOS or Windows — a 960 × 600 window, no emulated
//! panel scaling, the standard library everywhere.
//!
//! | Tab | Shows |
//! |-----|-------|
//! | Overview | the operating system, CPU count and a wall clock fed by a background thread |
//! | Files | a file browser over `std::fs` (open folders, go up, refresh) as a keyed list |
//! | Notes | a multi-line editor saved to and loaded from a file in the temp directory |
//! | Jobs | work handed to a worker thread through an `Outbox`, progress back through a `Channel` |
//!
//! **Threads.** The UI runs on the main thread; other threads talk to it only through
//! `'static` cells that wake it: a [`Latest`] (the clock: the newest value wins), a
//! [`Channel`] (job progress, queued in order) and an [`Outbox`] (job requests, UI → worker).
//! The app function receives them in a [`Ports`] struct and names no global, so a test can
//! hand it fresh ones. Between two events the UI does no work at all.

mod files;
mod jobs;
mod notes;
mod overview;

use std::sync::OnceLock;
use std::thread::Thread;
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use twine::core::ColorFormat;
use twine::prelude::*;
use twine_sim::SimConfig;

use jobs::{JobEvent, JobRequest};

/// The connections between the UI and the program's other threads.
#[derive(Clone, Copy)]
pub struct Ports {
    /// Seconds since the Unix epoch, updated every second by the clock thread.
    pub clock: &'static Latest<u64>,
    /// Job requests from the UI to the worker thread.
    pub jobs: &'static Outbox<JobRequest, 8>,
    /// Job progress from the worker thread to the UI.
    pub progress: &'static Channel<JobEvent, 32>,
}

static CLOCK: Latest<u64> = Latest::new(0);
static JOBS: Outbox<JobRequest, 8> = Outbox::new();
static PROGRESS: Channel<JobEvent, 32> = Channel::new().on_full(Overflow::DropOldest);
/// The worker thread, unparked when the UI sends it a job.
static WORKER: OnceLock<Thread> = OnceLock::new();

/// `path` for display: the home directory shown as `~`.
pub fn display_path(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(std::path::Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn app(cx: Scope, ports: Ports) -> impl View {
    let page = cx.signal(0usize);
    tabview(
        page,
        (
            tab(text!("{} Overview", Symbol::Home), overview::page(cx, ports)),
            tab(text!("{} Files", Symbol::Directory), files::page(cx)),
            tab(text!("{} Notes", Symbol::Edit), notes::page(cx)),
            tab(text!("{} Jobs", Symbol::Shuffle), jobs::page(cx, ports)),
        ),
    )
    .bar_position(Side::Left)
    .bar_size(170)
    .fill()
}

fn main() {
    // The clock: one value a second; the UI shows whatever is newest.
    CLOCK.set(unix_now());
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(StdDuration::from_millis(250));
            CLOCK.set(unix_now());
        }
    });

    // The worker: sleeps until the UI sends a job, then reports progress.
    let worker = std::thread::spawn(|| {
        loop {
            while let Some(job) = JOBS.try_recv() {
                jobs::run(job, &PROGRESS);
            }
            std::thread::park();
        }
    });
    let _ = WORKER.set(worker.thread().clone());
    JOBS.waker().set_notify(|| {
        if let Some(t) = WORKER.get() {
            t.unpark();
        }
    });

    let ports = Ports {
        clock: &CLOCK,
        jobs: &JOBS,
        progress: &PROGRESS,
    };
    twine_sim::run(
        SimConfig::new(960, 600)
            .title("Twine desktop")
            .scale(1)
            .format(ColorFormat::Xrgb8888)
            .theme(DefaultTheme::light()),
        move |cx| app(cx, ports),
    );
}
