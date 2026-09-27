//! Board support for the STM32F429I-DISC1, shared by the demo firmware (`main.rs`) and the
//! render benchmark (`bin/bench.rs`): clocks, the 8 MiB SDRAM, the on-board ILI9341 in RGB
//! interface mode scanned out by the LTDC, the [`LtdcDisplay`] framebuffer display, the DMA2D
//! clock and the heap.
//!
//! Sequences and timings follow ST's STM32F429I-Discovery BSP (`stm32f429i_discovery.c`,
//! `stm32f429i_discovery_sdram.c`, `stm32f429i_discovery_lcd.c`, `ili9341.c`).
#![no_std]

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::ltdc::{
    Ltdc, LtdcConfiguration, LtdcLayer, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::spi::{self, Spi};
use embassy_stm32::time::Hertz;
use embassy_stm32::{Peri, Peripherals, pac, peripherals};
use embedded_hal::delay::DelayNs;
use twine::core::ColorFormat;
use twine::engine::MemInfo;
use twine::hal::{DisplayInfo, DrawBufferMem, FramebufferDisplay};

/// Panel width (portrait).
pub const WIDTH: u16 = 240;
/// Panel height (portrait).
pub const HEIGHT: u16 = 320;
/// Bytes of one RGB565 framebuffer.
pub const FB_BYTES: usize = WIDTH as usize * HEIGHT as usize * 2;
/// Size of the SDRAM (FMC bank 2, mapped at `0xD000_0000`).
pub const SDRAM_BYTES: usize = 8 * 1024 * 1024;

/// The clock tree: HSE 8 MHz (ST-LINK MCO) → PLL → 168 MHz SYSCLK, APB1 42 MHz, APB2 84 MHz,
/// 48 MHz on Q; PLLSAI 192 MHz / R 4 = 48 MHz, divided by 8 (`DCKCFGR.PLLSAIDIVR`, set in
/// [`Board::init`]) = 6 MHz LCD pixel clock, as in the ST BSP.
pub fn clocks() -> embassy_stm32::Config {
    use embassy_stm32::rcc::{
        AHBPrescaler, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPDiv, PllPreDiv, PllQDiv, PllRDiv,
        PllSource, Sysclk,
    };
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(8_000_000),
        mode: HseMode::Bypass,
    });
    config.rcc.pll_src = PllSource::HSE;
    config.rcc.pll = Some(Pll {
        prediv: PllPreDiv::DIV8, // PLLM is shared with PLLSAI on the F429: 1 MHz VCO input
        mul: PllMul::MUL336,
        divp: Some(PllPDiv::DIV2), // 168 MHz
        divq: Some(PllQDiv::DIV7), // 48 MHz
        divr: None,
    });
    config.rcc.pllsai = Some(Pll {
        prediv: PllPreDiv::DIV8,
        mul: PllMul::MUL192,
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV4),
    });
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV4;
    config.rcc.apb2_pre = APBPrescaler::DIV2;
    config.rcc.sys = Sysclk::PLL1_P;
    config
}

/// The peripherals left for the application after [`Board::init`].
#[allow(missing_docs)]
pub struct Board {
    /// The LTDC framebuffer display (two framebuffers at the start of the SDRAM).
    pub display: LtdcDisplay,
    /// SDRAM after the two framebuffers (≈ 7.7 MiB), e.g. for images or benchmark buffers.
    pub sdram_rest: &'static mut [u8],
    /// STMPE811 touch controller: I2C3 (SCL PA8, SDA PC9), `INT` on PA15 (EXTI 15).
    pub touch_i2c: Peri<'static, peripherals::I2C3>,
    pub touch_scl: Peri<'static, peripherals::PA8>,
    pub touch_sda: Peri<'static, peripherals::PC9>,
    pub touch_int: Peri<'static, peripherals::PA15>,
    pub touch_exti: Peri<'static, peripherals::EXTI15>,
    /// User LEDs LD3 (green, PG13) and LD4 (red, PG14).
    pub led_green: Peri<'static, peripherals::PG13>,
    pub led_red: Peri<'static, peripherals::PG14>,
}

