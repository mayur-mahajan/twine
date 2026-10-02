//! The async runtime (feature `async`): [`AsyncUi`] renders frames chunk by chunk and flushes
//! them through an [`AsyncDisplayDriver`], overlapping the transfer of one chunk (DMA) with the
//! rendering of the next.
//!
//! Built with [`Ui::builder_async`]; run by `twine-embassy` (or any executor: call
//! [`update_async`](AsyncUi::update_async) and sleep as the returned [`Wake`] says).
//!
//! Waiting for a flush is cooperative by construction: the flush future is awaited, so the
//! executor runs other tasks meanwhile (`EngineConfig::cooperative_flush` does not apply).
//!
//! Every flush is bounded by `EngineConfig::flush_timeout` (see [`AsyncUi`] § Bounded flushes).

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;

use twine_core::fault::{FaultCounts, FaultKind, Faults};
use twine_core::{Duration, Fraction, Instant, Rect, Rotation};
use twine_engine::{
    BufferMode, DisplayCmd, DisplayId, DisplayRequests, DisplayResponses, DriverErrorCode, Engine,
    EngineConfig, EngineError, FaultHook, FaultRecord, IntoTheme, Wake,
};
use twine_hal::{
    AsyncDisplayDriver, AsyncInputWait, AsyncPlatform, DrawBufferMem, InputData, InputDevice, InputKind,
    PollHint,
};
use twine_reactive::{Channel, Runtime, Scope, UiWaker};

use crate::Ui;
use crate::config::AppConfig;
use crate::display::DisplayParts;
use crate::error::UiError;
use crate::typestate::{HasBuffers, HasPlatform, HasRuntime, NoBuffers, NoPlatform, NoRuntime};
#[cfg(doc)]
use crate::ui::UiBuilder;
use crate::ui::{Settings, UiCore};
use crate::view::View;

/// The input waited on by an [`AsyncUi`] without [`input_wait`](AsyncUiBuilder::input_wait):
/// never signals.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoInputWait;

impl AsyncInputWait for NoInputWait {
    async fn wait_for_interrupt(&mut self) {
        core::future::pending::<()>().await;
    }
}

/// An input device shared between the engine (which reads it) and the [`AsyncUi`] (which
/// waits for its interrupt while the UI sleeps; never at the same time).
struct SharedInput<W>(Rc<RefCell<W>>);

impl<W: InputDevice> InputDevice for SharedInput<W> {
    fn kind(&self) -> InputKind {
        self.0.borrow().kind()
    }
    fn read(&mut self) -> InputData {
        self.0.borrow_mut().read()
    }
    fn poll_hint(&self) -> PollHint {
        self.0.borrow().poll_hint()
    }
    fn rearm(&mut self) {
        self.0.borrow_mut().rearm();
    }
    fn health(&self) -> twine_hal::DeviceHealth {
        self.0.borrow().health()
    }
    fn fit_to_display(&mut self, info: &twine_hal::DisplayInfo) {
        self.0.borrow_mut().fit_to_display(info);
    }
}

/// Configures and builds an [`AsyncUi`] (see [`Ui::builder_async`]). The methods are those of
/// [`UiBuilder`], plus [`input_wait`](Self::input_wait); the platform is an
/// [`AsyncPlatform`] ([`platform`](Self::platform)) instead of a clock.
///
/// The required parts are type parameters ([`typestate`](crate::typestate)): `R` the reactive
/// runtime ([`runtime`](Self::runtime)), `P` the platform ([`platform`](Self::platform)), `B`
/// the draw buffers ([`buffers`](Self::buffers)); [`build`](Self::build) /
/// [`try_build`](Self::try_build) exist only once all three are given.
///
/// ```compile_fail,E0277
/// // No platform: "the `AsyncUi` has no platform".
/// # use twine_core::{ColorFormat, Rect};
/// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
/// # use twine_view::prelude::*;
/// # struct Panel;
/// # impl AsyncDisplayDriver for Panel {
/// #     type Error = ();
/// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
/// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
/// # }
/// let ui = Ui::builder_async(Panel)
///     .runtime(Runtime::take().unwrap())
///     .buffers(BufferMode::alloc(BufferSpec::default()))
///     .build(|_| label("hi"));
/// ```
///
/// ```compile_fail,E0277
/// // No draw buffers: "the display has no draw buffers".
/// # use twine_core::{ColorFormat, Rect};
/// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
/// # use twine_testing::MockPlatform;
/// # use twine_view::prelude::*;
/// # struct Panel;
/// # impl AsyncDisplayDriver for Panel {
/// #     type Error = ();
/// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
/// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
/// # }
/// let ui = Ui::builder_async(Panel)
///     .runtime(Runtime::take().unwrap())
///     .platform(&MockPlatform::new())
///     .build(|_| label("hi"));
/// ```
///
/// ```compile_fail,E0277
/// // No runtime: "the `Ui` has no reactive runtime".
/// # use twine_core::{ColorFormat, Rect};
/// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
/// # use twine_testing::MockPlatform;
/// # use twine_view::prelude::*;
/// # struct Panel;
/// # impl AsyncDisplayDriver for Panel {
/// #     type Error = ();
/// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
/// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
/// # }
/// let ui = Ui::builder_async(Panel)
///     .platform(&MockPlatform::new())
///     .buffers(BufferMode::alloc(BufferSpec::default()))
///     .build(|_| label("hi"));
/// ```
#[must_use]
pub struct AsyncUiBuilder<D, W = NoInputWait, R = NoRuntime, P = NoPlatform, B = NoBuffers> {
    display: D,
    parts: DisplayParts,
    settings: Settings,
    runtime: R,
    platform: P,
    buffers: B,
    wait: Option<Rc<RefCell<W>>>,
}

