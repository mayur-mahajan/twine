use twine_view::prelude::*;

fn main() {
    let cx = Runtime::current_thread().create_root();
    let on = cx.signal(true);
    let _ = label("x").width(on);
    cx.dispose();
}
