//! [`Dma2d`]: [`DrawAccel`] over the DMA2D (Chrom-ART) register interface.
//!
//! Register programming follows RM0090 §9.3 (functional description) and §9.5 (registers);
//! register and field names in the comments are those of the reference manual.

use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};

use crate::bits::{
    AM_MULTIPLY, AM_NO_MODIFY, AM_REPLACE, CM_A8, CM_ARGB8888, CM_L8, CM_RGB565, CM_RGB888, CR_ABORT,
    CR_MODE_SHIFT, CR_START, IFCR_ALL, ISR_CAEIF, ISR_CEIF, ISR_ERRORS, MAX_NL, MAX_OFFSET, MAX_PL, MODE_M2M,
    MODE_M2M_BLEND, MODE_M2M_PFC, MODE_R2M, NLR_PL_SHIFT, PFCCR_ALPHA_SHIFT, PFCCR_AM_SHIFT, PFCCR_CS_SHIFT,
    PFCCR_START,
};
use crate::{Dma2dRegs, Reg};

/// The CLUT used for twine's 8-bit luminance (`L8`) images: DMA2D's `L8` mode is an 8-bit index
/// into a CLUT, so luminance is expressed as a gray ramp (ARGB8888 entries).
static GRAY_CLUT: [u32; 256] = gray_clut();

const fn gray_clut() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = 0xFF00_0000 | (i as u32 * 0x0001_0101);
        i += 1;
    }
    t
}

/// The output side of an operation: `DMA2D_OMAR`, `OOR`, `NLR`, `OPFCCR`.
#[derive(Clone, Copy, Debug)]
struct Out {
    /// Address of the first pixel (area origin).
    addr: usize,
    /// `OPFCCR.CM` (also `BGPFCCR.CM` when the destination is the background).
    cm: u32,
    /// The destination is `Xrgb8888`: its alpha byte is undefined and replaced by 255 when read.
    ignore_alpha: bool,
    /// Pixels per line.
    w: u32,
    /// Lines.
    h: u32,
    /// Line offset in pixels.
    offset: u32,
    /// Bytes from the first to one past the last byte written.
    len: usize,
}

/// The foreground side of an operation: `DMA2D_FGMAR`, `FGOR`, `FGPFCCR`, `FGCOLR`, `FGCMAR`.
#[derive(Clone, Copy, Debug)]
struct Fg {
    addr: usize,
    /// Bytes read from `addr` (for the D-cache clean).
    len: usize,
    offset: u32,
    cm: u32,
    am: u32,
    alpha: u8,
    /// `FGCOLR` (`0x00RRGGBB`), the RGB of `A8` pixels.
    color: u32,
    /// `(address, entries)` of an ARGB8888 CLUT to load for `L8`.
    clut: Option<(usize, u32)>,
    /// The pixels can be translucent (forces blending).
    has_alpha: bool,
}

