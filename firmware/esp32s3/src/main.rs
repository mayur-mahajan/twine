//! Example firmware: Twine on an ESP32-S3 with an SPI display (ILI9341, ILI9342, ST7789,
//! ST7796) or a quad-SPI AMOLED (CO5300, SH8601, RM67162) and a touch controller, or a mono
//! SSD1306 I2C OLED. A template to copy, not board support: every pin, the rotation and the bus
//! clocks are in the wiring block at the top of `main`; cargo features pick the display
//! (`panel-*`, `oled-ssd1306`), the touch driver (`touch-*`, or none) and the demo
//! (`demo-counter`, `demo-controls`, `demo-selection`, `demo-calibrate`, `demo-multilang`, `demo-lottie`). `firmware/README.md`
//! lists the pins of known boards.
//!
//! The display is flushed with SPI DMA while the next chunk renders (`twine::embassy::run`);
//! the UI sleeps until the touch interrupt, a channel message or its next deadline. Every
//! 5 s the log shows a `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

#[cfg(feature = "touch-xpt2046")]
use core::cell::RefCell;

#[cfg(feature = "touch-xpt2046")]
use embassy_sync::blocking_mutex::NoopMutex;
#[cfg(any(feature = "spi-panel", feature = "touch-xpt2046"))]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
#[cfg(feature = "touch")]
use esp_hal::gpio::{Input, InputConfig, Pull};
#[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
use esp_hal::gpio::{Level, Output, OutputConfig};
#[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
#[cfg(any(feature = "spi-panel", feature = "touch-xpt2046"))]
use static_cell::StaticCell;
use twine::core::Rotation;
use twine::engine::{HeapPeak, MemInfo};
use twine::hal::AsyncDisplayDriver;
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine::drivers::Calibration;
use twine::embassy::UiBuilderExt;

esp_bootloader_esp_idf::esp_app_desc!();

// Feature groups (use `--no-default-features` to switch, or `cargo xtask firmware <board>
// --features …`, which replaces the default of the same group): exactly one display, at most one
// touch controller, at most one demo.
twine::feature_rules! {
    exactly_one "display": ["panel-ili9341", "panel-ili9342", "panel-st7789", "panel-st7796", "panel-co5300", "panel-sh8601", "panel-rm67162", "oled-ssd1306"];
    at_most_one "touch": ["touch-xpt2046", "touch-ft6x36", "touch-gt911", "touch-cst816s"];
    at_most_one "demo": ["demo-counter", "demo-controls", "demo-selection", "demo-calibrate", "demo-lottie", "demo-multilang"];
    // The OLED uses the touch controller's I2C pins.
    excludes "oled-ssd1306": ["touch"];
    requires "demo-calibrate": ["touch"];
}

/// Display rotation: SPI panels turned to landscape (320 × 240, ST7796: 480 × 320).
#[cfg(any(feature = "panel-ili9341", feature = "panel-st7789", feature = "panel-st7796"))]
const ROTATION: Rotation = Rotation::Deg90;
/// Display rotation: the ILI9342C, the portrait AMOLEDs and the OLED as they are.
#[cfg(any(feature = "panel-ili9342", feature = "panel-co5300", feature = "panel-sh8601", feature = "oled-ssd1306"))]
const ROTATION: Rotation = Rotation::Deg0;
/// Display rotation: landscape 536 × 240, the vendor's default.
#[cfg(feature = "panel-rm67162")]
const ROTATION: Rotation = Rotation::Deg270;

