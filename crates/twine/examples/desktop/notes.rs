//! The notes tab: a multi-line editor whose text is saved to, and loaded from, a file in the
//! system's temp directory.

use std::path::PathBuf;

use twine::prelude::*;

/// The notes file, in the system's temp directory.
const NOTES_NAME: &str = "twine-desktop-notes.txt";

fn notes_file() -> PathBuf {
    std::env::temp_dir().join(NOTES_NAME)
}

pub fn page(cx: Scope) -> impl View {
    let path = notes_file();
    let text = cx.signal(std::fs::read_to_string(&path).unwrap_or_default());
    let saved = cx.signal(text.get_untracked());
    let status = cx.signal(format!("File: {NOTES_NAME} in the temp directory"));
    let dirty = cx.memo(move || text.with(|t| saved.with(|s| t != s)));

    let save = move || {
        let body = text.get_untracked();
        match std::fs::write(notes_file(), &body) {
            Ok(()) => {
                status.set(format!("Saved {} bytes to {NOTES_NAME}", body.len()));
                saved.set(body);
            }
            Err(e) => status.set(format!("Could not save: {e}")),
        }
    };
    let revert = move || text.set(saved.get_untracked());

    column((
        row((
            label("Notes").font(&fonts::MONTSERRAT_28).flex_grow(1),
            button(label(text!("{} Save", Symbol::Save)))
                .disabled(move || !dirty.get())
                .on_click(save)
                .test_id("save"),
            button(label("Revert"))
                .disabled(move || !dirty.get())
                .on_click(revert),
        ))
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        textarea(text)
            .placeholder("Write something; it survives a restart once saved.")
            .fill_width()
            .flex_grow(1)
            .test_id("notes"),
        label(text!(
            "{}{}",
            status.get(),
            if dirty.get() { " (unsaved changes)" } else { "" }
        ))
        .text_color(design::ON_SURFACE_MUTED)
        .long_mode(LongMode::Dots)
        .max_lines(1)
        .fill_width()
        .test_id("status"),
    ))
    .gap(10)
    .padding(24)
    .fill()
}
