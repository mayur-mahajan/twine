//! [`MockRegs`]: a recording DMA2D register file for host tests (feature `mock`).

use alloc::vec::Vec;

use crate::{Dma2dRegs, Reg, bits};

/// One access recorded by [`MockRegs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    /// A register write.
    Write(Reg, u32),
    /// A D-cache clean of `(addr, len)`.
    Clean(usize, usize),
    /// A D-cache clean + invalidate of `(addr, len)`.
    CleanInvalidate(usize, usize),
}

/// A DMA2D register file in RAM that records every write and simulates completion.
///
/// Writing `CR` with `START` set starts a "transfer" that stays busy for
/// [`with_busy_reads`](Self::with_busy_reads) reads of `CR` (default 0: finishes immediately),
/// then clears `START` and sets `ISR.TCIF` (plus any flags injected with
/// [`fail_next_transfer`](Self::fail_next_transfer)). A CLUT load (`FGPFCCR.START`) finishes
/// immediately. Writing `IFCR` clears the corresponding `ISR` flags. No pixels are moved.
///
/// ```
/// use twine_accel_stm32::{Dma2dRegs, Reg, bits, mock::{Access, MockRegs}};
///
/// let mut r = MockRegs::new();
/// r.write(Reg::Cr, bits::CR_START);
/// assert_eq!(r.read(Reg::Cr) & bits::CR_START, 0);
/// assert_eq!(r.read(Reg::Isr), bits::ISR_TCIF);
/// assert_eq!(r.log(), &[Access::Write(Reg::Cr, bits::CR_START)]);
/// ```
#[derive(Clone, Debug, Default)]
pub struct MockRegs {
    regs: [u32; 18],
    log: Vec<Access>,
    busy_reads: u32,
    remaining: u32,
    inject: u32,
    starts: u32,
    cr_reads: u32,
}

impl MockRegs {
    /// An idle register file with all registers zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every transfer stays busy for `n` reads of `CR`.
    #[must_use]
    pub fn with_busy_reads(mut self, n: u32) -> Self {
        self.busy_reads = n;
        self
    }

    /// The next transfer ends with `flags` (e.g. [`bits::ISR_CEIF`]) set in `ISR`.
    pub fn fail_next_transfer(&mut self, flags: u32) {
        self.inject = flags;
    }

    /// Every access so far, in order.
    #[must_use]
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    /// Forgets the recorded accesses.
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Current value of `reg` (no side effects).
    #[must_use]
    pub fn get(&self, reg: Reg) -> u32 {
        self.regs[reg.index()]
    }

    /// The last value written to `reg`, if any.
    #[must_use]
    pub fn last_write(&self, reg: Reg) -> Option<u32> {
        self.log.iter().rev().find_map(|a| match *a {
            Access::Write(r, v) if r == reg => Some(v),
            _ => None,
        })
    }

    /// Number of transfers started (`CR` writes with `START`).
    #[must_use]
    pub fn starts(&self) -> u32 {
        self.starts
    }

    /// Number of `CR` reads (polls) so far.
    #[must_use]
    pub fn cr_reads(&self) -> u32 {
        self.cr_reads
    }

    /// Whether a transfer is in progress.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.regs[Reg::Cr.index()] & bits::CR_START != 0
    }
}

impl Dma2dRegs for MockRegs {
    fn read(&mut self, reg: Reg) -> u32 {
        if reg == Reg::Cr {
            self.cr_reads += 1;
            if self.is_running() {
                if self.remaining > 0 {
                    self.remaining -= 1;
                } else {
                    self.regs[Reg::Cr.index()] &= !bits::CR_START;
                    self.regs[Reg::Isr.index()] |= bits::ISR_TCIF | core::mem::take(&mut self.inject);
                }
            }
        }
        self.regs[reg.index()]
    }

    fn write(&mut self, reg: Reg, value: u32) {
        self.log.push(Access::Write(reg, value));
        match reg {
            Reg::Isr => {}
            Reg::Ifcr => self.regs[Reg::Isr.index()] &= !value,
            Reg::Cr => {
                if value & bits::CR_START != 0 {
                    self.starts += 1;
                    self.remaining = self.busy_reads;
                }
                self.regs[Reg::Cr.index()] = value;
            }
            Reg::Fgpfccr | Reg::Bgpfccr if value & bits::PFCCR_START != 0 => {
                self.regs[reg.index()] = value & !bits::PFCCR_START;
                self.regs[Reg::Isr.index()] |= bits::ISR_CTCIF;
            }
            _ => self.regs[reg.index()] = value,
        }
    }

    fn clean_dcache(&mut self, addr: usize, len: usize) {
        self.log.push(Access::Clean(addr, len));
    }

    fn clean_invalidate_dcache(&mut self, addr: usize, len: usize) {
        self.log.push(Access::CleanInvalidate(addr, len));
    }
}
