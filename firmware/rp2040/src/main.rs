//! Example firmware: Twine on an RP2040 (e.g. Raspberry Pi Pico) with an SPI display and a
//! touch controller, or a mono I2C OLED. A template to copy, not board support: every pin, the
//! panel model, its rotation and the bus clocks are in the wiring block at the top of `main`;
//! cargo features pick the display (`panel-ili9341`, `panel-ili9342`, `panel-st7789`,
//! `panel-st7796`, `oled-ssd1306`), the touch driver (`touch-xpt2046`, `touch-ft6x36`,
//! `touch-gt911`, `touch-cst816s`, or none) and the demo (`demo-counter`, `demo-controls`,
//! `demo-selection`, `demo-calibrate`, `demo-multilang`).
//!
//! **Bare metal, no executor:** the blocking `Ui` runs in `twine::run::blocking` on
//! [`PicoPlatform`], a [`Platform`] over SysTick: the core sleeps (`WFE`) until the UI's next
//! deadline (a one-shot SysTick alarm, so an idle UI is not woken by a periodic tick) or a
//! wake-up (`SEV`, installed on the UI's waker by the run loop). embassy-rp is used as a HAL
//! only (pin/SPI/I2C setup, its PAC, `embassy-time` for the clock and init delays).
//!
//! SPI panels are flushed by DMA while the next chunk renders: [`PicoDmaSpi`] drives SPI0 and
//! DMA channel 0 through the registers and is the chip-specific half of `twine-drivers`'
//! `DmaSpiInterface`, under the generic `MipiDcs` panel driver. `begin_flush` only starts the
//! transfer, the engine renders the next chunk into the second of two static draw buffers, and
//! `poll_flush` hands a buffer back once the channel and the SPI are idle. The DMA-complete
//! interrupt only notifies the run loop (`SEV`), so a frame's last transfer is reclaimed
//! without waiting for the next deadline. The touch controller is polled every
//! `EngineConfig::read_period` (embassy-rp owns the GPIO interrupt; wire your own pin
//! interrupt to `ui.waker().wake()` to sleep until a touch instead). Every 5 s the RTT log
//! shows a `twine::perf` line (fps, CPU, render and flush time, heap).
#![no_std]
#![no_main]

use core::mem::MaybeUninit;
#[cfg(feature = "spi-panel")]
use core::sync::atomic::compiler_fence;

#[cfg(feature = "spi-panel")]
use embassy_rp::pac;

use cortex_m::peripheral::scb::VectActive;
use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::{SCB, SYST};
use cortex_m_rt::{entry, exception};
use portable_atomic::{AtomicBool, Ordering};
use twine::core::Rotation;
use twine::engine::{HeapPeak, MemInfo};
use twine::hal::{Clock, Platform};
use twine::prelude::*;
#[cfg(feature = "touch-xpt2046")]
use twine::drivers::Calibration;
use {defmt_rtt as _, panic_probe as _};

