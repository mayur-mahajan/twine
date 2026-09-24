//! Example firmware: Twine on an ESP32 (Xtensa) with an SPI display (ILI9341, ST7789) and a
//! touch controller. A template to copy, not board support: every pin, the rotation and the
//! bus clocks are in the wiring block at the top of `main`; cargo features pick the panel
//! (`panel-*`), the touch driver (`touch-*`) and the demo (`demo-counter`, `demo-controls`,
//! `demo-calibrate`). The default pins are those of the ESP32-2432S028R "Cheap Yellow Display"
//! (see `firmware/README.md`).
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
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use static_cell::{ConstStaticCell, StaticCell};
use twine::core::Rotation;
use twine::engine::{EngineConfig, MemInfo};
use twine::hal::AsyncDisplayDriver;
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine_drivers::Calibration;
use twine_embassy::UiBuilderExt;

esp_bootloader_esp_idf::esp_app_desc!();

/// Exactly one `panel-*` feature must be enabled (use `--no-default-features` to switch).
const _: () = assert!(
    cfg!(feature = "panel-ili9341") as u8 + cfg!(feature = "panel-st7789") as u8 == 1,
    "enable exactly one panel-* feature"
);

#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);
    esp_alloc::heap_allocator!(size: 48 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // ================================ Wiring: edit for your board ================================
    // SPI panel on SPI2 (HSPI) with DMA; the reset line is tied to EN on the Cheap Yellow Display.
    let (lcd_sck, lcd_mosi, lcd_cs, lcd_dc, lcd_backlight) = (p.GPIO14, p.GPIO13, p.GPIO15, p.GPIO2, p.GPIO21);
    let lcd_rst: Option<Output<'static>> = None;
    const LCD_MHZ: u32 = 40;
    const ROTATION: Rotation = Rotation::Deg90; // the panel turned 90° clockwise: 320 × 240
    // XPT2046 resistive touch on SPI3 (VSPI) (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_sck, touch_mosi, touch_miso, touch_cs, touch_irq) = (p.GPIO25, p.GPIO32, p.GPIO39, p.GPIO33, p.GPIO36);
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
    // FT6x36 capacitive touch on I2C0 (feature `touch-ft6x36`; the CYD's free connector pins).
    #[cfg(feature = "touch-ft6x36")]
    let (touch_sda, touch_scl, touch_irq) = (p.GPIO22, p.GPIO27, p.GPIO35);
    // ============================================================================================

    let delay = &mut embassy_time::Delay;
    let _backlight = Output::new(lcd_backlight, Level::High, OutputConfig::default());

    // Display: SPI panel through the shared-bus async `SpiDevice` (owns CS).
    let display = {
        use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
        type Bus = embassy_sync::mutex::Mutex<NoopRawMutex, esp_hal::spi::master::SpiDma<'static, esp_hal::Async>>;
        static BUS: StaticCell<Bus> = StaticCell::new();
        let spi = Spi::new(p.SPI2, SpiConfig::default().with_frequency(Rate::from_mhz(LCD_MHZ)))
            .unwrap()
            .with_sck(lcd_sck)
            .with_mosi(lcd_mosi)
            .with_dma(p.DMA_SPI2)
            .into_async();
        let bus = BUS.init(embassy_sync::mutex::Mutex::new(spi));
        let spi = SpiDevice::new(bus, Output::new(lcd_cs, Level::High, OutputConfig::default()));
        let dc = Output::new(lcd_dc, Level::Low, OutputConfig::default());
        #[cfg(feature = "panel-ili9341")]
        let d = twine_drivers::ili9341::new_async(spi, dc, lcd_rst, ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine_drivers::st7789::new_async(spi, dc, lcd_rst, &twine_drivers::st7789::ST7789, ROTATION, delay).await;
        d.unwrap_or_else(|e| panic!("display init failed: {e:?}"))
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
        // GPIO34–39 have no internal pull resistors; the touch module pulls T_IRQ up.
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::None));
        let cal = if CALIBRATE { Calibration::IDENTITY } else { TOUCH_CAL };
        twine_drivers::touch::Xpt2046::new(dev, Some(irq))
            .with_calibration(cal)
            .with_screen_size(if CALIBRATE { 4096 } else { info.width }, if CALIBRATE { 4096 } else { info.height })
    };
    #[cfg(all(feature = "touch-ft6x36", not(feature = "touch-xpt2046")))]
    let touch = {
        let i2c = esp_hal::i2c::master::I2c::new(p.I2C0, esp_hal::i2c::master::Config::default())
            .unwrap()
            .with_sda(touch_sda)
            .with_scl(touch_scl);
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::None)); // input-only pin
        let native = if ROTATION.swaps_axes() { (info.height, info.width) } else { (info.width, info.height) };
        let transform = twine_drivers::touch::TouchTransform::for_rotation(ROTATION, native.0, native.1);
        twine_drivers::touch::Ft6x36::new(i2c, Some(irq), transform)
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
    // SAFETY: the UI and every reactive handle live in this task on this core's executor and are
    // never touched from an interrupt handler, another executor or the second core; other
    // contexts only use channels and the UI waker.
    let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
    let ui = builder
        .buffers(BufferMode::partial_double(&mut BUF_A.take().0, &mut BUF_B.take().0))
        .input_wait(touch)
        .config(config)
        .theme(DefaultTheme::light())
        .with_embassy_clock()
        .build(demo::app);
    log::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine_embassy::run(ui).await
}

/// Bytes of one partial draw buffer: 40 rows of 320 px (RGB565).
const BUFFER_BYTES: usize = 320 * 40 * 2;

/// A 4-byte aligned draw buffer (the engine needs word-aligned buffers; on the ESP32 the SPI DMA
/// reads 4-byte aligned internal RAM in place).
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