/// DMA2D (Chrom-ART) accelerator for STM32F4/F7/H7, implementing [`DrawAccel`].
///
/// | Operation | DMA2D mode | Returns |
/// |-----------|------------|---------|
/// | `fill`, opaque | register-to-memory (`OCOLR`) | `Queued` |
/// | `fill`, translucent | memory-to-memory with blending; FG = `A8` with alpha replaced (fixed color), BG = destination | `Queued` |
/// | `blit`, same format, opaque | memory-to-memory | `Done` |
/// | `blit`, other format, opaque | memory-to-memory with PFC | `Done` |
/// | `blit`, alpha or `opa` < cover | memory-to-memory with blending, BG = destination | `Done` |
/// | `blend_a8` | memory-to-memory with blending, FG = `A8` + `FGCOLR` | `Done` |
///
/// Operations that read caller memory (`blit`, `blend_a8`) wait for the transfer to finish before
/// returning, because the borrowed source may be reused as soon as the call returns. Fills only
/// read registers, so they run in the background (`Queued`) while the CPU keeps rendering; the
/// renderer calls [`wait`](DrawAccel::wait) before touching the buffer again. Starting a new
/// operation first waits for the previous one (DMA2D has a single transfer queue).
///
/// **Destination formats**: `Rgb565`, `Rgb888`, `Argb8888`, `Xrgb8888`. `Rgb565Swapped` (SPI
/// panels) is [`Unsupported`](AccelResult::Unsupported) because DMA2D cannot swap bytes; the
/// LTDC of the STM32F429I-DISC1 uses plain `Rgb565`. **Source formats** (`blit`): `Argb8888`,
/// `Xrgb8888`, `Rgb888`, `Rgb565`, `L8` (through a gray CLUT), `A8`. Premultiplied, indexed,
/// sub-byte and planar formats fall back to software (4-bit DMA2D formats store the first pixel
/// in the low nibble, twine's in the high nibble).
///
/// Areas smaller than [`min_px`](Self::min_px) pixels (default 256) are `Unsupported`: the setup
/// cost exceeds the software path there. Results may differ from the software renderer by ±1
/// per channel (DMA2D rounds blending differently).
///
/// **Bounded waits**: [`new`](Self::new) polls `CR.START` until the hardware clears it, so a
/// hung DMA2D (bus fault, clock gated) hangs the renderer. [`with_timeout`](Self::with_timeout)
/// bounds every wait to a number of register polls; see there for what happens on a timeout.
///
/// ```
/// use twine_accel_stm32::{Dma2d, Reg, bits, mock::MockRegs};
/// use twine_core::{Color, ColorFormat, Opa, Rect};
/// use twine_render::{AccelResult, DrawAccel, DrawBuf};
///
/// let mut dma = Dma2d::new(MockRegs::new());
/// let mut px = vec![0u8; 32 * 32 * 2];
/// let mut buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 32, 32)).unwrap();
/// let r = dma.fill(&mut buf, Rect::from_xywh(0, 0, 32, 16), Color::RED, Opa::COVER);
/// assert_eq!(r, AccelResult::Queued);
/// assert_eq!(dma.regs().get(Reg::Ocolr), 0xF800);
/// dma.wait();
/// assert!(!dma.is_busy());
/// ```
#[derive(Debug)]
pub struct Dma2d<R: Dma2dRegs> {
    regs: R,
    busy: bool,
    min_px: u32,
    /// Destination range of the running transfer (invalidated from the D-cache when it ends).
    pending: (usize, usize),
    errors: u32,
    /// Maximum polls of a completion flag (`None`: unbounded).
    max_polls: Option<u32>,
    /// A wait timed out: the hardware is not used until `recover`.
    faulted: bool,
    /// Timeouts since creation.
    timeouts: u32,
    /// Timeouts not yet taken by `take_timeouts`.
    unreported: u32,
}

/// A wait for the DMA2D exceeded [`Dma2d::with_timeout`].
#[derive(Clone, Copy, Debug)]
struct TimedOut;

impl<R: Dma2dRegs> Dma2d<R> {
    /// Default [`min_px`](Self::min_px): below 256 pixels the software path is faster.
    pub const DEFAULT_MIN_PX: u32 = 256;

    /// An accelerator over `regs` (idle; the DMA2D clock must already be enabled).
    #[must_use]
    pub fn new(regs: R) -> Self {
        Self {
            regs,
            busy: false,
            min_px: Self::DEFAULT_MIN_PX,
            pending: (0, 0),
            errors: 0,
            max_polls: None,
            faulted: false,
            timeouts: 0,
            unreported: 0,
        }
    }

