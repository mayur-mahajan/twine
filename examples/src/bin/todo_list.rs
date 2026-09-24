//! `cargo xtask sim todo_list`: a keyed list (`for_each`). "Add" appends an item, each row can
//! be deleted or moved up. Rows are reused, never rebuilt: adding redraws only the new row
//! and the counter (F2), moving up moves one row (`RUST_LOG=twine::view=debug` logs
//! `for_each reconcile: … moved=1`).

use twine::prelude::*;
use twine_sim::SimConfig;

#[derive(Clone, PartialEq)]
struct Item {
    id: u32,
    text: String,
}

fn app(cx: Scope) -> impl View {
    let items = cx.signal(Vec::<Item>::new());
    let next = cx.signal(1u32);
    let add = move || {
        let id = next.get_untracked();
        next.set(id + 1);
        items.update(|v| {
            v.push(Item {
                id,
                text: format!("Item {id}"),
            });
        });
    };
    column((
        row((
            button(label("Add")).on_click(add),
            label(text!("{} items", items.with(Vec::len))).test_id("count"),
        ))
        .gap(12)
        .align_items(FlexAlign::Center),
        scroll_view(
            Dir::VER,
            for_each(
                move || items.get(),
                |it| it.id,
                move |_cx, it| {
                    let id = it.id;
                    row((
                        label(it.text).flex_grow(1),
                        button(label(symbols::UP)).on_click(move || {
                            items.update(|v| {
                                if let Some(i) = v.iter().position(|x| x.id == id) {
                                    if i > 0 {
                                        v.swap(i, i - 1);
                                    }
                                }
                            });
                        }),
                        button(label(symbols::CLOSE))
                            .on_click(move || items.update(|v| v.retain(|x| x.id != id))),
                    ))
                    .gap(6)
                    .align_items(FlexAlign::Center)
                    .width(Length::pct(100))
                },
            ),
        )
        .gap(6)
        .width(Length::pct(100))
        .flex_grow(1),
    ))
    .gap(10)
    .padding(10)
    .size(Length::pct(100), Length::pct(100))
}

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("todo list").scale(2), app);
}
