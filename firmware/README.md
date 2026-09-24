# Example firmware

One small example per chip family. They are **templates to copy, not board support**: each
`src/main.rs` starts with a single `Wiring: edit for your board` block (peripherals, pins,
clocks, rotation, touch calibration), and cargo features pick the drivers. They prove that the
stack builds, links and fits on each chip; the drivers themselves are in `twine-drivers` (one
cargo feature per driver) and work with any board.

| Example | Chip | Target | Toolchain | Display path | Flash with |
|---------|------|--------|-----------|--------------|------------|
| [`rp2040`](rp2040) | RP2040 (Pico) | `thumbv6m-none-eabi` | stable | SPI0 + DMA, async | `probe-rs` |
| [`rp2350`](rp2350) | RP2350A (Pico 2) | `thumbv8m.main-none-eabihf` | stable | SPI0 + DMA, async | `probe-rs` |
| [`stm32f411`](stm32f411) | STM32F411CE (BlackPill) | `thumbv7em-none-eabihf` | stable | SPI1 + DMA2, async | `probe-rs` |
| [`esp32c3`](esp32c3) | ESP32-C3 | `riscv32imc-unknown-none-elf` | stable | SPI2 + DMA, touch on the same bus | `espflash` |
| [`esp32c6`](esp32c6) | ESP32-C6 | `riscv32imac-unknown-none-elf` | stable | SPI2 + DMA, touch on I2C0 (or the same bus) | `espflash` |
| [`esp32s3`](esp32s3) | ESP32-S3 | `xtensa-esp32s3-none-elf` | `esp` | SPI2 + DMA, or QSPI AMOLED (`twine-esp`) | `espflash` |
| [`esp32`](esp32) | ESP32 | `xtensa-esp32-none-elf` | `esp` | SPI2 + DMA, touch on SPI3 | `espflash` |

Every example runs the Twine UI with `twine-embassy`: the display is flushed with SPI DMA while
the next chunk renders, and the UI sleeps until the touch interrupt, a channel message or its
next deadline. Every 5 s the log shows a `twine::perf` line with fps, CPU load, render and flush
time and the heap.

## Features

| Feature | Meaning |
|---------|---------|
| `panel-ili9341` (default), `panel-st7789`, `panel-jd9853` (`esp32c6`, its default) | SPI panel driver |
| `panel-co5300`, `panel-sh8601`, `panel-rm67162` (`esp32s3`) | QSPI AMOLED driver |
| `touch-xpt2046` (default), `touch-ft6x36`, `touch-cst816s` (`esp32s3`), `touch-axs5106l` (`esp32c6`, its default) | touch driver |
| `demo-counter` (default), `demo-controls`, `demo-calibrate` | the demo |

Exactly one `panel-*` and one `touch-*` feature: switch with `--no-default-features`, e.g.

```sh
cargo run --release --no-default-features --features panel-st7789,touch-ft6x36,demo-controls
```

## Building and flashing

```sh
cd firmware/rp2040        # or rp2350, stm32f411
cargo run --release       # probe-rs: flashes and shows the defmt log
```

ESP32-S3 and ESP32 (Xtensa) need Espressif's toolchain:

```sh
cargo install espup --locked && espup install   # once
. ~/export-esp.sh                               # in every shell
cargo install espflash --locked                 # once
cd firmware/esp32s3 && cargo run --release       # flashes and opens the serial monitor
```

The ESP32-C3 and ESP32-C6 build with the stable toolchain (`rustup target add
riscv32imc-unknown-none-elf` / `riscv32imac-unknown-none-elf`).
`cargo xtask firmware [example|all]` builds every example for its feature sets (listed under
`[package.metadata.twine]` in each `Cargo.toml`) and prints the flash and static RAM sizes.

| Example | Flash, counter / controls | Static RAM (incl. heap and draw buffers) |
|---------|---------------------------|------------------------------------------|
| rp2040 | 357 / 451 KiB | 154 KiB |
| rp2350 | 351 / 445 KiB | 314 KiB |
| stm32f411 | 353 / 447 KiB (of 512) | 81 KiB (of 128) |
| esp32c3 | 660 / 796 KiB | 226 KiB |
| esp32c6 | 854 / 985 KiB | 229 KiB |
| esp32s3 | 533 / 652 KiB | 394 KiB |
| esp32 | 529 / 646 KiB | 289 KiB |

