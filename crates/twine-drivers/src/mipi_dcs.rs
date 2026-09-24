//! The generic MIPI DCS panel driver shared by every MIPI panel: [`MipiDcs`] (blocking,
//! [`DisplayDriver`]) and `AsyncMipiDcs` (feature `async`, `AsyncDisplayDriver`).
//!
//! A panel model is a [`PanelSpec`]: size, controller memory size and offsets, `MADCTL` value
//! per rotation, `COLMOD`, inversion and a vendor init table. The per-panel modules
//! ([`ili9341`](crate::ili9341), [`st7789`](crate::st7789), …) provide specs and constructors.
//!
//! # Initialization
//!
//! 1. Hardware reset (RST low 10 µs, high, wait 120 ms) or, without a reset pin, `SWRESET` and
//!    150 ms.
//! 2. The spec's vendor [`init`](PanelSpec::init) table.
//! 3. `COLMOD`, `MADCTL` for the rotation, `INVON`/`INVOFF`, `SLPOUT` + 120 ms, `DISPON`.
//!
//! # Byte order
//!
//! RGB565 panels report [`ColorFormat::Rgb565Swapped`]: the engine renders big-endian RGB565,
//! exactly the byte order the panel expects over SPI, so pixel buffers are sent **as-is**
//! (no per-pixel work in the driver). RGB666 panels (`COLMOD` `0x66`, ILI9488) report
//! [`ColorFormat::Rgb888`] and also receive the rendered bytes unchanged.
//!
//! # Rotation
//!
//! Rotation is done by the controller (`MADCTL`), so [`DisplayInfo::hw_rotation`] is `true` and
//! flush areas are in logical (rotated) coordinates. Every `MADCTL` table turns the picture the
//! same way as the engine's software rotation (`Deg90`: panel turned 90° clockwise, logical
//! `(x, y)` on native pixel `(y, w − 1 − x)`), so switching a display between hardware and
//! software rotation never changes what is shown. Offsets of panels that show only part of
//! the controller memory are derived from the `MADCTL` mirror bits of each rotation.

use core::ops::{BitOr, BitOrAssign};

use embedded_hal::digital::OutputPin;
use heapless::Deque;
use twine_core::log::{error, trace, warn};
use twine_core::{ColorFormat, Rect};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, Rotation};

use crate::interface::DcsInterface;

/// MIPI DCS command codes (MIPI DCS 1.03 / ILI9341 datasheet §8.2).
pub mod cmd {
    /// Software reset.
    pub const SWRESET: u8 = 0x01;
    /// Enter sleep mode.
    pub const SLPIN: u8 = 0x10;
    /// Exit sleep mode.
    pub const SLPOUT: u8 = 0x11;
    /// Normal display mode on.
    pub const NORON: u8 = 0x13;
    /// Display inversion off.
    pub const INVOFF: u8 = 0x20;
    /// Display inversion on.
    pub const INVON: u8 = 0x21;
    /// Display off.
    pub const DISPOFF: u8 = 0x28;
    /// Display on.
    pub const DISPON: u8 = 0x29;
    /// Column address set.
    pub const CASET: u8 = 0x2A;
    /// Row (page) address set.
    pub const RASET: u8 = 0x2B;
    /// Memory write.
    pub const RAMWR: u8 = 0x2C;
    /// Tearing effect line off.
    pub const TEOFF: u8 = 0x34;
    /// Tearing effect line on.
    pub const TEON: u8 = 0x35;
    /// Memory access control (rotation, mirroring, RGB/BGR).
    pub const MADCTL: u8 = 0x36;
    /// Interface pixel format.
    pub const COLMOD: u8 = 0x3A;
    /// Write display brightness (OLED/AMOLED controllers).
    pub const WRDISBV: u8 = 0x51;
    /// Write CTRL display (brightness control block, dimming).
    pub const WRCTRLD: u8 = 0x53;
}

/// `MADCTL` (memory access control) bits.
///
/// ```
/// use twine_drivers::mipi_dcs::Madctl;
/// let m = Madctl::MX.union(Madctl::BGR);
/// assert_eq!(m.bits(), 0x48);
/// assert!(m.contains(Madctl::MX) && !m.contains(Madctl::MV));
/// assert_eq!(Madctl::MY | Madctl::MV, Madctl::from_bits(0xA0));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Madctl(u8);

impl Madctl {
    /// No bits set.
    pub const EMPTY: Self = Self(0);
    /// Row address order (mirror Y).
    pub const MY: Self = Self(0x80);
    /// Column address order (mirror X).
    pub const MX: Self = Self(0x40);
    /// Row/column exchange (swap axes).
    pub const MV: Self = Self(0x20);
    /// Vertical refresh order.
    pub const ML: Self = Self(0x10);
    /// BGR colour filter order.
    pub const BGR: Self = Self(0x08);
    /// Horizontal refresh order.
    pub const MH: Self = Self(0x04);

