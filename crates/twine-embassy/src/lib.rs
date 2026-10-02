//! # twine-embassy
//!
//! Runs a Twine UI on an [embassy](https://embassy.dev) executor: [`run`] updates the UI and
//! then sleeps until the UI's next deadline, a channel message (the UI waker) or an input
//! interrupt. An idle UI costs no CPU at all. [`run_with`] does the same and reports each step
//! of the loop ([`LoopEvent`]) to a callback with the UI, so the application keeps its UI and
//! display while the loop runs (backlight, panel sleep, faults).
//!
//! Both runtimes run ([`RunUi`]):
//!
//! - [`AsyncUi`] (`Ui::builder_async`): each frame is rendered while the previous chunk is
//!   still being transferred by DMA (`AsyncDisplayDriver`), and the loop also wakes on the
//!   `AsyncInputWait` input's interrupt.
//! - [`Ui`] (`Ui::builder`, `Ui::builder_fb`): blocking drivers, e.g. a memory-mapped
//!   framebuffer (LTDC); input interrupts reach it through its waker (a task awaiting the
//!   interrupt calls `ui.waker().wake()`).
//!
//! The loop is a thin adapter like `twine::run::blocking` (bare metal, RTOS, host threads),
//! with the same events. Applications usually reach this crate through the facade:
//! `twine::embassy` (feature `embassy`, which also enables `async` and `platform-embassy`).
//!
//! **Several displays.** A [`Ui`] with several displays (`UiBuilder::display`,
//! `Ui::mount_on`) is one UI to the loop: one `run` drives all its displays (one engine, one
//! waker). An [`AsyncUi`] drives one async display; for several, run one task per `AsyncUi`
//! (`run` in each, on the same executor and thread): they share the thread's reactive runtime,
//! and each update runs only its own UI's channel handlers and bindings, so signals shared
//! between them update both without cross-talk (`twine_view::UiCore` § Sharing the runtime).
//!
//! [`EmbassyPlatform`] (re-exported from `twine-hal`'s `platform-embassy`) is the `AsyncUi`'s
//! platform over `embassy-time`: its clock and the timer that bounds every display flush with
//! `EngineConfig::flush_timeout`. [`hires_now`] is the matching high-resolution timer for
//! `EngineConfig::hires_timer` (render/flush statistics).
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
//! # ) -> ! {
//! use twine_embassy::UiBuilderExt;
//! use twine_view::prelude::*;
//!
//! fn app(_cx: Scope) -> impl View {
//!     label("Hello")
//! }
//!
//! // Two DMA ping-pong buffers of 40 rows of a 320 px RGB565 panel, sized at compile time.
//! draw_buffers!(static BUFS: 2 x 40 rows x 320 px @ Rgb565Swapped);
//!
//! let rt = Runtime::take().expect("runtime already taken"); // keeps the UI in this task
//! let ui = Ui::builder_async(display)
//!     .runtime(rt)
//!     .buffers(BufferMode::partial_double_from(BUFS.take().expect("draw buffers already taken")))
//!     .input_wait(touch)
//!     .with_embassy_platform()
//!     .build(app);
//! twine_embassy::run(ui).await
//! # }
//! ```
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use core::future::{Future, poll_fn};
use core::task::Poll;

use embassy_futures::select::{select, select3};
use twine_core::Instant;
pub use twine_hal::EmbassyPlatform;
use twine_hal::{AsyncDisplayDriver, AsyncInputWait, AsyncPlatform, Clock};
use twine_reactive::UiWaker;
pub use twine_view::run::LoopEvent;
use twine_view::{AsyncUi, AsyncUiBuilder, Ui, UiBuilder, Wake};

/// The current `embassy-time` instant as a Twine [`Instant`] (for
/// `EngineConfig::hires_timer`; its resolution is the embassy tick rate). The same clock as
/// [`EmbassyPlatform`].
///
/// ```no_run
/// use twine_engine::EngineConfig;
/// let config = EngineConfig { hires_timer: Some(twine_embassy::hires_now), ..EngineConfig::default() };
/// # let _ = config;
/// ```
#[must_use]
pub fn hires_now() -> Instant {
    EmbassyPlatform.now()
}

/// Converts a Twine instant (µs since boot) to an `embassy-time` instant.
///
/// ```no_run
/// use twine_core::Instant;
/// let t = twine_embassy::to_embassy(Instant::from_millis(5));
/// assert_eq!(t.as_millis(), 5);
/// ```
#[must_use]
pub fn to_embassy(t: Instant) -> embassy_time::Instant {
    EmbassyPlatform::to_embassy(t)
}

/// Builder conveniences for embassy, for both runtimes.
pub trait UiBuilderExt {
    /// The builder with its time source set (the clock / platform part of its
    /// [typestate](twine_view::typestate) provided).
    type Output;

