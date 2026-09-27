//! Example firmware: Twine on an RP2040 (e.g. Raspberry Pi Pico) with an SPI display and a
//! touch controller, or a mono I2C OLED. A template to copy, not board support: every pin, the
//! panel model, its rotation and the bus clocks are in the wiring block at the top of `main`;
//! cargo features pick the display (`panel-ili9341`, `panel-ili9342`, `panel-st7789`,
//! `panel-st7796`, `oled-ssd1306`), the touch driver (`touch-xpt2046`, `touch-ft6x36`,
//! `touch-gt911`, `touch-cst816s`, or none) and the demo (`demo-counter`, `demo-controls`,
//! `demo-selection`, `demo-calibrate`, `demo-multilang`).
//!
//! SPI panels are flushed with DMA while the next chunk renders (`twine_embassy::run`); the UI
//! sleeps until the touch interrupt, a channel message or its next deadline. Every 5 s the RTT
//! log shows a `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use static_cell::ConstStaticCell;
use twine::core::Rotation;
use twine::engine::{EngineConfig, MemInfo};
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine_drivers::Calibration;
use twine_embassy::UiBuilderExt;
use {defmt_rtt as _, panic_probe as _};

// Feature groups (use `--no-default-features` to switch, or `cargo xtask firmware <board>
// --features …`, which replaces the default of the same group): exactly one display, at most one
// touch controller, at most one demo.
#[cfg(not(any(
    feature = "panel-ili9341",
    feature = "panel-ili9342",
    feature = "panel-st7789",
    feature = "panel-st7796",
    feature = "oled-ssd1306"
)))]
compile_error!("enable one display feature: panel-ili9341, panel-ili9342, panel-st7789, panel-st7796 or oled-ssd1306");
#[cfg(any(
    all(
        feature = "panel-ili9341",
        any(feature = "panel-ili9342", feature = "panel-st7789", feature = "panel-st7796", feature = "oled-ssd1306")
    ),
    all(feature = "panel-ili9342", any(feature = "panel-st7789", feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-st7789", any(feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-st7796", feature = "oled-ssd1306"),
))]
compile_error!("enable only one display feature (panel-* / oled-*)");
#[cfg(any(
    all(feature = "touch-xpt2046", any(feature = "touch-ft6x36", feature = "touch-gt911", feature = "touch-cst816s")),
    all(feature = "touch-ft6x36", any(feature = "touch-gt911", feature = "touch-cst816s")),
    all(feature = "touch-gt911", feature = "touch-cst816s"),
))]
compile_error!("enable at most one touch-* feature");
#[cfg(all(feature = "oled-ssd1306", feature = "touch"))]
compile_error!("oled-ssd1306 uses the touch I2C pins: disable the touch-* feature");
#[cfg(any(
    all(feature = "demo-counter", any(feature = "demo-controls", feature = "demo-selection", feature = "demo-calibrate", feature = "demo-multilang")),
    all(feature = "demo-controls", any(feature = "demo-selection", feature = "demo-calibrate", feature = "demo-multilang")),
    all(feature = "demo-selection", any(feature = "demo-calibrate", feature = "demo-multilang")),
    all(feature = "demo-calibrate", feature = "demo-multilang"),
))]
compile_error!("enable at most one demo-* feature");
#[cfg(all(feature = "demo-calibrate", not(feature = "touch")))]
compile_error!("demo-calibrate needs a touch-* feature");

/// Display rotation: the SPI panels turned to landscape (320 × 240, ST7796: 480 × 320).
#[cfg(any(feature = "panel-ili9341", feature = "panel-st7789", feature = "panel-st7796"))]
const ROTATION: Rotation = Rotation::Deg90;
/// Display rotation: the ILI9342C and the OLED are landscape natively.
#[cfg(any(feature = "panel-ili9342", feature = "oled-ssd1306"))]
const ROTATION: Rotation = Rotation::Deg0;

