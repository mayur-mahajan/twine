//! The async runtime (feature `async`): [`AsyncUi`] renders frames chunk by chunk and flushes
//! them through an [`AsyncDisplayDriver`], overlapping the transfer of one chunk (DMA) with the
//! rendering of the next.
//!
//! Built with [`Ui::builder_async`]; run by `twine-embassy` (or any executor: call
//! [`update_async`](AsyncUi::update_async) and sleep as the returned [`Wake`] says).

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;

use twine_core::{Instant, Rect};
use twine_engine::{BufferMode, DisplayId, Engine, EngineConfig, EngineError, ThemeHook, Wake};
use twine_hal::{
    AsyncDisplayDriver, AsyncInputWait, Clock, DrawBufferMem, InputData, InputDevice, InputKind, PollHint,
};
use twine_reactive::{Scope, UiWaker};

use crate::Ui;
use crate::ui::{DisplaySetup, UiBuilder, UiCore};
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
}

/// Registers the display with the engine for chunk-level refresh (the driver stays with the
/// [`AsyncUi`]).
struct Chunked {
    info: twine_hal::DisplayInfo,
    chunk_bytes: usize,
}

impl DisplaySetup for Chunked {
    fn add(self, engine: &mut Engine, _buffers: Option<BufferMode>) -> Result<DisplayId, EngineError> {
        engine.add_chunked_display(self.info, self.chunk_bytes)
    }
}

/// Configures and builds an [`AsyncUi`] (see [`Ui::builder_async`]). The methods are those of
/// [`UiBuilder`], plus [`input_wait`](Self::input_wait).
#[must_use]
pub struct AsyncUiBuilder<D: AsyncDisplayDriver, W: AsyncInputWait = NoInputWait> {
    display: D,
    inner: UiBuilder<Chunked>,
    buffers: Option<BufferMode>,
    wait: Option<Rc<RefCell<W>>>,
}

impl<D: AsyncDisplayDriver, W: AsyncInputWait> core::fmt::Debug for AsyncUiBuilder<D, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsyncUiBuilder")
            .field("inner", &self.inner)
            .field("input_wait", &self.wait.is_some())
            .finish_non_exhaustive()
    }
}

impl Ui {
    /// A builder for an async display ([`AsyncUi`]): partial buffers (two for DMA overlap),
    /// flushed through `display` chunk by chunk.
    pub fn builder_async<D: AsyncDisplayDriver + 'static>(display: D) -> AsyncUiBuilder<D> {
        let info = display.info();
        AsyncUiBuilder {
            display,
            inner: UiBuilder::new(Chunked { info, chunk_bytes: 0 }),
            buffers: None,
            wait: None,
        }
    }
}

impl<D: AsyncDisplayDriver + 'static, W: AsyncInputWait + 'static> AsyncUiBuilder<D, W> {
    /// The draw buffers: [`BufferMode::partial_double`] (render one chunk while the other is
    /// transferred) or [`BufferMode::partial_single`]. Required.
    pub fn buffers(mut self, m: BufferMode) -> Self {
        self.buffers = Some(m);
        self
    }

    /// Adds an input device read by the engine (see [`UiBuilder::input`]).
    pub fn input(mut self, d: impl InputDevice + 'static) -> Self {
        self.inner = self.inner.input(d);
        self
    }

    /// Adds the input device whose interrupt wakes the sleeping UI (e.g. a touch controller
    /// with its IRQ pin); it is also read by the engine like [`input`](Self::input).
    pub fn input_wait<T: InputDevice + AsyncInputWait + 'static>(self, d: T) -> AsyncUiBuilder<D, T> {
        let shared = Rc::new(RefCell::new(d));
        AsyncUiBuilder {
            display: self.display,
            inner: self.inner.input(SharedInput(shared.clone())),
            buffers: self.buffers,
            wait: Some(shared),
        }
    }

    /// The clock (required).
    pub fn clock(mut self, c: impl Clock + 'static) -> Self {
        self.inner = self.inner.clock(c);
        self
    }

    /// The theme of the display.
    pub fn theme(mut self, t: impl ThemeHook + 'static) -> Self {
        self.inner = self.inner.theme(t);
        self
    }

    /// The engine configuration.
    pub fn config(mut self, c: EngineConfig) -> Self {
        self.inner = self.inner.config(c);
        self
    }

    /// Binds the reactive runtime to the calling execution context (see
    /// [`UiBuilder::bind_to_current_context`]).
    ///
    /// # Safety
    ///
    /// Same contract as [`UiBuilder::bind_to_current_context`]: from now on the `AsyncUi` and
    /// every reactive handle are only used from this execution context (one task of one
    /// executor), never from an interrupt handler, another core or a task that can preempt it.
    #[allow(unsafe_code)]
    pub unsafe fn bind_to_current_context(mut self) -> Self {
        // SAFETY: forwarded; the caller upholds the single-context contract stated above.
        self.inner = unsafe { self.inner.bind_to_current_context() };
        self
    }

    /// Builds the `AsyncUi` (engine, display, theme, inputs; `app` built once).
    ///
    /// # Errors
    /// [`EngineError::InvalidConfig`] without a clock or buffers, [`EngineError::BufferModeMismatch`]
    /// for non-partial buffers; the engine's errors for the display and inputs.
    pub fn try_build<V: View>(mut self, app: impl FnOnce(Scope) -> V) -> Result<AsyncUi<D, W>, EngineError> {
        let (a, b) = match self.buffers.take() {
            Some(BufferMode::Partial { a, b }) => (a, b),
            Some(_) => return Err(EngineError::BufferModeMismatch),
            None => {
                twine_core::error!(target: "twine::view", "AsyncUi: no buffers (AsyncUiBuilder::buffers)");
                return Err(EngineError::InvalidConfig("AsyncUi needs buffers"));
            }
        };
        let chunk_bytes = b.as_ref().map_or(a.len(), |b| a.len().min(b.len()));
        self.inner.display.chunk_bytes = chunk_bytes;
        let (engine, core, clock) = self.inner.build_parts(app)?;
        twine_core::info!(
            target: "twine::view",
            "async ui: {} buffer(s) of {} B",
            if b.is_some() { 2 } else { 1 },
            chunk_bytes
        );
        Ok(AsyncUi {
            engine,
            core,
            clock,
            display: self.display,
            bufs: (a, b),
            wait: self.wait,
        })
    }

    /// [`try_build`](Self::try_build), panicking on a configuration error.
    ///
    /// # Panics
    /// Without a clock or buffers, or when the engine rejects the display or inputs.
    pub fn build<V: View>(self, app: impl FnOnce(Scope) -> V) -> AsyncUi<D, W> {
        match self.try_build(app) {
            Ok(ui) => ui,
            Err(e) => panic!("twine: cannot build the AsyncUi: {e:?}"),
        }
    }
}

