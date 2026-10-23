# Assets

Source fonts and images (inputs to `twine-cli`). Every file has its license and source URL + SHA-256 recorded in `SOURCES.md` of its folder.

`logo/` holds the Twine logo, the Rope mark: three strands, red `#EC4048`, green `#22B868` and
blue `#3070F0`, woven as a 3-strand plait (with transparent gaps where one strand passes over
another) that opens on the right into three rays, on a transparent background. It is Twine's
own artwork, covered by the repository license.

- `twine-logo-color.svg` — the mark in its three colors (shown at the top of `README.md`).
- `twine-logo-black.svg`, `twine-logo-white.svg` — single-color versions (near-black `#111114`
  for light backgrounds, white for dark ones).
- `twine-logo-{color,black,white}-{512,256,128}.png` — square PNGs rendered from the same
  geometry, the mark centered.
- `twine-logo-{color,black,white}-{32,16}.png` — hand-hinted pixel versions for icon sizes
  (strands set on the pixel grid by hand), not downscaled renders.

The sample image `images/twine_logo.png` (64 × 64, used by the examples and tests) is the same
mark redrawn from its geometry by `cargo xtask gen-assets`.

`readme/` holds the screenshots and recordings shown in the top-level `README.md`. They are
Twine's own output (no third-party content), captured from headless simulator runs of the
examples (`TWINE_SIM_HEADLESS=1` with a `.twinescript` that takes a `shot` every frame) and
assembled into GIFs with ImageMagick (`magick -delay 7 -loop 0 f*.png -layers Optimize`).

`readme/showcase.gif` replays `crates/twine/examples/scripts/showcase.twinescript` up to its
`README GIF ends here` comment, from 3.5 s of simulated time, with a `shot` every fifth frame
(80 ms), assembled with `-delay 8`.
