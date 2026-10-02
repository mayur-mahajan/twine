//! Power and rotation at run time on the async runtime (R3.S07): `AsyncUi` applies brightness,
//! sleep and rotation requests to its `AsyncDisplayDriver` between frames (hardware rotation,
//! or software rotation when the driver cannot), calls `idle` when nothing more is drawn, and
//! takes display commands from a channel.
#![allow(clippy::unused_async_trait_impl)] // the mock panel completes every operation at once

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use twine_core::{ColorFormat, Rect};
use twine_hal::{AsyncDisplayDriver, ControlError, DisplayInfo};
use twine_testing::MockPlatform;
use twine_view::prelude::*;

const W: u16 = 64;
const H: u16 = 48;

/// Shared state of the mock panel.
#[derive(Clone)]
struct Panel {
    /// Pixels in the panel's flush coordinates (logical once rotated in hardware).
    fb: Rc<RefCell<Vec<u8>>>,
    info: Rc<Cell<DisplayInfo>>,
    hw_rotation: bool,
    brightness: Rc<Cell<Option<Fraction>>>,
    asleep: Rc<Cell<bool>>,
    idles: Rc<Cell<u32>>,
}

impl Panel {
    fn new(info: DisplayInfo, hw_rotation: bool) -> Self {
        let (w, h) = if info.hw_rotation {
            (info.width, info.height)
        } else {
            info.native_size()
        };
        Panel {
            fb: Rc::new(RefCell::new(vec![0; usize::from(w) * usize::from(h) * 2])),
            info: Rc::new(Cell::new(info)),
            hw_rotation,
            brightness: Rc::default(),
            asleep: Rc::default(),
            idles: Rc::default(),
        }
    }
}

struct AsyncPanel(Panel);

impl AsyncDisplayDriver for AsyncPanel {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        self.0.info.get()
    }
    async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), ()> {
        let info = self.0.info.get();
        let stride = if info.hw_rotation {
            info.width
        } else {
            info.native_size().0
        };
        let w = area.width() as usize;
        for (row, y) in (area.y0..area.y1).enumerate() {
            let dst = (y as usize * usize::from(stride) + area.x0 as usize) * 2;
            self.0.fb.borrow_mut()[dst..dst + w * 2].copy_from_slice(&buf[row * w * 2..(row + 1) * w * 2]);
        }
        Ok(())
    }
    async fn idle(&mut self) {
        self.0.idles.set(self.0.idles.get() + 1);
    }
    async fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<()>> {
        self.0.brightness.set(Some(level));
        Ok(())
    }
    async fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<()>> {
        self.0.asleep.set(sleep);
        Ok(Duration::ms(if sleep { 5 } else { 120 }))
    }
    async fn set_rotation(&mut self, rotation: Rotation) -> Result<DisplayInfo, ControlError<()>> {
        if !self.0.hw_rotation {
            return Err(ControlError::Unsupported);
        }
        let old = self.0.info.get();
        let (nw, nh) = old.native_size();
        let (w, h) = if rotation.swaps_axes() { (nh, nw) } else { (nw, nh) };
        let info = DisplayInfo {
            width: w,
            height: h,
            rotation,
            hw_rotation: true,
            ..old
        };
        self.0.info.set(info);
        Ok(info)
    }
}

fn app(_cx: Scope) -> impl View {
    container((
        label("Twine").align(Align::TopLeft).pos(2, 2),
        container(())
            .width(Length::pct(50))
            .height(10)
            .align(Align::BottomRight)
            .bg(Color::hex(0x1E_88_E5)),
    ))
    .fill()
}

fn ui(panel: &Panel, platform: &MockPlatform) -> AsyncUi<AsyncPanel> {
    Ui::builder_async(AsyncPanel(panel.clone()))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 }))
        .platform(platform)
        .reserve_rotation()
        .build(app)
}