// Feature groups (use `--no-default-features` to switch, or `cargo xtask firmware <board>
// --features …`, which replaces the default of the same group): exactly one display, at most one
// touch controller, at most one demo.
twine::feature_rules! {
    exactly_one "display": ["panel-ili9341", "panel-ili9342", "panel-st7789", "panel-st7796", "oled-ssd1306"];
    at_most_one "touch": ["touch-xpt2046", "touch-ft6x36", "touch-gt911", "touch-cst816s"];
    at_most_one "demo": ["demo-counter", "demo-controls", "demo-selection", "demo-calibrate", "demo-multilang"];
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

#[entry]
fn main() -> ! {
    init_heap();
    let p = embassy_rp::init(embassy_rp::config::Config::default());
    let core = cortex_m::Peripherals::take().expect("core peripherals taken once");

    // ================================ Wiring: edit for your board ================================
    // SPI panel on SPI0 (the defaults match a Pico with a 2.8" ILI9341 + XPT2046 module); every
    // `panel-*` feature uses these pins.
    #[cfg(feature = "spi-panel")]
    let (lcd_spi, lcd_dma, lcd_sck, lcd_mosi) = (p.SPI0, p.DMA_CH0, p.PIN_18, p.PIN_19);
    #[cfg(feature = "spi-panel")]
    let (lcd_cs, lcd_dc, lcd_rst, lcd_backlight) = (p.PIN_17, p.PIN_20, p.PIN_21, p.PIN_22);
    #[cfg(feature = "spi-panel")]
    const LCD_HZ: u32 = 62_500_000;
    // XPT2046 resistive touch on SPI1 (feature `touch-xpt2046`).
    #[cfg(feature = "touch-xpt2046")]
    let (touch_spi, touch_sck, touch_mosi, touch_miso, touch_cs) =
        (p.SPI1, p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13);
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_HZ: u32 = 2_000_000;
    /// Replace with the output of `--features demo-calibrate`.
    #[cfg(feature = "touch-xpt2046")]
    const TOUCH_CAL: Calibration = Calibration::DEFAULT_240X320;
    // Capacitive touch (FT6x36, GT911, CST816S) on I2C0: SDA GP4, SCL GP5 (INT unused: polled).
    #[cfg(feature = "i2c-touch")]
    let (touch_i2c, touch_sda, touch_scl) = (p.I2C0, p.PIN_4, p.PIN_5);
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
        use twine::drivers::interface::DmaSpiInterface;
        // embassy-rp configures SPI0 and its pins (blocking mode); `PicoDmaSpi` then drives the
        // data register and DMA channel 0 directly.
        let mut cfg = spi::Config::default();
        cfg.frequency = LCD_HZ;
        let bus = PicoDmaSpi::new(Spi::new_blocking_txonly(lcd_spi, lcd_sck, lcd_mosi, cfg), lcd_dma);
        // ~0.3 s of polls (a 40-row buffer takes ~3.3 ms at 62.5 MHz): a hung channel becomes
        // a flush error and the engine's `flush_timeout` fault instead of a freeze.
        let iface = DmaSpiInterface::new(
            bus,
            Output::new(lcd_cs, Level::High),
            Output::new(lcd_dc, Level::Low),
        )
        .with_max_polls(2_000_000);
        let rst = Some(Output::new(lcd_rst, Level::High));
        let delay = &mut embassy_time::Delay;
        #[cfg(feature = "panel-ili9341")]
        let spec = &twine::drivers::ili9341::ILI9341;
        #[cfg(feature = "panel-ili9342")]
        let spec = &twine::drivers::ili9342::ILI9342C;
        #[cfg(feature = "panel-st7789")]
        let spec = &twine::drivers::st7789::ST7789;
        #[cfg(feature = "panel-st7796")]
        let spec = &twine::drivers::st7796::ST7796;
        twine::drivers::mipi_dcs::MipiDcs::new(iface, rst, spec, ROTATION, delay)
            .unwrap_or_else(|e| defmt::panic!("display init failed: {}", e))
    };
    #[cfg(feature = "oled-ssd1306")]
    let display = {
        let mut cfg = embassy_rp::i2c::Config::default();
        cfg.frequency = 400_000;
        let i2c = embassy_rp::i2c::I2c::new_blocking(oled_i2c, oled_scl, oled_sda, cfg);
        let iface = twine::drivers::interface::I2cInterface::new(i2c, 0x3C);
        twine::drivers::ssd1306::Ssd1306::new(iface, twine::drivers::ssd1306::Ssd1306Size::Size128x64, ROTATION)
            .unwrap_or_else(|e| defmt::panic!("display init failed: {}", e))
    };
    let info = twine::hal::DisplayDriver::info(&display);

    // Touch.
    #[cfg(feature = "touch-xpt2046")]
    let touch = {
        use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
        use embassy_rp::gpio::{Input, Level, Output};
        use embassy_rp::spi::{self, Spi};
        use embassy_sync::blocking_mutex::NoopMutex;
        use static_cell::StaticCell;
        type TouchBus =
            NoopMutex<core::cell::RefCell<Spi<'static, embassy_rp::peripherals::SPI1, spi::Blocking>>>;
        static TOUCH_BUS: StaticCell<TouchBus> = StaticCell::new();
        let mut cfg = spi::Config::default();
        cfg.frequency = TOUCH_HZ;
        let bus = TOUCH_BUS.init(NoopMutex::new(core::cell::RefCell::new(Spi::new_blocking(
            touch_spi, touch_sck, touch_mosi, touch_miso, cfg,
        ))));
        let dev = SpiDevice::new(bus, Output::new(touch_cs, Level::High));
        let cal = if CALIBRATE {
            Calibration::IDENTITY
        } else {
            TOUCH_CAL
        };
        // No interrupt pin: the engine polls the controller every `read_period`.
        twine::drivers::touch::Xpt2046::new(dev, None::<Input<'static>>)
            .with_calibration(cal)
    };
    #[cfg(feature = "i2c-touch")]
    let touch = {
        use embassy_rp::gpio::Input;
        let i2c = embassy_rp::i2c::I2c::new_blocking(
            touch_i2c,
            touch_scl,
            touch_sda,
            embassy_rp::i2c::Config::default(),
        );
        // No interrupt pin: the engine polls the controller every `read_period`.
        let irq = None::<Input<'static>>;
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

    // The UI, drawing into `BUFS` (declared with `draw_buffers!` below).
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
    config.engine.hires_timer = Some(hires_now);
    // The runtime belongs to `main`: the `!Send` token keeps the UI and every reactive handle
    // here; interrupt handlers only use channels and the UI waker.
    let rt = Runtime::take().expect("runtime already taken");
    let mut platform = PicoPlatform::new(core.SYST);
    // The platform owns SysTick (not `Clone`), so the `Ui` gets its clock; `run::blocking`
    // installs the platform's `notify` on the UI's waker.
    let builder = Ui::builder(display).runtime(rt).clock(PicoClock);
    // Two buffers on the SPI panel: chunk k+1 renders while DMA sends chunk k.
    #[cfg(feature = "spi-panel")]
    let builder = builder.buffers(BufferMode::partial_double_from(
        BUFS.take().expect("draw buffers already taken"),
    ));
    // The OLED's I2C writes are blocking: a second buffer would not overlap anything.
    #[cfg(feature = "oled-ssd1306")]
    let builder = builder.buffers(BufferMode::partial_single_from(
        BUFS.take().expect("draw buffer already taken"),
    ));
    #[cfg(feature = "touch")]
    let builder = builder.input(touch);
    let ui = builder.app_config(config).build(demo::app);
    defmt::info!(
        "twine: {} demo on {}x{} (bare metal)",
        demo::NAME,
        info.width,
        info.height
    );
    twine::run::blocking(ui, &mut platform)
}

/// The clock: the RP2040's 1 MHz `TIMER` through `embassy-time` (its time driver only reads
/// the counter here; nothing schedules alarms).
#[derive(Clone, Copy)]
struct PicoClock;

impl Clock for PicoClock {
    fn now(&self) -> Instant {
        Instant::from_micros(embassy_time::Instant::now().as_micros())
    }
}

/// `EngineConfig::hires_timer` (render and flush statistics).
fn hires_now() -> Instant {
    PicoClock.now()
}

/// The bare-metal [`Platform`]: sleeps with `WFE` until a one-shot SysTick alarm at the
/// deadline (at most 2²⁴ core cycles ahead, ~134 ms at 125 MHz: a later deadline just wakes
/// the loop once more, which the `Platform` contract allows) or a `SEV`.
struct PicoPlatform {
    syst: SYST,
    cycles_per_us: u32,
}

impl PicoPlatform {
    fn new(mut syst: SYST) -> Self {
        syst.disable_counter();
        syst.set_clock_source(SystClkSource::Core);
        syst.enable_interrupt();
        Self {
            syst,
            cycles_per_us: embassy_rp::clocks::clk_sys_freq() / 1_000_000,
        }
    }
}

impl Clock for PicoPlatform {
    fn now(&self) -> Instant {
        PicoClock.now()
    }
}

impl Platform for PicoPlatform {
    fn wait(&mut self, deadline: Option<Instant>) {
        if let Some(t) = deadline {
            let us = t.saturating_duration_since(self.now()).as_micros();
            if us == 0 {
                return;
            }
            let cycles = us
                .saturating_mul(u64::from(self.cycles_per_us))
                .clamp(1, 0x00FF_FFFF);
            self.syst.set_reload(u32::try_from(cycles).unwrap_or(0x00FF_FFFF));
            self.syst.clear_current();
            self.syst.enable_counter();
        }
        // The event register latches a `SEV` made since the last `WFE` (a wake-up, or the
        // alarm below firing before we get here): no wake-up is lost.
        cortex_m::asm::wfe();
        self.syst.disable_counter();
    }

    fn notify() {
        cortex_m::asm::sev();
    }

    fn in_interrupt() -> bool {
        SCB::vect_active() != VectActive::ThreadMode
    }
}

/// The deadline alarm: wakes the `WFE` of [`PicoPlatform::wait`].
#[exception]
fn SysTick() {
    cortex_m::asm::sev();
}

/// The DMA channel of the display's SPI.
#[cfg(feature = "spi-panel")]
const LCD_DMA_CH: usize = 0;

/// SPI0 with DMA channel 0, through the registers: the RP2040 half of `twine-drivers`'
/// `DmaSpiInterface` (`DmaSpiBus`). Commands are written to the data register; a draw buffer
/// is sent by the channel, paced by SPI0's TX request, and handed back once the channel and
/// the SPI shifter are idle.
#[cfg(feature = "spi-panel")]
struct PicoDmaSpi {
    /// Owns SPI0 and its pins (configured by embassy-rp: clock, format, TX DMA request on).
    _spi: embassy_rp::spi::Spi<'static, embassy_rp::peripherals::SPI0, embassy_rp::spi::Blocking>,
    /// Owns the DMA channel, so nothing else uses it.
    _ch: embassy_rp::Peri<'static, embassy_rp::peripherals::DMA_CH0>,
    /// The buffer of the running (or finished, not yet taken) transfer.
    buf: Option<twine::hal::DrawBufferMem>,
}

#[cfg(feature = "spi-panel")]
impl PicoDmaSpi {
    fn new(
        spi: embassy_rp::spi::Spi<'static, embassy_rp::peripherals::SPI0, embassy_rp::spi::Blocking>,
        ch: embassy_rp::Peri<'static, embassy_rp::peripherals::DMA_CH0>,
    ) -> Self {
        use embassy_rp::interrupt::InterruptExt;
        // The channel's completion raises DMA_IRQ_0 (handled by `DmaDone`).
        let inte = pac::DMA.inte(0);
        inte.write_value(inte.read() | 1 << LCD_DMA_CH);
        embassy_rp::interrupt::DMA_IRQ_0.unpend();
        // SAFETY: `DmaDone` (bound below) only acknowledges the channel and executes `SEV`; it
        // shares no data with the code it interrupts.
        unsafe { embassy_rp::interrupt::DMA_IRQ_0.enable() };
        Self {
            _spi: spi,
            _ch: ch,
            buf: None,
        }
    }

    /// Discards what the receiver collected while sending (TX-only use) and its overrun flag.
    fn drain_rx() {
        let spi = pac::SPI0;
        while spi.sr().read().rne() {
            let _ = spi.dr().read();
        }
        spi.icr().write(|w| w.set_roric(true));
    }
}

#[cfg(feature = "spi-panel")]
impl twine::drivers::interface::DmaSpiBus for PicoDmaSpi {
    type Error = core::convert::Infallible;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let spi = pac::SPI0;
        for &b in bytes {
            while !spi.sr().read().tnf() {}
            spi.dr().write(|w| w.set_data(u16::from(b)));
        }
        while spi.sr().read().bsy() {}
        Self::drain_rx();
        Ok(())
    }

    fn start_dma(
        &mut self,
        buf: twine::hal::DrawBufferMem,
        len: usize,
    ) -> Result<(), (Self::Error, twine::hal::DrawBufferMem)> {
        use pac::dma::vals::{DataSize, TreqSel};
        let ch = pac::DMA.ch(LCD_DMA_CH);
        ch.read_addr().write_value(buf.addr() as u32);
        ch.write_addr().write_value(pac::SPI0.dr().as_ptr() as u32);
        ch.trans_count().write_value(len as u32);
        compiler_fence(Ordering::SeqCst);
        ch.ctrl_trig().write(|w| {
            w.set_treq_sel(TreqSel::SPI0_TX);
            w.set_data_size(DataSize::SIZE_BYTE);
            w.set_incr_read(true);
            w.set_incr_write(false);
            w.set_chain_to(LCD_DMA_CH as u8);
            w.set_en(true);
        });
        self.buf = Some(buf);
        Ok(())
    }

    fn finish_dma(&mut self) -> Option<twine::hal::DrawBufferMem> {
        // `BSY`: the TX FIFO is not empty or a frame is being shifted out.
        if self.buf.is_none()
            || pac::DMA.ch(LCD_DMA_CH).ctrl_trig().read().busy()
            || pac::SPI0.sr().read().bsy()
        {
            return None;
        }
        compiler_fence(Ordering::SeqCst);
        Self::drain_rx();
        self.buf.take()
    }
}

/// DMA_IRQ_0: acknowledges the display channel and wakes the run loop (`SEV`, the platform's
/// `notify`), which reclaims the buffer in its next update. Nothing else: interrupt-safe.
#[cfg(feature = "spi-panel")]
struct DmaDone;

#[cfg(feature = "spi-panel")]
impl embassy_rp::interrupt::typelevel::Handler<embassy_rp::interrupt::typelevel::DMA_IRQ_0> for DmaDone {
    unsafe fn on_interrupt() {
        pac::DMA.ints(0).write_value(1 << LCD_DMA_CH);
        PicoPlatform::notify();
    }
}

#[cfg(feature = "spi-panel")]
embassy_rp::bind_interrupts!(
    /// Binds [`DmaDone`] to DMA_IRQ_0.
    #[allow(dead_code)]
    struct Irqs {
        DMA_IRQ_0 => DmaDone;
    }
);

// The draw buffers: sized at compile time, 4-byte aligned by their type, zeroed `.bss`, taken
// once in `main`.
twine::draw_buffers! {
    /// Two partial draw buffers for the DMA flushes: 40 rows of the 320 px panel (RGB565).
    #[cfg(feature = "spi-panel")]
    static BUFS: 2 x 40 rows x 320 px @ Rgb565Swapped;
    /// A draw buffer of the whole 128 × 64 OLED at 1 bpp.
    #[cfg(feature = "oled-ssd1306")]
    static BUFS: 1 x 64 rows x 128 px @ I1;
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
    #[cfg(not(any(
        feature = "demo-controls",
        feature = "demo-calibrate",
        feature = "demo-selection",
        feature = "demo-multilang"
    )))]
    pub use twine_demos::counter::app;
    #[cfg(feature = "demo-multilang")]
    pub use twine_demos::multilang::app;
    #[cfg(feature = "demo-selection")]
    pub use twine_demos::selection::app;

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
    static PEAK: HeapPeak = HeapPeak::new();
    let m = PEAK.sample(HEAP.used(), HEAP.free());
    if m.used_percent() > 90 {
        defmt::warn!("heap above 90%: {} of {} bytes", m.used, m.used + m.free);
    }
    m
}
