//! Example firmware: Twine on an ESP32-C6 (RISC-V) with an SPI display (JD9853, ILI9341,
//! ILI9342, ST7789, ST7796) and a touch controller (AXS5106L, FT6x36, GT911, CST816S or
//! XPT2046), or a mono SSD1306 I2C OLED. A template to copy, not board support: every pin, the
//! rotation and the bus clocks are in the wiring block at the top of `main`; cargo features pick
//! the display (`panel-*`, `oled-ssd1306`), the touch driver (`touch-*`, or none) and the demo
//! (`demo-counter`, `demo-controls`, `demo-selection`, `demo-calibrate`).
//!
//! The default pins and features fit a 1.47" 172 × 320 JD9853 IPS panel with AXS5106L touch
//! (the wiring of Waveshare's ESP32-C6-Touch-LCD-1.47). The display is flushed with SPI DMA
//! while the next chunk renders (`twine_embassy::run`); an XPT2046 would share the bus with its
//! own chip select, capacitive touch is on I2C0. The UI sleeps until the touch interrupt, a
//! channel message or its next deadline. Every 5 s the log shows a `twine::perf` line (fps,
//! CPU, render and flush time, heap).
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
use static_cell::ConstStaticCell;
#[cfg(feature = "spi-panel")]
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
    feature = "panel-jd9853",
    feature = "panel-ili9341",
    feature = "panel-ili9342",
    feature = "panel-st7789",
    feature = "panel-st7796",
    feature = "oled-ssd1306"
)))]
compile_error!("enable one display feature: panel-jd9853, panel-ili9341, panel-ili9342, panel-st7789, panel-st7796, oled-ssd1306");
#[cfg(any(
    all(feature = "panel-jd9853", any(feature = "panel-ili9341", feature = "panel-ili9342", feature = "panel-st7789", feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-ili9341", any(feature = "panel-ili9342", feature = "panel-st7789", feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-ili9342", any(feature = "panel-st7789", feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-st7789", any(feature = "panel-st7796", feature = "oled-ssd1306")),
    all(feature = "panel-st7796", feature = "oled-ssd1306"),
))]
compile_error!("enable only one display feature (panel-* / oled-*)");
#[cfg(any(
    all(feature = "touch-axs5106l", any(feature = "touch-ft6x36", feature = "touch-gt911", feature = "touch-cst816s", feature = "touch-xpt2046")),
    all(feature = "touch-ft6x36", any(feature = "touch-gt911", feature = "touch-cst816s", feature = "touch-xpt2046")),
    all(feature = "touch-gt911", any(feature = "touch-cst816s", feature = "touch-xpt2046")),
    all(feature = "touch-cst816s", feature = "touch-xpt2046"),
))]
compile_error!("enable at most one touch-* feature");
#[cfg(all(feature = "oled-ssd1306", feature = "touch"))]
compile_error!("oled-ssd1306 uses the touch I2C pins: disable the touch-* feature");
#[cfg(any(
    all(feature = "demo-counter", any(feature = "demo-controls", feature = "demo-selection", feature = "demo-calibrate")),
    all(feature = "demo-controls", any(feature = "demo-selection", feature = "demo-calibrate")),
    all(feature = "demo-selection", feature = "demo-calibrate"),
))]
compile_error!("enable at most one demo-* feature");
#[cfg(all(feature = "demo-calibrate", not(feature = "touch")))]
compile_error!("demo-calibrate needs a touch-* feature");