impl<D, W, R, P, B> core::fmt::Debug for AsyncUiBuilder<D, W, R, P, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsyncUiBuilder")
            .field("app", &self.settings.app)
            .field("input_wait", &self.wait.is_some())
            .finish_non_exhaustive()
    }
}

impl Ui {
    /// A builder for an async display ([`AsyncUi`]): partial buffers (two for DMA overlap),
    /// flushed through `display` chunk by chunk.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_double(buf(), buf())) // DMA overlap
    ///     .platform(&MockPlatform::new())
    ///     .build(|_| label("hi"));
    /// assert!(ui.display_health().is_some());
    /// ```
    pub fn builder_async<D: AsyncDisplayDriver + 'static>(display: D) -> AsyncUiBuilder<D> {
        AsyncUiBuilder {
            display,
            parts: DisplayParts::default(),
            settings: Settings::default(),
            runtime: NoRuntime,
            platform: NoPlatform,
            buffers: NoBuffers,
            wait: None,
        }
    }
}

impl<D, W, R, P, B> AsyncUiBuilder<D, W, R, P, B> {
    /// The draw buffers, **required** (as for [`Ui::builder`]): `static` buffers declared with
    /// [`draw_buffers!`](crate::draw_buffers) and passed with
    /// [`BufferMode::partial_double_from`] (render one chunk while the other is transferred) or
    /// [`BufferMode::partial_single_from`]; any `'static` memory
    /// ([`BufferMode::partial_double`] / [`partial_single`](BufferMode::partial_single)); or heap
    /// buffers ([`BufferMode::alloc`], allocated only once the display has been accepted).
    /// Without buffers, [`try_build`](Self::try_build) fails with
    /// [`EngineError::InvalidConfig`]; with `Full` / `Direct` it fails with
    /// [`EngineError::BufferModeMismatch`].
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn buffers(self, m: BufferMode) -> AsyncUiBuilder<D, W, R, P, BufferMode> {
        AsyncUiBuilder {
            display: self.display,
            parts: self.parts,
            settings: self.settings,
            runtime: self.runtime,
            platform: self.platform,
            buffers: m,
            wait: self.wait,
        }
    }

