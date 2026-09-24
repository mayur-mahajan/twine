//! The async runtime overlaps the flush of chunk k (DMA) with the rendering of chunk k + 1,
//! renders exactly the pixels of the blocking runtime, and its run loop sleeps without a timer
//! when idle and wakes on channel messages.

use std::cell::RefCell;
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake as TaskWake, Waker};

use embassy_time::{Duration as EmbDuration, MockDriver};
use twine_core::{ColorFormat, Rect};
use twine_embassy::{EmbassyClock, UiBuilderExt};
use twine_hal::{AsyncDisplayDriver, DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 48;
const ROWS: usize = 8;

/// What happened, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    FlushStarted(Rect),
    FlushFinished(Rect),
    Rendered,
}

thread_local! {
    static LOG: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
}

fn log(e: Ev) {
    LOG.with(|l| l.borrow_mut().push(e));
}

fn take_log() -> Vec<Ev> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

type Fb = Rc<RefCell<Vec<u8>>>;

fn blit(fb: &Fb, area: Rect, buf: &[u8]) {
    let w = area.width() as usize;
    for (row, y) in (area.y0..area.y1).enumerate() {
        let dst = (y as usize * usize::from(W) + area.x0 as usize) * 2;
        fb.borrow_mut()[dst..dst + w * 2].copy_from_slice(&buf[row * w * 2..(row + 1) * w * 2]);
    }
}

/// An async display whose flush starts on the first poll and finishes on the second, like a
/// DMA transfer (it returns `Pending` once).
struct MockAsync {
    fb: Fb,
}

struct Flush<'a> {
    fb: &'a Fb,
    area: Rect,
    buf: &'a [u8],
    started: bool,
}

impl Future for Flush<'_> {
    type Output = Result<(), ()>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        if !self.started {
            self.started = true;
            log(Ev::FlushStarted(self.area));
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        blit(self.fb, self.area, self.buf);
        log(Ev::FlushFinished(self.area));
        Poll::Ready(Ok(()))
    }
}

impl AsyncDisplayDriver for MockAsync {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(W, H, ColorFormat::Rgb565)
    }
    async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), ()> {
        Flush {
            fb: &self.fb,
            area,
            buf,
            started: false,
        }
        .await
    }
}

/// The blocking twin: copies each flush at once.
struct MockBlocking {
    fb: Fb,
    held: Option<DrawBufferMem>,
}

impl DisplayDriver for MockBlocking {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(W, H, ColorFormat::Rgb565)
    }
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), ()> {
        blit(&self.fb, area, buf.as_slice());
        self.held = Some(buf);
        Ok(())
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.held.take()
    }
}

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

fn app(_cx: Scope) -> impl View {
    column((label("Twine"), button(label("Click me"))))
        .gap(4)
        .padding(4)
        .bg(Color::hex(0x1E_88_E5))
        .size(Length::Pct(100), Length::Pct(100))
}

fn fb() -> Fb {
    Rc::new(RefCell::new(vec![0; usize::from(W) * usize::from(H) * 2]))
}

fn async_ui(double: bool, fb: &Fb) -> AsyncUi<MockAsync> {
    let len = usize::from(W) * 2 * ROWS;
    let bufs = if double {
        BufferMode::partial_double(leak(len), leak(len))
    } else {
        BufferMode::partial_single(leak(len))
    };
    let mut ui = Ui::builder_async(MockAsync { fb: fb.clone() })
        .buffers(bufs)
        .theme(DefaultTheme::light())
        .with_embassy_clock()
        .build(app);
    ui.engine_mut().set_render_hook(Some(|_| log(Ev::Rendered)));
    ui
}

/// Polls a future to completion on this thread (the mocks complete after one `Pending`).
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

