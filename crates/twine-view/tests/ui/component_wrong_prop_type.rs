use twine_view::prelude::*;

#[component]
fn meter(value: Prop<i32>, #[prop(default)] caption: TextProp) -> impl ViewExt<Widget = Obj> {
    column((label(caption), bar(value)))
}

fn main() {
    let _ = meter("high");
    let _ = meter(1).caption(42);
}