    /// From a raw register value.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// The raw register value.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Both sets of bits (usable in `const` context, unlike `|`).
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// `self` without the bits of `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether all bits of `other` are set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Madctl {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for Madctl {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// The common rotation → `MADCTL` mapping for panels whose unmirrored memory is upright:
/// `Deg0: —, Deg90: MY|MV, Deg180: MX|MY, Deg270: MX|MV`, each combined with `extra` (e.g.
/// [`Madctl::BGR`]).
///
/// The rotations follow the engine's convention ([`Rotation`]): `Deg90` means the panel is
/// turned 90° clockwise, so the picture is drawn turned 90° counter-clockwise in panel
/// memory — logical `(x, y)` lands on native pixel `(y, w − 1 − x)`, exactly where the
/// engine's software rotation puts it. (`TFT_eSPI`'s and mipidsi's rotation 1 is the other
/// direction: their `Deg90` is this `Deg270`.)
#[must_use]
pub const fn standard_madctl(extra: Madctl) -> [Madctl; 4] {
    [
        extra,
        extra.union(Madctl::MY).union(Madctl::MV),
        extra.union(Madctl::MX).union(Madctl::MY),
        extra.union(Madctl::MX).union(Madctl::MV),
    ]
}

/// One entry of a vendor init table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitOp {
    /// A command with its parameter bytes.
    Cmd(u8, &'static [u8]),
    /// Wait this many milliseconds.
    DelayMs(u32),
}

/// Description of a MIPI DCS panel model.
///
/// Sizes and offsets refer to the unrotated panel (`MADCTL` without `MV`): `native_w ×
/// native_h` visible pixels inside a controller memory of `ram_w × ram_h`, starting at
/// `(offset_x, offset_y)` when neither `MX` nor `MY` is set. The offsets for other rotations
/// are derived from the mirror bits of [`madctl`](Self::madctl) (see [`offset`](Self::offset)).
///
/// Variants of a model are `const` copies with the builder methods:
///
/// ```
/// use twine_drivers::st7789::ST7789;
/// use twine_drivers::mipi_dcs::PanelSpec;
///
/// // An ST7789 module whose glass needs BGR and no inversion:
/// static MY_PANEL: PanelSpec = ST7789.with_bgr(true).with_invert(false).with_name("my panel");
/// assert!(!MY_PANEL.invert);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PanelSpec {
    /// Model name (logs, documentation).
    pub name: &'static str,
    /// Visible width at `Deg0` orientation of the memory (no `MV`).
    pub native_w: u16,
    /// Visible height at `Deg0` orientation of the memory (no `MV`).
    pub native_h: u16,
    /// Controller frame memory width (columns).
    pub ram_w: u16,
    /// Controller frame memory height (rows).
    pub ram_h: u16,
    /// First visible column with `MX` clear.
    pub offset_x: u16,
    /// First visible row with `MY` clear.
    pub offset_y: u16,
    /// `MADCTL` per [`Rotation`] (`Deg0`, `Deg90`, `Deg180`, `Deg270`).
    pub madctl: [Madctl; 4],
    /// `COLMOD` parameter: `0x55` (or `0x05`) = 16-bit RGB565, `0x66` (or `0x06`) = 18-bit RGB666.
    pub colmod: u8,
    /// Colour inversion (`INVON`) — many IPS panels need it.
    pub invert: bool,
    /// Flush areas' x, y, width and height must be multiples of this ([`DisplayInfo::align`];
    /// 2 for most AMOLED controllers, otherwise 1).
    pub align: u8,
    /// The controller cannot exchange rows and columns (no usable `MADCTL.MV`): `MADCTL` stays
    /// at the `Deg0` entry, [`DisplayInfo::hw_rotation`] is `false` and the engine rotates in
    /// software, so flush areas are in native panel coordinates.
    pub sw_rotation: bool,
    /// Vendor init table, run after reset and before `COLMOD`/`MADCTL`/`SLPOUT`/`DISPON`.
    pub init: &'static [InitOp],
}

const fn rot_index(r: Rotation) -> usize {
    match r {
        Rotation::Deg0 => 0,
        Rotation::Deg90 => 1,
        Rotation::Deg180 => 2,
        Rotation::Deg270 => 3,
    }
}

impl PanelSpec {
    /// The pixel format the engine must render for this panel: [`ColorFormat::Rgb888`] for
    /// 18-bit `COLMOD` (`x6`), otherwise [`ColorFormat::Rgb565Swapped`].
    #[must_use]
    pub const fn format(&self) -> ColorFormat {
        if self.colmod & 0x07 == 0x06 {
            ColorFormat::Rgb888
        } else {
            ColorFormat::Rgb565Swapped
        }
    }

    /// Bytes per pixel sent to the panel (2 or 3).
    #[must_use]
    pub const fn bytes_per_pixel(&self) -> usize {
        if self.colmod & 0x07 == 0x06 { 3 } else { 2 }
    }

    /// The rotation done by the controller for display rotation `r`: `r` itself, or `Deg0`
    /// for [`sw_rotation`](Self::sw_rotation) panels.
    #[must_use]
    pub const fn hw_rotation(&self, r: Rotation) -> Rotation {
        if self.sw_rotation { Rotation::Deg0 } else { r }
    }

    /// The `MADCTL` value for a rotation (always the `Deg0` entry for
    /// [`sw_rotation`](Self::sw_rotation) panels).
    #[must_use]
    pub const fn madctl_for(&self, r: Rotation) -> Madctl {
        self.madctl[rot_index(self.hw_rotation(r))]
    }

    /// Logical `(width, height)` for a rotation (swapped when its `MADCTL` has `MV`, or for
    /// 90°/270° on [`sw_rotation`](Self::sw_rotation) panels).
    #[must_use]
    pub const fn logical_size(&self, r: Rotation) -> (u16, u16) {
        let swapped = if self.sw_rotation {
            r.swaps_axes()
        } else {
            self.madctl_for(r).contains(Madctl::MV)
        };
        if swapped {
            (self.native_h, self.native_w)
        } else {
            (self.native_w, self.native_h)
        }
    }

