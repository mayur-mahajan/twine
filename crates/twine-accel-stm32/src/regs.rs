//! Register access: the [`Dma2dRegs`] trait, the [`Reg`] names and the bit layout ([`bits`]).
//!
//! [`Dma2d`](crate::Dma2d) never touches memory-mapped registers itself; it programs whole
//! 32-bit register values through [`Dma2dRegs`]. On hardware that is `PacRegs` (feature per chip,
//! backed by `stm32-metapac`); on the host it is `MockRegs` (feature `mock`), a recording
//! register file.

/// The DMA2D registers used by [`Dma2d`](crate::Dma2d), named as in RM0090 §9.5 (STM32F42x/43x),
/// RM0385 §9.5 (STM32F7) and RM0433 §18.6 (STM32H7). The CLUT memories and `LWR`/`AMTCR` are not
/// used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Reg {
    /// `DMA2D_CR`: control (mode, start).
    Cr,
    /// `DMA2D_ISR`: interrupt status (read only).
    Isr,
    /// `DMA2D_IFCR`: interrupt flag clear (write 1 to clear).
    Ifcr,
    /// `DMA2D_FGMAR`: foreground memory address.
    Fgmar,
    /// `DMA2D_FGOR`: foreground line offset (pixels skipped after each line).
    Fgor,
    /// `DMA2D_BGMAR`: background memory address.
    Bgmar,
    /// `DMA2D_BGOR`: background line offset.
    Bgor,
    /// `DMA2D_FGPFCCR`: foreground PFC control (color mode, alpha mode, alpha, CLUT load).
    Fgpfccr,
    /// `DMA2D_FGCOLR`: foreground color (RGB of A8/A4 foregrounds).
    Fgcolr,
    /// `DMA2D_BGPFCCR`: background PFC control.
    Bgpfccr,
    /// `DMA2D_BGCOLR`: background color.
    Bgcolr,
    /// `DMA2D_FGCMAR`: foreground CLUT memory address.
    Fgcmar,
    /// `DMA2D_BGCMAR`: background CLUT memory address.
    Bgcmar,
    /// `DMA2D_OPFCCR`: output PFC control (output color mode).
    Opfccr,
    /// `DMA2D_OCOLR`: output color (register-to-memory fills), in the output color mode.
    Ocolr,
    /// `DMA2D_OMAR`: output memory address.
    Omar,
    /// `DMA2D_OOR`: output line offset.
    Oor,
    /// `DMA2D_NLR`: number of lines (`NL`, bits 15:0) and pixels per line (`PL`, bits 29:16).
    Nlr,
}

impl Reg {
    /// Every register, in address order.
    pub const ALL: [Reg; 18] = [
        Reg::Cr,
        Reg::Isr,
        Reg::Ifcr,
        Reg::Fgmar,
        Reg::Fgor,
        Reg::Bgmar,
        Reg::Bgor,
        Reg::Fgpfccr,
        Reg::Fgcolr,
        Reg::Bgpfccr,
        Reg::Bgcolr,
        Reg::Fgcmar,
        Reg::Bgcmar,
        Reg::Opfccr,
        Reg::Ocolr,
        Reg::Omar,
        Reg::Oor,
        Reg::Nlr,
    ];

    /// Byte offset of the register from the DMA2D base address (identical on v1 and v2).
    ///
    /// ```
    /// use twine_accel_stm32::Reg;
    /// assert_eq!(Reg::Nlr.offset(), 0x44);
    /// ```
    #[must_use]
    pub const fn offset(self) -> usize {
        match self {
            Reg::Cr => 0x00,
            Reg::Isr => 0x04,
            Reg::Ifcr => 0x08,
            Reg::Fgmar => 0x0C,
            Reg::Fgor => 0x10,
            Reg::Bgmar => 0x14,
            Reg::Bgor => 0x18,
            Reg::Fgpfccr => 0x1C,
            Reg::Fgcolr => 0x20,
            Reg::Bgpfccr => 0x24,
            Reg::Bgcolr => 0x28,
            Reg::Fgcmar => 0x2C,
            Reg::Bgcmar => 0x30,
            Reg::Opfccr => 0x34,
            Reg::Ocolr => 0x38,
            Reg::Omar => 0x3C,
            Reg::Oor => 0x40,
            Reg::Nlr => 0x44,
        }
    }

    /// Index into [`Reg::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.offset() / 4
    }
}

