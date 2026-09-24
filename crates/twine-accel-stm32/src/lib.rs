//! STM32 DMA2D (Chrom-ART) draw acceleration for the Twine renderer.
//!
//! [`Dma2d`] implements `twine_render::DrawAccel`: attach it to a `Painter` (or to the engine's
//! configuration) and large fills, image blits and glyph blending run on the DMA2D of
//! STM32F4/F7/H7 parts while the software renderer handles everything else. The software path
//! stays complete; whatever DMA2D cannot do returns `AccelResult::Unsupported` and is drawn in
//! software.
//!
//! # Layers
//!
//! - [`Dma2d`] holds all register programming and is generic over [`Dma2dRegs`], a 32-bit
//!   register read/write interface.
//! - `PacRegs` (chip features `stm32f429zi`, `stm32f746ng`, `stm32h743zi`) implements
//!   [`Dma2dRegs`] with the `stm32-metapac` register definitions of the chip, and (Cortex-M7,
//!   feature `dcache`) D-cache maintenance through the core's `SCB`.
//! - `mock::MockRegs` (feature `mock`) is a recording register file for host tests.
//!
//! ```
//! use twine_accel_stm32::{Dma2d, mock::MockRegs};
//! use twine_core::{Color, ColorFormat, Opa, Rect};
//! use twine_render::{DrawBuf, Painter, RenderCaches};
//!
//! let mut dma = Dma2d::new(MockRegs::new());
//! let mut caches = RenderCaches::default();
//! let mut px = vec![0u8; 64 * 64 * 2];
//! {
//!     let buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 64, 64)).unwrap();
//!     let mut p = Painter::new(buf, &mut caches).with_accel(&mut dma);
//!     p.fill(Rect::from_xywh(0, 0, 64, 32), Color::BLUE, Opa::COVER); // programs the DMA2D
//! } // dropping the painter waits for the transfer
//! assert_eq!(dma.regs().starts(), 1);
//! ```
//!
//! On a board (the PAC does not enforce single ownership of the peripheral):
//!
//! ```ignore
//! let mut dma = Dma2d::new(PacRegs::new());               // F4
//! let mut dma = Dma2d::new(PacRegs::new().with_scb(scb)); // F7/H7 with the D-cache on
//! ```
//!
//! # Hardware notes
//!
//! - Enable the DMA2D clock first (`RCC_AHB1ENR.DMA2DEN` on F4/F7, `RCC_AHB3ENR.DMA2DEN` on H7).
//! - Buffers must be in memory the DMA2D can reach (not the F4's CCM RAM, not the H7's DTCM).
//! - Completion is polled (`CR.START`); no interrupt is used.
//! - Blending rounds differently from the software renderer (±1 per channel).
#![no_std]

#[cfg(any(test, feature = "mock"))]
extern crate alloc;

mod dma2d;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
#[cfg(any(feature = "stm32f429zi", feature = "stm32f746ng", feature = "stm32h743zi"))]
mod pac;
mod regs;

pub use dma2d::Dma2d;
#[cfg(any(feature = "stm32f429zi", feature = "stm32f746ng", feature = "stm32h743zi"))]
pub use pac::PacRegs;
pub use regs::{Dma2dRegs, Reg, bits};
