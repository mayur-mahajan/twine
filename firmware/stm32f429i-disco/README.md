# STM32F429I-DISC1: full framebuffer, DMA2D

Twine on ST's STM32F429I-DISC1 (STM32F429ZIT6, 2 MiB flash, 192 KiB SRAM + 64 KiB CCM, 8 MiB
SDRAM, 2.4" 240 × 320 ILI9341 in RGB interface mode, STMPE811 resistive touch). This is the
full-framebuffer example: the LTDC scans out one of two RGB565 framebuffers in SDRAM while the
engine renders the dirty areas of the next frame into the other (`BufferMode::Full`); the DMA2D
does fills, blits and glyph blending (`Engine::set_accel`), and the buffers swap at vertical
blanking. Unlike the SPI examples there is no flush: presenting a frame is one register write.

| Part | How |
|------|-----|
| Clocks | HSE 8 MHz (ST-LINK MCO, bypass) → 168 MHz SYSCLK; PLLSAI 192 MHz / 4 / 8 = 6 MHz pixel clock |
| SDRAM | IS42S16400J on FMC bank 2 (`0xD000_0000`), 16 bit, SDCLK 84 MHz, CAS 2 (`stm32-fmc` timings = ST BSP) |
| Panel | ILI9341 configured over SPI5 (SCK PF7, MOSI PF9, CS PC2, D/CX PD13) for RGB mode (ST BSP `ili9341_Init`) |
| LTDC | layer 1 RGB565 240 × 320; HSYNC 10, HBP 20, HFP 10, VSYNC 2, VBP 2, VFP 4; 18 data lines (R/G/B 2..7) |
| Framebuffers | 2 × 150 KiB at the start of the SDRAM; `present` writes `L1CFBAR` + `SRCR.VBR`, done when `VBR` clears |
| DMA2D | `twine_accel_stm32::Dma2d<PacRegs>` (clock enabled in `Board::init`) |
| Touch | STMPE811 on I2C3 (SCL PA8, SDA PC9), `INT` PA15 (EXTI 15) |
| Heap | 128 KiB (`embedded-alloc`) in internal SRAM |

Board support (clocks, SDRAM, panel, LTDC, `LtdcDisplay`, heap) is in `src/lib.rs`; `src/main.rs`
runs a demo, `src/bin/bench.rs` the renderer benchmark.

## Build and flash

```sh
cd firmware/stm32f429i-disco
cargo run --release                                # counter demo (default)
cargo run --release --features demo-controls       # controls demo
cargo run --release --features demo-controls,no-dma2d   # software rendering, for comparison
cargo run --release --bin bench                    # renderer benchmark, software vs DMA2D
```

`probe-rs` flashes through the on-board ST-LINK/V2-B and shows the defmt log. From the workspace
root, `cargo xtask firmware stm32f429i-disco` builds every feature set and reports sizes.

Features: `demo-counter` (default), `demo-controls`, `no-dma2d`. The data widgets demo is
added once Phase 20 provides it.

## Hardware-in-the-loop checks (manual)

Not yet run: no board was available when this example was written. The init sequences
are ports of ST's BSP. Check these on a board:

1. **Boot and SDRAM**: the log shows `sdram: 8192 KiB at 0xd0000000`, `ili9341: RGB interface
   mode` and `ltdc: 240x320 RGB565, framebuffers at 0xd0000000 and 0xd0025800`.
2. **Full-screen fill**: `cargo run --release --bin bench`. Every scenario appears on the panel
   for a moment, starting with a full-screen blue fill. No tearing, no shifted or wrapped
   lines (a shift means wrong porches or pixel-clock polarity).
3. **Bench numbers**: one `bench <name>: sw … dma2d … x…` line per scenario. The
   `fill_fullscreen` speed-up must be ≥ 2×. Record the numbers in `docs/perf.md`.
4. **Touch**: in the counter demo, tapping the button increments the label. If taps land in
   the wrong place, adjust `TOUCH_CAL` in `src/main.rs` (derived from the BSP's
   `x = (3870 − raw_x) / 15`, `y = (raw_y − 360) / 11`).
5. **Frame rate**: with `demo-controls`, drag a slider and read the `twine::perf` lines (fps,
   CPU, render time). Repeat with `no-dma2d` and record both.
6. **Idle**: with no input, the `twine::perf` line shows 0 fps and the CPU sleeps.
