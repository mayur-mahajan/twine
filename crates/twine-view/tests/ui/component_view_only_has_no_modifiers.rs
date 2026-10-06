use twine_view::prelude::*;

#[component]
fn maybe(on: bool) -> impl View {
    when(move || on, |_| label("on"))
}

fn main() {
    let _ = maybe(true).padding(4);
}
