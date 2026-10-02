//! Example firmware: Twine on an RP2350 (e.g. Raspberry Pi Pico 2) with an SPI display and a
//! touch controller, or a mono I2C OLED. A template to copy, not board support: every pin, the
//! panel model, its rotation and the bus clocks are in the wiring block at the top of `main`;
//! cargo features pick the display (`panel-ili9341`, `panel-ili9342`, `panel-st7789`,
//! `panel-st7796`, `oled-ssd1306`), the touch driver (`touch-xpt2046`, `touch-ft6x36`,
//! `touch-gt911`, `touch-cst816s`, or none) and the demo (`demo-counter`, `demo-controls`,
//! `demo-selection`, `demo-calibrate`, `demo-lottie`).
//!
//! SPI panels are flushed with DMA while the next chunk renders (`twine::embassy::run`); the UI
//! sleeps until the touch interrupt, a channel message or its next deadline. Every 5 s the RTT
//! log shows a `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use portable_atomic::{AtomicBool, Ordering};
use twine::core::Rotation;
use twine::engine::{HeapPeak, MemInfo};
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine::drivers::Calibration;
use twine::embassy::UiBuilderExt;
use {defmt_rtt as _, panic_probe as _};

/// The RP2350 boot ROM only starts images that carry an `IMAGE_DEF` block.
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: embassy_rp::block::ImageDef = embassy_rp::block::ImageDef::secure_exe();