#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 180 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // ================================ Wiring: edit for your board ================================
    // SPI panel on SPI2 with DMA (ESP32-S3-DevKitC-1 + 2.8" ILI9341 module); every SPI
    // `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (lcd_sck, lcd_mosi, lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.GPIO12, p.GPIO11, p.GPIO10, p.GPIO9, p.GPIO8, p.GPIO7);
    #[cfg(feature = "spi-panel")]
    const LCD_MHZ: u32 = 60;
    // QSPI AMOLED on SPI2 with DMA (defaults: Waveshare ESP32-S3-Touch-AMOLED-2.06).
    #[cfg(feature = "qspi-panel")]
    let (qspi_sck, qspi_cs, qspi_d0, qspi_d1, qspi_d2, qspi_d3, lcd_rst) =
        (p.GPIO11, p.GPIO12, p.GPIO4, p.GPIO5, p.GPIO6, p.GPIO7, p.GPIO8);
    #[cfg(feature = "qspi-panel")]
    const LCD_MHZ: u32 = 40;
    // XPT2046 resistive touch on SPI3 (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_sck, touch_mosi, touch_miso, touch_cs, touch_irq) = (p.GPIO4, p.GPIO5, p.GPIO6, p.GPIO15, p.GPIO16);
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_240X320;
    // Capacitive touch on I2C0 (FT6x36/FT3168, GT911, CST816S): SDA GPIO15, SCL GPIO14,
    // INT GPIO21.
    #[cfg(feature = "i2c-touch")]
    let (touch_sda, touch_scl, touch_irq) = (p.GPIO15, p.GPIO14, p.GPIO21);
    // SSD1306 OLED on the same I2C0 pins (feature `oled-ssd1306`, address 0x3C, 400 kHz).
    #[cfg(feature = "oled-ssd1306")]
    let (oled_sda, oled_scl) = (p.GPIO15, p.GPIO14);
    // ============================================================================================

    #[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
    let delay = &mut embassy_time::Delay;
    #[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
    let rst = Output::new(lcd_rst, Level::High, OutputConfig::default());
    #[cfg(feature = "spi-panel")]
    let _backlight = Output::new(lcd_backlight, Level::High, OutputConfig::default());

    // Display: SPI panels through the shared-bus async `SpiDevice` (owns CS).
    #[cfg(feature = "spi-panel")]
    let display = {
        use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
        type Bus = embassy_sync::mutex::Mutex<NoopRawMutex, esp_hal::spi::master::SpiDma<'static, esp_hal::Async>>;
        static BUS: StaticCell<Bus> = StaticCell::new();
        let spi = Spi::new(p.SPI2, SpiConfig::default().with_frequency(Rate::from_mhz(LCD_MHZ)))
            .unwrap()
            .with_sck(lcd_sck)
            .with_mosi(lcd_mosi)
            .with_dma(p.DMA_CH0)
            .into_async();
        let bus = BUS.init(embassy_sync::mutex::Mutex::new(spi));
        let spi = SpiDevice::new(bus, Output::new(lcd_cs, Level::High, OutputConfig::default()));
        let dc = Output::new(lcd_dc, Level::Low, OutputConfig::default());
        #[cfg(feature = "panel-ili9341")]
        let d = twine::drivers::ili9341::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine::drivers::ili9342::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine::drivers::st7789::new_async(spi, dc, Some(rst), &twine::drivers::st7789::ST7789, ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine::drivers::st7796::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        d.unwrap_or_else(|e| panic!("display init failed: {e:?}"))
    };
    // Display: QSPI AMOLEDs through `twine_esp::EspQspi` (hardware CS).
    #[cfg(feature = "qspi-panel")]
    let display = {
        let spi = Spi::new(p.SPI2, SpiConfig::default().with_frequency(Rate::from_mhz(LCD_MHZ)))
            .unwrap()
            .with_sck(qspi_sck)
            .with_cs(qspi_cs)
            .with_sio0(qspi_d0)
            .with_sio1(qspi_d1)
            .with_sio2(qspi_d2)
            .with_sio3(qspi_d3)
            .with_dma(p.DMA_CH0)
            .into_async();
        let bus = twine_esp::EspQspi::new(spi);
        #[cfg(feature = "panel-co5300")]
        let d = twine::drivers::co5300::new_async(bus, Some(rst), &twine::drivers::co5300::CO5300_410X502, ROTATION, delay).await;
        #[cfg(feature = "panel-sh8601")]
        let d = twine::drivers::sh8601::new_async(bus, Some(rst), &twine::drivers::sh8601::SH8601_368X448, ROTATION, delay).await;
        #[cfg(feature = "panel-rm67162")]
        let d = twine::drivers::rm67162::new_async(bus, Some(rst), &twine::drivers::rm67162::RM67162_240X536, ROTATION, delay).await;
        d.unwrap_or_else(|e| panic!("display init failed: {e:?}"))
    };
    #[cfg(feature = "oled-ssd1306")]
    let display = {
        let cfg = esp_hal::i2c::master::Config::default().with_frequency(Rate::from_khz(400));
        let i2c = esp_hal::i2c::master::I2c::new(p.I2C0, cfg)
            .unwrap()
            .with_sda(oled_sda)
            .with_scl(oled_scl)
            .into_async();
        let iface = twine::drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine::drivers::ssd1306::AsyncSsd1306::new(iface, twine::drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .await
            .unwrap_or_else(|e| panic!("display init failed: {e:?}"))
    };
    let info = display.info();

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
        type Bus = NoopMutex<RefCell<Spi<'static, esp_hal::Blocking>>>;
        static BUS: StaticCell<Bus> = StaticCell::new();
        let spi = Spi::new(p.SPI3, SpiConfig::default().with_frequency(Rate::from_mhz(2)))
            .unwrap()
            .with_sck(touch_sck)
            .with_mosi(touch_mosi)
            .with_miso(touch_miso);
        let bus = BUS.init(NoopMutex::new(RefCell::new(spi)));
        let dev = SpiDevice::new(bus, Output::new(touch_cs, Level::High, OutputConfig::default()));
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
        let cal = if CALIBRATE { Calibration::IDENTITY } else { TOUCH_CAL };
        twine::drivers::touch::Xpt2046::new(dev, Some(irq))
            .with_calibration(cal)
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        let i2c = esp_hal::i2c::master::I2c::new(p.I2C0, esp_hal::i2c::master::Config::default())
            .unwrap()
            .with_sda(touch_sda)
            .with_scl(touch_scl);
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
        #[cfg(feature = "touch-ft6x36")]
        let t = twine::drivers::touch::Ft6x36::new(i2c, Some(irq)).with_model(twine::drivers::touch::FtModel::Ft3168);
        #[cfg(feature = "touch-gt911")]
        let t = twine::drivers::touch::Gt911::new(i2c, Some(irq));
        #[cfg(feature = "touch-cst816s")]
        let t = twine::drivers::touch::Cst816s::new(i2c, Some(irq));
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
    log::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine::embassy::run(ui).await
}

// The draw buffers: sized at compile time, 4-byte aligned by their type, zeroed `.bss`, taken
// once in `main`.
twine::draw_buffers! {
    /// Two DMA-pipelined partial draw buffers of 25 KiB: 40 rows of a 320 px panel (wider
    /// panels render fewer rows per chunk). The DMA reads them in place.
    #[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
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
    #[cfg(feature = "demo-multilang")]
    pub use twine_demos::multilang::app;
    #[cfg(feature = "demo-lottie")]
    pub use twine_demos::lottie::app;
    #[cfg(not(any(
        feature = "demo-controls",
        feature = "demo-selection",
        feature = "demo-calibrate",
        feature = "demo-multilang",
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
    } else if cfg!(feature = "demo-multilang") {
        "multilang"
    } else if cfg!(feature = "demo-lottie") {
        "lottie"
    } else {
        "counter"
    };
}

/// Heap statistics for the `twine::perf` log line; warns when the heap is more than 90 % full.
fn mem_info() -> MemInfo {
    static PEAK: HeapPeak = HeapPeak::new();
    let m = PEAK.sample(esp_alloc::HEAP.used(), esp_alloc::HEAP.free());
    if m.used_percent() > 90 {
        log::warn!("heap above 90%: {} of {} bytes", m.used, m.used + m.free);
    }
    m
}
