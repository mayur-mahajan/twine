//! Displays: [`BufferMode`], [`BufferSpec`], the object-safe driver wrappers, screens and layers.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use twine_core::fault::FaultKind;
use twine_core::{ColorFormat, Duration, Fraction, Rect, Rotation};
use twine_hal::{
    ControlError, DisplayDriver, DisplayInfo, DrawBuffer, DrawBufferMem, FramebufferDisplay, buffer_bytes,
};

use crate::refresh::Refresher;
use crate::{
    DisplayId, Engine, EngineError, EventCode, EventParam, FaultRecord, InvalidateReason, NodeId, Obj,
    ObjFlags, Widget, fmt_node_id,
};

/// Maximum number of displays per engine.
pub const MAX_DISPLAYS: usize = 4;

/// Nodes every display creates when it is added: bottom layer, first screen, top layer and
/// system layer (they count against `EngineConfig::max_nodes`).
const DISPLAY_NODES: usize = 4;

/// A request for partial draw buffers on the heap: how many (one or two) and how many
/// full-width rows each ([`BufferMode::Alloc`]).
///
/// The engine allocates the buffers (zeroed, 4-byte aligned, leaked like `'static` firmware
/// memory) only **after** it has accepted the display, sized for that display:
/// [`bytes_per_buffer`](Self::bytes_per_buffer). A display that is refused allocates nothing.
///
/// `Copy`, so simulators and test harnesses keep it in their configuration and rebuild from
/// it. Firmware usually declares `static` buffers instead (`twine::draw_buffers!`).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_engine::BufferSpec;
/// use twine_hal::DisplayInfo;
///
/// let info = DisplayInfo::new(320, 240, ColorFormat::Rgb565);
/// assert_eq!(BufferSpec::default(), BufferSpec::PartialDouble { rows: 40 });
/// assert_eq!(BufferSpec::default().bytes_per_buffer(&info), 320 * 2 * 40);
/// assert_eq!(BufferSpec::PartialSingle { rows: 500 }.bytes_per_buffer(&info), 320 * 2 * 240);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BufferSpec {
    /// One partial buffer of `rows` full-width rows: render, flush, wait, repeat.
    PartialSingle {
        /// Rows per buffer.
        rows: u16,
    },
    /// Two partial buffers of `rows` rows each (rendering overlaps flushing, e.g. DMA).
    PartialDouble {
        /// Rows per buffer.
        rows: u16,
    },
}

impl Default for BufferSpec {
    /// Two buffers of 40 rows.
    fn default() -> Self {
        BufferSpec::PartialDouble { rows: 40 }
    }
}

impl BufferSpec {
    /// Number of buffers (1 or 2).
    #[must_use]
    pub const fn buffer_count(&self) -> usize {
        match self {
            BufferSpec::PartialSingle { .. } => 1,
            BufferSpec::PartialDouble { .. } => 2,
        }
    }

    /// Rows per buffer as requested.
    #[must_use]
    pub const fn rows(&self) -> u16 {
        match *self {
            BufferSpec::PartialSingle { rows } | BufferSpec::PartialDouble { rows } => rows,
        }
    }

    /// Bytes of **one** buffer for the display `info` ([`buffer_bytes`] of its logical width).
    ///
    /// The rows are clamped to `align..=height` (`align` = [`DisplayInfo::align`], at least
    /// 1): a buffer taller than the screen is useless, and one shorter than the display's row
    /// alignment cannot render anything. Never panics.
    #[must_use]
    pub fn bytes_per_buffer(&self, info: &DisplayInfo) -> usize {
        let min = u16::from(info.align.max(1));
        let rows = self.rows().max(min).min(info.height);
        buffer_bytes(info.width, rows, info.format)
    }
}

/// The draw buffers of a display (LVGL's render modes): one concept for caller-provided
/// memory, heap allocation and driver-owned framebuffers.
///
/// | Mode | Driver | Memory | Behaviour |
/// |------|--------|--------|-----------|
/// | `Partial { a, b: None }` | [`DisplayDriver`] | the caller's (`static`) | render a chunk, flush, wait, repeat |
/// | `Partial { a, b: Some(_) }` | [`DisplayDriver`] | the caller's (`static`) | ping-pong: render into one buffer while the other is flushed (DMA) |
/// | `Alloc(spec)` | [`DisplayDriver`] | heap, allocated by the engine | like `Partial`, sized by the [`BufferSpec`] |
/// | `Full` | [`FramebufferDisplay`] with two framebuffers | the driver's | render dirty areas into the back buffer, present, sync areas |
/// | `Direct` | [`FramebufferDisplay`] | the driver's | render dirty areas in place |
///
/// Partial buffers hold whole rows (at least one, and at least [`DisplayInfo::align`] rows),
/// start 4-byte aligned and are `'static` (see [`twine_hal::buffer`]).
///
/// # Choosing
///
/// - **Firmware:** declare the buffers with `twine::draw_buffers!` (size computed at compile
///   time, alignment guaranteed by the type, zeroed `.bss`) and pass them with
///   [`partial_double_from`](Self::partial_double_from) or
///   [`partial_single_from`](Self::partial_single_from).
/// - **Heap** (hosts, simulators, tests, MCUs with a large heap): [`alloc`](Self::alloc). The
///   engine allocates only after the display was accepted (colour format and size checked
///   first), so a refused display leaks nothing. Heap buffers are never chosen implicitly:
///   `Ui::builder` without buffers is an error.
/// - **Other memory** (linker sections, external RAM): [`partial_single`](Self::partial_single)
///   / [`partial_double`](Self::partial_double) with any `&'static mut [u8]`; misaligned memory
///   is refused with [`EngineError::BufferMisaligned`].
///
/// ```
/// use twine_engine::{BufferMode, BufferSpec};
/// use twine_hal::{DrawBuffer, buffer_bytes};
/// use twine_core::ColorFormat;
///
/// // What `twine::draw_buffers!` declares (here leaked from the heap to keep the example short).
/// const BYTES: usize = buffer_bytes(320, 40, ColorFormat::Rgb565);
/// let bufs: &'static mut [DrawBuffer<BYTES>; 2] = Box::leak(Box::new([DrawBuffer::ZEROED; 2]));
/// let mode = BufferMode::partial_double_from(bufs);
/// assert!(matches!(mode, BufferMode::Partial { b: Some(_), .. }));
///
/// let heap = BufferMode::alloc(BufferSpec::PartialSingle { rows: 20 });
/// assert!(matches!(heap, BufferMode::Alloc(BufferSpec::PartialSingle { rows: 20 })));
/// ```
#[derive(Debug)]
pub enum BufferMode {
    /// One or two partial draw buffers provided by the caller.
    Partial {
        /// The first buffer.
        a: DrawBufferMem,
        /// The optional second buffer (DMA ping-pong).
        b: Option<DrawBufferMem>,
    },
    /// Partial draw buffers the engine allocates on the heap (and never frees, like `'static`
    /// memory) once the display has been accepted.
    Alloc(BufferSpec),
    /// Two full framebuffers owned by a [`FramebufferDisplay`].
    Full,
    /// One full framebuffer owned by a [`FramebufferDisplay`], rendered in place.
    Direct,
}