    /// Bounds every wait for the hardware to `max_polls` reads of the completion flag
    /// (`CR.START`, `FGPFCCR.START` for CLUT loads; at least 1). One poll is one register
    /// read, so the bound in time is `max_polls` × (read latency + loop), a few tens of
    /// nanoseconds per poll on a 180 MHz Cortex-M4: pick it well above the longest transfer
    /// (a full-screen fill), e.g. `1_000_000` polls ≈ tens of milliseconds.
    ///
    /// The bound costs nothing while the hardware works: fills still run in the background
    /// ([`Queued`](AccelResult::Queued)) and the poll loop is the same, with a counter.
    ///
    /// On a timeout the transfer is aborted (`CR.ABORT`), the timeout is counted
    /// ([`timeout_count`](Self::timeout_count), and [`DrawAccel::take_timeouts`]) and the
    /// accelerator stops using the hardware ([`is_faulted`](Self::is_faulted): every later
    /// operation is `Unsupported`, i.e. drawn in software) until [`recover`](Self::recover).
    /// What the aborted transfer should have drawn is lost — for a queued fill the timeout is
    /// only seen at the next wait, after the painter moved on. Correcting it is the caller's
    /// job: the engine checks `take_timeouts` after every chunk and, when it is non-zero,
    /// raises `FaultKind::AccelTimeout` and renders the whole chunk again in software. A
    /// caller driving a `Painter` itself must do the same.
    ///
    /// ```
    /// use twine_accel_stm32::{Dma2d, mock::MockRegs};
    /// use twine_core::{Color, ColorFormat, Opa, Rect};
    /// use twine_render::{AccelResult, DrawAccel, DrawBuf};
    ///
    /// let mut dma = Dma2d::new(MockRegs::new().hang()).with_timeout(100);
    /// let mut px = vec![0u8; 32 * 32 * 2];
    /// let mut buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, Rect::from_xywh(0, 0, 32, 32)).unwrap();
    /// let area = Rect::from_xywh(0, 0, 32, 16);
    /// // The fill is queued as usual; the transfer never ends and the wait gives up.
    /// assert_eq!(dma.fill(&mut buf, area, Color::RED, Opa::COVER), AccelResult::Queued);
    /// dma.wait();
    /// assert!(dma.is_faulted());
    /// // Everything is drawn in software now (and the fill must be redrawn by the caller).
    /// assert_eq!(dma.fill(&mut buf, area, Color::RED, Opa::COVER), AccelResult::Unsupported);
    /// assert_eq!(dma.take_timeouts(), 1);
    /// dma.recover(); // after resetting the peripheral
    /// assert!(!dma.is_faulted());
    /// ```
    #[must_use]
    pub fn with_timeout(mut self, max_polls: u32) -> Self {
        self.max_polls = Some(max_polls.max(1));
        self
    }

    /// The bound set with [`with_timeout`](Self::with_timeout) (`None`: waits are unbounded).
    #[must_use]
    pub fn timeout(&self) -> Option<u32> {
        self.max_polls
    }

    /// Whether a wait timed out: the hardware is not used (every operation is drawn in
    /// software) until [`recover`](Self::recover).
    #[must_use]
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    /// Number of waits that timed out since creation.
    #[must_use]
    pub fn timeout_count(&self) -> u32 {
        self.timeouts
    }

    /// Uses the hardware again after a timeout. Reset the peripheral first (e.g. through its
    /// `RCC` reset bit): Twine cannot tell whether a hung DMA2D works again.
    pub fn recover(&mut self) {
        if self.faulted {
            twine_core::info!(target: "twine::accel", "dma2d: recovered after {} timeouts", self.timeouts);
        }
        self.faulted = false;
    }

    /// Sets the smallest area (in pixels) handled by the hardware.
    #[must_use]
    pub fn with_min_px(mut self, min_px: u32) -> Self {
        self.min_px = min_px;
        self
    }

    /// The smallest area (in pixels) handled by the hardware.
    #[must_use]
    pub fn min_px(&self) -> u32 {
        self.min_px
    }

    /// Whether a transfer may still be running (until the next [`wait`](DrawAccel::wait)).
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Number of transfers that ended with an error flag (`TEIF`, `CAEIF`, `CEIF`). Always 0 on
    /// correctly configured hardware; each error is also logged.
    #[must_use]
    pub fn error_count(&self) -> u32 {
        self.errors
    }

    /// The register interface.
    #[must_use]
    pub fn regs(&self) -> &R {
        &self.regs
    }

    /// The register interface, mutably (waits for the running transfer first; bounded by
    /// [`with_timeout`](Self::with_timeout)).
    pub fn regs_mut(&mut self) -> &mut R {
        self.wait();
        &mut self.regs
    }

