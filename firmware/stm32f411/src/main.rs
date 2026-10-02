//! Example firmware: Twine on an STM32F411 (e.g. WeAct BlackPill, 25 MHz HSE) with an SPI
//! display and a touch controller, or a mono I2C OLED. A template to copy, not board support:
//! every pin, the clocks, the rotation and the bus clocks are in the wiring block at the top of
//! `main`; cargo features pick the display (`panel-ili9341`, `panel-ili9342`, `panel-st7789`,
//! `panel-st7796`, `oled-ssd1306`), the touch driver (`touch-xpt2046`, `touch-ft6x36`,
//! `touch-gt911`, `touch-cst816s`, or none) and the demo (`demo-counter`, `demo-controls`,
//! `demo-selection`, `demo-calibrate`).
//!
//! 128 KiB of SRAM: a 48 KiB heap and 2 × 20-row draw buffers (25 KiB). SPI panels are flushed
//! with DMA while the next chunk renders (`twine_embassy::run`); the UI sleeps until the touch
//! interrupt, a channel message or its next deadline. Every 5 s the RTT log shows a
//! `twine::perf` line (fps, CPU, render and flush time, heap) and the free stack.
#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_executor::Spawner;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals};
use static_cell::ConstStaticCell;
use twine::core::Rotation;
use twine::engine::{EngineConfig, MemInfo};
use twine::hal::AsyncDisplayDriver;
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
    all(feature = "demo-counter", any(feature = "demo-controls", feature = "demo-selection", feature = "demo-calibrate")),
    all(feature = "demo-controls", any(feature = "demo-selection", feature = "demo-calibrate")),
    all(feature = "demo-selection", feature = "demo-calibrate"),
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