    /// Adds an input device read by the engine (see [`UiBuilder::input`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// use twine_testing::MockPointer;
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).input(MockPointer::new()).build(|_| label("hi"));
    /// assert_eq!(ui.engine().inputs().count(), 1);
    /// ```
    pub fn input(mut self, d: impl InputDevice + 'static) -> Self {
        self.parts.input(d);
        self
    }

    /// Attaches a draw accelerator that renders into the chunks of this UI (see
    /// [`UiBuilder::accel`]), taking ownership of it. Default: software only.
    ///
    /// ```
    /// # use twine_core::{Color, ColorFormat, Opa, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// # struct Decline;
    /// # impl DrawAccel for Decline {
    /// #     fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult { AccelResult::Unsupported }
    /// #     fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult { AccelResult::Unsupported }
    /// #     fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult { AccelResult::Unsupported }
    /// #     fn wait(&mut self) {}
    /// # }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .accel(Decline)
    ///     .build(|_| label("hi"));
    /// assert!(ui.engine().has_accel());
    /// ```
    pub fn accel(mut self, accel: impl twine_render::DrawAccel + 'static) -> Self {
        self.settings.accel(accel);
        self
    }

    /// Caller memory for the engine's layer buffer, as
    /// [`UiBuilder::layer_buf`](crate::UiBuilder::layer_buf) (typically a
    /// [`LayerBuffer`](crate::LayerBuffer) static; without it the layer buffer is heap). A
    /// later call replaces it; shorter than 4 KiB fails the build. Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// static LAYER: LayerBuffer<4096> = LayerBuffer::zeroed();
    /// let ui = Ui::builder_async(Panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .platform(&MockPlatform::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
    ///     .layer_buf(LAYER.take().unwrap())
    ///     .build(|_| label("hi"));
    /// assert!(ui.memory_report().engine.layer_buf_static);
    /// ```
    pub fn layer_buf(mut self, buf: &'static mut [u8]) -> Self {
        self.settings.layer_buf = Some(buf);
        self
    }

    /// Adds the input device whose interrupt wakes the sleeping UI (e.g. a touch controller
    /// with its IRQ pin); it is also read by the engine like [`input`](Self::input).
    /// The device implements both [`InputDevice`] and [`AsyncInputWait`]; the `AsyncUi` waits
    /// for its interrupt in [`AsyncUi::wait_input`].
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// use twine_hal::{AsyncInputWait, InputData, InputDevice, InputKind};
    /// use twine_testing::MockPointer;
    ///
    /// /// A touch controller whose interrupt line wakes the UI.
    /// struct Touch(MockPointer);
    /// impl InputDevice for Touch {
    ///     fn kind(&self) -> InputKind { self.0.kind() }
    ///     fn read(&mut self) -> InputData { self.0.read() }
    /// }
    /// impl AsyncInputWait for Touch {
    ///     async fn wait_for_interrupt(&mut self) { /* await the IRQ pin */ }
    /// }
    ///
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .input_wait(Touch(MockPointer::new()))
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.engine().inputs().count(), 1);
    /// ```
    pub fn input_wait<T: InputDevice + AsyncInputWait + 'static>(
        mut self,
        d: T,
    ) -> AsyncUiBuilder<D, T, R, P, B> {
        let shared = Rc::new(RefCell::new(d));
        self.parts.input(SharedInput(shared.clone()));
        AsyncUiBuilder {
            display: self.display,
            parts: self.parts,
            settings: self.settings,
            runtime: self.runtime,
            platform: self.platform,
            buffers: self.buffers,
            wait: Some(shared),
        }
    }

    /// The platform (required): a clone of `platform` becomes the `AsyncUi`'s clock and the
    /// timer that bounds every flush with `EngineConfig::flush_timeout` (see the
    /// [`AsyncUi`] § Bounded flushes), and
    /// [`P::in_interrupt`](AsyncPlatform::in_interrupt) the reactive runtime's interrupt probe
    /// ([`twine_reactive::set_interrupt_probe`]). Typically `twine_embassy`'s
    /// `EmbassyPlatform`; in tests, `twine_testing::MockPlatform`. A later call replaces it.
    /// Never panics; the clone is boxed once.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let platform = MockPlatform::new();
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&platform)
    ///     .build(|_| label("hi"));
    /// platform.advance(Duration::ms(5));
    /// assert_eq!(ui.now(), Instant::from_millis(5));
    /// ```
    pub fn platform<Q: AsyncPlatform + Clone + 'static>(
        mut self,
        platform: &Q,
    ) -> AsyncUiBuilder<D, W, R, Box<dyn AsyncPlatform>, B> {
        self.settings.interrupt_probe = Some(Q::in_interrupt);
        AsyncUiBuilder {
            display: self.display,
            parts: self.parts,
            settings: self.settings,
            runtime: self.runtime,
            platform: Box::new(platform.clone()),
            buffers: self.buffers,
            wait: self.wait,
        }
    }

    /// The theme of the display.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).theme(DefaultTheme::dark()).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn theme(mut self, t: impl IntoTheme) -> Self {
        self.settings.app.theme = Some(t.into_theme());
        self
    }

    /// The engine configuration.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// use twine_engine::EngineConfig;
    /// let config = EngineConfig { max_nodes: 256, ..EngineConfig::default() };
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).config(config).build(|_| label("hi"));
    /// assert_eq!(ui.engine().config().max_nodes, 256);
    /// ```
    pub fn config(mut self, c: EngineConfig) -> Self {
        self.settings.app.engine = c;
        self
    }

    /// The `AsyncUi`'s waker, the application's own instead of a pooled one (see
    /// [`UiBuilder::waker`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// static WAKER: UiWaker = UiWaker::new();
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).waker(&WAKER).build(|_| label("hi"));
    /// assert!(core::ptr::eq(ui.waker(), &WAKER));
    /// ```
    pub fn waker(mut self, waker: &'static UiWaker) -> Self {
        self.settings.waker(waker);
        self
    }

    /// The function called for every fault (see [`UiBuilder::fault_hook`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// fn on_fault(r: &FaultRecord) {
    ///     let _ = r; // e.g. count it, or signal a supervisor through a `static`
    /// }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).fault_hook(on_fault).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn fault_hook(mut self, hook: FaultHook) -> Self {
        self.settings.app.fault_hook = Some(hook);
        self
    }

    /// How many engine commands the `AsyncUi` can queue while the engine is not lent (see
    /// [`UiBuilder::engine_queue_capacity`]): a full queue drops the command and raises a
    /// [`FaultKind::Capacity`] fault with code
    /// [`CapacityFault::EngineQueue`](crate::CapacityFault::EngineQueue); `0` drops (and
    /// reports) every such command.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).engine_queue_capacity(64).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn engine_queue_capacity(mut self, capacity: usize) -> Self {
        self.settings.app.engine_queue_capacity = capacity;
        self
    }

    /// How many messages each channel delivers per update (see
    /// [`UiBuilder::messages_per_channel`]; default
    /// [`DEFAULT_MESSAGES_PER_CHANNEL`](crate::DEFAULT_MESSAGES_PER_CHANNEL), `0` counts as `1`).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).messages_per_channel(4).build(|_| label("hi"));
    /// assert_eq!(ui.messages_per_channel(), 4);
    /// ```
    pub fn messages_per_channel(mut self, n: usize) -> Self {
        self.settings.app.messages_per_channel = n;
        self
    }

    /// The initial motion preference (see [`UiBuilder::motion`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).motion(Motion::Reduced).build(|_| label("hi"));
    /// assert_eq!(ui.engine().motion(), Motion::Reduced);
    /// ```
    pub fn motion(mut self, motion: twine_anim::Motion) -> Self {
        self.settings.app.motion = motion;
        self
    }

    /// The reactive runtime of the calling execution context (required; see
    /// [`UiBuilder::runtime`]): the `AsyncUi` keeps the token, which keeps it and every
    /// reactive handle of the application in this task. Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let rt = Runtime::take().expect("runtime already taken"); // once, in the UI task
    /// let ui = Ui::builder_async(Panel)
    ///     .runtime(rt)
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.root_scope().runtime(), rt);
    /// ```
    pub fn runtime(self, rt: Runtime) -> AsyncUiBuilder<D, W, Runtime, P, B> {
        AsyncUiBuilder {
            display: self.display,
            parts: self.parts,
            settings: self.settings,
            runtime: rt,
            platform: self.platform,
            buffers: self.buffers,
            wait: self.wait,
        }
    }

    /// Applies the [`DisplayCmd`]s sent to `commands` from any context (see
    /// [`UiBuilder::display_commands`]); the `AsyncUi` applies them to its async driver
    /// between frames.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// static DISPLAY: Channel<DisplayCmd, 4> = Channel::new();
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).platform(&MockPlatform::new()).display_commands(&DISPLAY).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn display_commands<const N: usize>(mut self, commands: &'static Channel<DisplayCmd, N>) -> Self {
        self.parts.display_commands(commands);
        self
    }

    /// Reserves the memory software rotation needs in every rotation (see
    /// [`UiBuilder::reserve_rotation`]), for async drivers without hardware rotation.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::alloc(BufferSpec::default())).platform(&MockPlatform::new()).reserve_rotation().build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn reserve_rotation(mut self) -> Self {
        self.parts.reserve_rotation();
        self
    }

    /// The application's configuration (see [`UiBuilder::app_config`]): replaces every
    /// setting it covers; the methods called afterwards refine it.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .app_config(AppConfig::new().theme(DefaultTheme::dark()).motion(Motion::Reduced))
    ///     .build(|_| label("hi"));
    /// assert_eq!(ui.engine().motion(), Motion::Reduced);
    /// ```
    pub fn app_config(mut self, config: AppConfig) -> Self {
        self.settings.app = config;
        self
    }

    /// The rotation the screens are designed for (see [`UiBuilder::rotation`]); the
    /// `AsyncUi` asks its driver before the first frame (software rotation needs
    /// [`reserve_rotation`](Self::reserve_rotation)).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .buffers(BufferMode::partial_single(buf()))
    ///     .platform(&MockPlatform::new())
    ///     .rotation(Rotation::Deg180)
    ///     .build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.settings.app.rotation = Some(rotation);
        self
    }

    /// Builds the `AsyncUi` (engine, display, theme, inputs; `app` built once). Exists only
    /// once the runtime, the platform and the draw buffers are given
    /// ([`typestate`](crate::typestate)).
    ///
    /// # Errors
    /// What only the device can tell: [`UiError::Engine`] with
    /// [`EngineError::BufferModeMismatch`](twine_engine::EngineError::BufferModeMismatch) for
    /// `Full` / `Direct` buffers,
    /// [`EngineError::BufferMisaligned`](twine_engine::EngineError::BufferMisaligned) for
    /// misaligned caller memory, the engine's errors for the display and inputs. Heap buffers
    /// ([`BufferMode::alloc`]) are allocated only after the display has been accepted.
    /// [`UiError::Build`]: a widget of `app` could not be created (see
    /// [`UiBuilder::try_build`](crate::UiBuilder::try_build)).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let rt = Runtime::take().unwrap();
    /// let full = Ui::builder_async(Panel).runtime(rt).buffers(BufferMode::Full).platform(&MockPlatform::new()).try_build(|_| label("hi"));
    /// assert!(full.is_err()); // an async display renders in chunks
    /// assert!(Ui::builder_async(Panel).runtime(rt).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).try_build(|_| label("hi")).is_ok());
    /// ```
    pub fn try_build<V: View>(self, app: impl FnOnce(Scope) -> V) -> Result<AsyncUi<D, W>, UiError>
    where
        D: AsyncDisplayDriver + 'static,
        W: AsyncInputWait + 'static,
        R: HasRuntime,
        P: HasPlatform,
        B: HasBuffers,
    {
        let buffers = self.buffers.into_part();
        let platform = self.platform.into_part();
        // Sized (and caller memory checked) without allocating; heap buffers are allocated
        // below, once the engine has accepted the display (F10b).
        let info = self.display.info();
        let chunk_bytes = buffers.partial_bytes(&info)?;
        let parts = self.parts;
        let (engine, core) = self.settings.build(
            self.runtime.into_part(),
            |e| {
                let display = e.add_chunked_display(info, chunk_bytes)?;
                Ok((display, parts.install(e, display)?))
            },
            app,
        )?;
        let bufs_heap = matches!(buffers, BufferMode::Alloc(_));
        let (a, b) = buffers.into_partial(&info)?;
        twine_core::info!(
            target: "twine::view",
            "async ui: {} buffer(s) of {} B",
            if b.is_some() { 2 } else { 1 },
            chunk_bytes
        );
        Ok(AsyncUi {
            engine,
            core,
            platform,
            display: self.display,
            bufs: (a, b),
            bufs_heap,
            wait: self.wait,
        })
    }

    /// [`try_build`](Self::try_build), panicking on an error.
    ///
    /// # Panics
    /// When the engine rejects the display, buffers or inputs, or when a widget of `app`
    /// cannot be created. A missing runtime, platform or buffers is a compile error.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// # let _ = ui;
    /// ```
    pub fn build<V: View>(self, app: impl FnOnce(Scope) -> V) -> AsyncUi<D, W>
    where
        D: AsyncDisplayDriver + 'static,
        W: AsyncInputWait + 'static,
        R: HasRuntime,
        P: HasPlatform,
        B: HasBuffers,
    {
        match self.try_build(app) {
            Ok(ui) => ui,
            Err(e) => crate::error::build_failed("AsyncUi", &e),
        }
    }
}