#[cfg(feature = "spi-panel")]
bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>;
});
#[cfg(feature = "oled-ssd1306")]
bind_interrupts!(struct Irqs {
    I2C0_IRQ => embassy_rp::i2c::InterruptHandler<embassy_rp::peripherals::I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    init_heap();
    let p = embassy_rp::init(embassy_rp::config::Config::default());

    // ================================ Wiring: edit for your board ================================
    // SPI panel on SPI0 with TX DMA (the defaults match a Pico with a 2.8" ILI9341 + XPT2046
    // module); every `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (lcd_spi, lcd_dma, lcd_sck, lcd_mosi) = (p.SPI0, p.DMA_CH0, p.PIN_18, p.PIN_19);
    #[cfg(feature = "spi-panel")]
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.PIN_17, p.PIN_20, p.PIN_21, p.PIN_22);
    #[cfg(feature = "spi-panel")]
    const LCD_HZ: u32 = 62_500_000;
    // XPT2046 resistive touch on SPI1 (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_spi, touch_sck, touch_mosi, touch_miso, touch_cs, touch_irq) =
        (p.SPI1, p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13, p.PIN_14);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_HZ: u32 = 2_000_000;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
    // Capacitive touch (FT6x36, GT911, CST816S) on I2C0: SDA GP4, SCL GP5, INT GP14.
    #[cfg(feature = "i2c-touch")]
    let (touch_i2c, touch_sda, touch_scl, touch_irq) = (p.I2C0, p.PIN_4, p.PIN_5, p.PIN_14);
    // SSD1306 OLED on the same I2C0 pins (feature `oled-ssd1306`, address 0x3C).
    #[cfg(feature = "oled-ssd1306")]
    let (oled_i2c, oled_sda, oled_scl) = (p.I2C0, p.PIN_4, p.PIN_5);
    // ============================================================================================

    // Display.
    #[cfg(feature = "spi-panel")]
    let _backlight = embassy_rp::gpio::Output::new(lcd_backlight, embassy_rp::gpio::Level::High);
    #[cfg(feature = "spi-panel")]
    let display = {
        use embassy_rp::gpio::{Level, Output};
        use embassy_rp::spi::{self, Spi};
        use static_cell::StaticCell;
        type LcdBus = embassy_sync::mutex::Mutex<NoopRawMutex, Spi<'static, embassy_rp::peripherals::SPI0, spi::Async>>;
        // Async SPI with DMA, shared-bus `SpiDevice` (owns CS).
        static LCD_BUS: StaticCell<LcdBus> = StaticCell::new();
        let mut cfg = spi::Config::default();
        cfg.frequency = LCD_HZ;
        let bus = LCD_BUS.init(embassy_sync::mutex::Mutex::new(Spi::new_txonly(lcd_spi, lcd_sck, lcd_mosi, lcd_dma, Irqs, cfg)));
        let lcd = embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice::new(bus, Output::new(lcd_cs, Level::High));
        let dc = Output::new(lcd_dc, Level::Low);
        let rst = Some(Output::new(lcd_rst, Level::High));
        let delay = &mut embassy_time::Delay;
        #[cfg(feature = "panel-ili9341")]
        let d = twine_drivers::ili9341::new_async(lcd, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine_drivers::ili9342::new_async(lcd, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine_drivers::st7789::new_async(lcd, dc, rst, &twine_drivers::st7789::ST7789, ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine_drivers::st7796::new_async(lcd, dc, rst, ROTATION, delay).await;
        d.unwrap_or_else(|e| defmt::panic!("display init failed: {}", defmt::Debug2Format(&e)))
    };
    #[cfg(feature = "oled-ssd1306")]
    let display = {
        let mut cfg = embassy_rp::i2c::Config::default();
        cfg.frequency = 400_000;
        let i2c = embassy_rp::i2c::I2c::new_async(oled_i2c, oled_scl, oled_sda, Irqs, cfg);
        let iface = twine_drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine_drivers::ssd1306::AsyncSsd1306::new(iface, twine_drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .await
            .unwrap_or_else(|e| defmt::panic!("display init failed: {}", defmt::Debug2Format(&e)))
    };
    let info = twine::hal::AsyncDisplayDriver::info(&display);

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
        use embassy_rp::gpio::{Input, Level, Output, Pull};
        use embassy_rp::spi::{self, Spi};
        use embassy_sync::blocking_mutex::NoopMutex;
        use static_cell::StaticCell;
        type TouchBus = NoopMutex<core::cell::RefCell<Spi<'static, embassy_rp::peripherals::SPI1, spi::Blocking>>>;
        static TOUCH_BUS: StaticCell<TouchBus> = StaticCell::new();
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
    #[cfg(feature = "i2c-touch")]
    let touch = {
        use embassy_rp::gpio::{Input, Pull};
        let i2c = embassy_rp::i2c::I2c::new_blocking(touch_i2c, touch_scl, touch_sda, embassy_rp::i2c::Config::default());
        let irq = Some(Input::new(touch_irq, Pull::Up));
        let native = if ROTATION.swaps_axes() { (info.height, info.width) } else { (info.width, info.height) };
        let transform = twine_drivers::touch::TouchTransform::for_rotation(ROTATION, native.0, native.1);
        #[cfg(feature = "touch-ft6x36")]
        let t = twine_drivers::touch::Ft6x36::new(i2c, irq, transform);
        #[cfg(feature = "touch-gt911")]
        let t = twine_drivers::touch::Gt911::new(i2c, irq, transform);
        #[cfg(feature = "touch-cst816s")]
        let t = twine_drivers::touch::Cst816s::new(i2c, irq, transform);
        t
    };

    // `demo-calibrate`: raw readings go to the calibration demo, not to the UI.
    #[cfg(feature = "demo-calibrate")]
    let touch = twine_demos::calibration::RawTouchInput::new(touch);

    // The UI: two DMA-pipelined partial buffers (SPI panels: 40 rows of 320 px; OLED: the
    // whole 128 × 64 × 1 bpp frame).
    static BUF_A: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    static BUF_B: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    let mut config = EngineConfig::default();
    config.mem_info = Some(mem_info);
    config.hires_timer = Some(twine_embassy::hires_now);
    // SAFETY: the UI and every reactive handle live in this task on the thread-mode executor and
    // are never touched from an interrupt handler, another executor or the second core; other
    // contexts only use channels and the UI waker.
    let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
    let builder = builder.buffers(BufferMode::partial_double(&mut BUF_A.take().0, &mut BUF_B.take().0));
    #[cfg(feature = "touch")]
    let builder = builder.input_wait(touch);
    #[cfg(feature = "spi-panel")]
    let theme = DefaultTheme::light();
    // White on black: lit OLED pixels are the foreground.
    #[cfg(feature = "oled-ssd1306")]
    let theme = MonoTheme::new(true, &twine::assets::fonts::MONTSERRAT_14);
    let ui = builder.config(config).theme(theme).with_embassy_clock().build(demo::app);
    defmt::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine_embassy::run(ui).await
}

#[cfg(feature = "spi-panel")]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

/// Bytes of one partial draw buffer: 40 rows of 320 px (RGB565).
#[cfg(feature = "spi-panel")]
const BUFFER_BYTES: usize = 320 * 40 * 2;
/// Bytes of one draw buffer: the whole 128 × 64 OLED at 1 bpp.
#[cfg(feature = "oled-ssd1306")]
const BUFFER_BYTES: usize = 128 * 64 / 8;

/// A 4-byte aligned draw buffer (the engine needs word-aligned buffers).
#[repr(C, align(4))]
struct DrawBuffer([u8; BUFFER_BYTES]);

/// Calibration mode: the touch driver reports raw readings (identity calibration, no clamping).
#[cfg(feature = "touch-xpt2046")]
const CALIBRATE: bool = cfg!(feature = "demo-calibrate");

/// The demo selected by cargo feature (the counter without a `demo-*` feature).
mod demo {
    #[cfg(feature = "demo-calibrate")]
    pub use twine_demos::calibration::app;
    #[cfg(feature = "demo-controls")]
    pub use twine_demos::controls::app;
    #[cfg(feature = "demo-selection")]
    pub use twine_demos::selection::app;
    #[cfg(feature = "demo-multilang")]
    pub use twine_demos::multilang::app;
    #[cfg(not(any(
        feature = "demo-controls",
        feature = "demo-calibrate",
        feature = "demo-selection",
        feature = "demo-multilang"
    )))]
    pub use twine_demos::counter::app;

    /// Name of the demo (for the log).
    pub const NAME: &str = if cfg!(feature = "demo-calibrate") {
        "calibrate"
    } else if cfg!(feature = "demo-controls") {
        "controls"
    } else if cfg!(feature = "demo-selection") {
        "selection"
    } else if cfg!(feature = "demo-multilang") {
        "multilang"
    } else {
        "counter"
    };
}

/// Heap size (the RP2040 has 264 KiB of SRAM; the draw buffers are static). The selection
/// demo needs more: ~94 KB for its largest page plus the engine's caches (`docs/perf.md`); the
/// multilang demo keeps its eight decoded avatars (51 KB) in the image cache.
const HEAP_BYTES: usize = if cfg!(any(feature = "demo-selection", feature = "demo-multilang")) {
    160 * 1024
} else {
    96 * 1024
};

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