    /// Column/row offset of the logical origin in controller memory for a rotation:
    /// `MX` mirrors the column offset inside `ram_w`, `MY` the row offset inside `ram_h`, `MV`
    /// swaps them (the method of mipidsi's `set_address_window`).
    #[must_use]
    pub const fn offset(&self, r: Rotation) -> (u16, u16) {
        let m = self.madctl_for(r);
        let mut ox = self.offset_x;
        let mut oy = self.offset_y;
        if m.contains(Madctl::MX) {
            ox = self.ram_w.saturating_sub(self.native_w).saturating_sub(ox);
        }
        if m.contains(Madctl::MY) {
            oy = self.ram_h.saturating_sub(self.native_h).saturating_sub(oy);
        }
        if m.contains(Madctl::MV) {
            (oy, ox)
        } else {
            (ox, oy)
        }
    }

    /// `CASET` and `RASET` parameters (big-endian inclusive start/end) for an area of the
    /// flush coordinate space of rotation `r` (logical, or native for
    /// [`sw_rotation`](Self::sw_rotation) panels), or `None` if the area is empty, outside the
    /// screen or not a multiple of [`align`](Self::align).
    #[must_use]
    pub fn window(&self, r: Rotation, area: Rect) -> Option<([u8; 4], [u8; 4])> {
        let r = self.hw_rotation(r);
        let (w, h) = self.logical_size(r);
        let a = i32::from(self.align.max(1));
        if [area.x0, area.y0, area.x1, area.y1].iter().any(|v| v % a != 0) {
            return None;
        }
        if area.is_empty() || !Rect::new(0, 0, i32::from(w), i32::from(h)).contains_rect(&area) {
            return None;
        }
        let (ox, oy) = self.offset(r);
        let enc = |a: i32, b: i32, o: u16| {
            let s = (a as u16).wrapping_add(o).to_be_bytes();
            let e = ((b - 1) as u16).wrapping_add(o).to_be_bytes();
            [s[0], s[1], e[0], e[1]]
        };
        Some((enc(area.x0, area.x1, ox), enc(area.y0, area.y1, oy)))
    }

    /// Returns a copy with another name.
    #[must_use]
    pub const fn with_name(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Returns a copy with colour inversion on or off.
    #[must_use]
    pub const fn with_invert(mut self, invert: bool) -> Self {
        self.invert = invert;
        self
    }

    /// Returns a copy with the `BGR` bit set or cleared in every rotation.
    #[must_use]
    pub const fn with_bgr(mut self, bgr: bool) -> Self {
        let mut i = 0;
        while i < 4 {
            self.madctl[i] = if bgr {
                self.madctl[i].union(Madctl::BGR)
            } else {
                self.madctl[i].difference(Madctl::BGR)
            };
            i += 1;
        }
        self
    }

    /// Returns a copy with another visible size and offset inside the same controller memory.
    #[must_use]
    pub const fn with_size(mut self, native_w: u16, native_h: u16, offset_x: u16, offset_y: u16) -> Self {
        self.native_w = native_w;
        self.native_h = native_h;
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        self
    }

    /// Returns a copy with another flush alignment.
    #[must_use]
    pub const fn with_align(mut self, align: u8) -> Self {
        self.align = align;
        self
    }

    /// Returns a copy with another `MADCTL` table.
    #[must_use]
    pub const fn with_madctl(mut self, madctl: [Madctl; 4]) -> Self {
        self.madctl = madctl;
        self
    }
}

/// Errors of [`MipiDcs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DcsError<E> {
    /// The interface (bus or DC pin) failed.
    Interface(E),
    /// The reset pin failed.
    ResetPin,
    /// The flush area is empty or outside the display, or the buffer is too short for it.
    BadArea,
}

/// One step of an initialization or command sequence (shared by the blocking and async
/// drivers, so both send identical bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Rst(bool),
    DelayUs(u32),
    Cmd(u8, &'static [u8]),
    Cmd1(u8, u8),
}

/// The full init sequence (see the module docs).
pub(crate) fn init_steps(
    spec: &'static PanelSpec,
    rotation: Rotation,
    has_rst: bool,
) -> impl Iterator<Item = Step> {
    let reset = if has_rst {
        [
            Some(Step::Rst(false)),
            Some(Step::DelayUs(10)),
            Some(Step::Rst(true)),
            Some(Step::DelayUs(120_000)),
        ]
    } else {
        [
            Some(Step::Cmd(cmd::SWRESET, &[])),
            Some(Step::DelayUs(150_000)),
            None,
            None,
        ]
    };
    let tail = [
        Step::Cmd1(cmd::COLMOD, spec.colmod),
        Step::Cmd1(cmd::MADCTL, spec.madctl_for(rotation).bits()),
        Step::Cmd(if spec.invert { cmd::INVON } else { cmd::INVOFF }, &[]),
        Step::Cmd(cmd::SLPOUT, &[]),
        Step::DelayUs(120_000),
        Step::Cmd(cmd::DISPON, &[]),
    ];
    reset
        .into_iter()
        .flatten()
        .chain(spec.init.iter().map(|op| match *op {
            InitOp::Cmd(c, p) => Step::Cmd(c, p),
            InitOp::DelayMs(ms) => Step::DelayUs(ms.saturating_mul(1000)),
        }))
        .chain(tail)
}

