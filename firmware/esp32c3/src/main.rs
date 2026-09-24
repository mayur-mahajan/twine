//! Example firmware: Twine on an ESP32-C3 (RISC-V) with an SPI display (ILI9341, ST7789) and a
//! touch controller. A template to copy, not board support: every pin, the rotation and the
//! bus clocks are in the wiring block at the top of `main`; cargo features pick the panel
//! (`panel-*`), the touch driver (`touch-*`) and the demo (`demo-counter`, `demo-controls`,
//! `demo-calibrate`).
//!
//! The C3 has one general-purpose SPI (SPI2): the display and an XPT2046 share it with their own
//! chip selects. The display is flushed with SPI DMA while the next chunk renders
//! (`twine_embassy::run`); the touch reads use the bus between flushes. The UI sleeps until the
//! touch interrupt, a channel message or its next deadline. Every 5 s the log shows a
//! `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::spi::master::{Config as SpiConfig, Spi, SpiDma};
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

/// The shared SPI bus.
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
    let (spi_sck, spi_mosi, spi_miso) = (p.GPIO6, p.GPIO7, p.GPIO5);
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.GPIO10, p.GPIO4, p.GPIO3, p.GPIO1);
    const LCD_MHZ: u32 = 40;
    const ROTATION: Rotation = Rotation::Deg90; // the panel turned 90° clockwise: 320 × 240
    // XPT2046 on the same bus (feature `touch-xpt2046`). GPIO8 is a strapping pin with a pull-up
    // (and drives the DevKitM-1's RGB LED); T_IRQ idles high, which keeps it compatible.
    #[cfg(feature = "touch-xpt2046")]
    let (touch_cs, touch_irq) = (p.GPIO0, p.GPIO8);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_MHZ: u32 = 2;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
    // FT6x36 capacitive touch on I2C0 (feature `touch-ft6x36`).
    #[cfg(feature = "touch-ft6x36")]
    let (touch_sda, touch_scl, touch_irq) = (p.GPIO0, p.GPIO2, p.GPIO8);
    // ============================================================================================

    let delay = &mut embassy_time::Delay;
    let _backlight = Output::new(lcd_backlight, Level::High, OutputConfig::default());
    let lcd_config = SpiConfig::default().with_frequency(Rate::from_mhz(LCD_MHZ));

    // The bus: SPI2 + DMA (the display writes its buffers in place, without a copy).
    static BUS: StaticCell<Bus> = StaticCell::new();
    let spi = Spi::new(p.SPI2, lcd_config)
        .unwrap()
        .with_sck(spi_sck)
        .with_mosi(spi_mosi)
        .with_miso(spi_miso)
        .with_dma(p.DMA_CH0)
        .into_async();
    let bus: &'static Bus = BUS.init(Mutex::new(spi));

    // Display: async `SpiDevice` with its own clock (re-applied per transaction).
    let display = {
        use embassy_embedded_hal::shared_bus::asynch::spi::SpiDeviceWithConfig;
        let spi = SpiDeviceWithConfig::new(bus, Output::new(lcd_cs, Level::High, OutputConfig::default()), lcd_config);
        let dc = Output::new(lcd_dc, Level::Low, OutputConfig::default());
        let rst = Output::new(lcd_rst, Level::High, OutputConfig::default());
        #[cfg(feature = "panel-ili9341")]
        let d = twine_drivers::ili9341::new_async(spi, dc, Some(rst), ROTATION, delay).await;
        #[cfg(feature = "panel-st7789")]
        let d = twine_drivers::st7789::new_async(spi, dc, Some(rst), &twine_drivers::st7789::ST7789, ROTATION, delay).await;
        d.unwrap_or_else(|e| panic!("display init failed: {e:?}"))
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
        let irq = Input::new(touch_irq, InputConfig::default().with_pull(Pull::Up));
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
    // SAFETY: the UI and every reactive handle live in this task on the executor and are never
    // touched from an interrupt handler or another executor; other contexts only use channels
    // and the UI waker.
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

/// Bytes of one partial draw buffer: 40 rows of 320 px (RGB565).
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
