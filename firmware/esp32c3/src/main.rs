//! Example firmware: Twine on an ESP32-C3 (RISC-V) with an SPI display and a touch
//! controller, or a mono I2C OLED. A template to copy, not board support: every pin, the
//! rotation and the bus clocks are in the wiring block at the top of `main`; cargo features pick
//! the display (`panel-ili9341`, `panel-ili9342`, `panel-st7789`, `panel-st7796`,
//! `oled-ssd1306`), the touch driver (`touch-xpt2046`, `touch-ft6x36`, `touch-gt911`,
//! `touch-cst816s`, or none) and the demo (`demo-counter`, `demo-controls`, `demo-selection`,
//! `demo-calibrate`).
//!
//! The C3 has one general-purpose SPI (SPI2): the display and an XPT2046 share it with their own
//! chip selects. The display is flushed with SPI DMA while the next chunk renders
//! (`twine::embassy::run`); the touch reads use the bus between flushes. The UI sleeps until the
//! touch interrupt, a channel message or its next deadline. Every 5 s the log shows a
//! `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

#[cfg(feature = "spi-panel")]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
#[cfg(feature = "spi-panel")]
use embassy_sync::mutex::Mutex;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
#[cfg(feature = "touch")]
use esp_hal::gpio::{Input, InputConfig, Pull};
#[cfg(feature = "spi-panel")]
use esp_hal::gpio::{Level, Output, OutputConfig};
#[cfg(feature = "spi-panel")]
use esp_hal::spi::master::{Config as SpiConfig, Spi, SpiDma};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
#[cfg(feature = "spi-panel")]
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
    exactly_one "display": ["panel-ili9341", "panel-ili9342", "panel-st7789", "panel-st7796", "oled-ssd1306"];
    at_most_one "touch": ["touch-xpt2046", "touch-ft6x36", "touch-gt911", "touch-cst816s"];
    at_most_one "demo": ["demo-counter", "demo-controls", "demo-selection", "demo-calibrate"];
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

/// The shared SPI bus.
#[cfg(feature = "spi-panel")]
type Bus = Mutex<NoopRawMutex, SpiDma<'static, esp_hal::Async>>;

