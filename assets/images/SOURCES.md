# Image sources

Every image in this folder is drawn procedurally by `cargo xtask gen-assets`
(`tools/twine-cli/src/image/assets.rs`) and encoded with the `image` crate. No third-party
artwork is used; the files are covered by the repository license (MIT OR Apache-2.0).

| File | Content | Encoder |
|------|---------|---------|
| `twine_logo.png`, `twine_logo.qoi` | 64 × 64 Twine logo: the Rope mark of `../logo/` (red, green and blue strands plaited, opening into three rays) redrawn from its geometry, transparent background | `image` PNG / QOI |
| `photo.png`, `photo.qoi`, `photo.bmp`, `photo.jpg` | 96 × 64 landscape (sky, sun, hills), opaque | `image` PNG / QOI / BMP (24-bit) / JPEG |
| `spinner.gif` | 48 × 48 spinner, 8 frames × 80 ms, loops forever, transparent background | `image` GIF |
| `avatars/{en,de,fr,zh,ja,he,ar,fa}.qoi` | 40 × 40 avatars of the multilang demo (colored disc, head-and-shoulders silhouette) | `image` QOI |

The SVG icons in `svg/` (`home.svg`, `star.svg`, `badge.svg`, `chart.svg`) are hand-written for
Twine (paths, gradients, strokes, dashes) and covered by the repository license.

The `image_gallery` example and the image tests convert these files while they compile
(`twine::include_image!`); nothing converted is committed.

Decoder tests in `crates/twine-image/tests/` generate their test images at run time (procedural
patterns encoded with the `image`, `png` and `jpeg-encoder` crates), so no image test suites are
vendored.