impl Board {
    /// Brings up the SDRAM, the panel and the LTDC, and enables the DMA2D clock. Call once,
    /// right after `embassy_stm32::init(clocks())`.
    pub fn init(p: Peripherals, delay: &mut impl DelayNs) -> Board {
        // ---------------------------------------------------------------- SDRAM (FMC bank 2)
        let mut sdram = embassy_stm32::fmc::Fmc::sdram_a12bits_d16bits_4banks_bank2(
            p.FMC,
            // A0..A11
            p.PF0,
            p.PF1,
            p.PF2,
            p.PF3,
            p.PF4,
            p.PF5,
            p.PF12,
            p.PF13,
            p.PF14,
            p.PF15,
            p.PG0,
            p.PG1,
            // BA0, BA1
            p.PG4,
            p.PG5,
            // D0..D15
            p.PD14,
            p.PD15,
            p.PD0,
            p.PD1,
            p.PE7,
            p.PE8,
            p.PE9,
            p.PE10,
            p.PE11,
            p.PE12,
            p.PE13,
            p.PE14,
            p.PE15,
            p.PD8,
            p.PD9,
            p.PD10,
            // NBL0, NBL1
            p.PE0,
            p.PE1,
            // SDCKE1, SDCLK, SDNCAS, SDNE1, SDNRAS, SDNWE
            p.PB5,
            p.PG8,
            p.PG15,
            p.PB6,
            p.PF11,
            p.PC0,
            stm32_fmc::devices::is42s16400j_7::Is42s16400j {},
        );
        // Mode register, refresh (15.625 µs × 84 MHz) and SDCR/SDTR from the chip definition;
        // SDCLK = HCLK / 2 = 84 MHz.
        let base = sdram.init(delay).cast::<u8>();
        defmt::info!("sdram: {} KiB at {=usize:#x}", SDRAM_BYTES / 1024, base as usize);
        // SAFETY: the FMC maps the initialised SDRAM at `base` for `SDRAM_BYTES`. `Board::init`
        // consumes the `Peripherals` singleton, so this runs once and the slice is the only
        // reference to that memory.
        let sdram: &'static mut [u8] = unsafe { core::slice::from_raw_parts_mut(base, SDRAM_BYTES) };
        sdram.fill(0);
        let (fb0, rest) = sdram.split_at_mut(FB_BYTES);
        let (fb1, sdram_rest) = rest.split_at_mut(FB_BYTES);

        // ------------------------------------------------ ILI9341 over SPI5 → RGB interface
        let mut cfg = spi::Config::default();
        cfg.frequency = Hertz(5_000_000);
        let mut spi = Spi::new_blocking_txonly(p.SPI5, p.PF7, p.PF9, cfg);
        let mut panel = PanelSpi {
            spi: &mut spi,
            cs: Output::new(p.PC2, Level::High, Speed::Medium),
            dcx: Output::new(p.PD13, Level::Low, Speed::Medium),
        };
        panel.init_rgb_mode(delay);

        // ------------------------------------------------------------------ LTDC, layer 1
        // 18-bit RGB (R2..R7, G2..G7, B2..B7) + HSYNC, VSYNC, DE, CLK; embassy's pin
        // constructor needs all 24 data lines, so the pins are set up here.
        let _ltdc_pins = (
            p.PA3, p.PA4, p.PA6, p.PA11, p.PA12, p.PB0, p.PB1, p.PB8, p.PB9, p.PB10, p.PB11, p.PC6, p.PC7,
            p.PC10, p.PD3, p.PD6, p.PF10, p.PG6, p.PG7, p.PG10, p.PG11, p.PG12,
        );
        alternate(pac::GPIOA, &[3, 4, 6, 11, 12], 14);
        alternate(pac::GPIOB, &[8, 9, 10, 11], 14);
        alternate(pac::GPIOB, &[0, 1], 9);
        alternate(pac::GPIOC, &[6, 7, 10], 14);
        alternate(pac::GPIOD, &[3, 6], 14);
        alternate(pac::GPIOF, &[10], 14);
        alternate(pac::GPIOG, &[6, 7, 11], 14);
        alternate(pac::GPIOG, &[10, 12], 9);
        let mut ltdc = Ltdc::new(p.LTDC);
        // `Ltdc::new` selects PLLSAIDIVR /2; the panel wants 48 MHz / 8 = 6 MHz.
        pac::RCC
            .dckcfgr()
            .modify(|w| w.set_pllsaidivr(pac::rcc::vals::Pllsaidivr::DIV8));
        ltdc.init(&LtdcConfiguration {
            active_width: WIDTH,
            active_height: HEIGHT,
            h_back_porch: 20,
            h_front_porch: 10,
            v_back_porch: 2,
            v_front_porch: 4,
            h_sync: 10,
            v_sync: 2,
            h_sync_polarity: PolarityActive::ActiveLow,
            v_sync_polarity: PolarityActive::ActiveLow,
            data_enable_polarity: PolarityActive::ActiveLow,
            pixel_clock_polarity: PolarityEdge::RisingEdge,
        });
        ltdc.init_layer(
            &LtdcLayerConfig {
                layer: LtdcLayer::Layer1,
                pixel_format: PixelFormat::RGB565,
                window_x0: 0,
                window_x1: WIDTH,
                window_y0: 0,
                window_y1: HEIGHT,
            },
            None,
        );
        let addrs = [fb0.as_ptr() as u32, fb1.as_ptr() as u32];
        let regs = pac::LTDC;
        regs.layer(0).cfbar().write(|w| w.set_cfbadd(addrs[0]));
        regs.srcr().write(|w| w.set_imr(pac::ltdc::vals::Imr::RELOAD));
        while regs.srcr().read().imr() == pac::ltdc::vals::Imr::RELOAD {}
        defmt::info!(
            "ltdc: {}x{} RGB565, framebuffers at {=u32:#x} and {=u32:#x}",
            WIDTH,
            HEIGHT,
            addrs[0],
            addrs[1]
        );

        // DMA2D clock (the accelerator itself is `twine_accel_stm32::Dma2d<PacRegs>`).
        pac::RCC.ahb1enr().modify(|w| w.set_dma2den(true));

        Board {
            display: LtdcDisplay {
                _ltdc: ltdc,
                fbs: Some((DrawBufferMem::new(fb0), DrawBufferMem::new(fb1))),
                addrs,
                presented: 0,
            },
            sdram_rest,
            touch_i2c: p.I2C3,
            touch_scl: p.PA8,
            touch_sda: p.PC9,
            touch_int: p.PA15,
            touch_exti: p.EXTI15,
            led_green: p.PG13,
            led_red: p.PG14,
        }
    }
}