    /// Uses [`EmbassyPlatform`] as the platform: the `AsyncUi`'s clock and flush timer
    /// (`AsyncUiBuilder::platform(&EmbassyPlatform)`), or the `Ui`'s clock
    /// (`UiBuilder::clock(EmbassyPlatform)`), so the deadlines the UI returns are the
    /// `embassy-time` instants [`run`] sleeps until. Provides the builder's required platform
    /// (async) or clock (blocking) part.
    ///
    /// ```no_run
    /// # fn build(display: impl twine_hal::AsyncDisplayDriver + 'static, buf: &'static mut [u8]) {
    /// use twine_embassy::UiBuilderExt;
    /// use twine_view::prelude::*;
    /// let ui = Ui::builder_async(display)
    ///     .runtime(Runtime::take().expect("runtime already taken"))
    ///     .buffers(BufferMode::partial_single(buf))
    ///     .with_embassy_platform()
    ///     .build(|_| label("hi"));
    /// # let _ = ui;
    /// # }
    /// ```
    fn with_embassy_platform(self) -> Self::Output;
}

impl<D, W, R, P, B> UiBuilderExt for AsyncUiBuilder<D, W, R, P, B> {
    type Output = AsyncUiBuilder<D, W, R, Box<dyn AsyncPlatform>, B>;

    fn with_embassy_platform(self) -> Self::Output {
        self.platform(&EmbassyPlatform)
    }
}

impl<S, R, C, B> UiBuilderExt for UiBuilder<S, R, C, B> {
    type Output = UiBuilder<S, R, Box<dyn Clock>, B>;

    fn with_embassy_platform(self) -> Self::Output {
        self.clock(EmbassyPlatform)
    }
}

/// Completes when `waker` is set (a channel send or `notify_input` from any context); does not
/// clear it (the next update does).
///
/// ```no_run
/// # async fn example(waker: &'static twine_reactive::UiWaker) {
/// twine_embassy::wait_waker(waker).await; // returns at once if a wake-up is pending
/// # }
/// ```
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

/// A UI that [`run`] and [`run_with`] can drive: [`Ui`] and [`AsyncUi`].
///
/// Implement it for your own type to run it with these loops (e.g. a wrapper that also owns
/// a peripheral the callback of [`run_with`] needs).
///
/// ```no_run
/// // `no_run`: `run_with` never returns (it needs a board and an embassy executor).
/// use twine_embassy::{LoopEvent, RunUi};
/// use twine_hal::AsyncDisplayDriver;
/// use twine_reactive::UiWaker;
/// use twine_view::{AsyncUi, Wake};
///
/// /// The UI plus the backlight its idle policy dims.
/// struct Device<D: AsyncDisplayDriver> {
///     ui: AsyncUi<D>,
///     backlight_pct: u8, // e.g. a PWM channel
/// }
///
/// impl<D: AsyncDisplayDriver> RunUi for Device<D> {
///     async fn step(&mut self) -> Wake {
///         self.ui.update_async().await
///     }
///     fn waker(&self) -> &'static UiWaker {
///         self.ui.waker()
///     }
///     async fn wait_input(&mut self) {
///         self.ui.wait_input().await;
///     }
/// }
///
/// async fn main_task<D: AsyncDisplayDriver>(device: Device<D>) -> ! {
///     twine_embassy::run_with(device, |dev, event| match event {
///         LoopEvent::Idle(None) => dev.backlight_pct = 10,
///         LoopEvent::Woken => dev.backlight_pct = 100,
///         _ => {}
///     })
///     .await
/// }
/// ```
#[allow(async_fn_in_trait)] // only awaited by this crate's loops, in the caller's task
pub trait RunUi {
    /// One update (and the frame it renders); returns when to run again.
    async fn step(&mut self) -> Wake;

    /// The UI's waker: the loop sleeps until it is set (or a deadline or input interrupt).
    fn waker(&self) -> &'static UiWaker;

    /// Completes when an input interrupt asks for the next update (and tells it to read the
    /// inputs); never completes for a UI without an interrupt-driven input.
    async fn wait_input(&mut self);
}

/// The blocking runtime on embassy: its update runs to completion in the task (rendering and
/// flushing with blocking drivers, e.g. into a framebuffer). Build it with
/// [`with_embassy_platform`](UiBuilderExt::with_embassy_platform) (its deadlines are then
/// `embassy-time` instants). It has no async input: an input interrupt wakes the loop through
/// the [waker](Ui::waker) — e.g. a task awaiting the touch controller's interrupt pin calls
/// `waker.wake()`.
impl RunUi for Ui {
    #[inline]
    fn step(&mut self) -> impl Future<Output = Wake> {
        // The update runs to completion when called; the future only hands its result over.
        core::future::ready(self.update())
    }

    #[inline]
    fn waker(&self) -> &'static UiWaker {
        Ui::waker(self)
    }

    async fn wait_input(&mut self) {
        core::future::pending::<()>().await;
    }
}

