//! Example firmware: Twine on the STM32F429I-DISC1 — the full-framebuffer path.
//!
//! The 240 × 320 ILI9341 is scanned out by the LTDC from two RGB565 framebuffers in the 8 MiB
//! SDRAM ([`LtdcDisplay`](twine_example_stm32f429i_disco::LtdcDisplay)); the engine renders the
//! dirty areas of each frame straight into the back buffer (`BufferMode::Full`), the DMA2D does
//! fills, blits and glyph blending (`UiBuilder::accel`, which owns it; feature `no-dma2d` renders
//! in software for comparison), and the buffers swap at vertical blanking. The STMPE811 resistive touch
//! controller is on I2C3 with its interrupt on PA15.
//!
//! The blocking `Ui` runs on embassy (`twine::embassy::run`): it sleeps until its next deadline
//! or its waker, which a small task sets on the touch interrupt (or a channel message sets).
//! Every 5 s the RTT log shows a `twine::perf` line (fps, CPU, render time, heap).
//! Features pick the demo: `demo-counter` (default) or `demo-controls`.
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::{bind_interrupts, interrupt, mode, pac};
use twine::hal::Calibration;
use twine::prelude::*;
use twine::embassy::UiBuilderExt;
use twine_example_stm32f429i_disco::{Board, HEIGHT, WIDTH, clocks, init_heap, mem_info};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    EXTI15_10 => exti::InterruptHandler<interrupt::typelevel::EXTI15_10>;
});

/// STMPE811 raw readings (12 bit) → native panel pixels (the display runs at `Deg0`), from the ST BSP (`BSP_TS_GetState`:
/// `x = (3870 − raw_x) / 15`, `y = (raw_y − 360) / 11`). Replace after checking your board.
const TOUCH_CAL: Calibration = Calibration {
    a: -11,
    b: 0,
    c: 3870 * 11,
    d: 0,
    e: 15,
    f: -360 * 15,
    div: 165,
};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    init_heap();
    let p = embassy_stm32::init(clocks());
    let delay = &mut embassy_time::Delay;
    let board = Board::init(p, delay);
    let _led = Output::new(board.led_green, Level::High, Speed::Low);

    // Touch: the driver reads the INT level through `IntLevel`; `touch_irq` waits on its edge.
    let int = ExtiInput::new(board.touch_int, board.touch_exti, Pull::Up, Irqs);
    let i2c = I2c::new_blocking(
        board.touch_i2c,
        board.touch_scl,
        board.touch_sda,
        i2c::Config::default(),
    );
    // Fitted to the display (clamped to 240 × 320) when the `Ui` is built.
    let mut touch = twine::drivers::touch::Stmpe811::new(i2c, Some(IntLevel)).with_calibration(TOUCH_CAL);
    if touch.init(delay).is_err() {
        defmt::warn!("stmpe811: init failed (touch disabled)");
    }

    // The demos' configuration — the one their simulator examples and tests run — plus this
    // board's hooks.
    let mut config = twine_demos::config();
    config.engine.mem_info = Some(mem_info);
    config.engine.hires_timer = Some(twine::embassy::hires_now);
    // The runtime belongs to this task: the `!Send` token keeps the UI and every reactive
    // handle here; interrupt handlers and other tasks only use channels and the UI waker.
    let rt = Runtime::take().expect("runtime already taken");
    let builder = Ui::builder_fb(board.display).runtime(rt);
    // The DMA2D belongs to the UI from here on. Bounded waits: a hung DMA2D is aborted after
    // ~1M polls (tens of ms at 180 MHz), the operation is drawn in software and the engine
    // raises `FaultKind::AccelTimeout`.
    #[cfg(not(feature = "no-dma2d"))]
    let builder = {
        let regs = twine_accel_stm32::PacRegs::new(board.dma2d).unwrap_or_else(|_| defmt::panic!("DMA2D already owned"));
        builder.accel(twine_accel_stm32::Dma2d::new(regs).with_timeout(1_000_000))
    };
    let ui = builder
        .buffers(BufferMode::Full)
        .input(touch)
        .with_embassy_platform()
        .app_config(config)
        .build(demo::app);
    defmt::info!(
        "twine: {} demo on {}x{} LTDC, DMA2D {}",
        demo::NAME,
        WIDTH,
        HEIGHT,
        if cfg!(feature = "no-dma2d") { "off" } else { "on" }
    );

    spawner.spawn(touch_irq(int, ui.waker()).expect("touch task already running"));
    twine::embassy::run(ui).await
}

/// Wakes the UI while the STMPE811 interrupt (PA15, active low: touched, or FIFO data) is
/// asserted; the UI reads the controller in its next update, which releases the line. While
/// the panel is held, the engine polls it on its own deadline.
#[embassy_executor::task]
async fn touch_irq(mut int: ExtiInput<'static, mode::Async>, waker: &'static UiWaker) -> ! {
    loop {
        int.wait_for_low().await;
        waker.wake();
        int.wait_for_high().await;
    }
}

/// The STMPE811 `INT` level (PA15, active low), read from the GPIO input register: the pin
/// itself belongs to the `ExtiInput` that `touch_irq` waits on.
struct IntLevel;

impl embedded_hal::digital::ErrorType for IntLevel {
    type Error = core::convert::Infallible;
}

impl embedded_hal::digital::InputPin for IntLevel {
    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(pac::GPIOA.idr().read().idr(15) == pac::gpio::vals::Idr::HIGH)
    }

    fn is_low(&mut self) -> Result<bool, Self::Error> {
        self.is_high().map(|h| !h)
    }
}

/// The demo selected by cargo feature.
mod demo {
    #[cfg(feature = "demo-controls")]
    pub use twine_demos::controls::app;
    #[cfg(not(feature = "demo-controls"))]
    pub use twine_demos::counter::app;

    /// Name of the demo (for the log).
    pub const NAME: &str = if cfg!(feature = "demo-controls") {
        "controls"
    } else {
        "counter"
    };
}