### Atomics on RP2040 and ESP32-C3

The RP2040 (Cortex-M0+) and the ESP32-C3 (RV32IMC) have no atomic read-modify-write
instructions. Twine's crates use `portable-atomic` and leave the choice of fallback to the
application:

- **RP2040:** `portable-atomic = { version = "1", features = ["critical-section"] }` plus a
  critical-section implementation (here `embassy-rp`'s `critical-section-impl`, which is correct
  on both cores).
- **ESP32-C3:** nothing to add — `esp-hal` enables `portable-atomic`'s `unsafe-assume-single-core`.
  Other single-core RISC-V chips: either the `critical-section` feature with a critical-section
  implementation, or `--cfg portable_atomic_unsafe_assume_single_core` in `RUSTFLAGS`.

Never enable both: `portable-atomic` rejects the combination.

### DMA and copies on ESP32

esp-hal 1.2's `SpiDma` writes the draw buffers **in place** (no copy) because they are
4-byte aligned statics in internal RAM. It only copies data it cannot reach by DMA (flash,
PSRAM on some chips, unaligned data on the ESP32), and without a configured copy buffer it
reports an error instead. Keep draw buffers in internal RAM.

## Known boards

The examples' default pins fit these boards; pins come from the vendors' published schematics
and example code.

| Board | Example and features | Pins (display / touch) | Notes |
|-------|----------------------|------------------------|-------|
| Raspberry Pi Pico + 2.8" ILI9341 SPI module with XPT2046 (red PCB) | `rp2040` defaults | SCK 18, MOSI 19, CS 17, DC 20, RST 21, LED 22 / T_CLK 10, T_DIN 11, T_DO 12, T_CS 13, T_IRQ 14 | module wired by hand |
| Raspberry Pi Pico 2 + the same module | `rp2350` defaults | as the Pico | |
| WeAct BlackPill STM32F411CE + the same module | `stm32f411` defaults | SCK PA5, MOSI PA7, CS PA4, DC PB0, RST PB1, LED PB10 / T_CLK PB13, T_DIN PB15, T_DO PB14, T_CS PB12, T_IRQ PA8 | KEY button on PA0 |
| ESP32-C3-DevKitM-1 + the same module | `esp32c3` defaults | SCK 6, MOSI 7, MISO 5, CS 10, DC 4, RST 3, LED 1 / T_CS 0, T_IRQ 8 (shared SPI) | GPIO8 also drives the RGB LED |
| ESP32-S3-DevKitC-1 + the same module | `esp32s3` defaults | SCK 12, MOSI 11, CS 10, DC 9, RST 8, LED 7 / T_CLK 4, T_DIN 5, T_DO 6, T_CS 15, T_IRQ 16 | avoid GPIO26–37 on N8R8 modules (flash, octal PSRAM) |
| ESP32-2432S028R "Cheap Yellow Display" | `esp32` defaults (`panel-st7789` for the ST7789 revision) | HSPI: SCK 14, MOSI 13, MISO 12, CS 15, DC 2, BL 21, RST tied to EN / VSPI: CLK 25, MOSI 32, MISO 39, CS 33, IRQ 36 | RGB LED 4/16/17 active low; pins from [witnessmenow/ESP32-Cheap-Yellow-Display `PINS.md`](https://github.com/witnessmenow/ESP32-Cheap-Yellow-Display/blob/main/PINS.md) |
| Waveshare ESP32-S3-Touch-AMOLED-2.06 | `esp32s3`, `panel-co5300,touch-ft6x36` (defaults of the QSPI branch) | QSPI: SCLK 11, CS 12, SIO0–3 4/5/6/7, RST 8 / I2C SDA 15, SCL 14, INT 38 (TP_RST 9) | 410 × 502, column offset 22; set the touch INT pin to 38; pins from the board's `pin_config.h` ([waveshareteam/ESP32-S3-Touch-AMOLED-2.06](https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-2.06)) |
| Waveshare ESP32-S3-Touch-AMOLED-1.8 | `esp32s3`, `panel-sh8601,touch-ft6x36` | QSPI: SCLK 11, CS 12, SIO0–3 4/5/6/7, no RST pin / I2C SDA 15, SCL 14, INT 21 | 368 × 448. Panel and touch reset go through a TCA9554 I/O expander at I2C `0x20`: drive its pins 0–2 low, wait 20 ms, drive them high before initializing the panel (pass `None` as reset). Pins from [waveshareteam/ESP32-S3-Touch-AMOLED-1.8](https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-1.8) |
| Waveshare ESP32-C6-Touch-LCD-1.47 | `esp32c6` defaults (`panel-jd9853,touch-axs5106l`) | SPI: SCK 1, MOSI 2, (MISO 3), CS 14, DC 15, RST 22, BL 23 / I2C SDA 18, SCL 19, INT 21, TP_RST 20 | JD9853 172 × 320 IPS: the 172 visible columns sit at column 34 of the 240-column controller memory (the driver applies the offset for every rotation); backlight GPIO23, active high (PWM it for dimming); AXS5106L touch at I2C `0x63` with its X axis mirrored against the display (`TOUCH_MIRROR_RAW_X`); SD card on the same SPI bus with CS 4. Pins from the board's demo (`ESP32-C6-Touch-LCD-1.47-Demo.zip`, ESP-IDF `components/esp_bsp`, [wiki](https://www.waveshare.com/wiki/ESP32-C6-Touch-LCD-1.47)) |
| LilyGO T-Display-S3 AMOLED 1.91" | `esp32s3`, `panel-rm67162,touch-cst816s` | QSPI: SCLK 47, CS 6, SIO0–3 18/7/48/5, RST 17, TE 9 / I2C SDA 3, SCL 2, INT 21 | 240 × 536; drive GPIO38 high first (panel power enable); pins from [Xinyuan-LilyGO/LilyGo-AMOLED-Series `LilyGo_AMOLED.h`](https://github.com/Xinyuan-LilyGO/LilyGo-AMOLED-Series) |

### Using another board

1. Copy the example for your chip.
2. Edit the wiring block: peripherals, pins, clock, rotation (`Rotation::Deg90` = the panel
   turned 90° clockwise).
3. Pick the panel and touch features. A panel that `twine-drivers` does not know yet is a
   `twine_drivers::mipi_dcs::PanelSpec` (size, offsets, `MADCTL` per rotation, init table)
   passed to `mipi_dcs::AsyncMipiDcs::new`.
4. For resistive touch, run `--features demo-calibrate`: the example wraps the touch driver in
   `twine_demos::calibration::RawTouchInput` (raw readings, no clamping), you tap the three
   crosses, and the log prints `Calibration { a: …, div: … }`. Paste it as `TOUCH_CAL` into the
   wiring block; the five crosses that follow show the remaining error of each tap.

A blocking super-loop without embassy also works: `Ui::builder(display)` with a blocking
driver (`twine_drivers::ili9341::new`) over a blocking `SpiDevice`, calling `ui.update()` and
sleeping until the returned deadline. Transfers then do not overlap rendering.

## Hardware checklist

Run it on every board you have and record the results:

1. **Picture:** the counter shows within 1 s of reset; colours are right (the blue button is
   blue, not orange — otherwise the panel needs the other RGB/BGR order or byte order); the
   picture is upright for the chosen rotation (mirrored → wrong panel variant / `MADCTL`;
   white screen → check RST and DC).
2. **Touch:** after calibration (`demo-calibrate`) the five verification targets are hit
   within ±4 px (the verification page shows the error).
3. **Partial refresh:** tapping "Click me" updates the label within one frame, and the log
   (`twine::refresh` at debug level) shows one small refreshed area.
4. **Controls demo:** dragging a slider is smooth; the perf line shows the fps; no tearing
   beyond the diagonal shear expected on SPI panels without a TE line.
5. **Idle:** after 10 s without touch the perf line shows `cpu=0%` and `fps=0`; if you have a
   current meter, note the idle and active current.
6. **Stability:** the controls demo runs 10 min with a stable heap value in the perf lines.

Board-specific items:

- **Waveshare ESP32-C6-Touch-LCD-1.47:** (a) colours: the blue button is blue and white is
  white, not black (inversion) or orange (RGB/BGR); (b) orientation at all four rotations:
  upright, not mirrored, and no garbage stripe along the long edges (a stripe means the
  34-column offset is wrong); (c) touch: tapping each corner and the centre of the counter demo
  hits the tapped spot within ±4 px at `Deg0` and `Deg90` (a mirrored X means
  `TOUCH_MIRROR_RAW_X` is wrong); (d) the log shows `axs5106l id …` at boot.
