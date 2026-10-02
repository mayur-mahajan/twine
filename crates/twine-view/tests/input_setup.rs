//! Input setup through the builder (R3.S06): `Ui::builder(..).input(touch)` fits a touch driver
//! to the display — its rotation and native size, no transform in application code — and the
//! driver's `with_fail_after` reaches the `Ui`'s input health.

use std::cell::RefCell;
use std::rc::Rc;

use twine_core::{ColorFormat, Rotation};
use twine_drivers::NoPin;
use twine_drivers::testkit::Recorder;
use twine_drivers::touch::{Ft6x36, TouchMount, Xpt2046};
use twine_hal::{Calibration, DeviceHealth, DisplayInfo};
use twine_testing::{MemoryDisplay, MockClock};
use twine_view::prelude::*;

/// A 240 × 320 panel turned to landscape (logical 320 × 240).
fn landscape() -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(320, 240, ColorFormat::Rgb565).with_rotation(Rotation::Deg90))
}

/// Answers the `FT6x36`'s 5-byte status read with `regs`.
fn ft_bus(regs: Rc<RefCell<[u8; 5]>>) -> Recorder {
    let rec = Recorder::new();
    rec.set_i2c_responder(move |_addr, _written, rx| {
        let r = regs.borrow();
        for (b, v) in rx.iter_mut().zip(r.iter()) {
            *b = *v;
        }
    });
    rec
}

/// Two buttons: `hit` where native (10, 20) lands at 90° — logical (299, 10) — and `raw`
/// at logical (10, 20), where an untransformed point would land.
fn app(cx: Scope, hit: Signal<u32>, raw: Signal<u32>) -> impl View {
    let _ = cx;
    container((
        button(label(""))
            .pos(290, 0)
            .size(20, 20)
            .on_click(move || hit.update(|n| *n += 1)),
        button(label(""))
            .pos(0, 10)
            .size(20, 20)
            .on_click(move || raw.update(|n| *n += 1)),
    ))
    .fill()
}

fn tap(ui: &mut Ui, clock: &MockClock, regs: &Rc<RefCell<[u8; 5]>>, raw: (u8, u8)) {
    *regs.borrow_mut() = [0x01, 0x80, raw.0, 0x00, raw.1]; // one point, contact
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
    *regs.borrow_mut() = [0; 5]; // released
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
}

#[test]
fn builder_fits_a_touch_driver_to_the_rotated_display() {
    let regs = Rc::new(RefCell::new([0u8; 5]));
    let rec = ft_bus(regs.clone());
    let clock = MockClock::new();
    let rt = Runtime::current_thread();
    let (hit, raw) = (rt.create_root().signal(0u32), rt.create_root().signal(0u32));
    // The driver as constructed: no transform, no native size, no rotation.
    let touch = Ft6x36::new(rec.i2c(), None::<NoPin>);
    let mut ui = Ui::builder(landscape())
        .runtime(rt)
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .input(touch)
        .build(move |cx| app(cx, hit, raw));
    tap(&mut ui, &clock, &regs, (10, 20));
    assert_eq!(
        hit.get(),
        1,
        "native (10, 20) is logical (299, 10) on the rotated display"
    );
    assert_eq!(raw.get(), 0, "the point was transformed");
}

#[test]
fn builder_composes_the_touch_mount_with_the_rotation() {
    let regs = Rc::new(RefCell::new([0u8; 5]));
    let rec = ft_bus(regs.clone());
    let clock = MockClock::new();
    let rt = Runtime::current_thread();
    let (hit, raw) = (rt.create_root().signal(0u32), rt.create_root().signal(0u32));
    // A film with its raw X axis mirrored: raw x 229 is native x 10.
    let touch = Ft6x36::new(rec.i2c(), None::<NoPin>).with_mount(TouchMount {
        mirror_x: true,
        ..TouchMount::ALIGNED
    });
    let mut ui = Ui::builder(landscape())
        .runtime(rt)
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .input(touch)
        .build(move |cx| app(cx, hit, raw));
    tap(&mut ui, &clock, &regs, (229, 20));
    assert_eq!((hit.get(), raw.get()), (1, 0));
}

#[test]
fn with_fail_after_reaches_the_ui_through_the_builder() {
    let regs = Rc::new(RefCell::new([0u8; 5]));
    let rec = ft_bus(regs);
    let clock = MockClock::new();
    let touch = Ft6x36::new(rec.i2c(), None::<NoPin>).with_fail_after(2);
    let mut ui = Ui::builder(landscape())
        .runtime(Runtime::current_thread())
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .input(touch)
        .build(|_| label("hi"));
    let id = ui.engine().inputs().next().expect("one input");
    rec.set_bus_down(true);
    let mut seen = Vec::new();
    for _ in 0..3 {
        let _ = ui.update();
        seen.push(ui.input_health(id));
        clock.advance(Duration::ms(50));
    }
    assert_eq!(
        seen,
        [
            Some(DeviceHealth::Degraded { errors: 1 }),
            Some(DeviceHealth::Failed),
            Some(DeviceHealth::Failed)
        ],
        "failed after the 2 errors set with `with_fail_after`"
    );
}

#[test]
fn builder_fits_a_resistive_driver_to_the_display_size() {
    // XPT2046 with identity calibration (raw = native panel pixels): a reading beyond the
    // panel is rotated to the display the builder fitted it to and clamped (unfitted, it would
    // stay raw). Native (4000, 4000) is beyond the native corner (239, 319), which a 90°
    // landscape display shows at logical (0, 239).
    let rec = Recorder::new();
    rec.set_spi_responder(|tx, rx| {
        let v: u16 = match tx[0] {
            0xB3 | 0xC3 => 2000, // Z1, Z2: pressed
            0xD3 | 0x93 => 4000,
            _ => 0,
        };
        let s = v << 3;
        rx[1] = (s >> 8) as u8;
        rx[2] = s as u8;
    });
    let clock = MockClock::new();
    let rt = Runtime::current_thread();
    let hit = rt.create_root().signal(0u32);
    let touch = Xpt2046::new(rec.spi(), None::<NoPin>).with_calibration(Calibration::IDENTITY);
    let mut ui = Ui::builder(landscape())
        .runtime(rt)
        .clock(clock.clone())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .input(touch)
        .build(move |_| {
            let corner = button(label("")).pos(0, 220).size(20, 20);
            container((corner.on_click(move || hit.update(|n| *n += 1)),)).fill()
        });
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
    // Released: Z1 = 0, Z2 = 4095 (no pressure).
    rec.set_spi_responder(|tx, rx| {
        let v: u16 = if tx[0] == 0xC3 { 4095 } else { 0 };
        let s = v << 3;
        rx[1] = (s >> 8) as u8;
        rx[2] = s as u8;
    });
    for _ in 0..3 {
        let _ = ui.update();
        clock.advance(Duration::ms(50));
    }
    assert_eq!(
        hit.get(),
        1,
        "native (4000, 4000) rotated and clamped to logical (0, 239), inside the corner button"
    );
}