/// The declarative UI runtime on an async display: like [`Ui`], but each frame is rendered
/// chunk by chunk and flushed with [`AsyncDisplayDriver::flush`]; with two buffers the flush
/// of one chunk (DMA) runs while the next chunk renders.
///
/// # Bounded flushes
///
/// The `AsyncUi`'s time source is an [`AsyncPlatform`]
/// ([`AsyncUiBuilder::platform`]): its clock times the updates, and its timer bounds every
/// flush with `EngineConfig::flush_timeout`. A flush still pending after its first poll is
/// raced against the deadline (measured from that poll, which starts the transfer); when the
/// deadline wins, the flush future is dropped (an embassy DMA transfer is aborted on drop),
/// and [`Engine::report_flush_timeout`] applies exactly what the blocking path does on a
/// timeout: [`FaultKind::FlushTimeout`] (code = ms waited), the display
/// [`Failed`](twine_engine::DisplayState::Failed) (halted under
/// [`FlushPolicy::Halt`](twine_engine::FlushPolicy::Halt)), the chunks rendered but not shown
/// redrawn later (unless [`FlushPolicy::Ignore`](twine_engine::FlushPolicy::Ignore)), the rest
/// of the frame abandoned, and the next frame no earlier than `refr_period` later. The next
/// successful flush makes the display healthy again. A flush that completes on its first poll
/// costs nothing extra (the clock is not even read).
///
/// A side effect of an executor timer that cannot be cancelled (embassy's): once a flush was
/// pending, its deadline wakes the UI task once more after the flush completed (one empty
/// poll, `flush_timeout` after the last pending flush), then the task sleeps again.
pub struct AsyncUi<D: AsyncDisplayDriver, W: AsyncInputWait = NoInputWait> {
    engine: Engine,
    core: UiCore,
    platform: Box<dyn AsyncPlatform>,
    display: D,
    bufs: (DrawBufferMem, Option<DrawBufferMem>),
    /// `bufs` were allocated by `BufferMode::Alloc` (for `memory_report`).
    bufs_heap: bool,
    wait: Option<Rc<RefCell<W>>>,
}

