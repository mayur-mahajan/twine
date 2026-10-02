//! Example firmware: Twine on an ESP32-S3 with an SPI display (ILI9341, ILI9342, ST7789,
//! ST7796) or a quad-SPI AMOLED (CO5300, SH8601, RM67162) and a touch controller, or a mono
//! SSD1306 I2C OLED. A template to copy, not board support: every pin, the rotation and the bus
//! clocks are in the wiring block at the top of `main`; cargo features pick the display
//! (`panel-*`, `oled-ssd1306`), the touch driver (`touch-*`, or none) and the demo
//! (`demo-counter`, `demo-controls`, `demo-selection`, `demo-calibrate`, `demo-multilang`, `demo-lottie`). `firmware/README.md`
//! lists the pins of known boards.
//!
//! The display is flushed with SPI DMA while the next chunk renders (`twine_embassy::run`);
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
use static_cell::ConstStaticCell;
#[cfg(any(feature = "spi-panel", feature = "touch-xpt2046"))]
use static_cell::StaticCell;
use twine::core::Rotation;
use twine::engine::{EngineConfig, MemInfo};
use twine::hal::AsyncDisplayDriver;
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine_drivers::Calibration;
use twine_embassy::UiBuilderExt;

esp_bootloader_esp_idf::esp_app_desc!();

