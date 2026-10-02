# twine-drivers

Display, touch and input drivers for the Twine GUI library. Generic over `embedded-hal` 1.0,
`no_std`, allocation-free; every display driver exists in a blocking (`DisplayDriver`) and an
async (`AsyncDisplayDriver`, feature `async`) flavour.

Every driver and bus interface is a cargo feature and none is enabled by default. Applications
enable them on the `twine` facade, as `drivers-<name>` (`twine::drivers`): a panel's facade
feature also compiles the renderer for the panel's pixel format, and `async`, `log` and `defmt`
reach the drivers with the rest of Twine, e.g.

```toml
twine = { version = "0.1", features = ["drivers-ili9341", "drivers-xpt2046", "async"] }
```

Used on its own (without the facade), enable the same features here (`ili9341`, …) and the
panel's `color-*` feature on the engine yourself.

The drivers know nothing about boards or pins; constructors take your HAL's bus, pin and delay
objects, and each module's documentation shows the wiring. `all` enables everything.

## Displays

| Module | Panel | Size | Interface | Format | Notes |
|--------|-------|------|-----------|--------|-------|
| `ili9341` | ILI9341 | 240 × 320 | SPI, i80 | RGB565 | BGR, Adafruit rotation mapping |
| `ili9342` | ILI9342C | 320 × 240 | SPI, i80 | RGB565 | landscape native, BGR, inverted (M5Stack, ESP32-S3-BOX-3) |
| `ili9488` | ILI9488 | 320 × 480 | SPI | RGB666 (`Rgb888`) | 3 bytes/pixel: 1.5× slower than RGB565 |
| `st7789` | ST7789 | 240 × 320, 240 × 240, 135 × 240 | SPI, i80 | RGB565 | inverted, offsets per variant |
| `st7735` | ST7735R/S | 128 × 160 (green/red/black tab), 80 × 160 | SPI | RGB565 | tab-specific offsets and colour order |
| `st7796` | ST7796S | 320 × 480 | SPI, i80 | RGB565 | BGR |
| `gc9a01` | GC9A01 | 240 × 240 round | SPI | RGB565 | BGR, inverted |
| `jd9853` | JD9853 | 172 × 320 | SPI | RGB565 | inverted, column offset 34 (240-column memory) |
| `ssd1306` | SSD1306 | 128 × 64, 128 × 32, 72 × 40 | I2C, SPI | `I1` | page conversion, `align = 8` |
| `sh1106` | SH1106 | 128 × 64 | I2C, SPI | `I1` | 132-column RAM, offset 2 |
| `co5300` | CO5300 AMOLED | 410 × 502 | QSPI | RGB565 | software 90°/270°, even areas, brightness command |
| `sh8601` | SH8601 AMOLED | 368 × 448 | QSPI | RGB565 | software 90°/270°, even areas, brightness command |
| `rm67162` | RM67162 AMOLED | 240 × 536 | QSPI | RGB565 | even areas, brightness command |

Any other MIPI DCS panel works with a custom `mipi_dcs::PanelSpec` (size, offsets, `MADCTL` per
rotation, `COLMOD`, inversion, init table).

## Touch and input

| Module | Device | Bus |
|--------|--------|-----|
| `touch::xpt2046` | XPT2046 / ADS7843 resistive | SPI |
| `touch::ft6x36`, `touch::ft5x06` | Focaltech capacitive (also FT3168) | I2C |
| `touch::gt911` | Goodix GT911 capacitive | I2C |
| `touch::cst816s` | Hynitron CST816S capacitive | I2C |
| `touch::axs5106l` | AXS5106L capacitive | I2C |
| `touch::stmpe811` | ST STMPE811 resistive | I2C |
| `encoder` | rotary encoder (quadrature + button) | GPIO / interrupt |
| `keypad` | key matrix | GPIO |

IRQ-capable input drivers perform no bus traffic while idle.

## Rotation

`Rotation::Deg90` means the panel is turned 90° clockwise, for hardware rotation (`MADCTL`) and
the engine's software rotation alike; a test checks every panel's `MADCTL` table against the
software rotation.

## Driver matrix