impl<D: AsyncDisplayDriver, W: AsyncInputWait> core::fmt::Debug for AsyncUi<D, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsyncUi")
            .field("engine", &self.engine)
            .field("core", &self.core)
            .field("double_buffered", &self.bufs.1.is_some())
            .finish_non_exhaustive()
    }
}

impl<D: AsyncDisplayDriver, W: AsyncInputWait> Drop for AsyncUi<D, W> {
    fn drop(&mut self) {
        self.core.shut_down(&mut self.engine);
    }
}

/// Applies `req` to the async driver in the engine's order (rotation, brightness, sleep) and
/// collects its answers (errors as their codes).
async fn apply_requests<D: AsyncDisplayDriver>(display: &mut D, req: DisplayRequests) -> DisplayResponses {
    let code =
        |d: &D, e: twine_hal::ControlError<D::Error>| e.map(|e| DriverErrorCode::new(d.error_code(&e)));
    let mut resp = DisplayResponses::default();
    if let Some(r) = req.rotation {
        let res = display.set_rotation(r).await;
        resp.rotation = Some(res.map_err(|e| code(display, e)));
    }
    if let Some(l) = req.brightness {
        let res = display.set_brightness(l).await;
        resp.brightness = Some(res.map_err(|e| code(display, e)));
    }
    if let Some(sl) = req.sleep {
        let res = display.sleep(sl).await;
        resp.sleep = Some(res.map_err(|e| code(display, e)));
    }
    resp
}

/// Polls `fut` once with the current task's context: `Some(output)` if it completed.
async fn poll_once<F: Future + Unpin>(fut: &mut F) -> Option<F::Output> {
    poll_fn(|cx| {
        Poll::Ready(match core::pin::Pin::new(&mut *fut).poll(cx) {
            Poll::Ready(v) => Some(v),
            Poll::Pending => None,
        })
    })
    .await
}

impl<D: AsyncDisplayDriver, W: AsyncInputWait> AsyncUi<D, W> {
    /// One update (see [`UiCore::update`]), then the frame, if one is due: rendered chunk by
    /// chunk, each chunk's flush started before the next chunk renders. Returns when to run
    /// again.
    /// Never panics; a failed flush is reported to the engine ([`FaultKind::FlushError`],
    /// [`Engine::report_flush`]) and its area redrawn later; a flush still pending after
    /// `EngineConfig::flush_timeout` is dropped and reported ([`FaultKind::FlushTimeout`],
    /// [`Engine::report_flush_timeout`]; see [`AsyncUi`] § Bounded flushes).
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let mut update = core::pin::pin!(ui.update_async());
    /// // Any executor works; this panel's flush completes at once, so one poll finishes the frame.
    /// let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
    /// assert!(update.as_mut().poll(&mut cx).is_ready());
    /// ```
    pub async fn update_async(&mut self) -> Wake {
        let now = self.platform.now();
        let wake = self.core.update(&mut self.engine, now);
        let did = self.core.display();
        if let Some(req) = self.engine.take_display_requests(did, now) {
            // Brightness, sleep, rotation: between frames, nothing in flight.
            let resp = apply_requests(&mut self.display, req).await;
            if let Some(r) = self.engine.complete_display_requests(did, req, resp, now) {
                // A hardware rotation was refused by the engine: turn the panel back.
                let _ = self.display.set_rotation(r).await;
            }
            // Update again at once: it schedules the redraw (rotation), the settle time (wake)
            // and the requests a wake left for after it.
            return Wake::Now;
        }
        if self.engine.refresh_begin(now).is_some() {
            self.render_frame().await;
            self.engine.refresh_end();
            // A failed flush marks its area dirty again (`FlushPolicy::Reinvalidate`): wake up
            // for the frame that redraws it.
            if let Some(due) = self.engine.refresh_due() {
                return wake.min(Wake::At(due));
            }
            // Nothing more to draw: let the driver save power.
            self.display.idle().await;
        }
        wake
    }

