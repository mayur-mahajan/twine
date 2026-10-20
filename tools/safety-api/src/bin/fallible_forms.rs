//! The `Result` forms exist in the `safety` profile: this must compile.
use twine::hal::{Clock, DisplayDriver};
use twine::prelude::*;

#[derive(Clone, PartialEq)]
enum Route {
    Home,
}

#[allow(dead_code)]
fn mount<D: DisplayDriver + 'static>(panel: D, clock: impl Clock + 'static, rt: Runtime) -> Result<Ui, UiError> {
    Ui::builder(panel)
        .runtime(rt)
        .clock(clock)
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .try_build(|cx| {
            let _: Result<u8, MissingContext> = cx.use_context::<u8>();
            let _ = use_navigator::<Route>(cx).ok().map(|_| Route::Home);
            let _ = cx.try_signal(1u8).map(|s| s.try_get());
            label("hi")
        })
}

fn main() {}
