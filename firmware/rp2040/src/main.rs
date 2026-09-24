//! Example firmware: Twine on an RP2040 (e.g. Raspberry Pi Pico) with an SPI display and a
//! touch controller. A template to copy, not board support: every pin, the panel model, its
//! rotation and the bus clocks are in the wiring block at the top of `main`; cargo features
//! pick the panel driver (`panel-ili9341`, `panel-st7789`), the touch driver (`touch-xpt2046`,
//! `touch-ft6x36`) and the demo (`demo-counter`, `demo-controls`, `demo-calibrate`).
//!
//! The display is flushed with SPI DMA while the next chunk renders (`twine_embassy::run`);
//! the UI sleeps until the touch interrupt, a channel message or its next deadline. Every
//! 5 s the RTT log shows a `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{self, Spi};
use embassy_rp::{bind_interrupts, dma};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use static_cell::{ConstStaticCell, StaticCell};
use twine::core::Rotation;
use twine::engine::{EngineConfig, MemInfo};
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine_drivers::Calibration;
use twine_embassy::UiBuilderExt;
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    init_heap();
    let p = embassy_rp::init(embassy_rp::config::Config::default());

    // ================================ Wiring: edit for your board ================================
    // Display on SPI0 with TX DMA (the defaults match a Pico with a 2.8" ILI9341 + XPT2046 module).
    let lcd_spi = p.SPI0;
    let lcd_dma = p.DMA_CH0;
    let lcd_sck = p.PIN_18;
    let lcd_mosi = p.PIN_19;
    let lcd_cs = p.PIN_17;
    let lcd_dc = p.PIN_20;
    let lcd_rst = p.PIN_21;
    let lcd_backlight = p.PIN_22;
    const LCD_HZ: u32 = 62_500_000;
    const ROTATION: Rotation = Rotation::Deg90; // the panel turned 90° clockwise: 320 × 240
    // XPT2046 resistive touch on SPI1 (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_spi, touch_sck, touch_mosi, touch_miso, touch_cs, touch_irq) =
        (p.SPI1, p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13, p.PIN_14);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_HZ: u32 = 2_000_000;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
    // FT6x36 capacitive touch on I2C0 (feature `touch-ft6x36`).
    #[cfg(feature = "touch-ft6x36")]
    let (touch_i2c, touch_sda, touch_scl, touch_irq) = (p.I2C0, p.PIN_4, p.PIN_5, p.PIN_14);
    // ============================================================================================

    // Display: async SPI with DMA, shared-bus `SpiDevice` (owns CS).
    bind_interrupts!(struct Irqs {
        DMA_IRQ_0 => dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>;
    });
    static LCD_BUS: StaticCell<embassy_sync::mutex::Mutex<NoopRawMutex, Spi<'static, embassy_rp::peripherals::SPI0, spi::Async>>> =
        StaticCell::new();
    let mut cfg = spi::Config::default();
    cfg.frequency = LCD_HZ;
    let bus = LCD_BUS.init(embassy_sync::mutex::Mutex::new(Spi::new_txonly(lcd_spi, lcd_sck, lcd_mosi, lcd_dma, Irqs, cfg)));
    let lcd = embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice::new(bus, Output::new(lcd_cs, Level::High));
    let dc = Output::new(lcd_dc, Level::Low);
    let rst = Output::new(lcd_rst, Level::High);
    let _backlight = Output::new(lcd_backlight, Level::High);
    let delay = &mut embassy_time::Delay;
    #[cfg(feature = "panel-ili9341")]
    let display = twine_drivers::ili9341::new_async(lcd, dc, Some(rst), ROTATION, delay).await;
    #[cfg(all(feature = "panel-st7789", not(feature = "panel-ili9341")))]
    let display = twine_drivers::st7789::new_async(lcd, dc, Some(rst), &twine_drivers::st7789::ST7789, ROTATION, delay).await;
    let display = display.unwrap_or_else(|e| defmt::panic!("display init failed: {}", defmt::Debug2Format(&e)));
    let info = twine::hal::AsyncDisplayDriver::info(&display);

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
        static TOUCH_BUS: StaticCell<NoopMutex<core::cell::RefCell<Spi<'static, embassy_rp::peripherals::SPI1, spi::Blocking>>>> =
            StaticCell::new();
        let mut cfg = spi::Config::default();
        cfg.frequency = TOUCH_HZ;
        let bus = TOUCH_BUS.init(NoopMutex::new(core::cell::RefCell::new(Spi::new_blocking(
            touch_spi, touch_sck, touch_mosi, touch_miso, cfg,
        ))));
        let dev = SpiDevice::new(bus, Output::new(touch_cs, Level::High));
        let cal = if CALIBRATE { Calibration::IDENTITY } else { TOUCH_CAL };
        twine_drivers::touch::Xpt2046::new(dev, Some(Input::new(touch_irq, Pull::Up)))
            .with_calibration(cal)
            .with_screen_size(if CALIBRATE { 4096 } else { info.width }, if CALIBRATE { 4096 } else { info.height })
    };
    #[cfg(all(feature = "touch-ft6x36", not(feature = "touch-xpt2046")))]
    let touch = {
        let i2c = embassy_rp::i2c::I2c::new_blocking(touch_i2c, touch_scl, touch_sda, embassy_rp::i2c::Config::default());
        let native = if ROTATION.swaps_axes() { (info.height, info.width) } else { (info.width, info.height) };
        let transform = twine_drivers::touch::TouchTransform::for_rotation(ROTATION, native.0, native.1);
        twine_drivers::touch::Ft6x36::new(i2c, Some(Input::new(touch_irq, Pull::Up)), transform)
    };

    // `demo-calibrate`: raw readings go to the calibration demo, not to the UI.
    #[cfg(feature = "demo-calibrate")]
    let touch = twine_demos::calibration::RawTouchInput::new(touch);

    // The UI: two DMA-pipelined partial buffers of 40 rows.
    static BUF_A: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    static BUF_B: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    let mut config = EngineConfig::default();
    config.mem_info = Some(mem_info);
    config.hires_timer = Some(twine_embassy::hires_now);
    // SAFETY: the UI and every reactive handle live in this task on the thread-mode executor and
    // are never touched from an interrupt handler, another executor or the second core; other
    // contexts only use channels and the UI waker.
    let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
    let ui = builder
        .buffers(BufferMode::partial_double(&mut BUF_A.take().0, &mut BUF_B.take().0))
        .input_wait(touch)
        .config(config)
        .theme(DefaultTheme::light())
        .with_embassy_clock()
        .build(demo::app);
    defmt::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine_embassy::run(ui).await
}

