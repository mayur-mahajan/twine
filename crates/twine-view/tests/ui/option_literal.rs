// `Some(5)` is an `Option<i32>`, and there is no `From<Option<i32>> for Option<Length>`.
use twine_view::prelude::*;

fn opt_len<M>(w: impl IntoProp<Option<Length>, M>) -> Prop<Option<Length>> {
    w.into_prop()
}

fn main() {
    let _ = opt_len(Some(Length::Px(5))); // fine
    let _ = opt_len(Length::Px(5)); // fine
    let _ = opt_len(Some(5));
}