// SPI panel: display TX DMA on DMA2 stream 3; touch interrupt on EXTI 8.
#[cfg(all(feature = "spi-panel", feature = "touch"))]
bind_interrupts!(struct Irqs {
    DMA2_STREAM3 => embassy_stm32::dma::InterruptHandler<peripherals::DMA2_CH3>;
    EXTI9_5 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI9_5>;
});
#[cfg(all(feature = "spi-panel", not(feature = "touch")))]
bind_interrupts!(struct Irqs {
    DMA2_STREAM3 => embassy_stm32::dma::InterruptHandler<peripherals::DMA2_CH3>;
});
// OLED: I2C1 with TX on DMA1 stream 6, RX on DMA1 stream 0.
#[cfg(feature = "oled-ssd1306")]
bind_interrupts!(struct Irqs {
    I2C1_EV => embassy_stm32::i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => embassy_stm32::i2c::ErrorInterruptHandler<peripherals::I2C1>;
    DMA1_STREAM6 => embassy_stm32::dma::InterruptHandler<peripherals::DMA1_CH6>;
    DMA1_STREAM0 => embassy_stm32::dma::InterruptHandler<peripherals::DMA1_CH0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    paint_stack();
    init_heap();
    let p = embassy_stm32::init(clocks());

    // ================================ Wiring: edit for your board ================================
    // SPI panel on SPI1 (APB2 100 MHz / 2 = 50 MHz), TX on DMA2 stream 3 (see `Irqs` above);
    // every `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (lcd_spi, lcd_dma, lcd_sck, lcd_mosi) = (p.SPI1, p.DMA2_CH3, p.PA5, p.PA7);
    #[cfg(feature = "spi-panel")]
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.PA4, p.PB0, p.PB1, p.PB10);
    #[cfg(feature = "spi-panel")]
    const LCD_HZ: u32 = 50_000_000;
    // XPT2046 on SPI2 (APB1 50 MHz / 32), T_IRQ on EXTI 8 (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_spi, touch_sck, touch_mosi, touch_miso, touch_cs, touch_irq, touch_exti) =
        (p.SPI2, p.PB13, p.PB15, p.PB14, p.PB12, p.PA8, p.EXTI8);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_HZ: u32 = 1_500_000;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_320X240_ROT90;
    // Capacitive touch (FT6x36, GT911, CST816S) on I2C1: SCL PB6, SDA PB7, INT PA8 (EXTI 8).
    #[cfg(feature = "i2c-touch")]
    let (touch_i2c, touch_scl, touch_sda, touch_irq, touch_exti) = (p.I2C1, p.PB6, p.PB7, p.PA8, p.EXTI8);
    // SSD1306 OLED on the same I2C1 pins (feature `oled-ssd1306`, address 0x3C, 400 kHz).
    #[cfg(feature = "oled-ssd1306")]
    let (oled_i2c, oled_scl, oled_sda, oled_tx_dma, oled_rx_dma) = (p.I2C1, p.PB6, p.PB7, p.DMA1_CH6, p.DMA1_CH0);
    // ============================================================================================

    // Display.
    #[cfg(feature = "spi-panel")]
    let _backlight = embassy_stm32::gpio::Output::new(lcd_backlight, embassy_stm32::gpio::Level::High, embassy_stm32::gpio::Speed::Low);
    #[cfg(feature = "spi-panel")]
    let display = {
        use embassy_stm32::gpio::{Level, Output, Speed};
        use embassy_stm32::mode::Async;
        use embassy_stm32::spi::{self, Spi};
        use embassy_sync::blocking_mutex::raw::NoopRawMutex;
        use static_cell::StaticCell;
        // Async SPI with DMA, shared-bus `SpiDevice` (owns CS).
        type LcdBus = embassy_sync::mutex::Mutex<NoopRawMutex, Spi<'static, Async, spi::mode::Master>>;
        static LCD_BUS: StaticCell<LcdBus> = StaticCell::new();
        let mut cfg = spi::Config::default();
        cfg.frequency = Hertz(LCD_HZ);
        let bus = LCD_BUS.init(embassy_sync::mutex::Mutex::new(Spi::new_txonly(lcd_spi, lcd_sck, lcd_mosi, lcd_dma, Irqs, cfg)));
        let lcd = embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice::new(bus, Output::new(lcd_cs, Level::High, Speed::VeryHigh));
        let dc = Output::new(lcd_dc, Level::Low, Speed::VeryHigh);
        let rst = Some(Output::new(lcd_rst, Level::High, Speed::Low));
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
        let mut cfg = embassy_stm32::i2c::Config::default();
        cfg.frequency = Hertz(400_000);
        let i2c = embassy_stm32::i2c::I2c::new(oled_i2c, oled_scl, oled_sda, oled_tx_dma, oled_rx_dma, Irqs, cfg);
        let iface = twine_drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine_drivers::ssd1306::AsyncSsd1306::new(iface, twine_drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .await
            .unwrap_or_else(|e| defmt::panic!("display init failed: {}", defmt::Debug2Format(&e)))
    };
    let info = display.info();

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        use core::cell::RefCell;
        use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
        use embassy_stm32::exti::ExtiInput;
        use embassy_stm32::gpio::{Level, Output, Pull, Speed};
        use embassy_stm32::mode::Blocking;
        use embassy_stm32::spi::{self, Spi};
        use embassy_sync::blocking_mutex::NoopMutex;
        use static_cell::StaticCell;
        type TouchBus = NoopMutex<RefCell<Spi<'static, Blocking, spi::mode::Master>>>;
        static TOUCH_BUS: StaticCell<TouchBus> = StaticCell::new();
        let mut cfg = spi::Config::default();
        cfg.frequency = Hertz(TOUCH_HZ);
        let bus = TOUCH_BUS.init(NoopMutex::new(RefCell::new(Spi::new_blocking(touch_spi, touch_sck, touch_mosi, touch_miso, cfg))));
        let dev = SpiDevice::new(bus, Output::new(touch_cs, Level::High, Speed::Low));
        let irq = ExtiInput::new(touch_irq, touch_exti, Pull::Up, Irqs);
        let cal = if CALIBRATE { Calibration::IDENTITY } else { TOUCH_CAL };
        twine_drivers::touch::Xpt2046::new(dev, Some(irq))
            .with_calibration(cal)
            .with_screen_size(if CALIBRATE { 4096 } else { info.width }, if CALIBRATE { 4096 } else { info.height })
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        use embassy_stm32::exti::ExtiInput;
        use embassy_stm32::gpio::Pull;
        let i2c = embassy_stm32::i2c::I2c::new_blocking(touch_i2c, touch_scl, touch_sda, embassy_stm32::i2c::Config::default());
        let irq = Some(ExtiInput::new(touch_irq, touch_exti, Pull::Up, Irqs));
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

    // The UI: two DMA-pipelined partial buffers (SPI panels: 20 rows of 320 px; OLED: the
    // whole 128 × 64 × 1 bpp frame).
    static BUF_A: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    static BUF_B: ConstStaticCell<DrawBuffer> = ConstStaticCell::new(DrawBuffer([0; BUFFER_BYTES]));
    let mut config = EngineConfig::default();
    config.mem_info = Some(mem_info);
    config.hires_timer = Some(twine_embassy::hires_now);
    // SAFETY: the UI and every reactive handle live in this task on the thread-mode executor and
    // are never touched from an interrupt handler or another executor; other contexts only use
    // channels and the UI waker.
    let builder = unsafe { Ui::builder_async(display).bind_to_current_context() };
    let builder = builder.buffers(BufferMode::partial_double(&mut BUF_A.take().0, &mut BUF_B.take().0));
    #[cfg(feature = "touch")]
    let builder = builder.input_wait(touch);
    #[cfg(feature = "spi-panel")]
    let theme = DefaultTheme::light();
    // White on black: lit OLED pixels are the foreground.
    #[cfg(feature = "oled-ssd1306")]
    let theme = MonoTheme::builder().mode(ThemeMode::Dark).font(&twine::assets::fonts::MONTSERRAT_14).build();
    let ui = builder.config(config).theme(theme).with_embassy_clock().build(demo::app);
    defmt::info!("twine: {} demo on {}x{}, stack free {} B", demo::NAME, info.width, info.height, stack_free());
    twine_embassy::run(ui).await
}

/// The clock tree: HSE 25 MHz → PLL → 100 MHz SYSCLK, APB1 50 MHz, APB2 100 MHz.
fn clocks() -> embassy_stm32::Config {
    use embassy_stm32::rcc::{AHBPrescaler, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPDiv, PllPreDiv, PllQDiv, PllSource, Sysclk};
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(25_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll_src = PllSource::HSE;
    config.rcc.pll = Some(Pll {
        prediv: PllPreDiv::DIV25,
        mul: PllMul::MUL200,
        divp: Some(PllPDiv::DIV2), // 25 MHz / 25 × 200 / 2 = 100 MHz
        divq: Some(PllQDiv::DIV4),
        divr: None,
    });
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV2;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    config.rcc.sys = Sysclk::PLL1_P;
    config
}

/// Bytes of one partial draw buffer: 20 rows of 320 px (RGB565).
#[cfg(feature = "spi-panel")]
const BUFFER_BYTES: usize = 320 * 20 * 2;
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

/// Heap size.
const HEAP_BYTES: usize = 48 * 1024;

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
    if stack_free() < 1024 {
        defmt::warn!("stack: less than 1 KiB left unused");
    }
    MemInfo { used, peak, free }
}

/// The pattern painted over the unused stack at boot.
const STACK_PAINT: u32 = 0xDEAD_BEEF;
/// Words painted above the stack's lowest address (16 KiB: more than the UI task needs).
const STACK_PAINT_WORDS: usize = 4096;

unsafe extern "C" {
    /// Lowest address of the stack region (cortex-m-rt).
    static mut _stack_end: u32;
}

/// Paints the lowest [`STACK_PAINT_WORDS`] words of the stack (first thing in `main`).
fn paint_stack() {
    let bottom = (&raw mut _stack_end).cast::<u32>();
    // SAFETY: at the start of `main` the stack pointer is near the top of RAM, far above the
    // lowest 16 KiB of the stack region, so no live data is overwritten.
    unsafe {
        for i in 0..STACK_PAINT_WORDS {
            core::ptr::write_volatile(bottom.add(i), STACK_PAINT);
        }
    }
}

/// Unused stack in bytes: how much of the painted region is still intact (the high-water mark
/// is the stack size minus this).
fn stack_free() -> usize {
    let bottom = (&raw const _stack_end).cast::<u32>();
    let mut n = 0;
    // SAFETY: the painted words are plain RAM inside the stack region; volatile reads.
    unsafe {
        while n < STACK_PAINT_WORDS && core::ptr::read_volatile(bottom.add(n)) == STACK_PAINT {
            n += 1;
        }
    }
    n * 4
}
