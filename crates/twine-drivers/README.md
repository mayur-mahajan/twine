# twine-drivers

Display, touch and input drivers for the Twine GUI library. Generic over `embedded-hal` 1.0,
`no_std`, allocation-free; every display driver exists in a blocking (`DisplayDriver`) and an
async (`AsyncDisplayDriver`, feature `async`) flavour.

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
| `ssd1306` | SSD1306 | 128 × 64, 128 × 32, 72 × 40 | I2C, SPI | `I1` | page conversion, `align = 8` |
| `sh1106` | SH1106 | 128 × 64 | I2C, SPI | `I1` | 132-column RAM, offset 2 |

Any other MIPI DCS panel works with a custom `mipi_dcs::PanelSpec` (size, offsets, `MADCTL` per
rotation, `COLMOD`, inversion, init table).

## Touch and input

| Module | Device | Bus |
|--------|--------|-----|
| `touch::xpt2046` | XPT2046 / ADS7843 resistive | SPI |
| `touch::ft6x36`, `touch::ft5x06` | Focaltech capacitive | I2C |
| `touch::gt911` | Goodix GT911 capacitive | I2C |
| `touch::cst816s` | Hynitron CST816S capacitive | I2C |
| `touch::stmpe811` | ST STMPE811 resistive | I2C |
| `encoder` | rotary encoder (quadrature + button) | GPIO / interrupt |
| `keypad` | key matrix | GPIO |

IRQ-capable input drivers perform no bus traffic while idle.

## Tested boards

All drivers are verified on the host against recording mock buses (exact command and data
bytes, see the `testkit` feature). Hardware verification status:

| Board | Display | Touch | Status |
|-------|---------|-------|--------|
| Raspberry Pi Pico (RP2040) | ILI9341 SPI | XPT2046 | not verified yet |
| Raspberry Pi Pico 2 (RP2350) | ILI9341 SPI | XPT2046 | not verified yet |
| WeAct BlackPill (STM32F411) | ILI9341 SPI | XPT2046 | not verified yet |
| ESP32-C3-DevKitM-1 | ILI9341 SPI | XPT2046 | not verified yet |
| ESP32-S3-DevKitC-1 | ILI9341 SPI | XPT2046 | not verified yet |

## Init sequences

Init tables come from the controllers' datasheets and well-known open-source drivers, cited in
each module: Adafruit_ILI9341 and Adafruit-ST7735-Library (BSD), mipidsi (MIT/Apache-2.0), the
`ssd1306` and `sh1106` crates (MIT/Apache-2.0) and ST's STM32Cube BSP (BSD-3-Clause). See
`THIRD_PARTY.md` in the repository root.