impl BufferMode {
    /// Bytes of alignment padding each buffer of [`Alloc`](Self::Alloc) carries on the heap
    /// (a buffer of `n` bytes takes `n + ALLOC_PADDING`), as counted by
    /// [`Engine::memory_report`](crate::Engine::memory_report).
    pub const ALLOC_PADDING: usize = LEAK_PAD;

    /// One partial buffer from caller memory (any `&'static mut [u8]`; prefer
    /// [`partial_single_from`](Self::partial_single_from) for `draw_buffers!` statics). The
    /// alignment is checked when the display is added.
    #[must_use]
    pub fn partial_single(buf: &'static mut [u8]) -> Self {
        BufferMode::Partial {
            a: DrawBufferMem::new(buf),
            b: None,
        }
    }

    /// Two partial buffers from caller memory (rendering overlaps flushing); prefer
    /// [`partial_double_from`](Self::partial_double_from) for `draw_buffers!` statics. The
    /// alignment is checked when the display is added.
    #[must_use]
    pub fn partial_double(a: &'static mut [u8], b: &'static mut [u8]) -> Self {
        BufferMode::Partial {
            a: DrawBufferMem::new(a),
            b: Some(DrawBufferMem::new(b)),
        }
    }

    /// One partial buffer from a `twine::draw_buffers!` static of one buffer
    /// (`BUF.take()`), aligned by its type.
    ///
    /// ```
    /// use twine_engine::BufferMode;
    /// use twine_hal::DrawBuffer;
    ///
    /// let buf: &'static mut [DrawBuffer<1280>; 1] = Box::leak(Box::new([DrawBuffer::ZEROED; 1]));
    /// assert!(matches!(BufferMode::partial_single_from(buf), BufferMode::Partial { b: None, .. }));
    /// ```
    #[must_use]
    pub fn partial_single_from<const BYTES: usize>(bufs: &'static mut [DrawBuffer<BYTES>; 1]) -> Self {
        let [a] = bufs.each_mut();
        BufferMode::Partial { a: a.into(), b: None }
    }

    /// Two partial buffers from a `twine::draw_buffers!` static of two buffers
    /// (`BUFS.take()`), aligned by their type: rendering one overlaps flushing the other.
    ///
    /// ```
    /// use twine_engine::BufferMode;
    /// use twine_hal::DrawBuffer;
    ///
    /// let bufs: &'static mut [DrawBuffer<1280>; 2] = Box::leak(Box::new([DrawBuffer::ZEROED; 2]));
    /// assert!(matches!(BufferMode::partial_double_from(bufs), BufferMode::Partial { b: Some(_), .. }));
    /// ```
    #[must_use]
    pub fn partial_double_from<const BYTES: usize>(bufs: &'static mut [DrawBuffer<BYTES>; 2]) -> Self {
        let [a, b] = bufs.each_mut();
        BufferMode::Partial {
            a: a.into(),
            b: Some(b.into()),
        }
    }

    /// Partial buffers on the heap, allocated by the engine as `spec` describes once the
    /// display has been accepted (nothing is allocated for a refused display).
    ///
    /// ```
    /// use twine_engine::{BufferMode, BufferSpec};
    /// let mode = BufferMode::alloc(BufferSpec::PartialDouble { rows: 40 });
    /// assert!(matches!(mode, BufferMode::Alloc(_)));
    /// ```
    #[must_use]
    pub const fn alloc(spec: BufferSpec) -> Self {
        BufferMode::Alloc(spec)
    }

    /// Double framebuffer with area sync (framebuffer displays).
    #[must_use]
    pub const fn full() -> Self {
        BufferMode::Full
    }

    /// Single framebuffer rendered in place (framebuffer displays).
    #[must_use]
    pub const fn direct() -> Self {
        BufferMode::Direct
    }

    /// Bytes of the (smaller) partial buffer this mode provides for the display `info`:
    /// the caller's buffers' length, or what [`Alloc`](Self::Alloc) will allocate. Allocates
    /// nothing; used to size chunked rendering before the buffers exist.
    ///
    /// # Errors
    /// [`EngineError::BufferModeMismatch`] for `Full` / `Direct`;
    /// [`EngineError::BufferMisaligned`] for caller memory that is not 4-byte aligned.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{BufferMode, BufferSpec};
    /// use twine_hal::DisplayInfo;
    ///
    /// let info = DisplayInfo::new(64, 32, ColorFormat::Rgb565);
    /// let mode = BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 });
    /// assert_eq!(mode.partial_bytes(&info), Ok(64 * 2 * 8));
    /// assert!(BufferMode::full().partial_bytes(&info).is_err());
    /// ```
    pub fn partial_bytes(&self, info: &DisplayInfo) -> Result<usize, EngineError> {
        match self {
            BufferMode::Partial { a, b } => {
                check_aligned(a)?;
                if let Some(b) = b {
                    check_aligned(b)?;
                }
                Ok(b.as_ref().map_or(a.len(), |b| a.len().min(b.len())))
            }
            BufferMode::Alloc(spec) => Ok(spec.bytes_per_buffer(info)),
            BufferMode::Full | BufferMode::Direct => Err(EngineError::BufferModeMismatch),
        }
    }

