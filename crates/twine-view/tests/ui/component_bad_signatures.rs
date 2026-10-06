use twine_view::prelude::*;

#[component]
fn no_return(value: Prop<i32>) {}

#[component]
fn no_widget(value: Prop<i32>) -> impl ViewExt {
    bar(value)
}

#[component]
fn optional_impl(#[prop(default)] content: impl ViewSeq) -> impl View {
    column(content)
}

#[component]
fn pattern((a, b): (i32, i32)) -> impl View {
    label("")
}

#[component]
fn lifetime<'a>(s: &'a str) -> impl View {
    label("")
}

#[component]
fn unknown_prop_arg(#[prop(defualt)] v: i32) -> impl View {
    label("")
}

#[component]
fn into_on_prop(#[prop(into)] v: Prop<i32>) -> impl View {
    bar(v)
}

#[component]
fn nested_impl(v: Option<impl ViewSeq>) -> impl View {
    column(v)
}

#[component(widget = Obj)]
fn unknown_arg() -> impl View {
    label("")
}

#[component]
async fn not_async() -> impl View {
    label("")
}

fn main() {}