    /// Renders and flushes the frame begun by `refresh_begin`; every flush still pending after
    /// its first poll is bounded by `flush_timeout` on the platform's timer.
    async fn render_frame(&mut self) {
        let Self {
            engine,
            display,
            bufs,
            core,
            platform,
            ..
        } = self;
        let platform: &mut dyn AsyncPlatform = &mut **platform;
        let did = core.display();
        let timer = engine.config().hires_timer;
        let timeout = engine.config().flush_timeout;
        let us = move || timer.map(|f| f().as_micros());
        let (a, b) = bufs;
        let Some(b) = b.as_mut() else {
            // One buffer: render, flush, repeat (no overlap: all flush time is waiting).
            while let Some(area) = engine.render_chunk(a.as_mut_slice()) {
                let t0 = us();
                let result = {
                    let mut flush = pin!(display.flush(area, a.as_slice()));
                    if let Some(r) = poll_once(&mut flush).await {
                        Ok(r)
                    } else {
                        let since = Pending::since(platform, timeout);
                        finish_flush(&mut flush, platform, since).await
                    }
                };
                let t = elapsed(t0, us());
                engine.refresh_add_flush_time(t, t);
                match result {
                    Ok(result) => report(engine, display, did, area, result),
                    // The frame is abandoned: `render_chunk` returns `None` from now on.
                    Err(waited) => engine.report_flush_timeout(did, &[area], waited),
                }
            }
            return;
        };
        // Two buffers: the flush of chunk k (DMA) overlaps the rendering of chunk k + 1.
        let mut bufs = [a, b];
        let mut pending: Option<(Rect, usize)> = None;
        let mut free = 0;
        loop {
            let next = match pending.take() {
                None => engine.render_chunk(bufs[free].as_mut_slice()),
                Some((area, prev)) => {
                    let (lo, hi) = bufs.split_at_mut(1);
                    let (prev_buf, free_buf) = if prev == 0 {
                        (&*lo[0], &mut *hi[0])
                    } else {
                        (&*hi[0], &mut *lo[0])
                    };
                    let t0 = us();
                    let (next, t1, result) = {
                        let mut flush = pin!(display.flush(area, prev_buf.as_slice()));
                        // The first poll starts the transfer (DMA); render while it runs.
                        let done = poll_once(&mut flush).await;
                        let since = if done.is_none() {
                            Pending::since(platform, timeout)
                        } else {
                            Pending::default()
                        };
                        let next = engine.render_chunk(free_buf.as_mut_slice());
                        let t1 = us();
                        let result = match done {
                            Some(r) => Ok(r),
                            None => finish_flush(&mut flush, platform, since).await,
                        };
                        (next, t1, result)
                    };
                    let t2 = us();
                    engine.refresh_add_flush_time(elapsed(t0, t2), elapsed(t1, t2));
                    match result {
                        Ok(result) => report(engine, display, did, area, result),
                        Err(waited) => {
                            // The chunk rendered meanwhile was never sent either: both are
                            // redrawn by a later frame, and the rest of this one is abandoned.
                            match next {
                                Some(n) => engine.report_flush_timeout(did, &[area, n], waited),
                                None => engine.report_flush_timeout(did, &[area], waited),
                            }
                            return;
                        }
                    }
                    next
                }
            };
            match next {
                Some(area) => {
                    pending = Some((area, free));
                    free = 1 - free;
                }
                None => break,
            }
        }
    }

    /// Completes when the input registered with [`AsyncUiBuilder::input_wait`] signals its
    /// interrupt (never without one, and never while the engine is still polling it — e.g. a
    /// pressed touch screen, which the engine reads on its own deadline); tells the next update
    /// to read the inputs.
    /// ```no_run
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// async fn run(mut ui: AsyncUi<Panel>) -> ! {
    ///     loop {
    ///         let _wake = ui.update_async().await;
    ///         // A real loop races this with a timer for `_wake` and with `ui.waker()`
    ///         // (`twine-embassy` implements it); here it sleeps until the next interrupt.
    ///         ui.wait_input().await;
    ///     }
    /// }
    /// ```
    ///
    /// (`no_run`: it needs an executor and an interrupt-driven input.)
    #[allow(clippy::await_holding_refcell_ref)] // see below: the borrow never overlaps a read
    pub async fn wait_input(&mut self) {
        match &self.wait {
            Some(w) if self.engine.input_deadline().is_none() => {
                // The engine reads the device only inside `update_async`, which never runs while
                // this future is alive (the run loop drops it before the next update).
                w.borrow_mut().wait_for_interrupt().await;
                self.core.notify_input();
            }
            _ => core::future::pending::<()>().await,
        }
    }

