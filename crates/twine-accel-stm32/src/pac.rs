//! [`PacRegs`]: the DMA2D of the selected chip through `stm32-metapac`.

use core::sync::atomic::{AtomicBool, Ordering};

use stm32_metapac::dma2d::regs;

use crate::{Dma2dRegs, Reg};

/// Set while a [`PacRegs`] exists: the chip has one DMA2D.
static CLAIMED: AtomicBool = AtomicBool::new(false);

/// Releases [`CLAIMED`] when the [`PacRegs`] holding it is dropped (or released).
struct Claim;

impl Drop for Claim {
    fn drop(&mut self) {
        CLAIMED.store(false, Ordering::Release);
    }
}

/// The chip's DMA2D peripheral, accessed through the `stm32-metapac` register definitions.
/// Available with a chip feature (`stm32f429zi`, `stm32f746ng`, `stm32h743zi`).
///
/// **Owns the peripheral.** [`new`](Self::new) takes the HAL's DMA2D handle `P` by value —
/// embassy-stm32's `Peri<'static, DMA2D>` (`p.DMA2D`), or the PAC singleton of another HAL —
/// so nothing else can use the peripheral while the `PacRegs` (and the `Dma2d` or `Ui` owning
/// it) lives, and [`release`](Self::release) hands the handle back. A HAL hands its singleton
/// out once; in addition `new` refuses a second `PacRegs` while one exists (whatever `P` is),
/// so two register owners of the one DMA2D cannot exist.
///
/// The DMA2D clock must be enabled before use (`RCC_AHB1ENR.DMA2DEN` on F4/F7,
/// `RCC_AHB3ENR.DMA2DEN` on H7); the HAL usually does this, e.g. `embassy_stm32::rcc::enable_and_reset::<DMA2D>()`.
///
/// On Cortex-M7 parts (feature `dcache`, implied by the F7/H7 chip features), pass the core's
/// `SCB` with `with_scb` so that source memory is cleaned and destination
/// memory cleaned + invalidated around each transfer. Without it the D-cache must be disabled or
/// the buffers placed in non-cacheable memory (e.g. DTCM or an MPU region).
///
/// ```ignore
/// let p = embassy_stm32::init(config);
/// let regs = PacRegs::new(p.DMA2D).expect("one DMA2D owner");
/// let ui = Ui::builder_fb(display).accel(Dma2d::new(regs).with_timeout(1_000_000)) /* … */;
/// ```
pub struct PacRegs<P> {
    peripheral: P,
    _claim: Claim,
    #[cfg(feature = "dcache")]
    scb: Option<cortex_m::peripheral::SCB>,
}

impl<P> PacRegs<P> {
    /// Takes ownership of the chip's DMA2D through its HAL handle `peripheral` (e.g.
    /// embassy-stm32's `p.DMA2D`). Returns `Err(peripheral)` while another `PacRegs` exists
    /// (the chip has one DMA2D). Touches no register. Never panics.
    ///
    /// ```
    /// use twine_accel_stm32::PacRegs;
    ///
    /// // On a board: the HAL's handle, e.g. embassy-stm32's `p.DMA2D`. Any owned value works
    /// // as the handle here, since `new` touches no register.
    /// struct Dma2dHandle;
    /// let regs = PacRegs::new(Dma2dHandle).ok().expect("first owner");
    /// assert!(PacRegs::new(Dma2dHandle).is_err()); // one owner of the one DMA2D
    /// # drop(regs);
    /// ```
    pub fn new(peripheral: P) -> Result<Self, P> {
        if CLAIMED.swap(true, Ordering::Acquire) {
            twine_core::warn!(target: "twine::accel", "dma2d: PacRegs already exists (one owner per peripheral)");
            return Err(peripheral);
        }
        Ok(Self {
            peripheral,
            _claim: Claim,
            #[cfg(feature = "dcache")]
            scb: None,
        })
    }

    /// Gives the DMA2D handle back (a new `PacRegs` can then be created). An `SCB` passed to
    /// `with_scb` is dropped. Never panics.
    ///
    /// ```
    /// use twine_accel_stm32::PacRegs;
    ///
    /// struct Dma2dHandle; // the HAL's handle on a board (see `new`)
    /// let regs = PacRegs::new(Dma2dHandle).ok().expect("first owner");
    /// let handle: Dma2dHandle = regs.release(); // e.g. to give the DMA2D to other code
    /// assert!(PacRegs::new(handle).is_ok()); // released: a new owner may take it
    /// ```
    #[must_use]
    pub fn release(self) -> P {
        let Self { peripheral, .. } = self;
        peripheral
    }

    /// Performs D-cache maintenance through `scb` (Cortex-M7 with the D-cache enabled).
    #[cfg(feature = "dcache")]
    #[must_use]
    pub fn with_scb(mut self, scb: cortex_m::peripheral::SCB) -> Self {
        self.scb = Some(scb);
        self
    }
}

impl<P> core::fmt::Debug for PacRegs<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("PacRegs");
        #[cfg(feature = "dcache")]
        d.field("dcache", &self.scb.is_some());
        d.finish_non_exhaustive()
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

impl<P> Dma2dRegs for PacRegs<P> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A HAL's DMA2D handle: not `Copy`, so it moves into the one `PacRegs`.
    #[derive(Debug, PartialEq)]
    struct Dma2dHandle(u8);

    #[test]
    fn one_owner_per_peripheral() {
        // One test: `CLAIMED` is process-wide.
        let regs = PacRegs::new(Dma2dHandle(1)).expect("first owner");
        // A second owner is refused (even with another handle) and gets its handle back.
        assert_eq!(PacRegs::new(Dma2dHandle(2)).unwrap_err(), Dma2dHandle(2));
        // Releasing returns the handle and frees the peripheral.
        let handle = regs.release();
        assert_eq!(handle, Dma2dHandle(1));
        let regs = PacRegs::new(handle).expect("free again after release");
        assert!(alloc::format!("{regs:?}").starts_with("PacRegs"));
        // Dropping frees it as well.
        drop(regs);
        assert!(PacRegs::new(()).is_ok());
    }
}
