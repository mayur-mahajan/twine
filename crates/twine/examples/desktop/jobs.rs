//! The jobs tab: the UI hands work to a worker thread through an `Outbox` and follows its
//! progress through a `Channel`. The UI thread never blocks; the worker never touches a
//! widget.

use std::time::Duration as StdDuration;

use twine::prelude::*;

use crate::Ports;

/// A request from the UI: process `steps` units of work under job `id`.
#[derive(Clone, Copy, Debug)]
pub struct JobRequest {
    pub id: u32,
    pub steps: u32,
}

/// A report from the worker thread.
#[derive(Clone, Copy, Debug)]
pub enum JobEvent {
    /// Job `id` is `percent` done.
    Progress { id: u32, percent: i32 },
    /// Job `id` finished.
    Done { id: u32 },
}

/// The worker's side: runs on the worker thread, reports as it goes. A full channel drops
/// the oldest report (the UI only needs the newest).
pub fn run(job: JobRequest, progress: &'static Channel<JobEvent, 32>) {
    for step in 1..=job.steps {
        std::thread::sleep(StdDuration::from_millis(60));
        let percent = (step * 100 / job.steps) as i32;
        let _ = progress.try_send(JobEvent::Progress { id: job.id, percent });
    }
    let _ = progress.try_send(JobEvent::Done { id: job.id });
}

#[derive(Clone, PartialEq)]
struct Job {
    id: u32,
    percent: i32,
    done: bool,
}

pub fn page(cx: Scope, ports: Ports) -> impl View {
    let jobs = cx.signal(Vec::<Job>::new());
    let next_id = cx.signal(1u32);
    let running = cx.memo(move || jobs.with(|v| v.iter().filter(|j| !j.done).count()));

    // Progress arrives here, on the UI thread, in the order it was sent.
    cx.on_message(ports.progress, move |ev| {
        jobs.update(|v| {
            let (id, percent, done) = match ev {
                JobEvent::Progress { id, percent } => (id, percent, false),
                JobEvent::Done { id } => (id, 100, true),
            };
            if let Some(j) = v.iter_mut().find(|j| j.id == id) {
                j.percent = percent;
                j.done = done;
            }
        });
    });

    let start = move || {
        let id = next_id.get_untracked();
        let steps = 20 + (id % 4) * 15;
        if ports.jobs.try_send(JobRequest { id, steps }).is_ok() {
            next_id.set(id + 1);
            jobs.update(|v| {
                v.push(Job {
                    id,
                    percent: 0,
                    done: false,
                });
            });
        }
    };
    let clear = move || jobs.update(|v| v.retain(|j| !j.done));

    column((
        row((
            label("Jobs").font(&fonts::MONTSERRAT_28).flex_grow(1),
            button(label(text!("{} Start a job", Symbol::Play)))
                .on_click(start)
                .test_id("start"),
            button(label("Clear finished")).on_click(clear),
        ))
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        label(text!("{} running on the worker thread", running.get()))
            .text_color(design::ON_SURFACE_MUTED)
            .test_id("running"),
        column(for_each(
            jobs,
            |j| j.id,
            |_cx, job| {
                let id = job.get_untracked().id;
                let percent = job.map(|j| j.percent);
                let done = job.map(|j| j.done);
                card(
                    row((
                        label(format!("Job #{id}")).width(90),
                        bar(percent).animated(Duration::ms(150)).flex_grow(1),
                        label(text!(
                            "{}",
                            if done.get() {
                                "Done".to_string()
                            } else {
                                format!("{}%", percent.get())
                            }
                        ))
                        .width(60),
                    ))
                    .fill_width()
                    .gap(12)
                    .align_items(CrossAlign::Center),
                )
                .padding(12)
                .fill_width()
            },
        ))
        .fill_width()
        .gap(8),
    ))
    .gap(10)
    .padding(24)
    .fill()
    .scrollable(true)
}