#[cfg(feature = "touch-xpt2046")]
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

/// Bytes of one partial draw buffer: 40 rows of the widest side (320 px, RGB565).
const BUFFER_BYTES: usize = 320 * 40 * 2;

/// A 4-byte aligned draw buffer (the engine needs word-aligned buffers).
#[repr(C, align(4))]
struct DrawBuffer([u8; BUFFER_BYTES]);

/// Calibration mode: the touch driver reports raw readings (identity calibration, no clamping).
#[cfg(feature = "touch-xpt2046")]
const CALIBRATE: bool = cfg!(feature = "demo-calibrate");

/// The demo selected by cargo feature.
mod demo {
    #[cfg(feature = "demo-calibrate")]
    pub use twine_demos::calibration::app;
    #[cfg(all(feature = "demo-controls", not(feature = "demo-calibrate")))]
    pub use twine_demos::controls::app;
    #[cfg(not(any(feature = "demo-controls", feature = "demo-calibrate")))]
    pub use twine_demos::counter::app;

    /// Name of the demo (for the log).
    pub const NAME: &str = if cfg!(feature = "demo-calibrate") {
        "calibrate"
    } else if cfg!(feature = "demo-controls") {
        "controls"
    } else {
        "counter"
    };
}

/// Heap size (the RP2040 has 264 KiB of SRAM; the draw buffers are static).
const HEAP_BYTES: usize = 96 * 1024;

#[global_allocator]
static HEAP: embedded_alloc::LlffHeap = embedded_alloc::LlffHeap::empty();

/// Hands [`HEAP_BYTES`] of SRAM to the allocator (once).
fn init_heap() {
    static DONE: AtomicBool = AtomicBool::new(false);
    static mut MEMORY: [MaybeUninit<u8>; HEAP_BYTES] = [MaybeUninit::uninit(); HEAP_BYTES];
    assert!(!DONE.swap(true, Ordering::AcqRel), "init_heap called twice");
    // SAFETY: the flag guarantees this runs once, so `MEMORY` is handed to the allocator exactly
    // once and never accessed any other way; it lives for the whole program.
    unsafe { HEAP.init((&raw mut MEMORY).cast::<u8>() as usize, HEAP_BYTES) }
}

/// Heap statistics for the `twine::perf` log line; warns when the heap is more than 90 % full.
fn mem_info() -> MemInfo {
    static PEAK: AtomicU32 = AtomicU32::new(0);
    let used = HEAP.used() as u32;
    let free = HEAP.free() as u32;
    let peak = PEAK.fetch_max(used, Ordering::Relaxed).max(used);
    if u64::from(used) * 10 > u64::from(used + free) * 9 {
        defmt::warn!("heap above 90%: {} of {} bytes", used, used + free);
    }
    MemInfo { used, peak, free }
}