    /// Waits for the running transfer (bounded by [`with_timeout`](Self::with_timeout)) and
    /// returns the register interface.
    pub fn release(mut self) -> R {
        self.wait();
        self.regs
    }

    /// Polls `reg` until `bit` clears: at most `max_polls` reads when bounded, else until the
    /// hardware clears it. Returns whether it cleared.
    fn poll_clear(&mut self, reg: Reg, bit: u32) -> bool {
        let Some(max) = self.max_polls else {
            while self.regs.read(reg) & bit != 0 {
                core::hint::spin_loop();
            }
            return true;
        };
        for _ in 0..max {
            if self.regs.read(reg) & bit == 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    /// A wait timed out: aborts the transfer or CLUT load (RM0090 §9.3.9: `CR.ABORT`; START
    /// is reset with it), clears the flags and stops using the hardware until `recover`.
    fn abort(&mut self, what: &str) {
        let cr = self.regs.read(Reg::Cr);
        self.regs.write(Reg::Cr, cr | CR_ABORT);
        let stopped = self.poll_clear(Reg::Cr, CR_START);
        self.regs.write(Reg::Ifcr, IFCR_ALL);
        if self.busy {
            let (addr, len) = self.pending;
            self.regs.clean_invalidate_dcache(addr, len);
        }
        self.busy = false;
        self.faulted = true;
        self.timeouts = self.timeouts.saturating_add(1);
        self.unreported = self.unreported.saturating_add(1);
        twine_core::error!(
            target: "twine::accel",
            "dma2d: {} timed out after {} polls (aborted: {}); drawing in software until recover()",
            what,
            self.max_polls.unwrap_or(0),
            stopped
        );
    }

    /// Waits until `CR.START` clears, then clears all `ISR` flags. Returns the error flags, or
    /// [`TimedOut`] (transfer aborted) when the bound of `with_timeout` was exceeded.
    fn finish(&mut self) -> Result<u32, TimedOut> {
        if !self.busy {
            return Ok(0);
        }
        // RM0090 §9.3.9: START is reset by hardware when the transfer completes or on error.
        if !self.poll_clear(Reg::Cr, CR_START) {
            self.abort("transfer");
            return Err(TimedOut);
        }
        let isr = self.regs.read(Reg::Isr);
        self.regs.write(Reg::Ifcr, IFCR_ALL);
        let (addr, len) = self.pending;
        // The CPU may have speculatively loaded destination lines while the DMA2D wrote them.
        self.regs.clean_invalidate_dcache(addr, len);
        self.busy = false;
        let err = isr & ISR_ERRORS;
        if err != 0 {
            self.errors = self.errors.saturating_add(1);
            twine_core::warn!(target: "twine::accel", "dma2d: transfer error, ISR = {:#x}", isr);
        }
        Ok(err)
    }

    /// The output geometry of `area` in `dst`, or `None` when DMA2D cannot write it.
    fn output(&self, dst: &mut DrawBuf<'_>, area: Rect) -> Option<Out> {
        if self.faulted {
            return None; // drawn in software until `recover`
        }
        if area.area() < u64::from(self.min_px) || !dst.area().contains_rect(&area) || area.is_empty() {
            return None;
        }
        // OPFCCR.CM: output color mode.
        let (cm, bytes, ignore_alpha) = match dst.format() {
            ColorFormat::Argb8888 => (CM_ARGB8888, 4, false),
            ColorFormat::Xrgb8888 => (CM_ARGB8888, 4, true),
            ColorFormat::Rgb888 => (CM_RGB888, 3, false),
            ColorFormat::Rgb565 => (CM_RGB565, 2, false),
            // Rgb565Swapped: DMA2D cannot swap bytes. L8 / I1: no such output mode.
            _ => return None,
        };
        let (w, h) = (area.width() as u32, area.height() as u32);
        let stride = dst.stride();
        if stride % bytes != 0 || w > MAX_PL || h > MAX_NL {
            return None;
        }
        let offset = (stride / bytes) as u32 - w; // OOR.LO
        if offset > MAX_OFFSET {
            return None;
        }
        let d = dst.area();
        let start = (area.y0 - d.y0) as usize * stride + (area.x0 - d.x0) as usize * bytes;
        let addr = dst.data_mut().as_mut_ptr() as usize + start;
        if !aligned(addr, bytes) {
            return None;
        }
        Some(Out {
            addr,
            cm,
            ignore_alpha,
            w,
            h,
            offset,
            len: (h as usize - 1) * stride + w as usize * bytes,
        })
    }

    /// Loads the foreground CLUT (RM0090 §9.3.5) and waits for it. `false` on a CLUT error or
    /// a timeout.
    fn load_clut(&mut self, fg: &Fg, addr: usize, entries: u32) -> bool {
        self.regs.clean_dcache(addr, entries as usize * 4);
        self.regs.write(Reg::Fgcmar, addr as u32); // FGCMAR.MA
        // FGPFCCR: CM, CCM = 0 (ARGB8888 CLUT), CS = entries − 1, START.
        self.regs.write(
            Reg::Fgpfccr,
            fg.cm | ((entries - 1) << PFCCR_CS_SHIFT) | PFCCR_START,
        );
        if !self.poll_clear(Reg::Fgpfccr, PFCCR_START) {
            self.abort("CLUT load");
            return false;
        }
        let isr = self.regs.read(Reg::Isr);
        self.regs.write(Reg::Ifcr, IFCR_ALL);
        if isr & ISR_CAEIF != 0 {
            self.errors = self.errors.saturating_add(1);
            twine_core::warn!(target: "twine::accel", "dma2d: CLUT load error, ISR = {:#x}", isr);
            return false;
        }
        true
    }

    /// Programs and starts one transfer. `fg`: foreground (none for register-to-memory);
    /// `ocolr`: output color (register-to-memory only). Returns `false` if nothing was started.
    fn start(&mut self, mode: u32, out: &Out, fg: Option<&Fg>, ocolr: u32) -> bool {
        if self.finish().is_err() {
            return false;
        }
        if let Some(fg) = fg {
            self.regs.clean_dcache(fg.addr, fg.len);
            if let Some((addr, entries)) = fg.clut {
                if !self.load_clut(fg, addr, entries) {
                    return false;
                }
            }
            self.regs.write(Reg::Fgmar, fg.addr as u32); // FGMAR.MA
            self.regs.write(Reg::Fgor, fg.offset); // FGOR.LO
            // FGPFCCR: CM[3:0], CS[15:8] (kept for L8), AM[17:16], ALPHA[31:24].
            let cs = fg.clut.map_or(0, |(_, n)| (n - 1) << PFCCR_CS_SHIFT);
            self.regs.write(
                Reg::Fgpfccr,
                fg.cm | cs | (fg.am << PFCCR_AM_SHIFT) | (u32::from(fg.alpha) << PFCCR_ALPHA_SHIFT),
            );
            self.regs.write(Reg::Fgcolr, fg.color); // FGCOLR: RED[23:16] GREEN[15:8] BLUE[7:0]
        }
        if mode == MODE_M2M_BLEND {
            // The destination is the background: BGMAR = OMAR, BGOR = OOR.
            self.regs.write(Reg::Bgmar, out.addr as u32);
            self.regs.write(Reg::Bgor, out.offset);
            let am = if out.ignore_alpha {
                (AM_REPLACE << PFCCR_AM_SHIFT) | (0xFF << PFCCR_ALPHA_SHIFT)
            } else {
                AM_NO_MODIFY << PFCCR_AM_SHIFT
            };
            self.regs.write(Reg::Bgpfccr, out.cm | am); // BGPFCCR: CM, AM, ALPHA
        }
        self.regs.write(Reg::Opfccr, out.cm); // OPFCCR.CM
        if mode == MODE_R2M {
            self.regs.write(Reg::Ocolr, ocolr);
        }
        self.regs.write(Reg::Omar, out.addr as u32); // OMAR.MA
        self.regs.write(Reg::Oor, out.offset); // OOR.LO
        self.regs.write(Reg::Nlr, (out.w << NLR_PL_SHIFT) | out.h); // NLR: PL[29:16], NL[15:0]
        // Write back / drop cached destination lines before the DMA2D reads and writes them.
        self.regs.clean_invalidate_dcache(out.addr, out.len);
        self.regs.write(Reg::Ifcr, IFCR_ALL);
        self.regs.write(Reg::Cr, (mode << CR_MODE_SHIFT) | CR_START); // CR: MODE, START
        self.busy = true;
        self.pending = (out.addr, out.len);
        twine_core::trace!(target: "twine::accel", "dma2d: mode {} {}x{}", mode, out.w, out.h);
        true
    }

    /// Starts a transfer that reads caller memory and waits for it (the borrow ends on return).
    fn run_blocking(&mut self, mode: u32, out: &Out, fg: &Fg) -> AccelResult {
        if !self.start(mode, out, Some(fg), 0) {
            return AccelResult::Unsupported;
        }
        self.completed()
    }

    /// Waits for the transfer just started: `Done`, or `Unsupported` (drawn in software) after
    /// a configuration error (nothing was written) or a timeout (aborted).
    fn completed(&mut self) -> AccelResult {
        match self.finish() {
            Ok(err) if err & ISR_CEIF == 0 => AccelResult::Done,
            _ => AccelResult::Unsupported,
        }
    }
}

/// Whether `addr` suits a pixel of `bytes` bytes (DMA2D needs 32-bit aligned ARGB8888 and
/// 16-bit aligned 16-bit pixels).
fn aligned(addr: usize, bytes: usize) -> bool {
    match bytes {
        4 => addr % 4 == 0,
        2 => addr % 2 == 0,
        _ => true,
    }
}

/// `0x00RRGGBB` (the layout of `FGCOLR`, and of `OCOLR` in RGB888 mode).
fn rgb(c: Color) -> u32 {
    (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
}

/// `OCOLR` for an opaque fill in output mode `cm` (RM0090 §9.5.15: the layout follows
/// `OPFCCR.CM`).
fn ocolr(cm: u32, c: Color) -> u32 {
    match cm {
        CM_RGB565 => u32::from(c.to_rgb565()),
        CM_RGB888 => rgb(c),
        _ => 0xFF00_0000 | rgb(c),
    }
}

/// The foreground description of `src`, or `None` for formats DMA2D cannot read.
fn source(src: &ImagePixels<'_>, opa: Opa) -> Option<Fg> {
    if src.premultiplied {
        return None;
    }
    // FGPFCCR.CM, bytes per pixel, has alpha, CLUT.
    let (cm, bytes, has_alpha, clut): (u32, usize, bool, Option<(usize, u32)>) = match src.format {
        ColorFormat::Argb8888 => (CM_ARGB8888, 4, true, None),
        ColorFormat::Xrgb8888 => (CM_ARGB8888, 4, false, None),
        ColorFormat::Rgb888 => (CM_RGB888, 3, false, None),
        ColorFormat::Rgb565 => (CM_RGB565, 2, false, None),
        ColorFormat::L8 => (CM_L8, 1, false, Some((GRAY_CLUT.as_ptr() as usize, 256))),
        ColorFormat::A8 => (CM_A8, 1, true, None),
        _ => return None,
    };
    let (w, h) = (usize::from(src.w), usize::from(src.h));
    let stride = usize::from(src.stride);
    if w == 0 || h == 0 || stride % bytes != 0 || stride < w * bytes {
        return None;
    }
    let len = (h - 1) * stride + w * bytes;
    let addr = src.data.as_ptr() as usize;
    let offset = (stride / bytes - w) as u32;
    if src.data.len() < len || offset > MAX_OFFSET || !aligned(addr, bytes) {
        return None;
    }
    let alpha = if opa.is_cover() { 255 } else { opa.raw() };
    let am = if src.format == ColorFormat::Xrgb8888 {
        AM_REPLACE // the alpha byte is undefined: use ALPHA (= opa)
    } else if alpha == 255 {
        AM_NO_MODIFY
    } else {
        AM_MULTIPLY
    };
    Some(Fg {
        addr,
        len,
        offset,
        cm,
        am,
        alpha,
        color: 0, // A8 images are black with the texel's alpha (as in software)
        clut,
        has_alpha,
    })
}

impl<R: Dma2dRegs> DrawAccel for Dma2d<R> {
    fn fill(&mut self, dst: &mut DrawBuf<'_>, area: Rect, color: Color, opa: Opa) -> AccelResult {
        if opa.is_transparent() {
            return AccelResult::Done;
        }
        let Some(out) = self.output(dst, area) else {
            return AccelResult::Unsupported;
        };
        if opa.is_cover() {
            // Register-to-memory: OCOLR in the output format.
            return if self.start(MODE_R2M, &out, None, ocolr(out.cm, color)) {
                AccelResult::Queued
            } else {
                AccelResult::Unsupported
            };
        }
        // Translucent fill: blend a fixed-color foreground over the destination. The FG is read
        // as A8 from the destination itself (FGOR spans a destination line), but its alpha is
        // replaced by ALPHA = opa and its RGB comes from FGCOLR, so the bytes read do not matter.
        let line_bytes = dst.stride() as u32;
        if line_bytes - out.w > MAX_OFFSET {
            return AccelResult::Unsupported;
        }
        let fg = Fg {
            addr: out.addr,
            len: 0, // nothing the CPU wrote needs to reach memory for this read
            offset: line_bytes - out.w,
            cm: CM_A8,
            am: AM_REPLACE,
            alpha: opa.raw(),
            color: rgb(color),
            clut: None,
            has_alpha: true,
        };
        if self.start(MODE_M2M_BLEND, &out, Some(&fg), 0) {
            AccelResult::Queued
        } else {
            AccelResult::Unsupported
        }
    }

    fn blit(
        &mut self,
        dst: &mut DrawBuf<'_>,
        dst_area: Rect,
        src: &ImagePixels<'_>,
        opa: Opa,
    ) -> AccelResult {
        if opa.is_transparent() {
            return AccelResult::Done;
        }
        if dst_area.width() != i32::from(src.w) || dst_area.height() != i32::from(src.h) {
            return AccelResult::Unsupported;
        }
        let (Some(out), Some(fg)) = (self.output(dst, dst_area), source(src, opa)) else {
            return AccelResult::Unsupported;
        };
        let mode = if fg.has_alpha || fg.alpha != 255 {
            MODE_M2M_BLEND
        } else if src.format == dst.format() {
            MODE_M2M // same layout: plain copy
        } else {
            MODE_M2M_PFC
        };
        self.run_blocking(mode, &out, &fg)
    }

    fn blend_a8(
        &mut self,
        dst: &mut DrawBuf<'_>,
        area: Rect,
        color: Color,
        alpha: &[u8],
        stride: usize,
    ) -> AccelResult {
        let Some(out) = self.output(dst, area) else {
            return AccelResult::Unsupported;
        };
        let w = out.w as usize;
        let len = (out.h as usize - 1) * stride + w;
        if stride < w || alpha.len() < len || (stride - w) as u32 > MAX_OFFSET {
            return AccelResult::Unsupported;
        }
        // FG = A8 coverage with RGB from FGCOLR; BG = destination.
        let fg = Fg {
            addr: alpha.as_ptr() as usize,
            len,
            offset: (stride - w) as u32,
            cm: CM_A8,
            am: AM_NO_MODIFY,
            alpha: 255,
            color: rgb(color),
            clut: None,
            has_alpha: true,
        };
        self.run_blocking(MODE_M2M_BLEND, &out, &fg)
    }

    fn wait(&mut self) {
        // A timeout is counted and logged by `finish` and reported through `take_timeouts`:
        // the caller redraws what the aborted transfer missed.
        let _ = self.finish();
    }

    fn take_timeouts(&mut self) -> u32 {
        core::mem::take(&mut self.unreported)
    }
}
