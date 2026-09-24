# twine-embedded-graphics

[embedded-graphics](https://docs.rs/embedded-graphics) adapter for the Twine GUI library.

- `EgDisplay<T>`: any `DrawTarget` (an existing embedded-graphics display driver, the
  embedded-graphics simulator, a framebuffer) as a blocking twine `DisplayDriver`.
- `PainterTarget` (feature `drawtarget`): draw embedded-graphics primitives, fonts and images
  with a twine `Painter`, e.g. inside a canvas: `painter.as_draw_target::<Rgb565>()`.

| embedded-graphics colour | twine format |
|--------------------------|--------------|
| `Rgb565` | `Rgb565` |
| `Rgb888` | `Rgb888` |
| `Gray8` | `L8` |
| `BinaryColor` | `I1` (flush areas aligned to 8 pixels) |

Flushes go through `DrawTarget::fill_contiguous`, so drivers that implement it efficiently
(one address window + one pixel stream) are fast; drivers that only implement `draw_iter` work
but are slower. For SPI panels with a native twine driver, prefer the native driver (it sends
byte-swapped RGB565 without per-pixel conversion and supports DMA).