#[test]
fn flush_starts_before_render_of_next_chunk() {
    let fb = fb();
    let mut ui = async_ui(true, &fb);
    let _ = take_log();
    let _ = block_on(ui.update_async());
    let log = take_log();
    let renders: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, e)| **e == Ev::Rendered)
        .map(|(i, _)| i)
        .collect();
    let starts: Vec<(usize, Rect)> = log
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if let Ev::FlushStarted(a) = e {
                Some((i, *a))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(renders.len(), usize::from(H) / ROWS, "{log:?}");
    assert_eq!(starts.len(), renders.len());
    for k in 1..renders.len() {
        let (started, area) = starts[k - 1];
        let finished = log.iter().position(|e| *e == Ev::FlushFinished(area)).unwrap();
        assert!(
            started < renders[k] && renders[k] < finished,
            "chunk {k}: flush of chunk {} must start before and finish after its render: {log:?}",
            k - 1
        );
    }
    let d = ui.engine().default_display().unwrap();
    assert_eq!(usize::from(ui.engine().last_stats(d).chunks), renders.len());
}

#[test]
fn single_buffer_no_overlap() {
    let fb = fb();
    let mut ui = async_ui(false, &fb);
    let _ = take_log();
    let _ = block_on(ui.update_async());
    let log = take_log();
    // Render, start, finish — never a render while a flush is open.
    let mut open = false;
    for e in &log {
        match e {
            Ev::FlushStarted(_) => open = true,
            Ev::FlushFinished(_) => open = false,
            Ev::Rendered => assert!(!open, "render during a flush: {log:?}"),
        }
    }
    assert_eq!(
        log.iter().filter(|e| **e == Ev::Rendered).count(),
        usize::from(H) / ROWS
    );
}

#[test]
fn blocking_and_async_render_identical_pixels() {
    let fb_async = fb();
    let mut ui = async_ui(true, &fb_async);
    let _ = block_on(ui.update_async());

    let fb_blocking = fb();
    let len = usize::from(W) * 2 * ROWS;
    let mut blocking = Ui::builder(MockBlocking {
        fb: fb_blocking.clone(),
        held: None,
    })
    .buffers(BufferMode::partial_double(leak(len), leak(len)))
    .theme(DefaultTheme::light())
    .clock(EmbassyClock)
    .build(app);
    let _ = blocking.update();
    assert!(
        fb_blocking.borrow().iter().any(|b| *b != 0),
        "blocking frame drawn"
    );
    assert!(
        *fb_async.borrow() == *fb_blocking.borrow(),
        "async and blocking frames differ"
    );
}

/// A waker that counts its wake-ups.
struct Counter(AtomicUsize);

impl TaskWake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Polls `run(ui)` until it stops waking itself (the UI is asleep).
fn run_until_asleep<F: Future>(fut: &mut Pin<&mut F>, counter: &Arc<Counter>) {
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);
    for _ in 0..1000 {
        let before = counter.0.load(Ordering::SeqCst);
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        if counter.0.load(Ordering::SeqCst) == before {
            return;
        }
    }
    panic!("the run loop never went to sleep");
}

static MESSAGES: Channel<u32, 4> = Channel::new();
static RECEIVED: AtomicU32 = AtomicU32::new(0);

fn channel_app(cx: Scope) -> impl View {
    let n = cx.signal(0u32);
    cx.on_message(&MESSAGES, move |v| {
        RECEIVED.store(v, Ordering::SeqCst);
        n.set(v);
    });
    label(text!("{}", n.get()))
}

#[test]
fn idle_waits_without_timer_and_channel_send_wakes_run_loop() {
    let fb = fb();
    let len = usize::from(W) * 2 * ROWS;
    let ui = Ui::builder_async(MockAsync { fb: fb.clone() })
        .buffers(BufferMode::partial_double(leak(len), leak(len)))
        .with_embassy_clock()
        .build(channel_app);
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    let mut run = pin!(twine_embassy::run(ui));
    run_until_asleep(&mut run, &counter);
    // Idle: no timer is armed, so time passing wakes nothing.
    let before = counter.0.load(Ordering::SeqCst);
    MockDriver::get().advance(EmbDuration::from_secs(3600));
    assert_eq!(
        counter.0.load(Ordering::SeqCst),
        before,
        "an idle UI armed a timer"
    );
    // A message from another context wakes the loop, which delivers it.
    MESSAGES.try_send(7).unwrap();
    assert!(
        counter.0.load(Ordering::SeqCst) > before,
        "channel send did not wake the run loop"
    );
    run_until_asleep(&mut run, &counter);
    assert_eq!(RECEIVED.load(Ordering::SeqCst), 7);
}
