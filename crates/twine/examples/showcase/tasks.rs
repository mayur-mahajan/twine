//! The tasks tab: a keyed list. Adding, checking or deleting a task touches only that row —
//! `for_each` keeps every other row (and its widgets) as it is.

use twine::prelude::*;

#[derive(Clone, PartialEq)]
struct Task {
    id: u32,
    title: String,
    done: bool,
}

/// The rows' state and the next id, as one `Copy` handle the row closures share.
#[derive(Clone, Copy)]
struct Tasks {
    items: Signal<Vec<Task>>,
    next_id: Signal<u32>,
}

impl Tasks {
    fn add(self, title: &str) {
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let id = self.next_id.get_untracked();
        self.next_id.set(id + 1);
        self.items.update(|v| {
            v.push(Task {
                id,
                title,
                done: false,
            });
        });
    }

    fn toggle(self, id: u32, done: bool) {
        self.items.update(|v| {
            if let Some(t) = v.iter_mut().find(|t| t.id == id) {
                t.done = done;
            }
        });
    }

    fn remove(self, id: u32) {
        self.items.update(|v| v.retain(|t| t.id != id));
    }
}

pub fn page(cx: Scope) -> impl View {
    let seed = ["Water the plants", "Calibrate the sensor", "Ship v0.1"];
    let tasks = Tasks {
        items: cx.signal(
            seed.iter()
                .enumerate()
                .map(|(i, t)| Task {
                    id: i as u32,
                    title: (*t).into(),
                    done: i == 0,
                })
                .collect(),
        ),
        next_id: cx.signal(seed.len() as u32),
    };
    let draft = cx.signal(String::new());
    let open = cx.memo(move || tasks.items.with(|v| v.iter().filter(|t| !t.done).count()));

    let add = move || {
        tasks.add(&draft.get_untracked());
        draft.set(String::new());
    };

    column((
        row((
            textarea(draft)
                .one_line(true)
                .placeholder("New task")
                .on_ready(add)
                .flex_grow(1)
                .test_id("draft"),
            button(label("Add")).on_click(add).test_id("add"),
        ))
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        label(text!("{} open", open.get())).test_id("open"),
        column(for_each(
            tasks.items,
            |t| t.id,
            move |_cx, task| {
                let id = task.get_untracked().id;
                let done = task.map(|t| t.done);
                row((
                    checkbox(task.get_untracked().title, done.get_untracked())
                        .on_change(move |on| tasks.toggle(id, on))
                        .flex_grow(1),
                    button(label("Delete")).on_click(move || tasks.remove(id)),
                ))
                .fill_width()
                .align_items(CrossAlign::Center)
            },
        ))
        .fill_width()
        .gap(4),
    ))
    .gap(8)
    .padding(10)
    .fill()
    .scrollable(true)
}
