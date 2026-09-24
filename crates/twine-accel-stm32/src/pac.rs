//! [`PacRegs`]: the DMA2D of the selected chip through `stm32-metapac`.

use stm32_metapac::dma2d::regs;

use crate::{Dma2dRegs, Reg};

/// The chip's DMA2D peripheral (`stm32_metapac::DMA2D`), accessed through the PAC's register
/// definitions. Available with a chip feature (`stm32f429zi`, `stm32f746ng`, `stm32h743zi`).
///
/// The DMA2D clock must be enabled before use (`RCC_AHB1ENR.DMA2DEN` on F4/F7,
/// `RCC_AHB3ENR.DMA2DEN` on H7); the HAL usually does this, e.g. `embassy_stm32::rcc::enable_and_reset::<DMA2D>()`.
/// Owning a `PacRegs` means owning the peripheral: create at most one.
///
/// On Cortex-M7 parts (feature `dcache`, implied by the F7/H7 chip features), pass the core's
/// `SCB` with `with_scb` so that source memory is cleaned and destination
/// memory cleaned + invalidated around each transfer. Without it the D-cache must be disabled or
/// the buffers placed in non-cacheable memory (e.g. DTCM or an MPU region).
pub struct PacRegs {
    #[cfg(feature = "dcache")]
    scb: Option<cortex_m::peripheral::SCB>,
}

impl PacRegs {
    /// The DMA2D of the chip. The caller must not access the peripheral elsewhere while this
    /// value exists (the PAC itself does not enforce ownership).
    #[must_use]
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "dcache")]
            scb: None,
        }
    }

    /// Performs D-cache maintenance through `scb` (Cortex-M7 with the D-cache enabled).
    #[cfg(feature = "dcache")]
    #[must_use]
    pub fn with_scb(mut self, scb: cortex_m::peripheral::SCB) -> Self {
        self.scb = Some(scb);
        self
    }
}

impl core::fmt::Debug for PacRegs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("PacRegs");
        #[cfg(feature = "dcache")]
        d.field("dcache", &self.scb.is_some());
        d.finish()
    }
}

impl Default for PacRegs {
    fn default() -> Self {
        Self::new()
    }
}

/// Calls `$m!` with every read-write register as `Variant accessor` pairs.
macro_rules! rw_regs {
    ($m:ident) => {
        $m!(
            Cr cr, Ifcr ifcr, Fgmar fgmar, Fgor fgor, Bgmar bgmar, Bgor bgor, Fgpfccr fgpfccr,
            Fgcolr fgcolr, Bgpfccr bgpfccr, Bgcolr bgcolr, Fgcmar fgcmar, Bgcmar bgcmar,
            Opfccr opfccr, Ocolr ocolr, Omar omar, Oor oor, Nlr nlr
        )
    };
}

impl Dma2dRegs for PacRegs {
    fn read(&mut self, reg: Reg) -> u32 {
        let d = stm32_metapac::DMA2D;
        macro_rules! arms {
            ($($name:ident $f:ident),*) => {
                match reg {
                    Reg::Isr => d.isr().read().0,
                    $(Reg::$name => d.$f().read().0,)*
                }
            };
        }
        rw_regs!(arms)
    }

    fn write(&mut self, reg: Reg, value: u32) {
        let d = stm32_metapac::DMA2D;
        macro_rules! arms {
            ($($name:ident $f:ident),*) => {
                match reg {
                    // ISR is read only (its flags are cleared through IFCR).
                    Reg::Isr => {}
                    $(Reg::$name => d.$f().write_value(regs::$name(value)),)*
                }
            };
        }
        rw_regs!(arms);
    }

    #[cfg(feature = "dcache")]
    fn clean_dcache(&mut self, addr: usize, len: usize) {
        if let Some(scb) = self.scb.as_mut() {
            scb.clean_dcache_by_address(addr, len);
        }
    }

    #[cfg(feature = "dcache")]
    fn clean_invalidate_dcache(&mut self, addr: usize, len: usize) {
        if let Some(scb) = self.scb.as_mut() {
            scb.clean_invalidate_dcache_by_address(addr, len);
        }
    }
}
