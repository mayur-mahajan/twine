//! Example firmware: Twine on the STM32F429I-DISC1 — the full-framebuffer path.
//!
//! The 240 × 320 ILI9341 is scanned out by the LTDC from two RGB565 framebuffers in the 8 MiB
//! SDRAM ([`LtdcDisplay`](twine_example_stm32f429i_disco::LtdcDisplay)); the engine renders the
//! dirty areas of each frame straight into the back buffer (`BufferMode::Full`), the DMA2D does
//! fills, blits and glyph blending (`Engine::set_accel`; feature `no-dma2d` renders in software
//! for comparison), and the buffers swap at vertical blanking. The STMPE811 resistive touch
//! controller is on I2C3 with its interrupt on PA15.
//!
//! The UI sleeps until the touch interrupt, a channel message (the UI waker) or its next
//! deadline. Every 5 s the RTT log shows a `twine::perf` line (fps, CPU, render time, heap).
//! Features pick the demo: `demo-counter` (default) or `demo-controls`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;

use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::{bind_interrupts, interrupt, pac};
use twine::engine::EngineConfig;
use twine::hal::Calibration;
use twine::prelude::*;
use twine_embassy::{EmbassyClock, to_embassy, wait_waker};
use twine_example_stm32f429i_disco::{Board, HEIGHT, WIDTH, clocks, init_heap, mem_info};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    EXTI15_10 => exti::InterruptHandler<interrupt::typelevel::EXTI15_10>;
});

/// STMPE811 raw readings (12 bit) → screen pixels, from the ST BSP (`BSP_TS_GetState`:
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
async fn main(_spawner: Spawner) {
    init_heap();
    let p = embassy_stm32::init(clocks());
    let delay = &mut embassy_time::Delay;
    let board = Board::init(p, delay);
    let _led = Output::new(board.led_green, Level::High, Speed::Low);

    // Touch: the driver reads the INT level through `IntLevel`; this task waits on its edge.
    let mut int = ExtiInput::new(board.touch_int, board.touch_exti, Pull::Up, Irqs);
    let i2c = I2c::new_blocking(
        board.touch_i2c,
        board.touch_scl,
        board.touch_sda,
        i2c::Config::default(),
    );
    let mut touch = twine_drivers::touch::Stmpe811::new(i2c, Some(IntLevel))
        .with_calibration(TOUCH_CAL)
        .with_screen_size(WIDTH, HEIGHT);
    if touch.init(delay).is_err() {
        defmt::warn!("stmpe811: init failed (touch disabled)");
    }

    let config = EngineConfig {
        mem_info: Some(mem_info),
        hires_timer: Some(twine_embassy::hires_now),
        ..EngineConfig::default()
    };
    // SAFETY: the UI and every reactive handle live in this task on the thread-mode executor and
    // are never touched from an interrupt handler or another executor; other contexts only use
    // channels and the UI waker.
    let builder = unsafe { Ui::builder_fb(board.display).bind_to_current_context() };
    let mut ui = builder
        .buffers(BufferMode::Full)
        .input(touch)
        .clock(EmbassyClock)
        .config(config)
        .theme(DefaultTheme::light())
        .build(demo::app);
    if cfg!(not(feature = "no-dma2d")) {
        // Bounded waits: a hung DMA2D is aborted after ~1M polls (tens of ms at 180 MHz), the
        // operation is drawn in software and the engine raises `FaultKind::AccelTimeout`.
        let dma2d = twine_accel_stm32::Dma2d::new(twine_accel_stm32::PacRegs::new()).with_timeout(1_000_000);
        ui.engine_mut().set_accel(Some(Box::new(dma2d)));
    }
    defmt::info!(
        "twine: {} demo on {}x{} LTDC, DMA2D {}",
        demo::NAME,
        WIDTH,
        HEIGHT,
        if cfg!(feature = "no-dma2d") { "off" } else { "on" }
    );

    loop {
        let wake = ui.update();
        if matches!(wake, Wake::Now) {
            embassy_futures::yield_now().await;
            continue;
        }
        if int.is_low() {
            // Touched (or FIFO data) before this wait started: read it now.
            ui.notify_input();
            continue;
        }
        let deadline = async {
            match wake {
                Wake::At(t) => embassy_time::Timer::at(to_embassy(t)).await,
                _ => core::future::pending::<()>().await,
            }
        };
        if let Either3::Third(()) =
            select3(deadline, wait_waker(ui.waker()), int.wait_for_falling_edge()).await
        {
            ui.notify_input();
        }
    }
}

/// The STMPE811 `INT` level (PA15, active low), read from the GPIO input register: the pin
/// itself belongs to the `ExtiInput` the run loop waits on.
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