/// Access to one DMA2D instance.
///
/// Implementations only move 32-bit values; all register programming lives in
/// [`Dma2d`](crate::Dma2d). The cache hooks default to no-ops (Cortex-M4 parts such as the
/// STM32F4 have no data cache).
///
/// ```
/// use twine_accel_stm32::{Dma2dRegs, Reg};
///
/// /// A register file in RAM (what a unit test might use).
/// struct Ram([u32; 18]);
/// impl Dma2dRegs for Ram {
///     fn read(&mut self, reg: Reg) -> u32 { self.0[reg.index()] }
///     fn write(&mut self, reg: Reg, value: u32) { self.0[reg.index()] = value & !1 } // START self-clears
/// }
/// let mut r = Ram([0; 18]);
/// r.write(Reg::Nlr, 0x0010_0020);
/// assert_eq!(r.read(Reg::Nlr), 0x0010_0020);
/// ```
pub trait Dma2dRegs {
    /// Reads `reg`.
    fn read(&mut self, reg: Reg) -> u32;
    /// Writes `value` to `reg`.
    fn write(&mut self, reg: Reg, value: u32);
    /// Writes dirty D-cache lines covering `[addr, addr + len)` back to memory (before the DMA2D
    /// reads memory the CPU wrote). Default: no-op.
    fn clean_dcache(&mut self, addr: usize, len: usize) {
        let _ = (addr, len);
    }
    /// Cleans and invalidates D-cache lines covering `[addr, addr + len)` (before and after the
    /// DMA2D writes memory the CPU reads). Default: no-op.
    fn clean_invalidate_dcache(&mut self, addr: usize, len: usize) {
        let _ = (addr, len);
    }
}

impl<R: Dma2dRegs + ?Sized> Dma2dRegs for &mut R {
    fn read(&mut self, reg: Reg) -> u32 {
        (**self).read(reg)
    }
    fn write(&mut self, reg: Reg, value: u32) {
        (**self).write(reg, value);
    }
    fn clean_dcache(&mut self, addr: usize, len: usize) {
        (**self).clean_dcache(addr, len);
    }
    fn clean_invalidate_dcache(&mut self, addr: usize, len: usize) {
        (**self).clean_invalidate_dcache(addr, len);
    }
}

/// Register bit fields (RM0090 §9.5; v2 on the STM32H7 only adds bits that are not used here).
/// Cross-checked against the `stm32-metapac` field definitions in this crate's tests.
pub mod bits {
    /// `CR.START` (bit 0): starts the transfer; hardware clears it when the transfer ends.
    pub const CR_START: u32 = 1 << 0;
    /// `CR.MODE` position (bits 17:16 on v1, 18:16 on v2).
    pub const CR_MODE_SHIFT: u32 = 16;
    /// `CR.MODE = 00`: memory-to-memory (FG fetch only, no PFC).
    pub const MODE_M2M: u32 = 0b00;
    /// `CR.MODE = 01`: memory-to-memory with pixel format conversion.
    pub const MODE_M2M_PFC: u32 = 0b01;
    /// `CR.MODE = 10`: memory-to-memory with PFC and blending (FG over BG).
    pub const MODE_M2M_BLEND: u32 = 0b10;
    /// `CR.MODE = 11`: register-to-memory (fill with `OCOLR`).
    pub const MODE_R2M: u32 = 0b11;

    /// `ISR.TEIF` (bit 0): transfer error.
    pub const ISR_TEIF: u32 = 1 << 0;
    /// `ISR.TCIF` (bit 1): transfer complete.
    pub const ISR_TCIF: u32 = 1 << 1;
    /// `ISR.CAEIF` (bit 3): CLUT access error.
    pub const ISR_CAEIF: u32 = 1 << 3;
    /// `ISR.CTCIF` (bit 4): CLUT transfer complete.
    pub const ISR_CTCIF: u32 = 1 << 4;
    /// `ISR.CEIF` (bit 5): configuration error.
    pub const ISR_CEIF: u32 = 1 << 5;
    /// Error flags of `ISR`.
    pub const ISR_ERRORS: u32 = ISR_TEIF | ISR_CAEIF | ISR_CEIF;
    /// `IFCR`: `CTEIF | CTCIF | CTWIF | CAECIF | CCTCIF | CCEIF` (bits 5:0).
    pub const IFCR_ALL: u32 = 0x3F;