    /// The partial buffers of this mode for the display `info`: the caller's (checked for
    /// alignment) or, for [`Alloc`](Self::Alloc), freshly allocated ones (zeroed, 4-byte
    /// aligned, leaked). Call it only once the display has been accepted, so that a refused
    /// display allocates nothing — [`Engine::add_display`] does; integrations that render
    /// through [`Engine::add_chunked_display`] call it after that succeeded.
    ///
    /// # Errors
    /// [`EngineError::BufferModeMismatch`] for `Full` / `Direct`;
    /// [`EngineError::BufferMisaligned`] for misaligned caller memory;
    /// [`EngineError::InvalidConfig`] for an `Alloc` on a display without pixels,
    /// [`EngineError::BufferTooSmall`] for an `Alloc` on a display whose row alignment exceeds
    /// its height. Nothing is allocated on error.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{BufferMode, BufferSpec};
    /// use twine_hal::DisplayInfo;
    ///
    /// let info = DisplayInfo::new(64, 32, ColorFormat::Rgb565);
    /// let (a, b) = BufferMode::alloc(BufferSpec::PartialDouble { rows: 8 }).into_partial(&info).unwrap();
    /// assert_eq!((a.len(), b.map(|b| b.len())), (64 * 2 * 8, Some(64 * 2 * 8)));
    /// assert!(a.is_aligned(4));
    /// ```
    pub fn into_partial(
        self,
        info: &DisplayInfo,
    ) -> Result<(DrawBufferMem, Option<DrawBufferMem>), EngineError> {
        match self {
            BufferMode::Partial { a, b } => {
                check_aligned(&a)?;
                if let Some(b) = &b {
                    check_aligned(b)?;
                }
                Ok((a, b))
            }
            BufferMode::Alloc(spec) => {
                let len = spec.bytes_per_buffer(info);
                if len == 0 {
                    return Err(EngineError::InvalidConfig("display has no pixels"));
                }
                let align = info.align.max(1);
                if u16::from(align) > info.height {
                    // Not even the whole screen holds `align` rows: refuse before allocating.
                    return Err(EngineError::BufferTooSmall {
                        needed: info.bytes_per_row() * usize::from(align),
                        got: len,
                    });
                }
                let b = (spec.buffer_count() == 2).then(|| leak_buffer(len));
                Ok((leak_buffer(len), b))
            }
            BufferMode::Full | BufferMode::Direct => Err(EngineError::BufferModeMismatch),
        }
    }
}

/// Refuses caller memory that is not 4-byte aligned (cold: buffers are checked once, when a
/// display is added).
fn check_aligned(m: &DrawBufferMem) -> Result<(), EngineError> {
    if m.is_aligned(DrawBuffer::<0>::ALIGN) {
        Ok(())
    } else {
        misaligned(m)
    }
}

#[cold]
#[inline(never)]
fn misaligned(m: &DrawBufferMem) -> Result<(), EngineError> {
    twine_core::error!(
        target: "twine::engine",
        "draw buffer at {:#x} is not 4-byte aligned: declare it with `draw_buffers!` or use `BufferMode::alloc`",
        m.addr()
    );
    Err(EngineError::BufferMisaligned)
}

/// A zeroed, 4-byte aligned heap buffer of `len` bytes, leaked (allocated once, like a
/// `'static` MCU buffer; at most 3 bytes of padding).
pub(crate) fn leak_buffer(len: usize) -> DrawBufferMem {
    let v: &'static mut [u8] = Box::leak(alloc::vec![0u8; len + LEAK_PAD].into_boxed_slice());
    let off = v.as_ptr().align_offset(4).min(LEAK_PAD);
    DrawBufferMem::new(&mut v[off..off + len])
}