/// Configures GPIO `pins` of `port` as push-pull alternate function `af`, very high speed, no
/// pull.
fn alternate(port: pac::gpio::Gpio, pins: &[usize], af: u8) {
    use pac::gpio::vals::{Moder, Ospeedr, Ot, Pupdr};
    for &n in pins {
        port.afr(n / 8).modify(|w| w.set_afr(n % 8, af));
        port.otyper().modify(|w| w.set_ot(n, Ot::PUSH_PULL));
        port.ospeedr()
            .modify(|w| w.set_ospeedr(n, Ospeedr::VERY_HIGH_SPEED));
        port.pupdr().modify(|w| w.set_pupdr(n, Pupdr::FLOATING));
        port.moder().modify(|w| w.set_moder(n, Moder::ALTERNATE));
    }
}

/// The ILI9341's serial command interface (SPI5, `CSX` PC2, `WRX`/`D/CX` PD13).
struct PanelSpi<'a, 'd> {
    spi: &'a mut Spi<'d, embassy_stm32::mode::Blocking, spi::mode::Master>,
    cs: Output<'d>,
    dcx: Output<'d>,
}

impl PanelSpi<'_, '_> {
    /// Sends command `cmd` followed by its parameter bytes.
    fn cmd(&mut self, cmd: u8, params: &[u8]) {
        self.cs.set_low();
        self.dcx.set_low();
        let _ = self.spi.blocking_write(&[cmd]);
        if !params.is_empty() {
            self.dcx.set_high();
            let _ = self.spi.blocking_write(params);
        }
        self.cs.set_high();
    }

    /// ST BSP `ili9341_Init`: power and gamma settings, RGB interface (`0xB0`, `0xF6`), 16-bit
    /// pixels from the LTDC, sleep out, display on.
    fn init_rgb_mode(&mut self, delay: &mut impl DelayNs) {
        self.cmd(0xCA, &[0xC3, 0x08, 0x50]);
        self.cmd(0xCF, &[0x00, 0xC1, 0x30]); // power control B
        self.cmd(0xED, &[0x64, 0x03, 0x12, 0x81]); // power on sequence
        self.cmd(0xE8, &[0x85, 0x00, 0x78]); // driver timing control A
        self.cmd(0xCB, &[0x39, 0x2C, 0x00, 0x34, 0x02]); // power control A
        self.cmd(0xF7, &[0x20]); // pump ratio
        self.cmd(0xEA, &[0x00, 0x00]); // driver timing control B
        self.cmd(0xB1, &[0x00, 0x1B]); // frame rate
        self.cmd(0xB6, &[0x0A, 0xA2]); // display function control
        self.cmd(0xC0, &[0x10]); // power control 1
        self.cmd(0xC1, &[0x10]); // power control 2
        self.cmd(0xC5, &[0x45, 0x15]); // VCOM 1
        self.cmd(0xC7, &[0x90]); // VCOM 2
        self.cmd(0x36, &[0xC8]); // memory access control
        self.cmd(0xF2, &[0x00]); // 3-gamma off
        self.cmd(0xB0, &[0xC2]); // RGB interface signal control
        self.cmd(0xB6, &[0x0A, 0xA7, 0x27, 0x04]); // display function control
        self.cmd(0x2A, &[0x00, 0x00, 0x00, 0xEF]); // columns 0..239
        self.cmd(0x2B, &[0x00, 0x00, 0x01, 0x3F]); // pages 0..319
        self.cmd(0xF6, &[0x01, 0x00, 0x06]); // interface control: RGB, 16 bit
        self.cmd(0x2C, &[]);
        delay.delay_ms(200);
        self.cmd(0x26, &[0x01]); // gamma curve 1
        self.cmd(
            0xE0, // positive gamma
            &[
                0x0F, 0x29, 0x24, 0x0C, 0x0E, 0x09, 0x4E, 0x78, 0x3C, 0x09, 0x13, 0x05, 0x17, 0x11, 0x00,
            ],
        );
        self.cmd(
            0xE1, // negative gamma
            &[
                0x00, 0x16, 0x1B, 0x04, 0x11, 0x07, 0x31, 0x33, 0x42, 0x05, 0x0C, 0x0A, 0x28, 0x2F, 0x0F,
            ],
        );
        self.cmd(0x11, &[]); // sleep out
        delay.delay_ms(200);
        self.cmd(0x29, &[]); // display on
        self.cmd(0x2C, &[]);
        defmt::info!("ili9341: RGB interface mode");
    }
}

