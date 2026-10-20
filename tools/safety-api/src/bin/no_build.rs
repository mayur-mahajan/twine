//! `UiBuilder::build` (the panicking constructor) does not exist in the `safety` profile.
use twine::hal::{Clock, DisplayDriver};
use twine::prelude::*;

#[allow(dead_code)]
fn mount<D: DisplayDriver + 'static>(panel: D, clock: impl Clock + 'static, rt: Runtime) -> Ui {
    Ui::builder(panel)
        .runtime(rt)
        .clock(clock)
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .build(|_| label("hi"))
}

fn main() {}