/// Object-safe view of a [`DisplayDriver`] (errors mapped to [`EngineError::Driver`], with
/// the driver's error code).
pub(crate) trait FlushBackend {
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), EngineError>;
    fn poll_flush(&mut self) -> Option<DrawBufferMem>;
    fn wait_vsync(&mut self);
    fn idle(&mut self);
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<crate::DriverErrorCode>>;
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<crate::DriverErrorCode>>;
    fn set_rotation(
        &mut self,
        rotation: Rotation,
    ) -> Result<DisplayInfo, ControlError<crate::DriverErrorCode>>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: DisplayDriver + 'static> FlushBackend for D {
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<crate::DriverErrorCode>> {
        DisplayDriver::set_brightness(self, level).map_err(|e| self.control_code(e))
    }
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<crate::DriverErrorCode>> {
        DisplayDriver::sleep(self, sleep).map_err(|e| self.control_code(e))
    }
    fn set_rotation(
        &mut self,
        rotation: Rotation,
    ) -> Result<DisplayInfo, ControlError<crate::DriverErrorCode>> {
        DisplayDriver::set_rotation(self, rotation).map_err(|e| self.control_code(e))
    }
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), EngineError> {
        DisplayDriver::begin_flush(self, area, buf).map_err(|e| {
            let code = crate::DriverErrorCode::new(DisplayDriver::error_code(self, &e));
            let err = EngineError::driver(code, &e);
            if let EngineError::Driver { message, .. } = &err {
                twine_core::error!(target: "twine::driver", "flush {} failed ({}): {}", area, code, message.as_str());
            }
            err
        })
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        DisplayDriver::poll_flush(self)
    }
    fn wait_vsync(&mut self) {
        DisplayDriver::wait_vsync(self);
    }
    fn idle(&mut self) {
        DisplayDriver::idle(self);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Maps a driver's control error to its error code (cold: only on failure).
trait ControlCode: DisplayDriver {
    fn control_code(&self, e: ControlError<Self::Error>) -> ControlError<crate::DriverErrorCode> {
        e.map(|e| crate::DriverErrorCode::new(DisplayDriver::error_code(self, &e)))
    }
}

impl<D: DisplayDriver> ControlCode for D {}

/// Object-safe view of a [`FramebufferDisplay`].
pub(crate) trait FbBackend {
    fn present(&mut self, index: u8) -> Result<(), EngineError>;
    fn present_done(&mut self) -> bool;
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<crate::DriverErrorCode>>;
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<crate::DriverErrorCode>>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: FramebufferDisplay + 'static> FbBackend for D {
    fn present(&mut self, index: u8) -> Result<(), EngineError> {
        FramebufferDisplay::present(self, index).map_err(|e| {
            let code = crate::DriverErrorCode::new(FramebufferDisplay::error_code(self, &e));
            let err = EngineError::driver(code, &e);
            if let EngineError::Driver { message, .. } = &err {
                twine_core::error!(target: "twine::driver", "present {} failed ({}): {}", index, code, message.as_str());
            }
            err
        })
    }
    fn present_done(&mut self) -> bool {
        FramebufferDisplay::present_done(self)
    }
    fn set_brightness(&mut self, level: Fraction) -> Result<(), ControlError<crate::DriverErrorCode>> {
        FramebufferDisplay::set_brightness(self, level)
            .map_err(|e| e.map(|e| crate::DriverErrorCode::new(FramebufferDisplay::error_code(self, &e))))
    }
    fn sleep(&mut self, sleep: bool) -> Result<Duration, ControlError<crate::DriverErrorCode>> {
        FramebufferDisplay::sleep(self, sleep)
            .map_err(|e| e.map(|e| crate::DriverErrorCode::new(FramebufferDisplay::error_code(self, &e))))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The driver of a display.
pub(crate) enum Backend {
    Flush(Box<dyn FlushBackend>),
    Framebuffer(Box<dyn FbBackend>),
    /// No driver: the caller renders frames through the chunk-level refresh API
    /// ([`Engine::refresh_begin`], [`Engine::render_chunk`], [`Engine::refresh_end`]) and flushes
    /// the chunks itself (the async runtime).
    External(()),
}

impl Backend {
    fn as_any(&self) -> &dyn Any {
        match self {
            Backend::Flush(b) => b.as_any(),
            Backend::Framebuffer(b) => b.as_any(),
            Backend::External(u) => u,
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        match self {
            Backend::Flush(b) => b.as_any_mut(),
            Backend::Framebuffer(b) => b.as_any_mut(),
            Backend::External(u) => u,
        }
    }
}

/// One display: driver, refresher, layers and screens.
pub(crate) struct Display {
    pub(crate) id: DisplayId,
    pub(crate) backend: Backend,
    pub(crate) info: DisplayInfo,
    pub(crate) refresher: Refresher,
    pub(crate) bottom_layer: NodeId,
    pub(crate) top_layer: NodeId,
    pub(crate) sys_layer: NodeId,
    pub(crate) active_screen: NodeId,
    pub(crate) prev_screen: Option<NodeId>,
    pub(crate) screens: Vec<NodeId>,
    /// A screen load animation in progress (LVGL `scr_to_load`).
    pub(crate) screen_load: Option<crate::screen_anim::ScreenLoadRun>,
    /// The previous screen is drawn above the active one (LVGL `draw_prev_over_act`).
    pub(crate) draw_prev_over_act: bool,
    #[cfg(feature = "perf-monitor")]
    pub(crate) perf_overlay: Option<NodeId>,
    /// The display's theme (LVGL `lv_display_set_theme`).
    pub(crate) theme: Option<alloc::rc::Rc<dyn crate::ThemeHook>>,
    /// The theme's mode on this display.
    pub(crate) theme_mode: twine_style::ThemeMode,
    /// The theme's design element table for `theme_mode` (resolved by the style resolver).
    pub(crate) design: Option<alloc::rc::Rc<twine_style::design::ElementTable>>,
    /// Bumped whenever `theme`, `theme_mode` or `design` change.
    pub(crate) design_epoch: u32,
    /// Flush health (consecutive errors, halted).
    pub(crate) health: crate::health::HealthTracker,
    /// Brightness, sleep and rotation requests and state (see `display_control`).
    pub(crate) control: crate::display_control::ControlState,
    /// The partial draw buffers of the display (for `Engine::memory_report`).
    pub(crate) draw_buffers: DrawBufferUsage,
}

/// Bytes of a display's partial draw buffers by where they live (recorded once, when the
/// display is added). Framebuffers belong to their driver and chunked displays' buffers to
/// their caller: neither is counted here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DrawBufferUsage {
    /// Allocated by the engine ([`BufferMode::Alloc`]; alignment padding included).
    pub(crate) heap: usize,
    /// Caller memory ([`BufferMode::Partial`]).
    pub(crate) caller: usize,
}

impl DrawBufferUsage {
    fn of(heap: bool, a: &DrawBufferMem, b: Option<&DrawBufferMem>) -> Self {
        let bufs = [Some(a), b];
        let bufs = bufs.iter().flatten();
        if heap {
            Self {
                heap: bufs.map(|m| m.len() + LEAK_PAD).sum(),
                caller: 0,
            }
        } else {
            Self {
                heap: 0,
                caller: bufs.map(|m| m.len()).sum(),
            }
        }
    }
}

/// Extra bytes [`leak_buffer`] allocates to align a heap draw buffer to 4 bytes.
const LEAK_PAD: usize = 3;

impl Display {
    /// Whether `root` is currently shown (a layer, the active or the previous screen).
    pub(crate) fn shows_root(&self, root: NodeId) -> bool {
        root == self.active_screen
            || root == self.bottom_layer
            || root == self.top_layer
            || root == self.sys_layer
            || self.prev_screen == Some(root)
    }

    /// Whether `root` belongs to this display (a layer or a screen).
    pub(crate) fn owns_root(&self, root: NodeId) -> bool {
        root == self.bottom_layer
            || root == self.top_layer
            || root == self.sys_layer
            || self.screens.contains(&root)
    }

    /// The logical screen area.
    pub(crate) fn area(&self) -> Rect {
        self.info.area()
    }
}

impl Engine {
    /// Registers a display with an embedded frame memory (SPI/i80 panels). `buffers` must be
    /// [`BufferMode::Partial`] or [`BufferMode::Alloc`]. Creates the display's bottom layer,
    /// first screen, top layer and system layer, and schedules a full redraw. The first display
    /// becomes the default one.
    ///
    /// Everything that can refuse the display (kind, count, node budget, format, geometry) is
    /// checked before any buffer is allocated: a refused display allocates nothing for
    /// [`BufferMode::Alloc`].
    ///
    /// # Errors
    /// [`EngineError::BufferModeMismatch`] for `Full` / `Direct`;
    /// [`EngineError::TooManyDisplays`]; [`EngineError::TooManyNodes`] (and
    /// [`FaultKind::Capacity`] is raised) when [`EngineConfig::max_nodes`](crate::EngineConfig::max_nodes)
    /// leaves no room for the display's 4 layer and screen nodes; [`EngineError::FormatDisabled`] (and
    /// [`FaultKind::FormatDisabled`] is raised) when the renderer for the display's colour
    /// format is not compiled in; [`EngineError::BufferMisaligned`] for caller buffers that are
    /// not 4-byte aligned; [`EngineError::BufferTooSmall`] for buffers shorter than
    /// [`DisplayInfo::align`] rows (at least one); [`EngineError::InvalidConfig`] for a display
    /// without pixels.
    pub fn add_display(
        &mut self,
        driver: impl DisplayDriver + 'static,
        buffers: BufferMode,
    ) -> Result<DisplayId, EngineError> {
        if matches!(buffers, BufferMode::Full | BufferMode::Direct) {
            return Err(EngineError::BufferModeMismatch);
        }
        if self.displays.len() >= MAX_DISPLAYS {
            return Err(EngineError::TooManyDisplays);
        }
        self.check_display_nodes()?;
        let info = DisplayDriver::info(&driver);
        self.check_draw_format(&info, true)?;
        // F10b: heap buffers only for an accepted display (nothing below can refuse it for
        // lack of nodes: `check_display_nodes`).
        let heap = matches!(buffers, BufferMode::Alloc(_));
        let (a, b) = buffers.into_partial(&info)?;
        let usage = DrawBufferUsage::of(heap, &a, b.as_ref());
        let refresher = Refresher::new_partial(&info, a, b)?;
        self.push_display(Backend::Flush(Box::new(driver)), info, refresher, usage)
    }

    /// Registers a display without a driver, rendered through the chunk-level refresh API
    /// ([`refresh_begin`](Self::refresh_begin), [`render_chunk`](Self::render_chunk),
    /// [`refresh_end`](Self::refresh_end)): the caller owns the draw buffers (at least
    /// `chunk_bytes` each) and flushes each rendered chunk itself, e.g. through an async
    /// driver. Rotation, alignment and mono conversion work as with [`add_display`](Self::add_display).
    ///
    /// Fails with [`EngineError::FormatDisabled`] (and raises [`FaultKind::FormatDisabled`])
    /// when the renderer for the display's colour format is not compiled in.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_engine::{Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(64, 32, ColorFormat::Rgb565), 64 * 2 * 8).unwrap();
    /// let mut buf = vec![0u8; 64 * 2 * 8];
    /// assert!(e.refresh_begin(twine_core::Instant::from_millis(0)).is_some());
    /// let mut chunks = 0;
    /// while let Some(area) = e.render_chunk(&mut buf) {
    ///     assert_eq!(area.height(), 8); // 64 × 32 in 8-row chunks
    ///     chunks += 1;
    /// }
    /// e.refresh_end();
    /// assert_eq!((chunks, e.last_stats(d).chunks), (4, 4));
    /// ```
    pub fn add_chunked_display(
        &mut self,
        info: DisplayInfo,
        chunk_bytes: usize,
    ) -> Result<DisplayId, EngineError> {
        if self.displays.len() >= MAX_DISPLAYS {
            return Err(EngineError::TooManyDisplays);
        }
        self.check_display_nodes()?;
        self.check_draw_format(&info, true)?;
        let refresher = Refresher::new_external(&info, chunk_bytes)?;
        self.push_display(Backend::External(()), info, refresher, DrawBufferUsage::default())
    }

    /// Registers a memory-mapped display (LTDC, RGB, Linux fb). `buffers` must be
    /// [`BufferMode::Full`] (the driver must hand out two framebuffers) or
    /// [`BufferMode::Direct`]. Each framebuffer must hold exactly `width × height` pixels.
    /// Software rotation is not available in these modes.
    ///
    /// Fails with [`EngineError::FormatDisabled`] (and raises [`FaultKind::FormatDisabled`])
    /// when the renderer for the display's colour format is not compiled in.
    #[allow(clippy::needless_pass_by_value)] // symmetric with `add_display`, which consumes its buffers
    pub fn add_framebuffer_display(
        &mut self,
        mut driver: impl FramebufferDisplay + 'static,
        buffers: BufferMode,
    ) -> Result<DisplayId, EngineError> {
        let full = match buffers {
            BufferMode::Full => true,
            BufferMode::Direct => false,
            BufferMode::Partial { .. } | BufferMode::Alloc(_) => return Err(EngineError::BufferModeMismatch),
        };
        if self.displays.len() >= MAX_DISPLAYS {
            return Err(EngineError::TooManyDisplays);
        }
        self.check_display_nodes()?;
        let info = FramebufferDisplay::info(&driver);
        self.check_draw_format(&info, false)?;
        if info.rotation != Rotation::Deg0 && !info.hw_rotation {
            return Err(EngineError::InvalidConfig(
                "software rotation needs partial buffers",
            ));
        }
        if info.format == ColorFormat::I1 && info.width % 8 != 0 {
            return Err(EngineError::InvalidConfig(
                "I1 framebuffer width must be a multiple of 8",
            ));
        }
        let fbs = FramebufferDisplay::framebuffers(&mut driver).ok_or(EngineError::BufferModeMismatch)?;
        let refresher = Refresher::new_framebuffer(&info, fbs, full)?;
        self.push_display(
            Backend::Framebuffer(Box::new(driver)),
            info,
            refresher,
            DrawBufferUsage::default(),
        )
    }

    /// Refuses a display when the node budget ([`EngineConfig::max_nodes`](crate::EngineConfig::max_nodes),
    /// screens and layers included) has no room for its [`DISPLAY_NODES`] layer and screen
    /// nodes. Checked before anything is allocated for the display (F10b), so creating them
    /// in `push_display` cannot fail.
    fn check_display_nodes(&mut self) -> Result<(), EngineError> {
        if self.tree.len() + DISPLAY_NODES <= usize::from(self.config.max_nodes) {
            Ok(())
        } else {
            self.display_nodes_exhausted()
        }
    }

    #[cold]
    #[inline(never)]
    fn display_nodes_exhausted(&mut self) -> Result<(), EngineError> {
        twine_core::error!(
            target: "twine::engine",
            "display refused: {} live nodes, a display needs {} more (EngineConfig::max_nodes = {})",
            self.tree.len(),
            DISPLAY_NODES,
            self.config.max_nodes
        );
        self.raise_fault(FaultRecord::new(FaultKind::Capacity));
        Err(EngineError::TooManyNodes)
    }

    /// Refuses a display whose pixels the renderer cannot draw (see
    /// [`EngineError::FormatDisabled`]) and raises [`FaultKind::FormatDisabled`] for it
    /// (no display: it is not added; `code` = the draw format's LVGL discriminant). Runs once
    /// per registration and again for every rotation at run time (the driver reports a new
    /// description), never while rendering.
    pub(crate) fn check_draw_format(&mut self, info: &DisplayInfo, chunked: bool) -> Result<(), EngineError> {
        // Partial and chunked `I1` panels are drawn in `L8` and converted per chunk.
        let format = if chunked && info.format == ColorFormat::I1 {
            ColorFormat::L8
        } else {
            info.format
        };
        if twine_render::is_format_enabled(format) {
            return Ok(());
        }
        twine_core::error!(
            target: "twine::engine",
            "display {}x{} {} refused: drawing into {} is not compiled in (enable its `color-*` feature)",
            info.width,
            info.height,
            info.format,
            format
        );
        self.raise_fault(FaultRecord::new(FaultKind::FormatDisabled).code(u32::from(format as u8)));
        Err(EngineError::FormatDisabled(format))
    }

    fn push_display(
        &mut self,
        backend: Backend,
        info: DisplayInfo,
        refresher: Refresher,
        draw_buffers: DrawBufferUsage,
    ) -> Result<DisplayId, EngineError> {
        let id = DisplayId(self.displays.len() as u8);
        let area = info.area();
        // `check_display_nodes` reserved room for these `DISPLAY_NODES` nodes.
        let layer = |engine: &mut Engine, clickable: bool| -> Result<NodeId, EngineError> {
            let n = engine.tree.create(None, Box::new(Obj))?;
            if let Some(node) = engine.tree.node_mut(n) {
                node.coords = area;
                if !clickable {
                    node.flags.remove(ObjFlags::CLICKABLE);
                }
            }
            Ok(n)
        };
        let bottom_layer = layer(self, false)?;
        let active_screen = layer(self, true)?;
        let top_layer = layer(self, false)?;
        let sys_layer = layer(self, false)?;
        self.displays.push(Display {
            id,
            backend,
            info,
            refresher,
            bottom_layer,
            top_layer,
            sys_layer,
            active_screen,
            prev_screen: None,
            screens: alloc::vec![active_screen],
            screen_load: None,
            draw_prev_over_act: false,
            #[cfg(feature = "perf-monitor")]
            perf_overlay: None,
            theme: None,
            theme_mode: twine_style::ThemeMode::Light,
            design: None,
            design_epoch: 0,
            health: crate::health::HealthTracker::default(),
            control: crate::display_control::ControlState::new(&info),
            draw_buffers,
        });
        if self.default_display.is_none() {
            self.default_display = Some(id);
        }
        twine_core::info!(
            target: "twine::engine",
            "display {} added: {}x{} {} rotation {:?}{}",
            id,
            info.width,
            info.height,
            info.format,
            info.rotation,
            if info.hw_rotation { " (hw)" } else { "" }
        );
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        Ok(id)
    }

    fn display(&self, d: DisplayId) -> Option<&Display> {
        self.displays.get(d.index())
    }

    /// The first registered display.
    #[must_use]
    pub fn default_display(&self) -> Option<DisplayId> {
        self.default_display
    }

    /// Every registered display.
    pub fn displays(&self) -> impl Iterator<Item = DisplayId> + '_ {
        self.displays.iter().map(|d| d.id)
    }

    /// The description of `display`.
    #[must_use]
    pub fn display_info(&self, display: DisplayId) -> Option<DisplayInfo> {
        self.display(display).map(|d| d.info)
    }

    /// The driver of `display` as `D` (e.g. to read a test display's pixels).
    #[must_use]
    pub fn driver<D: 'static>(&self, display: DisplayId) -> Option<&D> {
        self.display(display)?.backend.as_any().downcast_ref::<D>()
    }

    /// The driver of `display` as `&mut D`.
    pub fn driver_mut<D: 'static>(&mut self, display: DisplayId) -> Option<&mut D> {
        self.displays
            .get_mut(display.index())?
            .backend
            .as_any_mut()
            .downcast_mut::<D>()
    }

    /// Creates a new (inactive) screen on `display`: a root [`Obj`] covering the display.
    pub fn create_screen(&mut self, display: DisplayId) -> Result<NodeId, EngineError> {
        let area = self
            .display(display)
            .ok_or(EngineError::DisplayNotFound(display))?
            .area();
        let s = self.tree.create(None, Box::new(Obj))?;
        if let Some(n) = self.tree.node_mut(s) {
            n.coords = area;
        }
        self.displays[display.index()].screens.push(s);
        self.apply_theme_on_create(s);
        Ok(s)
    }

    /// Makes `screen` the active screen of its display (instantly) and redraws the display
    /// (`ScreenUnloadStart`, `ScreenLoadStart`, `ScreenLoaded`, `ScreenUnloaded`). A screen load
    /// animation in progress is finished first. Same as
    /// [`load_screen_anim`](Self::load_screen_anim) with [`ScreenAnim::None`](crate::ScreenAnim::None).
    pub fn load_screen(&mut self, screen: NodeId) {
        self.load_screen_anim(screen, crate::ScreenAnim::None);
    }

    /// The instant screen switch of display `d` (LVGL `load_new_screen`).
    pub(crate) fn load_screen_now(&mut self, d: usize, screen: NodeId) {
        let old = self.displays[d].active_screen;
        if old == screen {
            return;
        }
        self.send_event(old, EventCode::ScreenUnloadStart, EventParam::None);
        self.send_event(screen, EventCode::ScreenLoadStart, EventParam::None);
        // Handlers may have loaded another screen or deleted this one.
        let Some(d) = self.displays.iter().position(|x| x.screens.contains(&screen)) else {
            return;
        };
        self.displays[d].active_screen = screen;
        let (id, area) = (self.displays[d].id, self.displays[d].area());
        twine_core::info!(target: "twine::engine", "display {}: screen {} loaded", id, fmt_node_id(screen));
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        // A press on the old screen must not continue on the new one.
        self.input_reset(None, None);
        self.send_event(screen, EventCode::ScreenLoaded, EventParam::None);
        if self.tree.contains(old) {
            self.send_event(old, EventCode::ScreenUnloaded, EventParam::None);
        }
    }

    /// The active screen of `display`.
    #[must_use]
    pub fn active_screen(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.active_screen)
    }

    /// The screens of `display`.
    #[must_use]
    pub fn screens(&self, display: DisplayId) -> &[NodeId] {
        self.display(display).map_or(&[], |d| &d.screens)
    }

    /// The layer below every screen (LVGL `lv_layer_bottom`).
    #[must_use]
    pub fn bottom_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.bottom_layer)
    }

    /// The layer above the screens, for popups (LVGL `lv_layer_top`).
    #[must_use]
    pub fn top_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.top_layer)
    }

    /// The topmost layer, for the performance overlay and cursors (LVGL `lv_layer_sys`).
    #[must_use]
    pub fn sys_layer(&self, display: DisplayId) -> Option<NodeId> {
        self.display(display).map(|d| d.sys_layer)
    }

    /// The display `node` belongs to (walks to the root).
    #[must_use]
    pub fn display_of(&self, node: NodeId) -> Option<DisplayId> {
        let root = self.tree.root_of(node)?;
        self.displays.iter().find(|d| d.owns_root(root)).map(|d| d.id)
    }

    /// Calls `wait_vsync` on the driver before the first flush of every frame of `display`
    /// (tear-effect synchronization).
    pub fn set_display_vsync(&mut self, display: DisplayId, on: bool) {
        match self.displays.get_mut(display.index()) {
            Some(d) => d.refresher.vsync = on,
            None => {
                twine_core::warn!(target: "twine::engine", "set_display_vsync: display {} not found", display);
            }
        }
    }

    /// The bytes of framebuffer `index` of a framebuffer display (`None` for other displays or
    /// while the buffer is lent out).
    #[must_use]
    pub fn framebuffer(&self, display: DisplayId, index: u8) -> Option<&[u8]> {
        self.display(display)?.refresher.framebuffer(index)
    }

    /// Creates a node for `widget` as the last child of `parent`, then calls `Widget::init`.
    /// Unknown parents log `warn!` and fail with [`EngineError::NodeNotFound`].
    pub fn create(&mut self, parent: NodeId, widget: Box<dyn Widget>) -> Result<NodeId, EngineError> {
        if !self.tree.contains(parent) {
            twine_core::warn!(target: "twine::engine", "create: parent {} not found", fmt_node_id(parent));
            return Err(EngineError::NodeNotFound(parent));
        }
        let id = self.tree_create(Some(parent), widget)?;
        self.mark_layout(id, crate::LayoutDirty::SELF);
        // LVGL `lv_obj_class_init_obj`: the theme first, then the constructor.
        self.apply_theme_on_create(id);
        self.init_widget(id);
        // The theme's and the constructor's style entries are all there: release the list's
        // growth slack (a node lives long; RAM is scarce on small devices).
        if let Some(n) = self.tree.node_mut(id) {
            n.styles.shrink_to_fit();
        }
        self.invalidate(id, InvalidateReason::Create);
        self.group_auto_add(id);
        if self.tree.contains(id) {
            self.send_event(id, EventCode::Create, EventParam::None);
        }
        if self.tree.contains(id) && self.tree.contains(parent) {
            // `target` is the new child, the handlers of the parent run.
            self.dispatch(id, parent, EventCode::ChildCreated, EventParam::None);
        }
        Ok(id)
    }

    /// [`Tree::create`](crate::Tree::create) within
    /// [`EngineConfig::max_nodes`](crate::EngineConfig::max_nodes), raising
    /// [`FaultKind::Capacity`] (with the parent) when the tree is full.
    fn tree_create(
        &mut self,
        parent: Option<NodeId>,
        widget: Box<dyn Widget>,
    ) -> Result<NodeId, EngineError> {
        let result = if self.tree.len() >= usize::from(self.config.max_nodes) {
            Err(EngineError::TooManyNodes)
        } else {
            self.tree.create(parent, widget)
        };
        result.inspect_err(|e| {
            if *e == EngineError::TooManyNodes {
                twine_core::error!(target: "twine::engine", "create: too many nodes");
                let r = FaultRecord::new(FaultKind::Capacity);
                self.raise_fault(if let Some(p) = parent { r.node(p) } else { r });
            }
        })
    }

    /// Creates a root node that belongs to no display (LVGL `lv_obj_create(NULL)` before the
    /// screen is used). Useful for tests and for building trees off-screen.
    pub fn create_root(&mut self, widget: Box<dyn Widget>) -> Result<NodeId, EngineError> {
        let id = self.tree_create(None, widget)?;
        self.init_widget(id);
        self.group_auto_add(id);
        if self.tree.contains(id) {
            self.send_event(id, EventCode::Create, EventParam::None);
        }
        Ok(id)
    }

    /// The widget of `id` as `W` (`None` for unknown ids or other widget types).
    #[must_use]
    pub fn widget<W: Widget>(&self, id: NodeId) -> Option<&W> {
        self.tree.get::<W>(id)
    }

    /// Calls `f` with the widget of `id` as `&mut W` and a [`WidgetCx`](crate::WidgetCx) for
    /// the node: the way to call a widget's setters (LVGL `lv_<widget>_set_*`). Returns `None`
    /// (and logs `warn!`) when the node does not exist or holds another widget type.
    ///
    /// While `f` runs the widget is taken out of its node (like during `Widget::init`);
    /// events the setter posts to the node ([`WidgetCx::post_event`](crate::WidgetCx::post_event),
    /// e.g. `ValueChanged`) are dispatched right after `f` returns, with the widget back.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// assert_eq!(e.with_widget_mut(n, |_o: &mut Obj, cx| cx.node()), Some(n));
    /// ```
    pub fn with_widget_mut<W: Widget, R>(
        &mut self,
        id: NodeId,
        f: impl FnOnce(&mut W, &mut crate::WidgetCx<'_>) -> R,
    ) -> Option<R> {
        let mut w = self.take_widget(id, core::any::TypeId::of::<W>())?;
        let r = w
            .downcast_mut::<W>()
            .map(|w| f(w, &mut crate::WidgetCx::new(self, id)));
        self.restore_widget(id, w);
        r
    }

    /// Takes the widget of `id` out of its node (leaving a `Detached` placeholder) if it is
    /// of type `ty`; logs `warn!` and returns `None` otherwise. The non-generic half of
    /// [`with_widget_mut`](Self::with_widget_mut), which is instantiated once per setter
    /// closure: the lookup and the diagnostics are shared (a direct call, no indirection).
    #[inline(never)]
    fn take_widget(&mut self, id: NodeId, ty: core::any::TypeId) -> Option<Box<dyn Widget>> {
        let Some(n) = self.tree.node_mut(id) else {
            twine_core::warn!(target: "twine::engine", "with_widget_mut: node {} not found", fmt_node_id(id));
            return None;
        };
        if Any::type_id((*n.widget).as_any()) != ty {
            twine_core::warn!(
                target: "twine::engine",
                "with_widget_mut: node {} is a {}, not the requested widget type",
                fmt_node_id(id),
                n.class().name
            );
            return None;
        }
        Some(core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached)))
    }

    fn init_widget(&mut self, id: NodeId) {
        let Some(n) = self.tree.node_mut(id) else {
            return;
        };
        let mut w: Box<dyn Widget> = core::mem::replace(&mut n.widget, Box::new(crate::obj::Detached));
        w.init(&mut crate::WidgetCx::new(self, id));
        self.restore_widget(id, w);
    }

    /// Deletes `id` and its subtree. Every node of the subtree receives `Delete` (children
    /// before their parent) while it still exists; then the nodes leave their focus groups,
    /// the area is invalidated, the nodes are freed and the parent receives `ChildDeleted`
    /// (with the deleted node as target). Layers and active screens cannot be deleted. The
    /// animations and style transitions of the deleted nodes stop.
    pub fn delete(&mut self, id: NodeId) -> Result<(), EngineError> {
        if !self.tree.contains(id) {
            twine_core::warn!(target: "twine::engine", "delete: node {} not found", fmt_node_id(id));
            return Err(EngineError::NodeNotFound(id));
        }
        for d in &self.displays {
            if id == d.bottom_layer || id == d.top_layer || id == d.sys_layer || id == d.active_screen {
                twine_core::warn!(target: "twine::engine", "delete: {} is a layer or the active screen", fmt_node_id(id));
                return Err(EngineError::InvalidConfig(
                    "cannot delete a layer or the active screen",
                ));
            }
        }
        let order = self.tree.post_order(id);
        for &n in &order {
            if self.tree.contains(n) {
                self.send_event(n, EventCode::Delete, EventParam::None);
            }
        }
        if !self.tree.contains(id) {
            return Ok(()); // a `Delete` handler deleted it already
        }
        for &n in &order {
            if self.tree.node(n).is_some_and(|x| x.group.is_some()) {
                self.group_remove(n);
            }
        }
        if !self.tree.contains(id) {
            return Ok(());
        }
        let parent = self.tree.parent(id);
        self.invalidate_subtree(id, InvalidateReason::Delete);
        let deleted = self.tree.delete(id)?;
        self.anims_forget_nodes(&deleted);
        self.screen_anims_forget(&deleted);
        self.forget_outside_presses();
        for d in &mut self.displays {
            d.screens.retain(|s| *s != id);
            if d.prev_screen == Some(id) {
                d.prev_screen = None;
            }
            #[cfg(feature = "perf-monitor")]
            if d.perf_overlay.is_some_and(|p| !self.tree.contains(p)) {
                d.perf_overlay = None;
            }
        }
        if let Some(p) = parent.filter(|p| self.tree.contains(*p)) {
            self.mark_layout(p, crate::LayoutDirty::CHILDREN);
            self.layout.readjust.push(p);
            self.scrollbar_invalidate_tracks(p);
            self.dispatch(id, p, EventCode::ChildDeleted, EventParam::None);
        }
        Ok(())
    }
}
