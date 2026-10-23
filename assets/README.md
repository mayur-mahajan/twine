# Assets

Source fonts and images (inputs to `twine-cli`). Every file has its license and source URL + SHA-256 recorded in `SOURCES.md` of its folder.

`readme/` holds the screenshots and recordings shown in the top-level `README.md`. They are
Twine's own output (no third-party content), captured from headless simulator runs of the
examples (`TWINE_SIM_HEADLESS=1` with a `.twinescript` that takes a `shot` every frame) and
assembled into GIFs with ImageMagick (`magick -delay 7 -loop 0 f*.png -layers Optimize`).

`readme/showcase.gif` replays `crates/twine/examples/scripts/showcase.twinescript` up to its
`README GIF ends here` comment, from 3.5 s of simulated time, with a `shot` every fifth frame
(80 ms), assembled with `-delay 8`.