/// The async runtime: each frame is flushed while the next chunk renders, and the loop also
/// wakes on the interrupt of the input registered with `AsyncUiBuilder::input_wait`.
impl<D: AsyncDisplayDriver, W: AsyncInputWait> RunUi for AsyncUi<D, W> {
    #[inline]
    async fn step(&mut self) -> Wake {
        self.update_async().await
    }

    #[inline]
    fn waker(&self) -> &'static UiWaker {
        AsyncUi::waker(self)
    }

    async fn wait_input(&mut self) {
        AsyncUi::wait_input(self).await;
    }
}

/// Runs the UI ([`Ui`] or [`AsyncUi`]) forever: [`run_with`] without a callback.
///
/// ```no_run
/// # async fn example(display: impl twine_hal::AsyncDisplayDriver + 'static, buf: &'static mut [u8]) -> ! {
/// use twine_embassy::UiBuilderExt;
/// use twine_view::prelude::*;
/// let ui = Ui::builder_async(display)
///     .runtime(Runtime::take().expect("runtime already taken"))
///     .buffers(BufferMode::partial_single(buf))
///     .with_embassy_platform()
///     .build(|_| label("hi"));
/// twine_embassy::run(ui).await
/// # }
/// ```
pub async fn run<U: RunUi>(ui: U) -> ! {
    run_with(ui, |_, _| {}).await
}

/// Runs the UI forever, reporting each step of the loop to `on_event` with the UI (see
/// [`LoopEvent`]): `BeforeUpdate`, the update, then — unless it returned [`Wake::Now`] —
/// `Idle(deadline)` (or `Inactive(d)` for [`Wake::IdleFor`], with `EngineConfig::idle_timeout`),
/// the sleep and `Woken`.
///
/// The sleep: until the UI's deadline ([`Wake::At`]), a waker signal (channel message,
/// `notify_input`) or an input interrupt ([`RunUi::wait_input`]); with [`Wake::Idle`] only the
/// latter two wake it (no timer is armed). [`Wake::Now`] yields to the other tasks
/// (`embassy_futures::yield_now`) and updates again: the UI returns it only while work is
/// pending, and every update makes progress on it, so the loop never spins while idle and
/// never starves other tasks. Allocates nothing per iteration; never panics (unless
/// `on_event` does).
///
/// ```no_run
/// # async fn example(display: impl twine_hal::AsyncDisplayDriver + 'static, buf: &'static mut [u8]) -> ! {
/// use twine_embassy::{LoopEvent, UiBuilderExt};
/// use twine_view::prelude::*;
/// let ui = Ui::builder_async(display)
///     .runtime(Runtime::take().expect("runtime already taken"))
///     .buffers(BufferMode::partial_single(buf))
///     .with_embassy_platform()
///     .build(|_| label("hi"));
/// twine_embassy::run_with(ui, |ui, event| match event {
///     LoopEvent::Idle(None) => { /* nothing scheduled: e.g. dim the backlight */ }
///     LoopEvent::Woken => { /* full brightness again */ }
///     LoopEvent::BeforeUpdate if ui.take_faults().contains(FaultKind::FlushTimeout) => {
///         ui.recover_display(); // e.g. after re-initialising the panel (`ui.display_driver_mut()`)
///     }
///     _ => {}
/// })
/// .await
/// # }
/// ```
pub async fn run_with<U, F>(mut ui: U, mut on_event: F) -> !
where
    U: RunUi,
    F: FnMut(&mut U, LoopEvent),
{
    twine_core::info!(target: "twine::embassy", "run loop started");
    loop {
        on_event(&mut ui, LoopEvent::BeforeUpdate);
        let deadline = match ui.step().await {
            Wake::Now => {
                embassy_futures::yield_now().await;
                continue;
            }
            Wake::At(t) => Some(t),
            Wake::Idle => None,
            Wake::IdleFor(inactive) => {
                // No input for `idle_timeout`: the same sleep, reported as `Inactive`.
                on_event(&mut ui, LoopEvent::Inactive(inactive));
                twine_core::trace!(target: "twine::embassy", "inactive: waiting for input or waker");
                select(wait_waker(ui.waker()), ui.wait_input()).await;
                on_event(&mut ui, LoopEvent::Woken);
                continue;
            }
        };
        on_event(&mut ui, LoopEvent::Idle(deadline));
        let waker = ui.waker();
        if let Some(t) = deadline {
            select3(
                embassy_time::Timer::at(to_embassy(t)),
                wait_waker(waker),
                ui.wait_input(),
            )
            .await;
        } else {
            twine_core::trace!(target: "twine::embassy", "idle: waiting for input or waker");
            select(wait_waker(waker), ui.wait_input()).await;
        }
        on_event(&mut ui, LoopEvent::Woken);
    }
}
