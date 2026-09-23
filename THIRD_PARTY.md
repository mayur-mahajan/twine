# Third-party material

| Material | Where | License |
|----------|-------|---------|
| Montserrat font (The Montserrat Project Authors) | `assets/fonts/Montserrat-Medium.ttf`, generated fonts in `crates/twine-assets/src/fonts/` | SIL Open Font License 1.1 — `assets/fonts/LICENSE-Montserrat.txt` |
| Font Awesome Free 5.15.4 fonts (Fonticons, Inc.) | `assets/fonts/fa-solid-900.ttf`, `assets/fonts/fa-brands-400.ttf`, symbol glyphs in the generated Montserrat fonts | SIL Open Font License 1.1 — `assets/fonts/LICENSE-FontAwesome.txt` |
| unscii 2 (viznut) | `assets/fonts/unscii-8.ttf`, `assets/fonts/unscii-16.ttf`, generated `unscii_*` fonts | Public domain |
| LVGL symbol code points (`LV_SYMBOL_*`, LVGL v9 `src/font/lv_symbol_def.h`) | `crates/twine-text/src/symbols.rs` | MIT (Copyright (c) 2021 LVGL Kft) |

Font and bitmap formats, the RLE glyph compression and several algorithms follow LVGL
(MIT license, https://github.com/lvgl/lvgl); the code is an independent Rust implementation.