// Feature groups (use `--no-default-features` to switch, or `cargo xtask firmware <board>
// --features …`, which replaces the default of the same group): exactly one display, at most one
// touch controller, at most one demo.
#[cfg(not(any(
    feature = "panel-ili9341",
    feature = "panel-ili9342",
    feature = "panel-st7789",
    feature = "panel-st7796",
    feature = "panel-co5300",
    feature = "panel-sh8601",
    feature = "panel-rm67162",
    feature = "oled-ssd1306"
)))]
compile_error!("enable one display feature: panel-ili9341, panel-ili9342, panel-st7789, panel-st7796, panel-co5300, panel-sh8601, panel-rm67162, oled-ssd1306");
#[cfg(any(
    all(feature = "panel-ili9341", any(feature = "panel-ili9342", feature = "panel-st7789", feature = "panel-st7796", feature = "panel-co5300", feature = "panel-sh8601", feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-ili9342", any(feature = "panel-st7789", feature = "panel-st7796", feature = "panel-co5300", feature = "panel-sh8601", feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-st7789", any(feature = "panel-st7796", feature = "panel-co5300", feature = "panel-sh8601", feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-st7796", any(feature = "panel-co5300", feature = "panel-sh8601", feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-co5300", any(feature = "panel-sh8601", feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-sh8601", any(feature = "panel-rm67162", feature = "oled-ssd1306")),
    all(feature = "panel-rm67162", feature = "oled-ssd1306"),
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
    all(feature = "demo-counter", any(feature = "demo-controls", feature = "demo-selection", feature = "demo-calibrate", feature = "demo-multilang", feature = "demo-lottie")),
    all(feature = "demo-controls", any(feature = "demo-selection", feature = "demo-calibrate", feature = "demo-multilang", feature = "demo-lottie")),
    all(feature = "demo-selection", any(feature = "demo-calibrate", feature = "demo-multilang", feature = "demo-lottie")),
    all(feature = "demo-calibrate", any(feature = "demo-multilang", feature = "demo-lottie")),
    all(feature = "demo-multilang", feature = "demo-lottie"),
))]
compile_error!("enable at most one demo-* feature");
#[cfg(all(feature = "demo-calibrate", not(feature = "touch")))]
compile_error!("demo-calibrate needs a touch-* feature");

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
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
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
        let d = twine_drivers::ili9341::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine_drivers::ili9342::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine_drivers::st7789::new_async(spi, dc, Some(rst), &twine_drivers::st7789::ST7789, ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine_drivers::st7796::new_async(spi, dc, Some(rst), ROTATION, delay).await;
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
        let d = twine_drivers::co5300::new_async(bus, Some(rst), &twine_drivers::co5300::CO5300_410X502, ROTATION, delay).await;
        #[cfg(feature = "panel-sh8601")]
        let d = twine_drivers::sh8601::new_async(bus, Some(rst), &twine_drivers::sh8601::SH8601_368X448, ROTATION, delay).await;
        #[cfg(feature = "panel-rm67162")]
        let d = twine_drivers::rm67162::new_async(bus, Some(rst), &twine_drivers::rm67162::RM67162_240X536, ROTATION, delay).await;
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
        let iface = twine_drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine_drivers::ssd1306::AsyncSsd1306::new(iface, twine_drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .await
            .unwrap_or_else(|e| panic!("display init failed: {e:?}"))
    };
    let info = display.info();
    let native = if ROTATION.swaps_axes() { (info.height, info.width) } else { (info.width, info.height) };

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
        twine_drivers::touch::Xpt2046::new(dev, Some(irq))
            .with_calibration(cal)
            .with_screen_size(if CALIBRATE { 4096 } else { info.width }, if CALIBRATE { 4096 } else { info.height })
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        let i2c = esp_hal::i2c::master::I2c::new(p.I2C0, esp_hal::i2c::master::Config::default())
            .unwrap()
            .with_sda(touch_sda)
            .with_scl(touch_scl);
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
        let transform = twine_drivers::touch::TouchTransform::for_rotation(ROTATION, native.0, native.1);
        #[cfg(feature = "touch-ft6x36")]
        let t = twine_drivers::touch::Ft6x36::new(i2c, Some(irq), transform).with_model(twine_drivers::touch::FtModel::Ft3168);
        #[cfg(feature = "touch-gt911")]
        let t = twine_drivers::touch::Gt911::new(i2c, Some(irq), transform);
        #[cfg(feature = "touch-cst816s")]
        let t = twine_drivers::touch::Cst816s::new(i2c, Some(irq), transform);
        t
    };
    let _ = native;

    // `demo-calibrate`: raw readings go to the calibration demo, not to the UI.
    #[cfg(feature = "demo-calibrate")]
    let touch = twine_demos::calibration::RawTouchInput::new(touch);

    // The UI: two DMA-pipelined partial buffers (25 KiB each: 40 rows of 320 px, fewer rows of
    // wider panels; each chunk is a single DMA transfer. OLED: the whole 128 × 64 × 1 bpp frame).
    static BUF_A: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    static BUF_B: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    let mut config = EngineConfig::default();
    config.mem_info = Some(mem_info);
    config.hires_timer = Some(twine_embassy::hires_now);
    // SAFETY: the UI and every reactive handle live in this task on this core's executor and are
    // never touched from an interrupt handler, another executor or the second core; other
    // contexts only use channels and the UI waker.
    let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
    let builder = builder.buffers(BufferMode::partial_double(&mut BUF_A.take().0, &mut BUF_B.take().0));
    #[cfg(feature = "touch")]
    let builder = builder.input_wait(touch);
    #[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
    let theme = DefaultTheme::light();
    // White on black: lit OLED pixels are the foreground.
    #[cfg(feature = "oled-ssd1306")]
    let theme = MonoTheme::builder().mode(ThemeMode::Dark).font(&twine::assets::fonts::MONTSERRAT_14).build();
    let ui = builder.config(config).theme(theme).with_embassy_clock().build(demo::app);
    log::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine_embassy::run(ui).await
}

/// Bytes of one partial draw buffer: 40 rows of a 320 px panel, fewer rows of wider ones.
#[cfg(any(feature = "spi-panel", feature = "qspi-panel"))]
const BUFFER_BYTES: usize = 320 * 40 * 2;
/// Bytes of one draw buffer: the whole 128 × 64 OLED at 1 bpp.
#[cfg(feature = "oled-ssd1306")]
const BUFFER_BYTES: usize = 128 * 64 / 8;

/// A 4-byte aligned draw buffer (the engine needs word-aligned buffers; DMA reads it in place).
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
    use core::sync::atomic::{AtomicU32, Ordering};
    static PEAK: AtomicU32 = AtomicU32::new(0);
    let used = esp_alloc::HEAP.used() as u32;
    let free = esp_alloc::HEAP.free() as u32;
    let peak = PEAK.fetch_max(used, Ordering::Relaxed).max(used);
    if u64::from(used) * 10 > u64::from(used + free) * 9 {
        log::warn!("heap above 90%: {used} of {} bytes", used + free);
    }
    MemInfo { used, peak, free }
}