/// Checks a flush and returns its `CASET`/`RASET` parameters and byte count.
fn check_flush<E>(
    spec: &PanelSpec,
    rotation: Rotation,
    area: Rect,
    len: usize,
) -> Result<([u8; 4], [u8; 4], usize), DcsError<E>> {
    let Some((c, r)) = spec.window(rotation, area) else {
        error!(target: "twine::driver", "{}: flush area {:?} outside the display or misaligned", spec.name, area);
        return Err(DcsError::BadArea);
    };
    let bytes = area.area() as usize * spec.bytes_per_pixel();
    if len < bytes {
        error!(target: "twine::driver", "{}: buffer of {} bytes too short for {} bytes", spec.name, len, bytes);
        return Err(DcsError::BadArea);
    }
    Ok((c, r, bytes))
}

fn display_info(spec: &PanelSpec, rotation: Rotation, dpi: u16) -> DisplayInfo {
    let (w, h) = spec.logical_size(rotation);
    DisplayInfo::new(w, h, spec.format())
        .with_rotation(rotation)
        .with_hw_rotation(!spec.sw_rotation)
        .with_align(spec.align.max(1))
        .with_dpi(dpi)
}

/// Default DPI reported by the drivers (LVGL's default).
pub const DEFAULT_DPI: u16 = 130;

/// Blocking MIPI DCS panel driver, generic over the interface and an optional reset pin.
///
/// `begin_flush` sends `CASET`, `RASET`, `RAMWR` and the pixels, then keeps the buffer until
/// `poll_flush` hands it back (the transfer is complete when `begin_flush` returns). To overlap
/// transfers with rendering use the async driver or a DMA-capable firmware driver.
///
/// ```
/// use twine_core::Rect;
/// use twine_drivers::ili9341::ILI9341;
/// use twine_drivers::interface::SpiInterface;
/// use twine_drivers::mipi_dcs::MipiDcs;
/// use twine_drivers::testkit::{BusOp, Recorder};
/// use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};
///
/// let rec = Recorder::new();
/// let iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
/// let mut lcd = MipiDcs::new(iface, Some(rec.pin("rst")), &ILI9341, Rotation::Deg90, &mut rec.delay()).unwrap();
/// assert_eq!((lcd.info().width, lcd.info().height), (320, 240));
///
/// let _ = rec.take_ops();
/// let buf = DrawBufferMem::new(Box::leak(Box::new([0u8; 8])));
/// lcd.begin_flush(Rect::from_xywh(0, 0, 2, 2), buf).unwrap();
/// assert!(lcd.poll_flush().is_some());
/// assert_eq!(rec.ops().last(), Some(&BusOp::Pixels(8)));
/// ```
pub struct MipiDcs<I, RST> {
    iface: I,
    rst: Option<RST>,
    spec: &'static PanelSpec,
    rotation: Rotation,
    dpi: u16,
    pending: Deque<DrawBufferMem, 2>,
}

impl<I, RST> core::fmt::Debug for MipiDcs<I, RST> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MipiDcs")
            .field("panel", &self.spec.name)
            .field("rotation", &self.rotation)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<I: DcsInterface, RST: OutputPin> MipiDcs<I, RST> {
    /// Resets and initializes the panel (see the module docs) and returns the driver.
    pub fn new(
        iface: I,
        rst: Option<RST>,
        spec: &'static PanelSpec,
        rotation: Rotation,
        delay: &mut impl embedded_hal::delay::DelayNs,
    ) -> Result<Self, DcsError<I::Error>> {
        let mut this = Self {
            iface,
            rst,
            spec,
            rotation,
            dpi: DEFAULT_DPI,
            pending: Deque::new(),
        };
        let has_rst = this.rst.is_some();
        for step in init_steps(spec, rotation, has_rst) {
            this.run(step, delay)?;
        }
        twine_core::log::info!(target: "twine::driver", "{} initialized, rotation {}°", spec.name, rotation.degrees());
        Ok(this)
    }

    /// Sets the DPI reported by [`info`](DisplayDriver::info) (default [`DEFAULT_DPI`]).
    #[must_use]
    pub fn with_dpi(mut self, dpi: u16) -> Self {
        self.dpi = dpi;
        self
    }

    /// The panel model.
    #[must_use]
    pub fn spec(&self) -> &'static PanelSpec {
        self.spec
    }

    fn run(
        &mut self,
        step: Step,
        delay: &mut impl embedded_hal::delay::DelayNs,
    ) -> Result<(), DcsError<I::Error>> {
        match step {
            Step::Rst(level) => {
                if let Some(rst) = self.rst.as_mut() {
                    rst.set_state(level.into()).map_err(|_| {
                        warn!(target: "twine::driver", "{}: reset pin failed", self.spec.name);
                        DcsError::ResetPin
                    })?;
                }
                Ok(())
            }
            Step::DelayUs(us) => {
                delay.delay_us(us);
                Ok(())
            }
            Step::Cmd(c, p) => self.command(c, p),
            Step::Cmd1(c, p) => self.command(c, &[p]),
        }
    }

    fn command(&mut self, c: u8, params: &[u8]) -> Result<(), DcsError<I::Error>> {
        self.iface.command(c, params).map_err(|e| {
            warn!(target: "twine::driver", "{}: command {:x} failed", self.spec.name, c);
            DcsError::Interface(e)
        })
    }

    /// Changes the rotation (`MADCTL`). The engine expects [`DisplayDriver::info`] not to
    /// change while it runs, so call this before building the UI.
    pub fn set_rotation(&mut self, r: Rotation) -> Result<(), DcsError<I::Error>> {
        self.command(cmd::MADCTL, &[self.spec.madctl_for(r).bits()])?;
        self.rotation = r;
        Ok(())
    }

    /// Sets the address window for a logical `area` (`CASET`, `RASET` incl. offsets) and
    /// starts a memory write (`RAMWR`). Pixels follow with the interface's `write_pixels`.
    pub fn set_window(&mut self, area: Rect) -> Result<(), DcsError<I::Error>> {
        let (c, r, _) = check_flush(self.spec, self.rotation, area, usize::MAX)?;
        self.command(cmd::CASET, &c)?;
        self.command(cmd::RASET, &r)?;
        self.command(cmd::RAMWR, &[])
    }

    /// Enters (`true`, `SLPIN` + 5 ms) or leaves (`false`, `SLPOUT` + 120 ms) sleep mode.
    pub fn sleep(
        &mut self,
        on: bool,
        delay: &mut impl embedded_hal::delay::DelayNs,
    ) -> Result<(), DcsError<I::Error>> {
        self.command(if on { cmd::SLPIN } else { cmd::SLPOUT }, &[])?;
        delay.delay_us(if on { 5_000 } else { 120_000 });
        Ok(())
    }

    /// Turns the display output on (`DISPON`) or off (`DISPOFF`, frame memory kept).
    pub fn display_on(&mut self, on: bool) -> Result<(), DcsError<I::Error>> {
        self.command(if on { cmd::DISPON } else { cmd::DISPOFF }, &[])
    }

    /// Enables the tearing-effect output (`TEON`, V-blank only) or disables it (`TEOFF`).
    pub fn tearing_effect(&mut self, on: bool) -> Result<(), DcsError<I::Error>> {
        if on {
            self.command(cmd::TEON, &[0x00])
        } else {
            self.command(cmd::TEOFF, &[])
        }
    }

    /// Sets the display brightness (`WRDISBV`, `0x51`; `0` = off, `255` = maximum) of
    /// controllers that dim themselves (AMOLED, OLED). LCD panels dim with their backlight pin.
    pub fn set_brightness(&mut self, level: u8) -> Result<(), DcsError<I::Error>> {
        self.command(cmd::WRDISBV, &[level])
    }

    /// Sends pixels of `area` immediately (the body of `begin_flush`).
    fn flush_now(&mut self, area: Rect, buf: &[u8]) -> Result<(), DcsError<I::Error>> {
        let (c, r, bytes) = check_flush(self.spec, self.rotation, area, buf.len())?;
        trace!(target: "twine::driver", "{}: flush {:?}", self.spec.name, area);
        self.command(cmd::CASET, &c)?;
        self.command(cmd::RASET, &r)?;
        self.command(cmd::RAMWR, &[])?;
        self.iface.write_pixels(&buf[..bytes]).map_err(|e| {
            warn!(target: "twine::driver", "{}: pixel write failed", self.spec.name);
            DcsError::Interface(e)
        })
    }

    /// Returns the interface and the reset pin.
    #[must_use]
    pub fn release(self) -> (I, Option<RST>) {
        (self.iface, self.rst)
    }
}