/// The declarative UI runtime on an async display: like [`Ui`], but each frame is rendered
/// chunk by chunk and flushed with [`AsyncDisplayDriver::flush`]; with two buffers the flush
/// of one chunk (DMA) runs while the next chunk renders.
pub struct AsyncUi<D: AsyncDisplayDriver, W: AsyncInputWait = NoInputWait> {
    engine: Engine,
    core: UiCore,
    clock: Box<dyn Clock>,
    display: D,
    bufs: (DrawBufferMem, Option<DrawBufferMem>),
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
        let root = self.core.root_scope();
        if root.is_alive() {
            crate::access::EngineAccess::provide(&mut self.engine, || root.dispose());
        }
    }
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
    pub async fn update_async(&mut self) -> Wake {
        let now = self.clock.now();
        let wake = self.core.update(&mut self.engine, now);
        if self.engine.refresh_begin(now).is_some() {
            self.render_frame().await;
            self.engine.refresh_end();
        }
        wake
    }

    /// Renders and flushes the frame begun by `refresh_begin`.
    async fn render_frame(&mut self) {
        let Self {
            engine,
            display,
            bufs,
            ..
        } = self;
        let timer = engine.config().hires_timer;
        let us = move || timer.map(|f| f().as_micros());
        let (a, b) = bufs;
        let Some(b) = b.as_mut() else {
            // One buffer: render, flush, repeat (no overlap: all flush time is waiting).
            while let Some(area) = engine.render_chunk(a.as_mut_slice()) {
                let t0 = us();
                flush_logged(display, area, a.as_slice()).await;
                let t = elapsed(t0, us());
                engine.refresh_add_flush_time(t, t);
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
                    let mut flush = pin!(display.flush(area, prev_buf.as_slice()));
                    // The first poll starts the transfer (DMA); render while it runs.
                    let done = poll_once(&mut flush).await;
                    let next = engine.render_chunk(free_buf.as_mut_slice());
                    let t1 = us();
                    let result = match done {
                        Some(r) => r,
                        None => flush.await,
                    };
                    let t2 = us();
                    engine.refresh_add_flush_time(elapsed(t0, t2), elapsed(t1, t2));
                    if let Err(e) = result {
                        log_flush_error(area, &e);
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
    #[must_use]
    pub fn waker(&self) -> &'static UiWaker {
        self.core.waker()
    }

    /// Tells the next update that an input device changed.
    pub fn notify_input(&self) {
        self.core.notify_input();
    }

    /// The current time of the UI clock.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The engine, mutably.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
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

/// Logs a failed flush with the driver error's `Debug` output (the next frame redraws nothing
/// of it: the pixels are lost until the area changes again).
fn log_flush_error(area: Rect, e: &dyn core::fmt::Debug) {
    if let EngineError::Driver(msg) = EngineError::driver(e) {
        twine_core::error!(target: "twine::driver", "async flush {} failed: {}", area, msg.as_str());
    }
}

/// Microseconds between two optional timestamps (0 without a timer).
fn elapsed(a: Option<u64>, b: Option<u64>) -> u32 {
    match (a, b) {
        (Some(a), Some(b)) => b.saturating_sub(a).min(u64::from(u32::MAX)) as u32,
        _ => 0,
    }
}

async fn flush_logged<D: AsyncDisplayDriver>(display: &mut D, area: Rect, buf: &[u8]) {
    if let Err(e) = display.flush(area, buf).await {
        log_flush_error(area, &e);
    }
}
