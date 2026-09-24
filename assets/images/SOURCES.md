# Image sources

Every image in this folder is drawn procedurally by `cargo xtask gen-assets`
(`tools/twine-cli/src/image/assets.rs`) and encoded with the `image` crate. No third-party
artwork is used; the files are covered by the repository license (MIT OR Apache-2.0).

| File | Content | Encoder |
|------|---------|---------|
| `twine_logo.png`, `twine_logo.qoi` | 64 × 64 Twine logo (rounded square, gradient, two woven threads, alpha) | `image` PNG / QOI |
| `photo.png`, `photo.qoi`, `photo.bmp`, `photo.jpg` | 96 × 64 landscape (sky, sun, hills), opaque | `image` PNG / QOI / BMP (24-bit) / JPEG |
| `spinner.gif` | 48 × 48 spinner, 8 frames × 80 ms, loops forever, transparent background | `image` GIF |

`images.toml` lists the conversions `cargo xtask images` writes into `examples/src/assets/`.

Decoder tests in `crates/twine-image/tests/` generate their test images at run time (procedural
patterns encoded with the `image`, `png` and `jpeg-encoder` crates), so no image test suites are
vendored.
