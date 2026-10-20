# Fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the image decoders and
decompressors of `twine-image`, the SVG parser of `twine-vector` and the Lottie loader of
`twine-lottie`, and — beyond the decoders (R5.S08) — operation sequences on the widget tree, the
input pipeline and the reactive runtime. Every decoder target feeds arbitrary bytes to a decoder,
which must return an error instead of panicking; the sequence targets decode the bytes into API
calls (`twine_testing::fuzz`) and check the library's invariants after each call.

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
| `path_raster` | path building (lines, curves, arcs, circles with extreme coordinates), transforms, strokes (joins, caps, dashes) and both fill rules, rasterized into 64 × 64 whole and in two chunks (the chunks must match the whole draw) |
| `lottie_load` | Lottie loader (`load`); compositions that load are rendered (3 frames) into a 32 × 32 buffer |
| `tree_ops` | widget-tree mutations on live and stale ids (create, delete, move, flags, states, sizes, flex layout, scrolling, render steps) with small `Limits`: `Tree::check_invariants`, node and depth bounds, only `Capacity` faults |
| `input_events` | pointer/key/encoder events, waits and pointer health changes on a screen of widgets: tree invariants, live focus, only `InputDevice` faults |
| `reactive_ops` | scopes, signals, memos, effects, writes, batches and reads on live and disposed handles, checked against a model (counts, values, effect runs, `Disposed` errors, everything freed) |

The sequence targets have property-test twins that run on stable with the workspace tests
(`crates/twine-testing/tests/fuzz_tree.rs`, `fuzz_input.rs`, `fuzz_reactive.rs`).

This directory is its own Cargo workspace (fuzzing needs nightly Rust and sanitizers).

```sh
cargo install cargo-fuzz
cargo xtask fuzz fuzz_png --time 60     # or: cd fuzz && cargo +nightly fuzz run fuzz_png
```

Crashes are written to `fuzz/artifacts/<target>/`; add a regression test for each one.