impl<I: DcsInterface, RST: OutputPin> DisplayDriver for MipiDcs<I, RST> {
    type Error = DcsError<I::Error>;

    fn info(&self) -> DisplayInfo {
        display_info(self.spec, self.rotation, self.dpi)
    }

    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        let r = self.flush_now(area, buf.as_slice());
        if self.pending.push_back(buf).is_err() {
            error!(target: "twine::driver", "{}: more than 2 buffers in flight; buffer dropped", self.spec.name);
        }
        r
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.pending.pop_front()
    }
}

#[cfg(feature = "async")]
pub use self::asynch::AsyncMipiDcs;

#[cfg(feature = "async")]
mod asynch {
    use embedded_hal::digital::OutputPin;
    use twine_core::Rect;
    use twine_core::log::{trace, warn};
    use twine_hal::{AsyncDisplayDriver, DisplayInfo, Rotation};

    use super::{DEFAULT_DPI, DcsError, PanelSpec, Step, check_flush, cmd, display_info, init_steps};
    use crate::interface::AsyncDcsInterface;

    /// Async MIPI DCS panel driver (feature `async`): same behaviour and bytes as
    /// [`MipiDcs`](super::MipiDcs), implementing [`AsyncDisplayDriver`].
    ///
    /// With a DMA-backed async `SpiDevice`, `flush` starts the pixel transfer on its first poll,
    /// so `join(display.flush(..), render_next_chunk)` overlaps DMA and rendering.
    pub struct AsyncMipiDcs<I, RST> {
        iface: I,
        rst: Option<RST>,
        spec: &'static PanelSpec,
        rotation: Rotation,
        dpi: u16,
    }