    /// `xPFCCR.CM` / `OPFCCR.CM` = ARGB8888.
    pub const CM_ARGB8888: u32 = 0x0;
    /// Color mode RGB888.
    pub const CM_RGB888: u32 = 0x1;
    /// Color mode RGB565.
    pub const CM_RGB565: u32 = 0x2;
    /// Color mode L8 (8-bit index into the CLUT).
    pub const CM_L8: u32 = 0x5;
    /// Color mode A8 (8-bit alpha, RGB from `xCOLR`).
    pub const CM_A8: u32 = 0x9;
    /// `xPFCCR.CCM` (bit 4): CLUT color mode (0 = ARGB8888).
    pub const PFCCR_CCM_RGB888: u32 = 1 << 4;
    /// `xPFCCR.START` (bit 5): starts loading the CLUT; hardware clears it when done.
    pub const PFCCR_START: u32 = 1 << 5;
    /// `xPFCCR.CS` position (bits 15:8): CLUT size − 1.
    pub const PFCCR_CS_SHIFT: u32 = 8;
    /// `xPFCCR.AM` position (bits 17:16): alpha mode.
    pub const PFCCR_AM_SHIFT: u32 = 16;
    /// `AM = 00`: keep the pixel's alpha.
    pub const AM_NO_MODIFY: u32 = 0b00;
    /// `AM = 01`: replace the pixel's alpha with `ALPHA`.
    pub const AM_REPLACE: u32 = 0b01;
    /// `AM = 10`: multiply the pixel's alpha by `ALPHA / 255`.
    pub const AM_MULTIPLY: u32 = 0b10;
    /// `xPFCCR.ALPHA` position (bits 31:24).
    pub const PFCCR_ALPHA_SHIFT: u32 = 24;

    /// `NLR.PL` position (bits 29:16): pixels per line.
    pub const NLR_PL_SHIFT: u32 = 16;
    /// Largest `NLR.PL` (14 bits).
    pub const MAX_PL: u32 = 0x3FFF;
    /// Largest `NLR.NL` (16 bits).
    pub const MAX_NL: u32 = 0xFFFF;
    /// Largest line offset in `FGOR`/`BGOR`/`OOR` (14 bits on v1; v2 has 16, the smaller limit
    /// is used everywhere).
    pub const MAX_OFFSET: u32 = 0x3FFF;
}

/// The bit layout above against the chip's `stm32-metapac` field definitions (run with a chip
/// feature, e.g. `cargo test -p twine-accel-stm32 --features stm32f429zi`).
#[cfg(all(
    test,
    any(feature = "stm32f429zi", feature = "stm32f746ng", feature = "stm32h743zi")
))]
mod pac_crosscheck {
    use stm32_metapac::dma2d::{regs, vals};

    use super::Reg;
    use super::bits::*;

    #[test]
    fn offsets_match_pac() {
        let d = stm32_metapac::DMA2D;
        let base = d.as_ptr() as usize;
        let at = |p: *mut u32| p as usize - base;
        let pairs = [
            (Reg::Cr, at(d.cr().as_ptr().cast())),
            (Reg::Isr, at(d.isr().as_ptr().cast())),
            (Reg::Ifcr, at(d.ifcr().as_ptr().cast())),
            (Reg::Fgmar, at(d.fgmar().as_ptr().cast())),
            (Reg::Fgor, at(d.fgor().as_ptr().cast())),
            (Reg::Bgmar, at(d.bgmar().as_ptr().cast())),
            (Reg::Bgor, at(d.bgor().as_ptr().cast())),
            (Reg::Fgpfccr, at(d.fgpfccr().as_ptr().cast())),
            (Reg::Fgcolr, at(d.fgcolr().as_ptr().cast())),
            (Reg::Bgpfccr, at(d.bgpfccr().as_ptr().cast())),
            (Reg::Bgcolr, at(d.bgcolr().as_ptr().cast())),
            (Reg::Fgcmar, at(d.fgcmar().as_ptr().cast())),
            (Reg::Bgcmar, at(d.bgcmar().as_ptr().cast())),
            (Reg::Opfccr, at(d.opfccr().as_ptr().cast())),
            (Reg::Ocolr, at(d.ocolr().as_ptr().cast())),
            (Reg::Omar, at(d.omar().as_ptr().cast())),
            (Reg::Oor, at(d.oor().as_ptr().cast())),
            (Reg::Nlr, at(d.nlr().as_ptr().cast())),
        ];
        for (reg, off) in pairs {
            assert_eq!(reg.offset(), off, "{reg:?}");
        }
    }

