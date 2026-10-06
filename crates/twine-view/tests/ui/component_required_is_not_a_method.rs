use twine_view::prelude::*;

#[component]
fn meter(value: Prop<i32>) -> impl ViewExt<Widget = Obj> {
    column(bar(value))
}

fn main() {
    let _ = meter(1).value(2);
}
