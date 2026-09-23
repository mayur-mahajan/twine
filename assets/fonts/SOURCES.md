# Font sources

Inputs of `cargo xtask fonts` (see `fonts.toml`). Hashes verified with `shasum -a 256`.

| File | Source URL | Version / tag | SHA-256 | License |
|------|------------|---------------|---------|---------|
| `Montserrat-Medium.ttf` | https://raw.githubusercontent.com/JulietaUla/Montserrat/v9.000/fonts/ttf/Montserrat-Medium.ttf | v9.000 | `c888ea70fdceef0d05752266472a940aeb86692a7bdf8f2211380587418ca223` | SIL OFL 1.1 (`LICENSE-Montserrat.txt`) |
| `LICENSE-Montserrat.txt` | https://raw.githubusercontent.com/JulietaUla/Montserrat/v9.000/OFL.txt | v9.000 | `8b7141c03fa4f8d44e6345d5d4931709290f0f67875e452e95ac1fd3a027802e` | — |
| `fa-solid-900.ttf` | https://raw.githubusercontent.com/FortAwesome/Font-Awesome/5.15.4/webfonts/fa-solid-900.ttf | Font Awesome Free 5.15.4 | `f9d6933d04c59a42aca30bd88eec38bb9cbeb69b1547fd550ef73eba0bce7a1a` | SIL OFL 1.1 (fonts; `LICENSE-FontAwesome.txt`) |
| `fa-brands-400.ttf` | https://raw.githubusercontent.com/FortAwesome/Font-Awesome/5.15.4/webfonts/fa-brands-400.ttf | Font Awesome Free 5.15.4 | `e4e76807a21a2ac963e707ddffb3623283618c04345724b26bdc23d0dafdfde6` | SIL OFL 1.1 (fonts; `LICENSE-FontAwesome.txt`) |
| `LICENSE-FontAwesome.txt` | https://raw.githubusercontent.com/FortAwesome/Font-Awesome/5.15.4/LICENSE.txt | 5.15.4 | `e779748dfe75e84f974df3c7bc07f842011a100159158b0f1f49b2f2a5a515cb` | — |
| `unscii-8.ttf` | http://viznut.fi/unscii/unscii-8.ttf | Unscii 2 (download of 2026-09-23) | `97a4eea8cfede2b57b3ce0f3fa111335edde58bc8b07b8670737351468a2c587` | Public domain |
| `unscii-16.ttf` | http://viznut.fi/unscii/unscii-16.ttf | Unscii 2 (download of 2026-09-23) | `0c889d6026f3bfe6cfa0e73833595678a548ed08dabeb1d71c5ad0fa9b309529` | Public domain |

Notes:
- Font Awesome's USB and Bluetooth icons are brand icons, hence `fa-brands-400.ttf`.
- LVGL's `LV_SYMBOL_NEW_LINE` (U+F8A2) is not part of Font Awesome Free 5; the generator draws it
  from "level-down-alt" (U+F3BE) turned 90° clockwise.
- Only `unscii-8.ttf` and `unscii-16.ttf` are used; the `unscii-16-full` variant (which has
  Unifont-derived glyphs under a different license) is not.