/// The LTDC as a twine [`FramebufferDisplay`]: two RGB565 framebuffers in SDRAM, swapped at
/// vertical blanking.
///
/// `present(i)` writes layer 1's `CFBAR` and requests a shadow reload at the next vertical
/// blanking (`SRCR.VBR`); `present_done` reports `true` once the hardware has cleared `VBR`,
/// i.e. the swap happened and the other buffer may be drawn into. Nothing blocks and no
/// interrupt is needed: the engine polls.
pub struct LtdcDisplay {
    _ltdc: Ltdc<'static, peripherals::LTDC>,
    fbs: Option<(DrawBufferMem, DrawBufferMem)>,
    addrs: [u32; 2],
    presented: u8,
}

impl LtdcDisplay {
    /// The framebuffer on screen (0 or 1).
    pub fn presented(&self) -> u8 {
        self.presented
    }
}

impl FramebufferDisplay for LtdcDisplay {
    type Error = core::convert::Infallible;

    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(WIDTH, HEIGHT, ColorFormat::Rgb565)
    }

    fn framebuffers(&mut self) -> Option<(DrawBufferMem, Option<DrawBufferMem>)> {
        self.fbs.take().map(|(a, b)| (a, Some(b)))
    }

    fn present(&mut self, index: u8) -> Result<(), Self::Error> {
        let i = usize::from(index & 1);
        let regs = pac::LTDC;
        regs.layer(0).cfbar().write(|w| w.set_cfbadd(self.addrs[i]));
        regs.srcr().write(|w| w.set_vbr(pac::ltdc::vals::Vbr::RELOAD));
        self.presented = index & 1;
        Ok(())
    }

    fn present_done(&mut self) -> bool {
        pac::LTDC.srcr().read().vbr() == pac::ltdc::vals::Vbr::NO_EFFECT
    }
}

/// Heap size (internal SRAM; the framebuffers are in SDRAM).
pub const HEAP_BYTES: usize = 128 * 1024;

#[global_allocator]
static HEAP: embedded_alloc::LlffHeap = embedded_alloc::LlffHeap::empty();

/// Hands [`HEAP_BYTES`] of internal SRAM to the allocator (once).
pub fn init_heap() {
    static DONE: AtomicBool = AtomicBool::new(false);
    static mut MEMORY: [MaybeUninit<u8>; HEAP_BYTES] = [MaybeUninit::uninit(); HEAP_BYTES];
    assert!(!DONE.swap(true, Ordering::AcqRel), "init_heap called twice");
    // SAFETY: the flag guarantees this runs once, so `MEMORY` is handed to the allocator exactly
    // once and never accessed any other way; it lives for the whole program.
    unsafe { HEAP.init((&raw mut MEMORY).cast::<u8>() as usize, HEAP_BYTES) }
}

/// Heap statistics for the `twine::perf` log line; warns when the heap is more than 90 % full.
pub fn mem_info() -> MemInfo {
    static PEAK: AtomicU32 = AtomicU32::new(0);
    let used = HEAP.used() as u32;
    let free = HEAP.free() as u32;
    let peak = PEAK.fetch_max(used, Ordering::Relaxed).max(used);
    if u64::from(used) * 10 > u64::from(used + free) * 9 {
        defmt::warn!("heap above 90%: {} of {} bytes", used, used + free);
    }
    MemInfo { used, peak, free }
}