/// Display rotation: portrait 172 × 320 on the 1.47" JD9853 panel.
#[cfg(feature = "panel-jd9853")]
const ROTATION: Rotation = Rotation::Deg0;
/// Display rotation: the other SPI panels turned to landscape (320 × 240, ST7796: 480 × 320).
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
    // The 64 KiB the second-stage bootloader used, plus 96 KiB of ordinary RAM.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 96 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // ================================ Wiring: edit for your board ================================
    // SPI2 with DMA. Defaults: Waveshare ESP32-C6-Touch-LCD-1.47 (pins from its demo's
    // `bsp_spi.h`, `bsp_display.h`, `bsp_i2c.h`, `bsp_touch.h`; the SD card slot shares SCK/MOSI/
    // MISO with its own CS on GPIO4). Avoid GPIO24–30 (flash) and GPIO12/13 (USB).
    // Every `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (spi_sck, spi_mosi, spi_miso) = (p.GPIO1, p.GPIO2, p.GPIO3);
    #[cfg(feature = "spi-panel")]
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.GPIO14, p.GPIO15, p.GPIO22, p.GPIO23);
    // The vendor demo clocks the JD9853 at 80 MHz; hand-wired modules are safer at 40.
    #[cfg(feature = "spi-panel")]
    const LCD_MHZ: u32 = if cfg!(feature = "panel-jd9853") { 80 } else { 40 };
    // Capacitive touch on I2C0: AXS5106L, FT6x36, GT911 or CST816S (SDA GPIO18, SCL GPIO19,
    // INT GPIO21).
    #[cfg(feature = "i2c-touch")]
    let (touch_sda, touch_scl, touch_irq) = (p.GPIO18, p.GPIO19, p.GPIO21);
    // SSD1306 OLED on the same I2C0 pins (feature `oled-ssd1306`, address 0x3C, 400 kHz).
    #[cfg(feature = "oled-ssd1306")]
    let (oled_sda, oled_scl) = (p.GPIO18, p.GPIO19);
    #[cfg(feature = "touch-axs5106l")]
    let touch_rst = p.GPIO20;
    /// The touch panel's raw X axis runs opposite to the display columns (true on the
    /// Waveshare 1.47" board, whose demo mirrors touch X at rotation 0).
    #[cfg(feature = "i2c-touch")]
    const TOUCH_MIRROR_RAW_X: bool = cfg!(feature = "touch-axs5106l");
    // XPT2046 on the same SPI bus (feature `touch-xpt2046`), T_IRQ idles high.
    #[cfg(feature = "touch-xpt2046")]
    let (touch_cs, touch_irq) = (p.GPIO4, p.GPIO21);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_MHZ: u32 = 2;
    /// Replace with the output of `--features demo-calibrate` (the placeholder assumes a
    /// 320 × 240 module at `Rotation::Deg90`).
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
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
        let spi = SpiDeviceWithConfig::new(
            bus,
            Output::new(lcd_cs, Level::High, OutputConfig::default()),
            lcd_config,
        );
        let dc = Output::new(lcd_dc, Level::Low, OutputConfig::default());
        let rst = Output::new(lcd_rst, Level::High, OutputConfig::default());
        #[cfg(feature = "panel-jd9853")]
        let d = twine_drivers::jd9853::new_async(
            spi,
            dc,
            Some(rst),
            &twine_drivers::jd9853::JD9853_172X320,
            ROTATION,
            delay,
        )
        .await;
        #[cfg(feature = "panel-ili9341")]
        let d = twine_drivers::ili9341::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-ili9342")]
        let d = twine_drivers::ili9342::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-st7796")]
        let d = twine_drivers::st7796::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine_drivers::st7789::new_async(
            spi,
            dc,
            Some(rst),
            &twine_drivers::st7789::ST7789,
            ROTATION,
            delay,
        )
        .await;
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

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        let dev = shared::TryLockDevice::new(
            bus,
            Output::new(touch_cs, Level::High, OutputConfig::default()),
            SpiConfig::default().with_frequency(Rate::from_mhz(TOUCH_MHZ)),
        );
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
        let cal = if CALIBRATE {
            Calibration::IDENTITY
        } else {
            TOUCH_CAL
        };
        twine_drivers::touch::Xpt2046::new(dev, Some(irq))
            .with_calibration(cal)
            .with_screen_size(
                if CALIBRATE { 4096 } else { info.width },
                if CALIBRATE { 4096 } else { info.height },
            )
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        use esp_hal::i2c::master::{Config as I2cConfig, I2c};
        let i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(400)))
            .unwrap()
            .with_sda(touch_sda)
            .with_scl(touch_scl);
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
        let native = if ROTATION.swaps_axes() {
            (info.height, info.width)
        } else {
            (info.width, info.height)
        };
        let mut transform = twine_drivers::touch::TouchTransform::for_rotation(ROTATION, native.0, native.1);
        if TOUCH_MIRROR_RAW_X {
            transform = transform.with_raw_mirror_x();
        }
        #[cfg(feature = "touch-axs5106l")]
        let t = {
            let rst = Output::new(touch_rst, Level::High, OutputConfig::default());
            let mut t = twine_drivers::touch::Axs5106l::new(i2c, Some(irq), transform).with_reset_pin(rst);
            t.reset(delay).unwrap_or_else(|e| match e {});
            match t.read_id() {
                Ok(id) => log::info!("axs5106l id {id:02x?}"),
                Err(e) => log::warn!("axs5106l: no answer ({e:?})"),
            }
            t
        };
        #[cfg(feature = "touch-ft6x36")]
        let t = twine_drivers::touch::Ft6x36::new(i2c, Some(irq), transform);
        #[cfg(feature = "touch-gt911")]
        let t = twine_drivers::touch::Gt911::new(i2c, Some(irq), transform);
        #[cfg(feature = "touch-cst816s")]
        let t = twine_drivers::touch::Cst816s::new(i2c, Some(irq), transform);
        t
    };

    // `demo-calibrate`: raw readings go to the calibration demo, not to the UI.
    #[cfg(feature = "demo-calibrate")]
    let touch = twine_demos::calibration::RawTouchInput::new(touch);

    // The UI: two DMA-pipelined partial buffers (SPI panels: 40 rows of 320 px; OLED: the
    // whole 128 × 64 × 1 bpp frame).
    static BUF_A: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    static BUF_B: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    let config = EngineConfig {
        mem_info: Some(mem_info),
        hires_timer: Some(twine_embassy::hires_now),
        ..EngineConfig::default()
    };
    // SAFETY: the UI and every reactive handle live in this task on the executor and are never
    // touched from an interrupt handler or another executor; other contexts only use channels
    // and the UI waker.
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
    log::info!("twine: {} demo on {}x{}", demo::NAME, info.width, info.height);
    twine_embassy::run(ui).await
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
        /// The SPI driver failed (the cause is only shown by `Debug`).
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
    use core::sync::atomic::{AtomicU32, Ordering};
    static PEAK: AtomicU32 = AtomicU32::new(0);
    let used = esp_alloc::HEAP.used() as u32;
    let free = esp_alloc::HEAP.free() as u32;
    let peak = PEAK.load(Ordering::Relaxed).max(used);
    PEAK.store(peak, Ordering::Relaxed);
    if u64::from(used) * 10 > u64::from(used + free) * 9 {
        log::warn!("heap above 90%: {used} of {} bytes", used + free);
    }
    MemInfo { used, peak, free }
}
