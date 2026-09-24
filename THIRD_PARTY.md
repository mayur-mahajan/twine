# Third-party material

| Material | Where | License |
|----------|-------|---------|
| Montserrat font (The Montserrat Project Authors) | `assets/fonts/Montserrat-Medium.ttf`, generated fonts in `crates/twine-assets/src/fonts/` | SIL Open Font License 1.1 — `assets/fonts/LICENSE-Montserrat.txt` |
| Font Awesome Free 5.15.4 fonts (Fonticons, Inc.) | `assets/fonts/fa-solid-900.ttf`, `assets/fonts/fa-brands-400.ttf`, symbol glyphs in the generated Montserrat fonts | SIL Open Font License 1.1 — `assets/fonts/LICENSE-FontAwesome.txt` |
| unscii 2 (viznut) | `assets/fonts/unscii-8.ttf`, `assets/fonts/unscii-16.ttf`, generated `unscii_*` fonts | Public domain |
| LVGL symbol code points (`LV_SYMBOL_*`, LVGL v9 `src/font/lv_symbol_def.h`) | `crates/twine-text/src/symbols.rs` | MIT (Copyright (c) 2021 LVGL Kft) |
| ILI9341 init table (Adafruit_ILI9341 `initcmd`, Adafruit Industries) | `crates/twine-drivers/src/ili9341.rs` | BSD (Adafruit_ILI9341 license) |
| CO5300 and SH8601 init tables (`Arduino_CO5300` / `Arduino_SH8601` of Arduino_GFX, as shipped in Waveshare's ESP32-S3-Touch-AMOLED-2.06 / -1.8 examples) | `crates/twine-drivers/src/co5300.rs`, `sh8601.rs` | BSD (Arduino_GFX license) |
| JD9853 init table and AXS5106L touch register protocol (Waveshare ESP32-C6-Touch-LCD-1.47 demo: ESP-IDF components `esp_lcd_jd9853` and `esp_lcd_touch_axs5106`, SPDX Copyright Espressif Systems) | `crates/twine-drivers/src/jd9853.rs`, `touch/axs5106l.rs` | Apache-2.0 |
| RM67162 init table (`rm67162_cmd`, LilyGO `LilyGo-AMOLED-Series`) | `crates/twine-drivers/src/rm67162.rs` | MIT |
| ST7735 init tables and rotation mapping (Adafruit-ST7735-Library `Rcmd1`/`Rcmd3`, `setRotation`) | `crates/twine-drivers/src/st7735.rs` | BSD (Adafruit-ST7735-Library license) |
| GC9A01, ST7789/ST7796, ILI934x/ILI948x init sequences and offset method (mipidsi 0.10) | `crates/twine-drivers/src/{gc9a01,st7789,st7796,ili9342,ili9488,mipi_dcs}.rs` | MIT OR Apache-2.0 |
| SSD1306 / SH1106 init sequences (`ssd1306` 0.10 and `sh1106` 0.5 crates) | `crates/twine-drivers/src/{ssd1306,sh1106}.rs` | MIT OR Apache-2.0 |
| STMPE811 touch-screen init values (STMicroelectronics STM32Cube BSP `stmpe811.c`) | `crates/twine-drivers/src/touch/stmpe811.rs` | BSD-3-Clause |

Font and bitmap formats, the RLE glyph compression and several algorithms follow LVGL
(MIT license, https://github.com/lvgl/lvgl); the code is an independent Rust implementation.
