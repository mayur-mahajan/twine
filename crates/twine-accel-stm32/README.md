# twine-accel-stm32

DMA2D (Chrom-ART) acceleration for the Twine renderer on STM32F4/F7/H7. `Dma2d` implements
`twine_render::DrawAccel`; whatever the DMA2D cannot do falls back to the software renderer.

| Operation | DMA2D mode |
|-----------|------------|
| opaque fill | register-to-memory (runs in the background) |
| translucent fill | memory-to-memory with blending, fixed-color foreground |
| image blit, same format | memory-to-memory |
| image blit, other format | memory-to-memory with pixel format conversion |
| image blit with alpha / opacity | memory-to-memory with blending |
| glyph (A8 coverage) | memory-to-memory with blending, `A8` foreground |

Destinations: `Rgb565`, `Rgb888`, `Argb8888`, `Xrgb8888` (not `Rgb565Swapped`: DMA2D cannot
swap bytes). Blit sources: `Argb8888`, `Xrgb8888`, `Rgb888`, `Rgb565`, `L8`, `A8`.

## Features

| Feature | Effect |
|---------|--------|
| `stm32f429zi`, `stm32f746ng`, `stm32h743zi` | `PacRegs` over the chip's DMA2D (`stm32-metapac`); enable exactly one |
| `dcache` | D-cache maintenance through the Cortex-M7 `SCB` (implied by the F7/H7 chips) |
| `mock` | `mock::MockRegs`, a recording register file for host tests |
| `log`, `defmt` | logging backend |

```rust,ignore
// Enable the DMA2D clock first (RCC_AHB1ENR.DMA2DEN on F4/F7, RCC_AHB3ENR.DMA2DEN on H7).
let mut dma = Dma2d::new(PacRegs::new());
let painter = Painter::new(buf, &mut caches).with_accel(&mut dma);
```

## Checking on a board

The register programming is tested on the host against a mock register file, and the bit layout
is cross-checked against `stm32-metapac`. What only hardware can confirm:

- a full-screen opaque fill shows the expected color (RGB565 `OCOLR` layout) and is faster than
  software;
- translucent fills, ARGB8888 image blits and anti-aliased text look identical to the software
  renderer (±1 per channel) — compare with the accelerator attached and detached;
- `L8` images appear as grays (CLUT load);
- `Dma2d::error_count()` stays 0 (no `CEIF`/`TEIF`);
- on F7/H7 with the D-cache enabled: no stale pixels or tearing artifacts when `PacRegs` has the
  `SCB` (`with_scb`), and visible corruption without it (confirms the maintenance matters).