    /// The waker, set by channel sends and input notifications (`'static`, usable from
    /// interrupts and other tasks).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let waker: &'static UiWaker = ui.waker(); // e.g. handed to an interrupt handler
    /// waker.wake();
    /// ```
    #[must_use]
    pub fn waker(&self) -> &'static UiWaker {
        self.core.waker()
    }

    /// Messages each channel delivers per update (see [`UiBuilder::messages_per_channel`]).
    #[must_use]
    pub fn messages_per_channel(&self) -> usize {
        self.core.messages_per_channel()
    }

    /// Changes how many messages each channel delivers per update (see
    /// [`UiBuilder::messages_per_channel`]; `0` counts as `1`). Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.set_messages_per_channel(16); // e.g. a burst of sensor samples per update
    /// assert_eq!(ui.messages_per_channel(), 16);
    /// ```
    pub fn set_messages_per_channel(&mut self, n: usize) {
        self.core.set_messages_per_channel(n);
    }

    /// Tells the next update that an input device changed.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.notify_input(); // e.g. from the input task, after a touch interrupt
    /// ```
    pub fn notify_input(&self) {
        self.core.notify_input();
    }

    /// The current time of the platform's clock (see [`AsyncUiBuilder::platform`]). Never
    /// panics itself (it calls the platform's `now`).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// assert_eq!(ui.now(), Instant::from_millis(0));
    /// ```
    #[must_use]
    pub fn now(&self) -> Instant {
        self.platform.now()
    }

    /// What the UI's memory is used for, by part ([`MemoryReport`](crate::MemoryReport)), as
    /// [`Ui::memory_report`]; the async UI's own draw buffers are reported in
    /// `engine.draw_buffers_heap` / `draw_buffers_static`. Allocates nothing; never panics;
    /// O(nodes).
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// let ui = Ui::builder_async(Panel)
    ///     .runtime(Runtime::take().unwrap())
    ///     .platform(&MockPlatform::new())
    ///     .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 }))
    ///     .build(|_| label("hi"));
    /// let m = ui.memory_report();
    /// assert_eq!(m.engine.draw_buffers_heap, 2 * (64 * 2 * 8 + BufferMode::ALLOC_PADDING));
    /// ```
    #[must_use]
    pub fn memory_report(&self) -> crate::MemoryReport {
        let mut m = self.core.memory_report(&self.engine);
        let (a, b) = &self.bufs;
        let bufs = [Some(a), b.as_ref()];
        let bufs = bufs.iter().flatten();
        if self.bufs_heap {
            m.engine.draw_buffers_heap += bufs.map(|b| b.len() + BufferMode::ALLOC_PADDING).sum::<usize>();
        } else {
            m.engine.draw_buffers_static += bufs.map(|b| b.len()).sum::<usize>();
        }
        m
    }

    /// The engine.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// assert!(ui.engine().default_display().is_some());
    /// ```
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The engine, mutably.
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.engine_mut().set_motion(Motion::Reduced);
    /// ```
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The faults raised since the last call, clearing them (see [`Ui::take_faults`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert!(ui.take_faults().contains(FaultKind::Capacity));
    /// assert!(ui.take_faults().is_empty());
    /// ```
    pub fn take_faults(&mut self) -> Faults {
        self.core.take_faults(&mut self.engine)
    }

    /// Occurrences of every fault kind since start-up (see [`Ui::fault_counts`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity).occurrences(2));
    /// assert_eq!(ui.fault_counts().get(FaultKind::Capacity), 2);
    /// ```
    #[must_use]
    pub fn fault_counts(&self) -> FaultCounts {
        self.engine.fault_counts()
    }

    /// The last record of `kind` (see [`Ui::last_fault`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::FlushTimeout).code(500));
    /// assert_eq!(ui.last_fault(FaultKind::FlushTimeout).map(|r| r.code), Some(500));
    /// ```
    #[must_use]
    pub fn last_fault(&self, kind: FaultKind) -> Option<&FaultRecord> {
        self.engine.last_fault(kind)
    }

    /// Sets the function called for every fault (see [`Ui::set_fault_hook`]).
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// fn on_fault(_: &FaultRecord) {}
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// ui.set_fault_hook(Some(on_fault));
    /// ui.set_fault_hook(None); // removed
    /// ```
    pub fn set_fault_hook(&mut self, hook: Option<FaultHook>) {
        self.engine.set_fault_hook(hook);
    }

    /// The flush health of the display (see [`Ui::display_health`]).
    #[must_use]
    pub fn display_health(&self) -> Option<twine_engine::DisplayHealth> {
        self.engine.display_health(self.core.display())
    }

    /// The health of input device `id` (see [`Engine::input_health`]).
    #[must_use]
    pub fn input_health(&self, id: twine_engine::InputId) -> Option<twine_hal::DeviceHealth> {
        self.engine.input_health(id)
    }

    /// Recovers the display after flush failures (see [`Ui::recover_display`]).
    pub fn recover_display(&mut self) {
        let _ = self.engine.recover_display(self.core.display());
    }

    /// Rotates the display at the next update (see [`Ui::set_rotation`]): the async driver's
    /// `set_rotation`, or software rotation (needs [`AsyncUiBuilder::reserve_rotation`]).
    /// Wakes the UI.
    ///
    /// Idempotent; never panics.
    ///
    /// # Errors
    /// None in practice (an `AsyncUi` display is never a framebuffer display); the signature
    /// matches [`Ui::set_rotation`].
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let d = ui.engine().default_display().unwrap();
    /// ui.set_rotation(Rotation::Deg180).unwrap();
    /// // Applied by the next `update().await`, between two frames.
    /// assert!(ui.engine().display_requests_pending(d));
    /// ```
    pub fn set_rotation(&mut self, rotation: Rotation) -> Result<(), UiError> {
        self.engine.set_rotation(self.core.display(), rotation)?;
        self.core.waker().wake();
        Ok(())
    }

    /// Sets the panel brightness at the next update (see [`Ui::set_brightness`]). Wakes the UI.
    /// Idempotent; never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let d = ui.engine().default_display().unwrap();
    /// ui.set_brightness(Fraction::pct(30));
    /// assert!(ui.engine().display_requests_pending(d)); // applied by the next `update().await`
    /// ```
    pub fn set_brightness(&mut self, level: Fraction) {
        let _ = self.engine.set_display_brightness(self.core.display(), level);
        self.core.waker().wake();
    }

    /// Puts the display to sleep or wakes it at the next update (see
    /// [`Ui::set_display_sleep`]). Wakes the UI. Idempotent; never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let d = ui.engine().default_display().unwrap();
    /// ui.set_display_sleep(true);
    /// assert!(!ui.display_asleep()); // requested: asleep after the next `update().await`
    /// assert!(ui.engine().display_requests_pending(d));
    /// ```
    pub fn set_display_sleep(&mut self, sleep: bool) {
        let _ = self.engine.set_display_sleep(self.core.display(), sleep);
        self.core.waker().wake();
    }

    /// Whether the display is asleep (a sleep request was applied by an update). Never
    /// panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// assert!(!ui.display_asleep());
    /// ```
    #[must_use]
    pub fn display_asleep(&self) -> bool {
        self.engine.display_asleep(self.core.display())
    }

    /// The description of the display: its current logical size and rotation. Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&MockPlatform::new()).build(|_| label("hi"));
    /// let info = ui.display_info();
    /// assert_eq!((info.width, info.height), (64, 32));
    /// ```
    #[must_use]
    pub fn display_info(&self) -> twine_hal::DisplayInfo {
        crate::ui::display_info(&self.engine, self.core.display())
    }

    /// The time since the last user input (see [`Ui::inactive_for`]), on the platform's
    /// clock. Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let platform = MockPlatform::new();
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&platform).build(|_| label("hi"));
    /// ui.trigger_activity();
    /// platform.advance(Duration::secs(3));
    /// assert_eq!(ui.inactive_for(), Duration::secs(3));
    /// ```
    #[must_use]
    pub fn inactive_for(&self) -> Duration {
        self.engine.inactive_for(self.platform.now())
    }

    /// Records user input that did not come through an input device (see
    /// [`Ui::trigger_activity`]). Never panics.
    ///
    /// ```
    /// # use twine_core::{ColorFormat, Rect};
    /// # use twine_hal::{AsyncDisplayDriver, DisplayInfo};
    /// # use twine_testing::MockPlatform;
    /// # use twine_view::prelude::*;
    /// # struct Panel;
    /// # impl AsyncDisplayDriver for Panel {
    /// #     type Error = ();
    /// #     fn info(&self) -> DisplayInfo { DisplayInfo::new(64, 32, ColorFormat::Rgb565) }
    /// #     async fn flush(&mut self, _: Rect, _: &[u8]) -> Result<(), ()> { Ok(()) }
    /// # }
    /// # fn buf() -> &'static mut [u8] { Box::leak(vec![0u8; 64 * 2 * 8].into_boxed_slice()) }
    /// let platform = MockPlatform::new();
    /// let mut ui = Ui::builder_async(Panel).runtime(Runtime::take().unwrap()).buffers(BufferMode::partial_single(buf())).platform(&platform).build(|_| label("hi"));
    /// platform.advance(Duration::secs(9));
    /// ui.trigger_activity(); // e.g. a hardware button the application reads itself
    /// assert_eq!(ui.inactive_for(), Duration::ZERO);
    /// ```
    pub fn trigger_activity(&mut self) {
        let now = self.platform.now();
        self.engine.trigger_activity(now);
    }

    /// The display driver (e.g. to change the brightness of an AMOLED).
    pub fn display_driver_mut(&mut self) -> &mut D {
        &mut self.display
    }

    /// The application's root scope.
    #[must_use]
    pub fn root_scope(&self) -> Scope {
        self.core.root_scope()
    }
}

