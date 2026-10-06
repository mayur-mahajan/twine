use twine_view::prelude::*;

#[component]
fn meter(value: Prop<i32>, #[prop(default)] caption: TextProp) -> impl ViewExt<Widget = Obj> {
    column((label(caption), bar(value)))
}

fn main() {
    // `value` is required: it is an argument, so leaving it out is a compile-time error.
    let _ = meter().caption("rpm");
}
