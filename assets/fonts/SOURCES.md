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
| `DejaVuSans.ttf` | https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2 (`ttf/DejaVuSans.ttf`; archive SHA-256 `fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7`) | 2.37 | `7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954` | Bitstream Vera / DejaVu (permissive; `LICENSE-DejaVu.txt`) |
| `LICENSE-DejaVu.txt` | same archive (`LICENSE`) | 2.37 | `7a083b136e64d064794c3419751e5c7dd10d2f64c108fe5ba161eae5e5958a93` | — |
| `SourceHanSansCN-Regular.otf` | https://github.com/adobe-fonts/source-han-sans/raw/release/SubsetOTF/CN/SourceHanSansCN-Regular.otf | branch `release` at `a4f7cf94edfb9d7ffbdfc4841de276358bd7e0f2` (2.005) | `e2bc8a2e7f37474b774fff8db758681ece40bb6947a90d571bce9dd60671a8e4` | SIL OFL 1.1 (`LICENSE-SourceHanSans.txt`) |
| `LICENSE-SourceHanSans.txt` | https://raw.githubusercontent.com/adobe-fonts/source-han-sans/release/LICENSE.txt | same | `fcac737e761ec63dbfbdce11030a1780161920d80315edba9c8beff1c2bac5a2` | — |
| `cjk_common_lvgl.txt` | the `--symbols` list of LVGL's `src/font/lv_font_source_han_sans_sc_16_cjk.c` (lvgl/lvgl `master` at `8781d6b7dfaf4e92970bb2d13b529e65d4670ce3`), ASCII removed, sorted, 40 per line | — | — | MIT (LVGL) |
| `unscii-16.ttf` | http://viznut.fi/unscii/unscii-16.ttf | Unscii 2 (download of 2026-09-23) | `0c889d6026f3bfe6cfa0e73833595678a548ed08dabeb1d71c5ad0fa9b309529` | Public domain |

Notes:
- Font Awesome's USB and Bluetooth icons are brand icons, hence `fa-brands-400.ttf`.
- LVGL's `LV_SYMBOL_NEW_LINE` (U+F8A2) is not part of Font Awesome Free 5; the generator draws it
  from "level-down-alt" (U+F3BE) turned 90° clockwise.
- Only `unscii-8.ttf` and `unscii-16.ttf` are used; the `unscii-16-full` variant (which has
  Unifont-derived glyphs under a different license) is not.
- CJK character list: the "通用规范汉字表" level-1 list (3 500 characters) has no clear license
  for redistribution as a data file, so the ~1 300 characters of LVGL's own CJK fonts (MIT) are
  used instead (Chinese and Japanese, including traditional forms). The generator adds
  Hiragana/Katakana (U+3040–U+30FF), CJK punctuation and full-width forms by range.
- Source Han Sans: the region subset `SourceHanSansCN-Regular.otf` (8 MB) is used instead of the
  16 MB full `SourceHanSansSC-Regular.otf`; it contains every character of the list.
- Only the characters of the listed ranges that a font actually contains are generated
  (unassigned code points of the Hebrew/Arabic blocks are skipped with a warning).