/// Polls a future to completion (the mock never returns `Pending`).
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// Updates until idle, advancing the platform's clock to every deadline.
fn settle(u: &mut AsyncUi<AsyncPanel>, platform: &MockPlatform) -> Wake {
    for _ in 0..20 {
        match block_on(u.update_async()) {
            Wake::At(t) => platform.set(t),
            Wake::Now => {}
            w => return w,
        }
    }
    panic!("not idle");
}

#[test]
fn async_rotation_matches_a_ui_built_rotated() {
    for (hw, rot) in [
        (true, Rotation::Deg90),
        (false, Rotation::Deg270),
        (false, Rotation::Deg180),
    ] {
        let panel = Panel::new(DisplayInfo::new(W, H, ColorFormat::Rgb565), hw);
        let platform = MockPlatform::new();
        let mut u = ui(&panel, &platform);
        settle(&mut u, &platform);
        platform.advance(Duration::ms(100));
        u.set_rotation(rot).unwrap();
        settle(&mut u, &platform);
        assert_eq!(u.display_info().rotation, rot);
        assert!(u.take_faults().is_empty());

        let (w, h) = if rot.swaps_axes() { (H, W) } else { (W, H) };
        let built = Panel::new(
            DisplayInfo::new(w, h, ColorFormat::Rgb565)
                .with_rotation(rot)
                .with_hw_rotation(hw),
            hw,
        );
        let platform = MockPlatform::new();
        let mut r = ui(&built, &platform);
        settle(&mut r, &platform);
        assert_eq!(*panel.fb.borrow(), *built.fb.borrow(), "hw {hw}, {rot:?}");
    }
}

#[test]
fn async_brightness_sleep_and_idle() {
    let panel = Panel::new(DisplayInfo::new(W, H, ColorFormat::Rgb565), true);
    let platform = MockPlatform::new();
    let mut u = ui(&panel, &platform);
    settle(&mut u, &platform);
    assert!(panel.idles.get() >= 1, "idle after the first frame");
    u.set_brightness(Fraction::pct(60));
    u.set_display_sleep(true);
    settle(&mut u, &platform);
    assert_eq!(panel.brightness.get(), Some(Fraction::pct(60)));
    assert!(panel.asleep.get() && u.display_asleep());
    // Asleep: a change is not drawn.
    let before = panel.fb.borrow().clone();
    let e = u.engine_mut();
    let screen = e.active_screen(e.default_display().unwrap()).unwrap();
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    e.set_local_prop(screen, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    settle(&mut u, &platform);
    assert_eq!(*panel.fb.borrow(), before);
    // Wake: sent once the 5 ms settle time of the sleep has passed, drawn 120 ms later.
    u.set_display_sleep(false);
    let t0 = u.now();
    settle(&mut u, &platform);
    assert!(!panel.asleep.get() && !u.display_asleep());
    assert!(u.now() >= t0 + Duration::ms(120));
    assert_ne!(*panel.fb.borrow(), before);
    assert!(u.take_faults().is_empty());
}

#[test]
fn async_display_commands() {
    static COMMANDS: Channel<DisplayCmd, 4> = Channel::new();
    let panel = Panel::new(DisplayInfo::new(W, H, ColorFormat::Rgb565), true);
    let platform = MockPlatform::new();
    let mut u = Ui::builder_async(AsyncPanel(panel.clone()))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .platform(&platform)
        .display_commands(&COMMANDS)
        .build(app);
    settle(&mut u, &platform);
    COMMANDS.try_send(DisplayCmd::Rotate(Rotation::Deg90)).unwrap();
    COMMANDS.try_send(DisplayCmd::Brightness(Fraction::HALF)).unwrap();
    settle(&mut u, &platform);
    assert_eq!(u.display_info().rotation, Rotation::Deg90);
    assert_eq!(panel.info.get().rotation, Rotation::Deg90);
    assert_eq!(panel.brightness.get(), Some(Fraction::HALF));
}