/// When a flush that did not complete on its first poll started, and its deadline (none
/// without a `flush_timeout`).
#[derive(Clone, Copy, Default)]
struct Pending {
    start: Option<Instant>,
    deadline: Option<Instant>,
}

impl Pending {
    /// Stamps a flush found pending now (only then is the clock read).
    fn since(platform: &dyn AsyncPlatform, timeout: Option<Duration>) -> Self {
        let start = platform.now();
        Pending {
            start: Some(start),
            deadline: timeout.map(|t| start + t),
        }
    }
}

/// Awaits a flush already polled once, racing it against its deadline on the platform's
/// timer: `Ok(output)`, or `Err(time waited)` once the deadline passed first (the caller
/// drops the flush future). Executor-agnostic: both are polled with the task's context.
async fn finish_flush<F: Future + Unpin>(
    flush: &mut F,
    platform: &mut dyn AsyncPlatform,
    pending: Pending,
) -> Result<F::Output, Duration> {
    let timed_out = poll_fn(|cx| {
        if let Poll::Ready(v) = core::pin::Pin::new(&mut *flush).poll(cx) {
            return Poll::Ready(Ok(v));
        }
        match pending.deadline {
            Some(d) if platform.poll_wait_until(d, cx).is_ready() => Poll::Ready(Err(())),
            _ => Poll::Pending,
        }
    })
    .await;
    timed_out.map_err(|()| {
        let start = pending.start.unwrap_or_else(|| platform.now());
        platform.now().saturating_duration_since(start)
    })
}

/// Microseconds between two optional timestamps (0 without a timer).
fn elapsed(a: Option<u64>, b: Option<u64>) -> u32 {
    match (a, b) {
        (Some(a), Some(b)) => b.saturating_sub(a).min(u64::from(u32::MAX)) as u32,
        _ => 0,
    }
}

/// Reports the outcome of the flush of `area` to the engine (health, retry of the area,
/// [`FaultKind::FlushError`] with the driver's error code), logging a failure.
fn report<D: AsyncDisplayDriver>(
    engine: &mut Engine,
    display: &D,
    did: DisplayId,
    area: Rect,
    result: Result<(), D::Error>,
) {
    let result = result.map_err(|e| {
        let code = DriverErrorCode::new(display.error_code(&e));
        if let EngineError::Driver { message, .. } = EngineError::driver(code, &e) {
            twine_core::error!(target: "twine::driver", "async flush {} failed ({}): {}", area, code, message.as_str());
        }
        code
    });
    engine.report_flush(did, area, result);
}