#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    esp_alloc::heap_allocator!(size: 96 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // ================================ Wiring: edit for your board ================================
    // SPI2 with DMA, shared by display and touch (ESP32-C3-DevKitM-1 + 2.8" ILI9341 module; avoid
    // GPIO12–17 (flash) and GPIO18/19 (USB)).
    // Every `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (spi_sck, spi_mosi, spi_miso) = (p.GPIO6, p.GPIO7, p.GPIO5);
    #[cfg(feature = "spi-panel")]
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.GPIO10, p.GPIO4, p.GPIO3, p.GPIO1);
    #[cfg(feature = "spi-panel")]
    const LCD_MHZ: u32 = 40;
    // XPT2046 on the same bus (feature `touch-xpt2046`). GPIO8 is a strapping pin with a pull-up
    // (and drives the DevKitM-1's RGB LED); T_IRQ idles high, which keeps it compatible.
    #[cfg(feature = "touch-xpt2046")]
    let (touch_cs, touch_irq) = (p.GPIO0, p.GPIO8);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_MHZ: u32 = 2;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_240X320;
    // Capacitive touch (FT6x36, GT911, CST816S) on I2C0: SDA GPIO0, SCL GPIO2, INT GPIO8.
    #[cfg(feature = "i2c-touch")]
    let (touch_sda, touch_scl, touch_irq) = (p.GPIO0, p.GPIO2, p.GPIO8);
    // SSD1306 OLED on the same I2C0 pins (feature `oled-ssd1306`, address 0x3C, 400 kHz).
    #[cfg(feature = "oled-ssd1306")]
    let (oled_sda, oled_scl) = (p.GPIO0, p.GPIO2);
    // ============================================================================================

    #[cfg(feature = "spi-panel")]
    let delay = &mut embassy_time::Delay;
    #[cfg(feature = "spi-panel")]
    let _backlight = Output::new(lcd_backlight, Level::High, OutputConfig::default());
    #[cfg(feature = "spi-panel")]
    let lcd_config = SpiConfig::default().with_frequency(Rate::from_mhz(LCD_MHZ));

    // The bus: SPI2 + DMA (the display writes its buffers in place, without a copy).
    #[cfg(feature = "spi-panel")]
    static BUS: StaticCell<Bus> = StaticCell::new();
    #[cfg(feature = "spi-panel")]
    let spi = Spi::new(p.SPI2, lcd_config)
        .unwrap()
        .with_sck(spi_sck)
        .with_mosi(spi_mosi)
        .with_miso(spi_miso)
        .with_dma(p.DMA_CH0)
        .into_async();
    #[cfg(feature = "spi-panel")]
    let bus: &'static Bus = BUS.init(Mutex::new(spi));

    // Display: async `SpiDevice` with its own clock (re-applied per transaction).
    #[cfg(feature = "spi-panel")]
    let display = {
        use embassy_embedded_hal::shared_bus::asynch::spi::SpiDeviceWithConfig;
        let spi = SpiDeviceWithConfig::new(bus, Output::new(lcd_cs, Level::High, OutputConfig::default()), lcd_config);
        let dc = Output::new(lcd_dc, Level::Low, OutputConfig::default());
        let rst = Some(Output::new(lcd_rst, Level::High, OutputConfig::default()));
        #[cfg(feature = "panel-ili9341")]
        let d = twine::drivers::ili9341::new_async(spi, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine::drivers::ili9342::new_async(spi, dc, rst, ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine::drivers::st7789::new_async(spi, dc, rst, &twine::drivers::st7789::ST7789, ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine::drivers::st7796::new_async(spi, dc, rst, ROTATION, delay).await;
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
        let dev = shared::TryLockDevice::new(
            bus,
            Output::new(touch_cs, Level::High, OutputConfig::default()),
            SpiConfig::default().with_frequency(Rate::from_mhz(TOUCH_MHZ)),
        );
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
        let irq = Some(Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up)));
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
    log::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine::embassy::run(ui).await
}

/// A blocking `SpiDevice` on the async display bus, for the touch controller.
#[cfg(feature = "touch-xpt2046")]
mod shared {
    use embassy_embedded_hal::SetConfig;
    use embedded_hal::spi::{ErrorKind, ErrorType, Operation, SpiBus, SpiDevice};
    use esp_hal::gpio::Output;
    use esp_hal::spi::master::Config;

    use super::Bus;

    /// Why a touch transaction failed.
    #[derive(Debug)]
    pub enum Error {
        /// The display holds the bus (a flush is in progress); the touch read is skipped.
        Busy,
        /// The SPI driver failed.
        Spi(#[allow(dead_code)] esp_hal::spi::Error),
    }

    impl embedded_hal::spi::Error for Error {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    /// Chip select + clock of one device on the shared bus. A transaction only runs when the
    /// bus is free (`try_lock`): the UI reads inputs between flushes, so it always is there.
    pub struct TryLockDevice {
        bus: &'static Bus,
        cs: Output<'static>,
        config: Config,
    }

    impl TryLockDevice {
        /// A device with chip select `cs` and clock `config`.
        pub fn new(bus: &'static Bus, cs: Output<'static>, config: Config) -> Self {
            Self { bus, cs, config }
        }
    }

    impl ErrorType for TryLockDevice {
        type Error = Error;
    }

    impl SpiDevice for TryLockDevice {
        fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Error> {
            let mut bus = self.bus.try_lock().map_err(|_| Error::Busy)?;
            bus.set_config(&self.config).map_err(|_| Error::Busy)?;
            self.cs.set_low();
            let r = operations.iter_mut().try_for_each(|op| match op {
                Operation::Read(buf) => SpiBus::read(&mut *bus, buf),
                Operation::Write(buf) => SpiBus::write(&mut *bus, buf),
                Operation::Transfer(read, write) => SpiBus::transfer(&mut *bus, read, write),
                Operation::TransferInPlace(buf) => SpiBus::transfer_in_place(&mut *bus, buf),
                Operation::DelayNs(ns) => {
                    esp_hal::delay::Delay::new().delay_nanos(*ns);
                    Ok(())
                }
            });
            let flushed = SpiBus::flush(&mut *bus);
            self.cs.set_high();
            r.and(flushed).map_err(Error::Spi)
        }
    }
}

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
    #[cfg(not(any(feature = "demo-controls", feature = "demo-calibrate", feature = "demo-selection")))]
    pub use twine_demos::counter::app;

    /// Name of the demo (for the log).
    pub const NAME: &str = if cfg!(feature = "demo-calibrate") {
        "calibrate"
    } else if cfg!(feature = "demo-controls") {
        "controls"
    } else if cfg!(feature = "demo-selection") {
        "selection"
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