// Feature groups (use `--no-default-features` to switch, or `cargo xtask firmware <board>
// --features …`, which replaces the default of the same group): exactly one display, at most one
// touch controller, at most one demo.
twine::feature_rules! {
    exactly_one "display": ["panel-ili9341", "panel-ili9342", "panel-st7789", "panel-st7796", "oled-ssd1306"];
    at_most_one "touch": ["touch-xpt2046", "touch-ft6x36", "touch-gt911", "touch-cst816s"];
    at_most_one "demo": ["demo-counter", "demo-controls", "demo-selection", "demo-calibrate", "demo-lottie"];
    // The OLED uses the touch controller's I2C pins.
    excludes "oled-ssd1306": ["touch"];
    requires "demo-calibrate": ["touch"];
}

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
    // SPI panel on SPI0 with TX DMA (the defaults match a Pico 2 with a 2.8" ILI9341 + XPT2046
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
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_240X320;
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
        let d = twine::drivers::ili9341::new_async(lcd, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine::drivers::ili9342::new_async(lcd, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine::drivers::st7789::new_async(lcd, dc, rst, &twine::drivers::st7789::ST7789, ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine::drivers::st7796::new_async(lcd, dc, rst, ROTATION, delay).await;
        d.unwrap_or_else(|e| defmt::panic!("display init failed: {}", e))
    };
    #[cfg(feature = "oled-ssd1306")]
    let display = {
        let mut cfg = embassy_rp::i2c::Config::default();
        cfg.frequency = 400_000;
        let i2c = embassy_rp::i2c::I2c::new_async(oled_i2c, oled_scl, oled_sda, Irqs, cfg);
        let iface = twine::drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine::drivers::ssd1306::AsyncSsd1306::new(iface, twine::drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .await
            .unwrap_or_else(|e| defmt::panic!("display init failed: {}", e))
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
        twine::drivers::touch::Xpt2046::new(dev, Some(Input::new(touch_irq, Pull::Up)))
            .with_calibration(cal)
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        use embassy_rp::gpio::{Input, Pull};
        let i2c = embassy_rp::i2c::I2c::new_blocking(touch_i2c, touch_scl, touch_sda, embassy_rp::i2c::Config::default());
        let irq = Some(Input::new(touch_irq, Pull::Up));
        #[cfg(feature = "touch-ft6x36")]
        let t = twine::drivers::touch::Ft6x36::new(i2c, irq);
        #[cfg(feature = "touch-gt911")]
        let t = twine::drivers::touch::Gt911::new(i2c, irq);
        #[cfg(feature = "touch-cst816s")]
        let t = twine::drivers::touch::Cst816s::new(i2c, irq);
        t
    };

    // `demo-calibrate`: raw readings go to the calibration demo (through `demo::RAW`), not to
    // the UI; the wrapper does not fit the driver to the display, so they stay unclamped.
    #[cfg(feature = "demo-calibrate")]
    let touch = twine_demos::calibration::RawTouchInput::new(touch, &demo::RAW);

    // The UI, drawing into the two DMA-pipelined `BUFS` (declared with `draw_buffers!` below).
    // The demos' configuration — the one their simulator examples and tests run — plus this
    // board's hooks. An OLED build takes it with the monochrome theme (white on black: lit
    // pixels are the foreground), so the default theme is not linked.
    #[cfg(not(feature = "oled-ssd1306"))]
    let mut config = twine_demos::config();
    #[cfg(feature = "oled-ssd1306")]
    let mut config = twine_demos::config_with_theme(
        MonoTheme::builder().mode(ThemeMode::Dark).font(&twine::assets::fonts::MONTSERRAT_14).build(),
    );
    config.engine.mem_info = Some(mem_info);
    config.engine.hires_timer = Some(twine::embassy::hires_now);
    // The runtime belongs to this task: the `!Send` token keeps the UI and every reactive
    // handle here; interrupt handlers and other tasks only use channels and the UI waker.
    let rt = Runtime::take().expect("runtime already taken");
    let builder = Ui::builder_async(display).runtime(rt);
    let builder = builder.buffers(BufferMode::partial_double_from(BUFS.take().expect("draw buffers already taken")));
    #[cfg(feature = "touch")]
    let builder = builder.input_wait(touch);
    let ui = builder.app_config(config).with_embassy_platform().build(demo::app);
    defmt::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine::embassy::run(ui).await
}

#[cfg(feature = "spi-panel")]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

// The draw buffers: sized at compile time, 4-byte aligned by their type, zeroed `.bss`, taken
// once in `main`.
twine::draw_buffers! {
    /// Two DMA-pipelined partial draw buffers: 40 rows of the 320 px panel (RGB565).
    #[cfg(feature = "spi-panel")]
    static BUFS: 2 x 40 rows x 320 px @ Rgb565Swapped;
    /// Two draw buffers of the whole 128 × 64 OLED at 1 bpp.
    #[cfg(feature = "oled-ssd1306")]
    static BUFS: 2 x 64 rows x 128 px @ I1;
}

/// Calibration mode: the touch driver reports raw readings (identity calibration; not clamped,
/// as `RawTouchInput` does not fit it to the display).
#[cfg(feature = "touch-xpt2046")]
const CALIBRATE: bool = cfg!(feature = "demo-calibrate");

/// The demo selected by cargo feature (the counter without a `demo-*` feature).
mod demo {
    /// The raw taps of the calibration demo (ports pattern): owned here, passed to
    /// `RawTouchInput` and to the demo.
    #[cfg(feature = "demo-calibrate")]
    pub static RAW: twine_demos::calibration::RawTaps = twine_demos::calibration::RawTaps::new();

    /// The calibration demo, reading the raw taps of [`RAW`].
    #[cfg(feature = "demo-calibrate")]
    pub fn app(cx: twine::prelude::Scope) -> impl twine::prelude::View {
        twine_demos::calibration::app(cx, &RAW)
    }
    #[cfg(feature = "demo-controls")]
    pub use twine_demos::controls::app;
    #[cfg(feature = "demo-selection")]
    pub use twine_demos::selection::app;
    #[cfg(feature = "demo-lottie")]
    pub use twine_demos::lottie::app;
    #[cfg(not(any(
        feature = "demo-controls",
        feature = "demo-selection",
        feature = "demo-calibrate",
        feature = "demo-lottie",
    )))]
    pub use twine_demos::counter::app;

    /// Name of the demo (for the log).
    pub const NAME: &str = if cfg!(feature = "demo-calibrate") {
        "calibrate"
    } else if cfg!(feature = "demo-controls") {
        "controls"
    } else if cfg!(feature = "demo-selection") {
        "selection"
    } else if cfg!(feature = "demo-lottie") {
        "lottie"
    } else {
        "counter"
    };
}

/// Heap size (the RP2350 has 520 KiB of SRAM; the draw buffers are static).
const HEAP_BYTES: usize = 256 * 1024;

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
    static PEAK: HeapPeak = HeapPeak::new();
    let m = PEAK.sample(HEAP.used(), HEAP.free());
    if m.used_percent() > 90 {
        defmt::warn!("heap above 90%: {} of {} bytes", m.used, m.used + m.free);
    }
    m
}