    #[test]
    fn fields_match_pac() {
        let mut cr = regs::Cr(0);
        cr.set_start(vals::CrStart::START);
        cr.set_mode(vals::Mode::REGISTER_TO_MEMORY);
        assert_eq!(cr.0, (MODE_R2M << CR_MODE_SHIFT) | CR_START);
        for (m, v) in [
            (vals::Mode::MEMORY_TO_MEMORY, MODE_M2M),
            (vals::Mode::MEMORY_TO_MEMORY_PFC, MODE_M2M_PFC),
            (vals::Mode::MEMORY_TO_MEMORY_PFCBLENDING, MODE_M2M_BLEND),
        ] {
            let mut cr = regs::Cr(0);
            cr.set_mode(m);
            assert_eq!(cr.0, v << CR_MODE_SHIFT);
        }

        let mut fg = regs::Fgpfccr(0);
        fg.set_cm(vals::FgpfccrCm::L8);
        fg.set_cs(255);
        fg.set_start(vals::FgpfccrStart::START);
        fg.set_am(vals::FgpfccrAm::MULTIPLY);
        fg.set_alpha(0x80);
        assert_eq!(
            fg.0,
            CM_L8
                | (255 << PFCCR_CS_SHIFT)
                | PFCCR_START
                | (AM_MULTIPLY << PFCCR_AM_SHIFT)
                | (0x80 << PFCCR_ALPHA_SHIFT)
        );
        for (cm, v) in [
            (vals::FgpfccrCm::ARGB8888, CM_ARGB8888),
            (vals::FgpfccrCm::RGB888, CM_RGB888),
            (vals::FgpfccrCm::RGB565, CM_RGB565),
            (vals::FgpfccrCm::A8, CM_A8),
        ] {
            let mut fg = regs::Fgpfccr(0);
            fg.set_cm(cm);
            assert_eq!(fg.0, v);
        }
        let mut fg = regs::Fgpfccr(0);
        fg.set_am(vals::FgpfccrAm::REPLACE);
        assert_eq!(fg.0, AM_REPLACE << PFCCR_AM_SHIFT);

        let mut bg = regs::Bgpfccr(0);
        bg.set_cm(vals::BgpfccrCm::RGB565);
        bg.set_am(vals::BgpfccrAm::REPLACE);
        bg.set_alpha(0xFF);
        assert_eq!(
            bg.0,
            CM_RGB565 | (AM_REPLACE << PFCCR_AM_SHIFT) | (0xFF << PFCCR_ALPHA_SHIFT)
        );

        let mut out = regs::Opfccr(0);
        out.set_cm(vals::OpfccrCm::RGB888);
        assert_eq!(out.0, CM_RGB888);

        let mut nlr = regs::Nlr(0);
        nlr.set_pl(MAX_PL as u16);
        nlr.set_nl(MAX_NL as u16);
        assert_eq!(nlr.0, (MAX_PL << NLR_PL_SHIFT) | MAX_NL);
        let mut nlr = regs::Nlr(0);
        nlr.set_pl(u16::MAX);
        assert_eq!(nlr.0 >> NLR_PL_SHIFT, MAX_PL, "PL is 14 bits");

        let mut oor = regs::Oor(0);
        oor.set_lo(MAX_OFFSET as u16);
        assert_eq!(oor.0, MAX_OFFSET);

        let mut col = regs::Fgcolr(0);
        col.set_red(0x12);
        col.set_green(0x34);
        col.set_blue(0x56);
        assert_eq!(col.0, 0x0012_3456);

        let mut isr = regs::Isr(0);
        isr.set_teif(true);
        isr.set_caeif(true);
        isr.set_ceif(true);
        assert_eq!(isr.0, ISR_ERRORS);
        let mut isr = regs::Isr(0);
        isr.set_tcif(true);
        assert_eq!(isr.0, ISR_TCIF);
        let mut isr = regs::Isr(0);
        isr.set_ctcif(true);
        assert_eq!(isr.0, ISR_CTCIF);

        let mut ifcr = regs::Ifcr(0);
        ifcr.set_cteif(vals::Cteif::CLEAR);
        ifcr.set_ctcif(vals::Ctcif::CLEAR);
        ifcr.set_ctwif(vals::Ctwif::CLEAR);
        ifcr.set_caecif(vals::Caecif::CLEAR);
        ifcr.set_cctcif(vals::Cctcif::CLEAR);
        ifcr.set_cceif(vals::Cceif::CLEAR);
        assert_eq!(ifcr.0, IFCR_ALL);
    }
}
