//! # twine-embassy
//!
//! Runs a Twine [`AsyncUi`] on an [embassy](https://embassy.dev) executor: [`run`] updates the
//! UI, renders each frame while the previous chunk is still being transferred by DMA, and then
//! sleeps until the UI's next deadline, a channel message (the UI waker) or an input interrupt.
//! An idle UI costs no CPU at all.
//!
//! [`EmbassyClock`] is the UI clock over `embassy-time`; [`hires_now`] is the matching
//! high-resolution timer for `EngineConfig::hires_timer` (render/flush statistics).
//!
//! Works with any embassy HAL (embassy-rp, embassy-stm32, esp-hal's `esp-rtos`): the display is
//! an `AsyncDisplayDriver` (e.g. a `twine-drivers` panel over an async, DMA-backed `SpiDevice`),
//! and the input waited on is an `AsyncInputWait` device (e.g. a touch controller with its
//! interrupt pin).
//!
//! ```no_run
//! # async fn example(
//! #     display: impl twine_hal::AsyncDisplayDriver + 'static,
//! #     touch: impl twine_hal::InputDevice + twine_hal::AsyncInputWait + 'static,
//! #     buf_a: &'static mut [u8],
//! #     buf_b: &'static mut [u8],
//! # ) -> ! {
//! use twine_embassy::UiBuilderExt;
//! use twine_view::prelude::*;
//!
//! fn app(_cx: Scope) -> impl View {
//!     label("Hello")
//! }
//!
//! // SAFETY: the UI and all reactive handles stay in this task (single execution context).
//! let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
//! let ui = builder
//!     .buffers(BufferMode::partial_double(buf_a, buf_b))
//!     .input_wait(touch)
//!     .with_embassy_clock()
//!     .build(app);
//! twine_embassy::run(ui).await
//! # }
//! ```
#![no_std]

use core::future::poll_fn;
use core::task::Poll;

use embassy_futures::select::{select, select3};
use twine_core::Instant;
use twine_hal::{AsyncDisplayDriver, AsyncInputWait, Clock};
use twine_reactive::UiWaker;
use twine_view::{AsyncUi, AsyncUiBuilder, Wake};

/// The UI clock: `embassy_time::Instant::now()` in microseconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmbassyClock;

impl Clock for EmbassyClock {
    fn now(&self) -> Instant {
        hires_now()
    }
}

/// The current `embassy-time` instant as a Twine [`Instant`] (for
/// `EngineConfig::hires_timer`; its resolution is the embassy tick rate).
#[must_use]
pub fn hires_now() -> Instant {
    Instant::from_micros(embassy_time::Instant::now().as_micros())
}

/// Converts a Twine instant (µs since boot) to an `embassy-time` instant.
#[must_use]
pub fn to_embassy(t: Instant) -> embassy_time::Instant {
    embassy_time::Instant::from_micros(t.as_micros())
}

/// Builder conveniences for embassy.
pub trait UiBuilderExt {
    /// Uses [`EmbassyClock`] as the UI clock.
    #[must_use]
    fn with_embassy_clock(self) -> Self;
}

impl<D: AsyncDisplayDriver + 'static, W: AsyncInputWait + 'static> UiBuilderExt for AsyncUiBuilder<D, W> {
    fn with_embassy_clock(self) -> Self {
        self.clock(EmbassyClock)
    }
}

/// Completes when `waker` is set (a channel send or `notify_input` from any context); does not
/// clear it (the next update does).
pub async fn wait_waker(waker: &UiWaker) {
    poll_fn(|cx| {
        if waker.is_set() {
            return Poll::Ready(());
        }
        waker.register(cx.waker());
        // A wake between the check and the registration would otherwise be missed.
        if waker.is_set() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

/// Runs the UI forever: update (with DMA-pipelined rendering), then sleep until the UI's
/// deadline ([`Wake::At`]), a waker signal or an input interrupt; with [`Wake::Idle`] only the
/// latter two wake it (no timer is armed); [`Wake::Now`] yields to other tasks and continues.
pub async fn run<D, W>(mut ui: AsyncUi<D, W>) -> !
where
    D: AsyncDisplayDriver + 'static,
    W: AsyncInputWait + 'static,
{
    twine_core::info!(target: "twine::embassy", "run loop started");
    loop {
        match ui.update_async().await {
            Wake::Now => embassy_futures::yield_now().await,
            Wake::At(t) => {
                let waker = ui.waker();
                select3(
                    embassy_time::Timer::at(to_embassy(t)),
                    wait_waker(waker),
                    ui.wait_input(),
                )
                .await;
            }
            Wake::Idle => {
                let waker = ui.waker();
                twine_core::trace!(target: "twine::embassy", "idle: waiting for input or waker");
                select(wait_waker(waker), ui.wait_input()).await;
            }
        }
    }
}
