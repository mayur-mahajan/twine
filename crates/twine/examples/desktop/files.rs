//! The files tab: a small browser over `std::fs`. The listing is a signal; `for_each` keeps a
//! row per entry name, so refreshing a folder only rebuilds the rows that changed.

use std::path::{Path, PathBuf};

use twine::prelude::*;

#[derive(Clone, PartialEq)]
struct Entry {
    name: String,
    dir: bool,
    bytes: u64,
}

/// The entries of `dir`: folders first, then files, each sorted by name. Unreadable entries
/// are skipped; an unreadable folder lists nothing.
fn read(dir: &Path) -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    Some(Entry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        dir: meta.is_dir(),
                        bytes: meta.len(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    v
}

fn size_text(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KiB", bytes / 1024),
        _ => format!("{} MiB", bytes / 1_048_576),
    }
}

pub fn page(cx: Scope) -> impl View {
    let cwd = cx.signal(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let entries = cx.signal(read(&cwd.get_untracked()));
    // Re-list whenever the folder changes.
    cx.effect(move || entries.set(read(&cwd.get())));
    let count = cx.memo(move || entries.with(Vec::len));

    let open = move |name: String| {
        cwd.update(|p| p.push(name));
    };
    let up = move || {
        cwd.update(|p| {
            p.pop();
        });
    };
    let refresh = move || entries.set(read(&cwd.get_untracked()));

    column((
        row((
            label("Files").font(&fonts::MONTSERRAT_28).flex_grow(1),
            button(label(text!("{} Up", Symbol::Up)))
                .on_click(up)
                .test_id("up"),
            button(label(text!("{} Refresh", Symbol::Refresh))).on_click(refresh),
        ))
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        label(text!(
            "{}  ({} entries)",
            cwd.with(|p| crate::display_path(p)),
            count.get()
        ))
        .text_color(design::ON_SURFACE_MUTED)
        .long_mode(LongMode::Dots)
        .max_lines(1)
        .fill_width()
        .test_id("path"),
        list(for_each(
            entries,
            |e| e.name.clone(),
            move |_cx, e| {
                let Entry { name, dir, bytes } = e.get_untracked();
                if dir {
                    list_button(Symbol::Directory, name.clone())
                        .on_click(move || open(name.clone()))
                        .into_any()
                } else {
                    list_button(Symbol::File, format!("{name}    {}", size_text(bytes))).into_any()
                }
            },
        ))
        .fill_width()
        .flex_grow(1),
    ))
    .gap(10)
    .padding(24)
    .fill()
}
