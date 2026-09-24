# Fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the image decoders and
decompressors of `twine-image`, the SVG parser of `twine-vector` and the Lottie loader of
`twine-lottie`. Every target feeds arbitrary bytes to a decoder, which must return an error
instead of panicking.

| Target | What it exercises |
|--------|-------------------|
| `fuzz_qoi` | QOI decoder (probe + decode) |
| `fuzz_png` | PNG decoder |
| `fuzz_jpeg` | JPEG decoder |
| `fuzz_bmp` | BMP decoder |
| `fuzz_gif` | GIF decoder and `GifPlayer` (all frames, twice) |
| `fuzz_rle` | LVGL RLE decompression (block size and output size from the first byte) |
| `fuzz_lz4` | LZ4 block decompression |
| `svg_parse` | SVG parser (`parse_svg`); documents that parse are rendered into a 32 × 32 buffer |
| `lottie_load` | Lottie loader (`load`); compositions that load are rendered (3 frames) into a 32 × 32 buffer |

This directory is its own Cargo workspace (fuzzing needs nightly Rust and sanitizers).

```sh
cargo install cargo-fuzz
cargo xtask fuzz fuzz_png --time 60     # or: cd fuzz && cargo +nightly fuzz run fuzz_png
```

Crashes are written to `fuzz/artifacts/<target>/`; add a regression test for each one.
