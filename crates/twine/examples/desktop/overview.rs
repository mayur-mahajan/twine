//! The overview tab: facts from the standard library and a clock driven by another thread.

use twine::prelude::*;

use crate::Ports;

/// `HH:MM:SS` (UTC) of a Unix time.
fn clock_text(secs: u64) -> String {
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

/// `1 h 02 min 03 s` of a duration in seconds.
fn uptime_text(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {:02} s", secs / 60, secs % 60),
        _ => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
    }
}

fn fact(name: &'static str, value: impl Into<String>) -> impl View {
    row((
        label(name).text_color(design::ON_SURFACE_MUTED).width(140),
        label(value.into())
            .long_mode(LongMode::Dots)
            .max_lines(1)
            .flex_grow(1),
    ))
    .fill_width()
}

pub fn page(cx: Scope, ports: Ports) -> impl View {
    let now = cx.watch(ports.clock);
    let started = now.get_untracked();
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let dir = std::env::current_dir()
        .map(|d| crate::display_path(&d))
        .unwrap_or_default();

    column((
        label("Overview").font(&fonts::MONTSERRAT_28),
        card((
            label(text!("{}", clock_text(now.get())))
                .font(&fonts::MONTSERRAT_48)
                .test_id("clock"),
            label(text!(
                "UTC, running for {}",
                uptime_text(now.get().saturating_sub(started))
            ))
            .text_color(design::ON_SURFACE_MUTED),
        ))
        .padding(20)
        .gap(4)
        .fill_width(),
        card((
            fact(
                "System",
                format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
            ),
            fact("CPU threads", cpus.to_string()),
            fact("Process id", std::process::id().to_string()),
            fact("Directory", dir),
        ))
        .padding(20)
        .gap(10)
        .fill_width(),
    ))
    .gap(16)
    .padding(24)
    .fill()
}