"Tested board" is hardware on which the driver was run; no driver has been run on hardware
yet. All are verified on the host against recording mock buses (exact command and data bytes,
see the `testkit` feature). "Firmware feature" names the cargo feature of the example firmware in
the repository's `firmware/` directory that builds the driver for every example board (RP2040,
RP2350, STM32F411, ESP32, ESP32-C3, ESP32-C6, ESP32-S3), so `cargo xtask firmware` catches
bit-rot. Max clock is the datasheet write clock; what modules run in practice is in the notes.

| Device | Interface | Tested board | Max clock | Firmware feature | Notes |
|--------|-----------|--------------|-----------|------------------|-------|
| ILI9341 | SPI, i80 | — (not tested on hardware) | 10 MHz | `panel-ili9341` (default) | modules routinely run at 40–62.5 MHz |
| ILI9342C | SPI, i80 | — (not tested on hardware) | 10 MHz | `panel-ili9342` | boards run it at 40 MHz |
| ILI9488 | SPI (RGB666) | — (not tested on hardware) | 20 MHz | — | 3 bytes/pixel; modules often run at 40 MHz |
| ST7789 | SPI, i80 | — (not tested on hardware) | 15 MHz | `panel-st7789` | modules usually run at 40–80 MHz |
| ST7735R/S | SPI | — (not tested on hardware) | 15 MHz | — | modules usually work at 20–40 MHz |
| ST7796S | SPI, i80 | — (not tested on hardware) | 15 MHz | `panel-st7796` | modules commonly run at 40 MHz |
| GC9A01 | SPI | — (not tested on hardware) | 100 MHz | — | modules typically 40–80 MHz |
| JD9853 | SPI | — (not tested on hardware) | 80 MHz (vendor demo) | `panel-jd9853` (ESP32-C6 only) | |
| CO5300, SH8601 | QSPI | — (not tested on hardware) | 80 MHz on ESP32-S3 | `panel-co5300`, `panel-sh8601` (ESP32-S3 only) | |
| RM67162 | QSPI | — (not tested on hardware) | 75 MHz (vendor) | `panel-rm67162` (ESP32-S3 only) | |
| SSD1306 | I2C, SPI | — (not tested on hardware) | I2C 400 kHz, SPI 10 MHz | `oled-ssd1306` (I2C, no touch) | `I1`, mono theme |
| SH1106 | I2C, SPI | — (not tested on hardware) | I2C 400 kHz, SPI 4 MHz | — | |
| XPT2046 | SPI | — (not tested on hardware) | 2.5 MHz (use ≤ 2 MHz) | `touch-xpt2046` (default) | may share the display bus with its own CS |
| FT6x36 / FT5x06 / FT3168 | I2C | — (not tested on hardware) | 400 kHz | `touch-ft6x36` | |
| GT911 | I2C | — (not tested on hardware) | 400 kHz | `touch-gt911` | |
| CST816S | I2C | — (not tested on hardware) | 400 kHz | `touch-cst816s` | |
| AXS5106L | I2C | — (not tested on hardware) | 400 kHz | `touch-axs5106l` (ESP32-C6 only) | |
| STMPE811 | I2C | — (not tested on hardware) | 400 kHz | — | used by the STM32F429I-DISC1 |
| rotary encoder | GPIO / interrupt | — (not tested on hardware) | — | — | quadrature decoder usable from an ISR |
| key matrix | GPIO | — (not tested on hardware) | — | — | debounced, 8-event queue |

The example firmware's hardware checklist lists what to verify when a board is first brought up.

## Init sequences

Init tables come from the controllers' datasheets and well-known open-source drivers, cited in
each module: Adafruit_ILI9341 and Adafruit-ST7735-Library (BSD), mipidsi (MIT/Apache-2.0), the
`ssd1306` and `sh1106` crates (MIT/Apache-2.0), ST's STM32Cube BSP (BSD-3-Clause), Waveshare's
ESP32-S3-Touch-AMOLED examples (`Arduino_GFX`, BSD), Waveshare's ESP32-C6-Touch-LCD-1.47 demo
(JD9853 table and AXS5106L protocol, Apache-2.0) and LilyGO's `LilyGo-AMOLED-Series` (MIT). See
`THIRD_PARTY.md` in the repository root.