    impl<I, RST> core::fmt::Debug for AsyncMipiDcs<I, RST> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("AsyncMipiDcs")
                .field("panel", &self.spec.name)
                .field("rotation", &self.rotation)
                .finish_non_exhaustive()
        }
    }

    impl<I: AsyncDcsInterface, RST: OutputPin> AsyncMipiDcs<I, RST> {
        /// Resets and initializes the panel; identical sequence to [`MipiDcs::new`](super::MipiDcs::new).
        pub async fn new(
            iface: I,
            rst: Option<RST>,
            spec: &'static PanelSpec,
            rotation: Rotation,
            delay: &mut impl embedded_hal_async::delay::DelayNs,
        ) -> Result<Self, DcsError<I::Error>> {
            let mut this = Self {
                iface,
                rst,
                spec,
                rotation,
                dpi: DEFAULT_DPI,
            };
            let has_rst = this.rst.is_some();
            for step in init_steps(spec, rotation, has_rst) {
                this.run(step, delay).await?;
            }
            twine_core::log::info!(target: "twine::driver", "{} initialized, rotation {}°", spec.name, rotation.degrees());
            Ok(this)
        }

        /// Sets the DPI reported by `info` (default [`DEFAULT_DPI`]).
        #[must_use]
        pub fn with_dpi(mut self, dpi: u16) -> Self {
            self.dpi = dpi;
            self
        }

        /// The panel model.
        #[must_use]
        pub fn spec(&self) -> &'static PanelSpec {
            self.spec
        }

        async fn run(
            &mut self,
            step: Step,
            delay: &mut impl embedded_hal_async::delay::DelayNs,
        ) -> Result<(), DcsError<I::Error>> {
            match step {
                Step::Rst(level) => {
                    if let Some(rst) = self.rst.as_mut() {
                        rst.set_state(level.into()).map_err(|_| {
                            warn!(target: "twine::driver", "{}: reset pin failed", self.spec.name);
                            DcsError::ResetPin
                        })?;
                    }
                    Ok(())
                }
                Step::DelayUs(us) => {
                    delay.delay_us(us).await;
                    Ok(())
                }
                Step::Cmd(c, p) => self.command(c, p).await,
                Step::Cmd1(c, p) => self.command(c, &[p]).await,
            }
        }

        async fn command(&mut self, c: u8, params: &[u8]) -> Result<(), DcsError<I::Error>> {
            match self.iface.command(c, params).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    warn!(target: "twine::driver", "{}: command {:x} failed", self.spec.name, c);
                    Err(DcsError::Interface(e))
                }
            }
        }

        /// Changes the rotation (`MADCTL`); call before building the UI.
        pub async fn set_rotation(&mut self, r: Rotation) -> Result<(), DcsError<I::Error>> {
            self.command(cmd::MADCTL, &[self.spec.madctl_for(r).bits()])
                .await?;
            self.rotation = r;
            Ok(())
        }

        /// Sets the address window for a logical `area` and starts a memory write (`RAMWR`).
        pub async fn set_window(&mut self, area: Rect) -> Result<(), DcsError<I::Error>> {
            let (c, r, _) = check_flush(self.spec, self.rotation, area, usize::MAX)?;
            self.command(cmd::CASET, &c).await?;
            self.command(cmd::RASET, &r).await?;
            self.command(cmd::RAMWR, &[]).await
        }

        /// Enters (`SLPIN` + 5 ms) or leaves (`SLPOUT` + 120 ms) sleep mode.
        pub async fn sleep(
            &mut self,
            on: bool,
            delay: &mut impl embedded_hal_async::delay::DelayNs,
        ) -> Result<(), DcsError<I::Error>> {
            self.command(if on { cmd::SLPIN } else { cmd::SLPOUT }, &[])
                .await?;
            delay.delay_us(if on { 5_000 } else { 120_000 }).await;
            Ok(())
        }

        /// Turns the display output on (`DISPON`) or off (`DISPOFF`).
        pub async fn display_on(&mut self, on: bool) -> Result<(), DcsError<I::Error>> {
            self.command(if on { cmd::DISPON } else { cmd::DISPOFF }, &[])
                .await
        }

        /// Enables (`TEON`, V-blank only) or disables (`TEOFF`) the tearing-effect output.
        pub async fn tearing_effect(&mut self, on: bool) -> Result<(), DcsError<I::Error>> {
            if on {
                self.command(cmd::TEON, &[0x00]).await
            } else {
                self.command(cmd::TEOFF, &[]).await
            }
        }

        /// Sets the display brightness (`WRDISBV`, `0x51`) of self-dimming controllers.
        pub async fn set_brightness(&mut self, level: u8) -> Result<(), DcsError<I::Error>> {
            self.command(cmd::WRDISBV, &[level]).await
        }

        /// Returns the interface and the reset pin.
        #[must_use]
        pub fn release(self) -> (I, Option<RST>) {
            (self.iface, self.rst)
        }
    }

    impl<I: AsyncDcsInterface, RST: OutputPin> AsyncDisplayDriver for AsyncMipiDcs<I, RST> {
        type Error = DcsError<I::Error>;

        fn info(&self) -> DisplayInfo {
            display_info(self.spec, self.rotation, self.dpi)
        }

        async fn flush(&mut self, area: Rect, buf: &[u8]) -> Result<(), Self::Error> {
            let (c, r, bytes) = check_flush(self.spec, self.rotation, area, buf.len())?;
            trace!(target: "twine::driver", "{}: flush {:?}", self.spec.name, area);
            self.command(cmd::CASET, &c).await?;
            self.command(cmd::RASET, &r).await?;
            self.command(cmd::RAMWR, &[]).await?;
            match self.iface.write_pixels(&buf[..bytes]).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    warn!(target: "twine::driver", "{}: pixel write failed", self.spec.name);
                    Err(DcsError::Interface(e))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::SpiInterface;
    use crate::mock::{BusOp, Recorder, RecordingPin, RecordingSpi, block_on};
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec;

    /// A small panel: 10×20 visible inside 12×24 memory at (1, 2).
    static TEST: PanelSpec = PanelSpec {
        name: "test",
        native_w: 10,
        native_h: 20,
        ram_w: 12,
        ram_h: 24,
        offset_x: 1,
        offset_y: 2,
        madctl: standard_madctl(Madctl::BGR),
        colmod: 0x55,
        align: 1,
        sw_rotation: false,
        invert: true,
        init: &[InitOp::Cmd(0xB1, &[0x00, 0x18]), InitOp::DelayMs(5)],
    };

    type Dut = MipiDcs<SpiInterface<RecordingSpi, RecordingPin>, RecordingPin>;

    fn dut(spec: &'static PanelSpec, rot: Rotation) -> (Recorder, Dut) {
        let rec = Recorder::new();
        let iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
        let d = MipiDcs::new(iface, Some(rec.pin("rst")), spec, rot, &mut rec.delay()).unwrap();
        (rec, d)
    }

    fn buf(len: usize) -> DrawBufferMem {
        DrawBufferMem::new(Box::leak(vec![0x5Au8; len].into_boxed_slice()))
    }

    #[test]
    fn init_runs_reset_then_table_then_sleep_out_display_on() {
        let (rec, _d) = dut(&TEST, Rotation::Deg0);
        assert_eq!(
            rec.ops(),
            [
                BusOp::Pin("rst", false),
                BusOp::DelayUs(10),
                BusOp::Pin("rst", true),
                BusOp::DelayUs(120_000),
                BusOp::Cmd(0xB1),
                BusOp::Data(vec![0x00, 0x18]),
                BusOp::DelayUs(5_000),
                BusOp::Cmd(cmd::COLMOD),
                BusOp::Data(vec![0x55]),
                BusOp::Cmd(cmd::MADCTL),
                BusOp::Data(vec![0x08]),
                BusOp::Cmd(cmd::INVON),
                BusOp::Cmd(cmd::SLPOUT),
                BusOp::DelayUs(120_000),
                BusOp::Cmd(cmd::DISPON),
            ]
        );
    }

    #[test]
    fn init_without_reset_pin_uses_swreset() {
        let rec = Recorder::new();
        let iface = SpiInterface::new(rec.spi(), rec.quiet_pin("dc"));
        let _d =
            MipiDcs::<_, RecordingPin>::new(iface, None, &TEST, Rotation::Deg0, &mut rec.delay()).unwrap();
        assert_eq!(
            rec.ops()[..2],
            [BusOp::Cmd(cmd::SWRESET), BusOp::DelayUs(150_000)]
        );
    }

    #[test]
    fn set_window_rotation0_encodes_be_coords() {
        static BIG: PanelSpec = TEST.with_size(300, 400, 0, 0).with_name("big");
        let mut d = dut(&BIG, Rotation::Deg0).1;
        let rec_ops = {
            let (c, r) = BIG.window(Rotation::Deg0, Rect::new(10, 260, 290, 300)).unwrap();
            (c, r)
        };
        assert_eq!(rec_ops, ([0x00, 10, 0x01, 0x21], [0x01, 0x04, 0x01, 0x2B]));
        d.set_window(Rect::new(10, 260, 290, 300)).unwrap();
    }

    #[test]
    fn set_window_applies_offsets() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        let _ = rec.take_ops();
        d.set_window(Rect::new(0, 0, 10, 20)).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::Cmd(cmd::CASET),
                BusOp::Data(vec![0, 1, 0, 10]),
                BusOp::Cmd(cmd::RASET),
                BusOp::Data(vec![0, 2, 0, 21]),
                BusOp::Cmd(cmd::RAMWR),
            ]
        );
    }

    #[test]
    fn set_window_rotation90_swaps_axes() {
        // Deg90 = MY|MV: logical 20×10; row offset mirrored (24 − 20 − 2 = 2), then swapped.
        let (rec, mut d) = dut(&TEST, Rotation::Deg90);
        assert_eq!((d.info().width, d.info().height), (20, 10));
        assert_eq!(TEST.offset(Rotation::Deg90), (2, 1));
        let _ = rec.take_ops();
        d.set_window(Rect::new(0, 0, 20, 10)).unwrap();
        assert_eq!(rec.ops()[1], BusOp::Data(vec![0, 2, 0, 21]));
        assert_eq!(rec.ops()[3], BusOp::Data(vec![0, 1, 0, 10]));
    }

    #[test]
    fn flush_sequence_caset_raset_ramwr_pixels() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        let _ = rec.take_ops();
        let before = rec.spi_transactions();
        d.begin_flush(Rect::from_xywh(2, 3, 4, 5), buf(4 * 5 * 2 + 6))
            .unwrap();
        let expected: Vec<BusOp> = vec![
            BusOp::Cmd(0x2A),
            BusOp::Data(vec![0x00, 0x03, 0x00, 0x06]),
            BusOp::Cmd(0x2B),
            BusOp::Data(vec![0x00, 0x05, 0x00, 0x09]),
            BusOp::Cmd(0x2C),
            BusOp::Pixels(40),
        ];
        assert_eq!(rec.ops(), expected);
        assert_eq!(rec.spi_transactions() - before, 6);
        assert_eq!(d.poll_flush().map(|b| b.len()), Some(46));
        assert!(d.poll_flush().is_none());
    }

    #[test]
    fn flush_writes_ramwr_then_pixels_in_order() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        let _ = rec.take_ops();
        let mut a = vec![1u8; 2];
        a.extend([2u8; 2]);
        d.begin_flush(Rect::from_xywh(0, 0, 2, 1), buf(0)).unwrap_err();
        let _ = d.poll_flush();
        d.begin_flush(
            Rect::from_xywh(0, 0, 2, 1),
            DrawBufferMem::new(Box::leak(a.into_boxed_slice())),
        )
        .unwrap();
        d.begin_flush(
            Rect::from_xywh(0, 1, 1, 1),
            DrawBufferMem::new(Box::leak(vec![3u8, 3].into_boxed_slice())),
        )
        .unwrap();
        let ops = rec.ops();
        let pos: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, o)| matches!(o, BusOp::Cmd(0x2C) | BusOp::Pixels(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(pos.len(), 4);
        assert_eq!(ops[pos[0]], BusOp::Cmd(0x2C));
        assert_eq!(ops[pos[1]], BusOp::Pixels(4));
        assert_eq!(pos[1], pos[0] + 1);
        assert_eq!(ops[pos[2]], BusOp::Cmd(0x2C));
        assert_eq!(ops[pos[3]], BusOp::Pixels(2));
        assert_eq!(rec.pixel_bytes(), [1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn two_buffers_returned_in_order() {
        let (_rec, mut d) = dut(&TEST, Rotation::Deg0);
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), buf(2)).unwrap();
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), buf(3)).unwrap();
        assert_eq!(d.poll_flush().map(|b| b.len()), Some(2));
        assert_eq!(d.poll_flush().map(|b| b.len()), Some(3));
    }

    #[test]
    fn flush_bad_area_errors() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        let _ = rec.take_ops();
        assert_eq!(
            d.begin_flush(Rect::from_xywh(8, 0, 4, 1), buf(8)),
            Err(DcsError::BadArea)
        );
        // The buffer is still handed back.
        assert!(d.poll_flush().is_some());
        assert_eq!(
            d.begin_flush(Rect::from_xywh(0, 0, 4, 1), buf(7)),
            Err(DcsError::BadArea)
        );
        assert!(d.poll_flush().is_some());
        assert_eq!(
            d.begin_flush(Rect::from_xywh(0, 0, 0, 1), buf(8)),
            Err(DcsError::BadArea)
        );
        assert!(d.poll_flush().is_some());
        assert!(rec.ops().is_empty());
    }

    #[test]
    fn rotation_changes_madctl_and_info() {
        let table = [
            (Rotation::Deg0, 0x08, (10, 20), (1, 2)),
            (Rotation::Deg90, 0xA8, (20, 10), (2, 1)),
            (Rotation::Deg180, 0xC8, (10, 20), (1, 2)),
            (Rotation::Deg270, 0x68, (20, 10), (2, 1)),
        ];
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        for (rot, madctl, size, offset) in table {
            let _ = rec.take_ops();
            d.set_rotation(rot).unwrap();
            assert_eq!(rec.ops(), [BusOp::Cmd(cmd::MADCTL), BusOp::Data(vec![madctl])]);
            let info = d.info();
            assert_eq!((info.width, info.height), size);
            assert_eq!(info.rotation, rot);
            assert!(info.hw_rotation);
            assert_eq!(info.format, ColorFormat::Rgb565Swapped);
            assert_eq!(TEST.offset(rot), offset);
        }
    }

    #[test]
    fn misc_commands() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        let _ = rec.take_ops();
        d.sleep(true, &mut rec.delay()).unwrap();
        d.sleep(false, &mut rec.delay()).unwrap();
        d.display_on(false).unwrap();
        d.tearing_effect(true).unwrap();
        d.tearing_effect(false).unwrap();
        assert_eq!(
            rec.ops(),
            [
                BusOp::Cmd(cmd::SLPIN),
                BusOp::DelayUs(5_000),
                BusOp::Cmd(cmd::SLPOUT),
                BusOp::DelayUs(120_000),
                BusOp::Cmd(cmd::DISPOFF),
                BusOp::Cmd(cmd::TEON),
                BusOp::Data(vec![0]),
                BusOp::Cmd(cmd::TEOFF),
            ]
        );
        assert_eq!(d.with_dpi(200).info().dpi, 200);
    }

    #[test]
    fn bus_error_is_reported() {
        let (rec, mut d) = dut(&TEST, Rotation::Deg0);
        rec.fail_next();
        assert!(matches!(d.display_on(true), Err(DcsError::Interface(_))));
        rec.fail_next();
        let r = MipiDcs::new(
            SpiInterface::new(rec.spi(), rec.quiet_pin("dc")),
            Some(rec.pin("rst")),
            &TEST,
            Rotation::Deg0,
            &mut rec.delay(),
        );
        assert_eq!(r.err(), Some(DcsError::ResetPin));
    }

    #[test]
    fn async_driver_same_bytes_and_starts_on_first_poll() {
        use twine_hal::AsyncDisplayDriver;
        let (a, mut d) = dut(&TEST, Rotation::Deg90);
        d.begin_flush(Rect::from_xywh(1, 1, 2, 2), buf(8)).unwrap();

        let b = Recorder::new();
        let mut ad = block_on(AsyncMipiDcs::new(
            SpiInterface::new(b.spi(), b.quiet_pin("dc")),
            Some(b.pin("rst")),
            &TEST,
            Rotation::Deg90,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(ad.info(), d.info());
        let data = [0x5Au8; 8];
        b.pending_once();
        {
            let mut fut = Box::pin(ad.flush(Rect::from_xywh(1, 1, 2, 2), &data));
            // The first poll already issued bus traffic (CASET) before returning Pending.
            assert!(crate::mock::poll_once(&mut fut).is_none());
            assert!(b.ops().ends_with(&[BusOp::Cmd(cmd::CASET)]));
            assert_eq!(block_on(fut), Ok(()));
        }
        assert_eq!(a.ops(), b.ops());
        assert_eq!(a.pixel_bytes(), b.pixel_bytes());
        assert_eq!(
            block_on(ad.flush(Rect::from_xywh(0, 0, 30, 1), &data)),
            Err(DcsError::BadArea)
        );
    }
}
