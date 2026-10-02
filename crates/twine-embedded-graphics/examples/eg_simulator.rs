//! `cargo xtask sim eg_simulator`: the counter demo running inside
//! [`embedded-graphics-simulator`](https://docs.rs/embedded-graphics-simulator) instead of
//! `twine-sim`, for users of the embedded-graphics ecosystem.
//!
//! Needs SDL2 and the `eg-sim` feature (which `cargo xtask sim eg_simulator` enables):
//! `brew install sdl2` on macOS, `apt install libsdl2-dev` on Linux.
//!
//! The simulator display is wrapped in an [`EgDisplay`], so the `Ui` flushes into it like into
//! any embedded-graphics driver; mouse events become a pointer input device ([`EgPointer`]).
//! `twine-sim` stays the primary simulator (headless scripts, snapshots, F-key debug tools,
//! time control); this example shows the adapter path only.

use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::rc::Rc;
use std::time::Duration as StdDuration;

use embedded_graphics::Pixel;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::{OriginDimensions, Size as EgSize};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics_simulator::sdl2::MouseButton;
use embedded_graphics_simulator::{OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window};
use twine::hal::{Clock, InputData, InputDevice, InputKind, PointerData};
use twine::prelude::*;
use twine_embedded_graphics::EgDisplay;
use twine_sim::SimClock;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
/// Longest sleep between event polls (the window must stay responsive while the UI is idle).
const MAX_SLEEP: StdDuration = StdDuration::from_millis(16);

/// The simulator display, shared between the `Ui` (which draws into it) and the window loop
/// (which shows it).
#[derive(Clone)]
struct SharedDisplay(Rc<RefCell<SimulatorDisplay<Rgb565>>>);

impl OriginDimensions for SharedDisplay {
    fn size(&self) -> EgSize {
        self.0.borrow().size()
    }
}

impl DrawTarget for SharedDisplay {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, pixels: I) -> Result<(), Infallible> {
        self.0.borrow_mut().draw_iter(pixels)
    }

    fn fill_contiguous<I: IntoIterator<Item = Rgb565>>(
        &mut self,
        area: &embedded_graphics::primitives::Rectangle,
        colors: I,
    ) -> Result<(), Infallible> {
        self.0.borrow_mut().fill_contiguous(area, colors)
    }
}

/// The simulator mouse as a twine pointer: the window loop writes, the engine reads.
#[derive(Clone, Default)]
struct EgPointer(Rc<Cell<PointerData>>);

impl EgPointer {
    /// Applies one simulator event; returns whether it concerned the pointer.
    fn handle(&self, event: &SimulatorEvent) -> bool {
        let mut d = self.0.get();
        match *event {
            SimulatorEvent::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                point,
            } => {
                d = PointerData {
                    point: Point::new(point.x, point.y),
                    pressed: true,
                };
            }
            SimulatorEvent::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                point,
            } => {
                d = PointerData {
                    point: Point::new(point.x, point.y),
                    pressed: false,
                };
            }
            SimulatorEvent::MouseMove { point } => d.point = Point::new(point.x, point.y),
            _ => return false,
        }
        self.0.set(d);
        true
    }
}

impl InputDevice for EgPointer {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        InputData::Pointer(self.0.get())
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("twine=info")).init();

    let display = SharedDisplay(Rc::new(RefCell::new(SimulatorDisplay::new(EgSize::new(
        WIDTH, HEIGHT,
    )))));
    let pointer = EgPointer::default();
    let clock = SimClock::wall();
    let mut ui = Ui::builder(EgDisplay::new(display.clone()))
        .buffers(BufferMode::partial_single(leak_buffer(WIDTH as usize * 40 * 2)))
        .input(pointer.clone())
        .clock(clock.clone())
        .theme(DefaultTheme::light())
        .build(twine_demos::counter::app);

    let settings = OutputSettingsBuilder::new().scale(2).build();
    let mut window = Window::new("Counter (embedded-graphics-simulator)", &settings);
    window.set_max_fps(1000); // pacing is done below, from the UI's wake requests
    let mut wake = ui.update();
    window.update(&display.0.borrow());
    loop {
        let mut input = false;
        for event in window.events() {
            if matches!(event, SimulatorEvent::Quit) {
                return;
            }
            input |= pointer.handle(&event);
        }
        if input {
            ui.notify_input();
        }
        let due = match wake {
            Wake::Now => true,
            Wake::At(t) => clock.now() >= t,
            Wake::Idle => false,
        };
        if input || due {
            wake = ui.update();
            window.update(&display.0.borrow());
        }
        let sleep = match wake {
            Wake::Now => StdDuration::ZERO,
            Wake::At(t) => StdDuration::from_micros((t - clock.now()).as_micros()).min(MAX_SLEEP),
            Wake::Idle => MAX_SLEEP,
        };
        std::thread::sleep(sleep);
    }
}

/// A 4-byte aligned draw buffer of `len` bytes, allocated once.
fn leak_buffer(len: usize) -> &'static mut [u8] {
    let v: &'static mut [u8] = Box::leak(vec![0u8; len + 3].into_boxed_slice());
    let off = v.as_ptr().align_offset(4).min(3);
    &mut v[off..off + len]
}
