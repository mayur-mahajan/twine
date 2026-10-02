//! Display power and rotation at run time: brightness, sleep / wake and rotation requests
//! ([`Engine::set_display_brightness`], [`Engine::set_display_sleep`], [`Engine::set_rotation`],
//! [`Engine::display_command`] with a [`DisplayCmd`]), applied to the driver between frames.
//!
//! How requests are applied, the asleep state, rotation and faults: see [`DisplayCmd`].

use twine_core::fault::FaultKind;
use twine_core::{Duration, Fraction, Instant, RectSet, Rotation};
use twine_hal::{ControlError, DisplayInfo};

use crate::display::Backend;
use crate::refresh::{Refit, Strategy};
use crate::{DisplayId, DriverErrorCode, Engine, EngineError, FaultRecord, InvalidateReason, LayoutDirty};

/// A display command from any context (a task, an interrupt, another thread): send it through
/// a `Channel<DisplayCmd, N>` that the view layer's `UiBuilder::display_commands` registers, or
/// apply it directly with [`Engine::display_command`] (or the `Engine::set_*` method it names).
///
/// # When requests are applied
///
/// A request is recorded on the display (the latest one of each kind wins) and applied by the
/// next step at a *safe point*: no frame in progress and no flush in flight, so a control
/// command never interleaves with a pixel transfer and a rotation never changes the geometry
/// of a frame half drawn. A display whose driver is busy is looked at again 1 ms later; a step
/// that has nothing else to do does not wait for anything. Displays added with
/// [`Engine::add_chunked_display`] have no driver in the engine: their owner (the view layer's
/// `AsyncUi`) takes the requests with [`Engine::take_display_requests`], calls its async
/// driver and reports with [`Engine::complete_display_requests`].
///
/// Requests are applied in this order: rotation, brightness, then sleep. A wake is applied
/// alone: the other requests wait until the panel's settle time (the `Duration` the driver's
/// `sleep` returned, e.g. 120 ms after MIPI DCS `SLPOUT`) has passed; the engine sends
/// nothing to the driver before then.
///
/// # Asleep
///
/// A display put to sleep is not drawn: invalidations accumulate and are drawn after the wake
/// (the panel keeps its frame memory in sleep mode); it no longer keeps the UI awake. This
/// holds even when the driver cannot sleep ([`DisplayControlFault::SleepUnsupported`] is
/// raised), so "sleep" always stops the drawing work.
///
/// # Rotation
///
/// The driver is asked first ([`DisplayDriver::set_rotation`](twine_hal::DisplayDriver::set_rotation),
/// hardware rotation). When it does not support it, the engine rotates each chunk in software,
/// which needs scratch memory reserved once at start-up with [`Engine::reserve_rotation`]:
/// a rotation never allocates. The new description is checked like a new display (draw format,
/// buffers holding at least `align` rows of the new width; fewer rows per chunk are fine), then
/// the screens and layers take the new size and are laid out again, the whole display is
/// redrawn and the display's input devices are fitted to it again
/// ([`InputDevice::fit_to_display`](twine_hal::InputDevice::fit_to_display)). A rotation that
/// does not fit is refused (the display keeps its rotation; a hardware rotation is undone) and
/// [`FaultKind::DisplayControl`] is raised. Framebuffer displays cannot be rotated at run time.
///
/// # Faults
///
/// Every request the display could not apply raises [`FaultKind::DisplayControl`] with the
/// display and a [`DisplayControlFault`] code, and logs an `error!`/`warn!`.
///
/// ```
/// use twine_core::{Fraction, Rotation};
/// use twine_engine::DisplayCmd;
///
/// let cmds = [DisplayCmd::Brightness(Fraction::pct(30)), DisplayCmd::Sleep, DisplayCmd::Wake, DisplayCmd::Rotate(Rotation::Deg90)];
/// assert_eq!(cmds.len(), 4);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DisplayCmd {
    /// Set the panel brightness ([`Engine::set_display_brightness`]).
    Brightness(Fraction),
    /// Put the display to sleep: the panel sleeps and nothing is drawn
    /// ([`Engine::set_display_sleep`] with `true`).
    Sleep,
    /// Wake the display and draw what changed meanwhile ([`Engine::set_display_sleep`] with
    /// `false`).
    Wake,
    /// Rotate the display ([`Engine::set_rotation`]).
    Rotate(Rotation),
}

/// The `code` of a [`FaultKind::DisplayControl`] fault: which request failed, and why.
///
/// ```
/// use twine_engine::DisplayControlFault;
/// let f = DisplayControlFault::RotationNotReserved;
/// assert_eq!(DisplayControlFault::from_code(f.code()), Some(f));
/// assert_eq!(DisplayControlFault::from_code(0), None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u32)]
#[non_exhaustive]
pub enum DisplayControlFault {
    /// The driver cannot set the brightness (e.g. an LCD dimmed by its backlight pin, which the
    /// application drives).
    BrightnessUnsupported = 1,
    /// The driver failed to set the brightness.
    BrightnessFailed = 2,
    /// The driver cannot sleep; the display is not drawn while "asleep" anyway.
    SleepUnsupported = 3,
    /// The driver failed to put the panel to sleep; the display is not drawn while "asleep".
    SleepFailed = 4,
    /// The driver failed to wake the panel; the display is drawn again anyway (the panel may
    /// stay dark until it recovers).
    WakeFailed = 5,
    /// The driver failed to rotate; the display keeps its rotation.
    RotationFailed = 6,
    /// The rotation needs software rotation and its scratch memory was not reserved
    /// ([`Engine::reserve_rotation`]); the display keeps its rotation.
    RotationNotReserved = 7,
    /// The rotated display does not fit its buffers or its draw format is not compiled in (a
    /// [`FaultKind::FormatDisabled`] fault is raised as well); the display keeps its rotation.
    RotationRefused = 8,
}

impl DisplayControlFault {
    const ALL: [DisplayControlFault; 8] = [
        Self::BrightnessUnsupported,
        Self::BrightnessFailed,
        Self::SleepUnsupported,
        Self::SleepFailed,
        Self::WakeFailed,
        Self::RotationFailed,
        Self::RotationNotReserved,
        Self::RotationRefused,
    ];

    /// The fault record's `code`.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The fault for a record's `code` (`None` for unknown codes).
    #[must_use]
    pub fn from_code(code: u32) -> Option<DisplayControlFault> {
        Self::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// Requests waiting to be applied to a display's driver (the latest of each kind).
///
/// The engine applies them itself for displays with a driver; for a chunked display its owner
/// takes them with [`Engine::take_display_requests`] and applies them to its async driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisplayRequests {
    /// A new brightness.
    pub brightness: Option<Fraction>,
    /// Sleep (`true`) or wake (`false`).
    pub sleep: Option<bool>,
    /// A new rotation.
    pub rotation: Option<Rotation>,
}

impl DisplayRequests {
    /// Whether nothing is requested.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.brightness.is_none() && self.sleep.is_none() && self.rotation.is_none()
    }
}

/// What the driver answered to the [`DisplayRequests`] taken with
/// [`Engine::take_display_requests`]: one entry per request, in the driver's words
/// (errors as their [`DriverErrorCode`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayResponses {
    /// The answer of `set_brightness`.
    pub brightness: Option<Result<(), ControlError<DriverErrorCode>>>,
    /// The answer of `sleep` (the settle time).
    pub sleep: Option<Result<Duration, ControlError<DriverErrorCode>>>,
    /// The answer of `set_rotation` (the new description).
    pub rotation: Option<Result<DisplayInfo, ControlError<DriverErrorCode>>>,
}

/// Per-display control state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ControlState {
    req: DisplayRequests,
    pub(crate) asleep: bool,
    brightness: Option<Fraction>,
    /// Nothing may be sent to the driver before this instant (sleep settle time).
    pub(crate) ready_at: Option<Instant>,
    /// The rotation of the driver's flush coordinates: what it rotates in hardware (`Deg0` when
    /// flush areas are native). Software rotation does the rest.
    base: Rotation,
}

impl ControlState {
    pub(crate) fn new(info: &DisplayInfo) -> Self {
        Self {
            req: DisplayRequests::default(),
            asleep: false,
            brightness: None,
            ready_at: None,
            base: if info.hw_rotation {
                info.rotation
            } else {
                Rotation::Deg0
            },
        }
    }

    /// Whether the refresh must look at the control state (one load per display and step).
    #[inline]
    pub(crate) fn needs_attention(&self) -> bool {
        self.asleep || self.ready_at.is_some() || !self.req.is_empty()
    }

    /// Whether requests wait.
    #[inline]
    pub(crate) fn pending(&self) -> bool {
        !self.req.is_empty()
    }

    /// Whether the display is asleep or settling (nothing may be drawn).
    pub(crate) fn blocks_drawing(&self, now: Instant) -> bool {
        self.asleep || self.ready_at.is_some_and(|t| now < t)
    }

    /// Takes the requests to apply now (see the module docs: a wake goes alone).
    fn take(&mut self) -> DisplayRequests {
        if self.req.sleep == Some(false) && self.asleep {
            self.req.sleep = None;
            return DisplayRequests {
                sleep: Some(false),
                ..DisplayRequests::default()
            };
        }
        let mut r = core::mem::take(&mut self.req);
        // Sleep while asleep / wake while awake: nothing to do.
        if r.sleep == Some(self.asleep) {
            r.sleep = None;
        }
        r
    }
}

impl Engine {
    fn control_display(&mut self, display: DisplayId, what: &str) -> Result<&mut ControlState, EngineError> {
        if let Some(d) = self.displays.get_mut(display.index()) {
            Ok(&mut d.control)
        } else {
            twine_core::warn!(target: "twine::engine", "{}: display {} not found", what, display);
            Err(EngineError::DisplayNotFound(display))
        }
    }

    /// Requests the panel brightness `level` (`Fraction::ZERO` darkest, `ONE` brightest) of
    /// `display`: applied by the next step between frames (see the [`DisplayCmd`] docs).
    /// Idempotent. A driver without brightness control raises [`FaultKind::DisplayControl`]
    /// ([`DisplayControlFault::BrightnessUnsupported`]).
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`].
    ///
    /// ```
    /// use twine_core::{ColorFormat, Fraction, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.set_display_brightness(d, Fraction::pct(40)).unwrap();
    /// e.step(Instant::from_millis(0));
    /// assert_eq!(e.display_brightness(d), Some(Fraction::pct(40)));
    /// assert_eq!(e.driver::<MemoryDisplay>(d).unwrap().brightness(), Some(Fraction::pct(40)));
    /// ```
    pub fn set_display_brightness(&mut self, display: DisplayId, level: Fraction) -> Result<(), EngineError> {
        let c = self.control_display(display, "set_display_brightness")?;
        c.req.brightness = (c.brightness != Some(level)).then_some(level);
        Ok(())
    }

    /// Requests sleep (`true`) or wake (`false`) for `display`, applied by the next step (see
    /// the [`DisplayCmd`] docs): asleep, the panel sleeps and nothing is drawn; after a wake,
    /// what changed meanwhile is drawn once the panel's settle time has passed. Idempotent.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`].
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig, Wake};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.set_display_sleep(d, true).unwrap();
    /// // The first frame is drawn before the request is taken, then the panel sleeps.
    /// let wake = e.step(Instant::from_millis(0));
    /// assert!(e.display_asleep(d));
    /// assert!(wake.is_idle());
    /// ```
    pub fn set_display_sleep(&mut self, display: DisplayId, sleep: bool) -> Result<(), EngineError> {
        let c = self.control_display(display, "set_display_sleep")?;
        c.req.sleep = (c.asleep != sleep || c.req.sleep.is_some()).then_some(sleep);
        Ok(())
    }

    /// Requests the rotation `rotation` for `display`, applied by the next step between frames:
    /// hardware rotation through the driver, or software rotation (needs
    /// [`reserve_rotation`](Self::reserve_rotation)); then a re-layout, a full redraw and the
    /// input devices fitted again (see the [`DisplayCmd`] docs). Idempotent: the current
    /// rotation cancels a pending request. Allocates nothing.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`]; [`EngineError::InvalidConfig`] for a framebuffer
    /// display (they cannot be rotated at run time). A rotation that turns out not to fit
    /// when it is applied raises [`FaultKind::DisplayControl`] instead.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant, Rotation};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_rotation_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.set_rotation(d, Rotation::Deg90).unwrap();
    /// e.step(Instant::from_millis(0));
    /// let info = e.display_info(d).unwrap();
    /// assert_eq!((info.width, info.height, info.rotation), (32, 64, Rotation::Deg90));
    /// ```
    pub fn set_rotation(&mut self, display: DisplayId, rotation: Rotation) -> Result<(), EngineError> {
        let Some(d) = self.displays.get_mut(display.index()) else {
            twine_core::warn!(target: "twine::engine", "set_rotation: display {} not found", display);
            return Err(EngineError::DisplayNotFound(display));
        };
        if matches!(d.backend, Backend::Framebuffer(_)) {
            twine_core::warn!(target: "twine::engine", "set_rotation: display {} is a framebuffer display", display);
            return Err(EngineError::InvalidConfig(
                "framebuffer displays cannot be rotated at run time",
            ));
        }
        d.control.req.rotation = (d.info.rotation != rotation).then_some(rotation);
        Ok(())
    }

    /// Applies a [`DisplayCmd`] to `display` (the `set_*` method it names).
    ///
    /// # Errors
    /// As the method it calls.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, DisplayCmd, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.display_command(d, DisplayCmd::Sleep).unwrap();
    /// e.step(Instant::from_millis(0));
    /// assert!(e.display_asleep(d));
    /// ```
    pub fn display_command(&mut self, display: DisplayId, cmd: DisplayCmd) -> Result<(), EngineError> {
        match cmd {
            DisplayCmd::Brightness(level) => self.set_display_brightness(display, level),
            DisplayCmd::Sleep => self.set_display_sleep(display, true),
            DisplayCmd::Wake => self.set_display_sleep(display, false),
            DisplayCmd::Rotate(r) => self.set_rotation(display, r),
        }
    }

    /// Whether `display` is asleep (a sleep request was applied, and no wake since). `false`
    /// for unknown displays. Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// assert!(!e.display_asleep(d));
    /// e.set_display_sleep(d, true).unwrap();
    /// assert!(!e.display_asleep(d)); // requested, applied by the next step
    /// e.step(Instant::from_millis(0));
    /// assert!(e.display_asleep(d));
    /// ```
    #[must_use]
    pub fn display_asleep(&self, display: DisplayId) -> bool {
        self.displays
            .get(display.index())
            .is_some_and(|d| d.control.asleep)
    }

    /// The brightness last applied to `display` (`None` before any, or for unknown displays).
    /// Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Fraction, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// assert_eq!(e.display_brightness(d), None); // nothing applied yet
    /// e.set_display_brightness(d, Fraction::HALF).unwrap();
    /// e.step(Instant::from_millis(0));
    /// assert_eq!(e.display_brightness(d), Some(Fraction::HALF));
    /// ```
    #[must_use]
    pub fn display_brightness(&self, display: DisplayId) -> Option<Fraction> {
        self.displays
            .get(display.index())
            .and_then(|d| d.control.brightness)
    }

    /// Whether requests for `display` wait to be applied (`false` for unknown displays).
    /// Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Fraction, Instant};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control();
    /// let d = e.add_display(panel, BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.set_display_brightness(d, Fraction::HALF).unwrap();
    /// assert!(e.display_requests_pending(d));
    /// e.step(Instant::from_millis(0));
    /// assert!(!e.display_requests_pending(d));
    /// ```
    #[must_use]
    pub fn display_requests_pending(&self, display: DisplayId) -> bool {
        self.displays
            .get(display.index())
            .is_some_and(|d| d.control.pending())
    }

    /// Reserves, once, the memory software rotation of `display` needs in **every** rotation:
    /// rotation scratch buffers sized for its draw buffers (and, for mono and chunked displays,
    /// their shadow buffers). Call it at start-up for a display that may be rotated at run time
    /// by a driver without hardware rotation; later [`set_rotation`](Self::set_rotation) calls
    /// then allocate nothing. Allocates nothing for memory the display already has (a display
    /// added in software rotation has it for that geometry). Without it, such a rotation is
    /// refused ([`DisplayControlFault::RotationNotReserved`]) — memory is never allocated
    /// silently while the UI runs.
    ///
    /// # Errors
    /// [`EngineError::DisplayNotFound`]; [`EngineError::InvalidConfig`] for a framebuffer
    /// display.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Instant, Rotation};
    /// use twine_engine::{BufferMode, BufferSpec, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// // A panel without hardware rotation.
    /// let d = e.add_display(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)), BufferMode::alloc(BufferSpec::default())).unwrap();
    /// e.reserve_rotation(d).unwrap();
    /// e.set_rotation(d, Rotation::Deg270).unwrap();
    /// e.step(Instant::from_millis(0));
    /// assert_eq!(e.display_info(d).unwrap().rotation, Rotation::Deg270);
    /// ```
    pub fn reserve_rotation(&mut self, display: DisplayId) -> Result<(), EngineError> {
        let Some(d) = self.displays.get_mut(display.index()) else {
            return Err(EngineError::DisplayNotFound(display));
        };
        let Strategy::Partial(p) = &mut d.refresher.strategy else {
            return Err(EngineError::InvalidConfig(
                "framebuffer displays cannot be rotated at run time",
            ));
        };
        let info = d.info;
        let (w, h) = info.native_size();
        for (w, h) in [(w, h), (h, w)] {
            p.reserve(&DisplayInfo {
                width: w,
                height: h,
                ..info
            });
        }
        twine_core::info!(target: "twine::engine", "display {}: software rotation reserved", display);
        Ok(())
    }

    /// For a display added with [`add_chunked_display`](Self::add_chunked_display): takes the
    /// requests to apply now, if any and if the display is ready for them (no frame in
    /// progress, the panel's settle time passed). The caller applies them to its driver in the
    /// order rotation, brightness, sleep, and reports with
    /// [`complete_display_requests`](Self::complete_display_requests) before the next frame.
    /// `None` for other displays (the engine applies their requests itself).
    ///
    /// ```
    /// use twine_core::{ColorFormat, Fraction, Instant};
    /// use twine_engine::{DisplayResponses, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(64, 32, ColorFormat::Rgb565), 64 * 2 * 8).unwrap();
    /// e.set_display_brightness(d, Fraction::HALF).unwrap();
    /// let now = Instant::from_millis(0);
    /// let req = e.take_display_requests(d, now).unwrap();
    /// assert_eq!(req.brightness, Some(Fraction::HALF));
    /// // … `display.set_brightness(level).await` …
    /// let resp = DisplayResponses { brightness: Some(Ok(())), ..DisplayResponses::default() };
    /// assert_eq!(e.complete_display_requests(d, req, resp, now), None);
    /// assert_eq!(e.display_brightness(d), Some(Fraction::HALF));
    /// ```
    pub fn take_display_requests(&mut self, display: DisplayId, now: Instant) -> Option<DisplayRequests> {
        let d = self.displays.get_mut(display.index())?;
        if !matches!(d.backend, Backend::External(())) || !d.control.pending() || d.refresher.job.is_some() {
            return None;
        }
        if d.control.ready_at.is_some_and(|t| now < t) {
            return None;
        }
        d.control.ready_at = None;
        let r = d.control.take();
        (!r.is_empty()).then_some(r)
    }

    /// Applies the driver's answers to the requests taken with
    /// [`take_display_requests`](Self::take_display_requests) (see the [`DisplayCmd`] docs):
    /// records brightness and sleep state, the settle time, and for a rotation re-checks,
    /// re-lays out and redraws the display, raising [`FaultKind::DisplayControl`] for what
    /// failed. Returns the rotation the caller must restore on its driver when a hardware
    /// rotation had to be refused (the engine keeps the old one), `None` otherwise. Unknown
    /// displays are ignored (`None`). Never panics.
    ///
    /// ```
    /// use twine_core::{ColorFormat, Duration, Instant};
    /// use twine_engine::{DisplayResponses, Engine, EngineConfig};
    /// use twine_hal::DisplayInfo;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let d = e.add_chunked_display(DisplayInfo::new(64, 32, ColorFormat::Rgb565), 64 * 2 * 8).unwrap();
    /// e.set_display_sleep(d, true).unwrap();
    /// let now = Instant::from_millis(0);
    /// let req = e.take_display_requests(d, now).unwrap();
    /// assert_eq!(req.sleep, Some(true));
    /// // … `display.sleep(true).await` answered with a 5 ms settle time …
    /// let resp = DisplayResponses { sleep: Some(Ok(Duration::ms(5))), ..DisplayResponses::default() };
    /// assert_eq!(e.complete_display_requests(d, req, resp, now), None);
    /// assert!(e.display_asleep(d));
    /// ```
    pub fn complete_display_requests(
        &mut self,
        display: DisplayId,
        requests: DisplayRequests,
        responses: DisplayResponses,
        now: Instant,
    ) -> Option<Rotation> {
        let d = display.index();
        if d >= self.displays.len() {
            return None;
        }
        let mut revert = None;
        if let (Some(r), Some(res)) = (requests.rotation, responses.rotation) {
            revert = self.finish_rotation(d, r, res);
        }
        if let (Some(level), Some(res)) = (requests.brightness, responses.brightness) {
            match res {
                Ok(()) => self.displays[d].control.brightness = Some(level),
                Err(e) => {
                    let f = if e.is_unsupported() {
                        DisplayControlFault::BrightnessUnsupported
                    } else {
                        DisplayControlFault::BrightnessFailed
                    };
                    self.control_fault(d, f, e);
                }
            }
        }
        if let (Some(sleep), Some(res)) = (requests.sleep, responses.sleep) {
            let settle = match res {
                Ok(t) => t,
                Err(e) => {
                    let f = match (sleep, e.is_unsupported()) {
                        (true, true) => DisplayControlFault::SleepUnsupported,
                        (true, false) => DisplayControlFault::SleepFailed,
                        (false, _) => DisplayControlFault::WakeFailed,
                    };
                    self.control_fault(d, f, e);
                    Duration::ZERO
                }
            };
            let disp = &mut self.displays[d];
            disp.control.asleep = sleep;
            disp.control.ready_at = (settle > Duration::ZERO).then(|| now + settle);
            twine_core::info!(target: "twine::engine", "display {} {}", disp.id, if sleep { "asleep" } else { "awake" });
        }
        revert
    }

    /// Raises the `DisplayControl` fault of display `d`.
    #[cold]
    #[inline(never)]
    fn control_fault(&mut self, d: usize, f: DisplayControlFault, e: ControlError<DriverErrorCode>) {
        let id = self.displays[d].id;
        match e {
            ControlError::Unsupported => {
                twine_core::warn!(target: "twine::engine", "display {}: {:?} (driver)", id, f);
            }
            ControlError::Driver(code) => {
                twine_core::error!(target: "twine::engine", "display {}: {:?} (driver error {})", id, f, code);
            }
        }
        self.raise_fault(
            FaultRecord::new(FaultKind::DisplayControl)
                .display(id)
                .code(f.code()),
        );
    }

    /// Applies the driver's answer to a rotation request: returns the rotation to restore on
    /// the driver when a hardware rotation is refused.
    fn finish_rotation(
        &mut self,
        d: usize,
        rotation: Rotation,
        res: Result<DisplayInfo, ControlError<DriverErrorCode>>,
    ) -> Option<Rotation> {
        let old = self.displays[d].info;
        let base_old = self.displays[d].control.base;
        let (info, base, hw) = match res {
            Ok(i) if i.rotation == rotation => {
                let base = if i.hw_rotation { i.rotation } else { Rotation::Deg0 };
                (i, base, true)
            }
            Ok(i) => {
                twine_core::error!(target: "twine::engine", "display {}: driver rotated to {:?}, not {:?}", self.displays[d].id, i.rotation, rotation);
                self.control_fault(
                    d,
                    DisplayControlFault::RotationFailed,
                    ControlError::Driver(DriverErrorCode::NONE),
                );
                return Some(old.rotation);
            }
            Err(ControlError::Unsupported) => {
                let (nw, nh) = old.native_size();
                let (width, height) = if rotation.swaps_axes() { (nh, nw) } else { (nw, nh) };
                let delta = rotation.relative_to(base_old);
                let info = DisplayInfo {
                    width,
                    height,
                    rotation,
                    hw_rotation: delta == Rotation::Deg0,
                    ..old
                };
                (info, base_old, false)
            }
            Err(e) => {
                self.control_fault(d, DisplayControlFault::RotationFailed, e);
                return None;
            }
        };
        let revert = hw.then_some(old.rotation);
        let chunked = !matches!(self.displays[d].backend, Backend::Framebuffer(_));
        if self.check_draw_format(&info, chunked).is_err() {
            self.control_fault(d, DisplayControlFault::RotationRefused, ControlError::Unsupported);
            return revert;
        }
        let delta = rotation.relative_to(base);
        let refit = match &mut self.displays[d].refresher.strategy {
            Strategy::Partial(p) => p.refit(&info, delta),
            _ => Err(Refit::TooSmall),
        };
        if let Err(why) = refit {
            let f = match why {
                Refit::NotReserved => DisplayControlFault::RotationNotReserved,
                Refit::TooSmall => DisplayControlFault::RotationRefused,
            };
            if f == DisplayControlFault::RotationNotReserved {
                twine_core::error!(target: "twine::engine", "display {}: software rotation needs `Engine::reserve_rotation` at start-up", self.displays[d].id);
            }
            self.control_fault(d, f, ControlError::Unsupported);
            return revert;
        }
        self.displays[d].control.base = base;
        self.apply_geometry(d, info);
        None
    }

    /// Commits a new description of display `d` (already checked): screens and layers take
    /// the new size and are laid out again, the whole display is redrawn and its input
    /// devices are fitted to it again.
    fn apply_geometry(&mut self, d: usize, info: DisplayInfo) {
        if self.displays[d].screen_load.is_some() {
            self.finish_screen_anim(d);
        }
        let disp = &mut self.displays[d];
        let id = disp.id;
        disp.info = info;
        let area = info.area();
        disp.refresher.job = None;
        disp.refresher.dirty = RectSet::new(area);
        disp.refresher.overlay_dirty = None;
        let roots = [disp.bottom_layer, disp.top_layer, disp.sys_layer];
        for r in roots {
            self.place(r, area);
            self.mark_layout(r, LayoutDirty::CHILDREN);
        }
        let mut i = 0;
        while let Some(&s) = self.displays[d].screens.get(i) {
            self.place(s, area);
            self.mark_layout(s, LayoutDirty::CHILDREN);
            i += 1;
        }
        self.invalidate_area(id, area, InvalidateReason::Explicit);
        self.refit_inputs(id, &info);
        twine_core::info!(
            target: "twine::engine",
            "display {} rotated: {}x{} rotation {:?}{}",
            id,
            info.width,
            info.height,
            info.rotation,
            if info.hw_rotation { " (hw)" } else { "" }
        );
    }

    /// Applies the pending requests of every display that is at a safe point (no frame in
    /// progress, no flush in flight, the panel's settle time passed) through its driver.
    /// Called by every step before the layout pass, so a rotation is laid out and drawn in the
    /// same step. Costs one flag test per display when nothing is pending.
    #[inline]
    pub(crate) fn apply_display_requests(&mut self, now: Instant) {
        for d in 0..self.displays.len() {
            if self.displays[d].control.pending() {
                self.apply_requests_of(d, now);
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn apply_requests_of(&mut self, d: usize, now: Instant) {
        let disp = &mut self.displays[d];
        if matches!(disp.backend, Backend::External(()))
            || disp.control.ready_at.is_some_and(|t| now < t)
            || (disp.refresher.job.is_some() && !disp.control.asleep)
            || disp.refresher.flush_pending()
            || disp.health.halted
        {
            // Not now: `display_control_outcome` schedules the next look.
            return;
        }
        disp.control.ready_at = None;
        let req = disp.control.take();
        let mut resp = DisplayResponses::default();
        match &mut disp.backend {
            Backend::Flush(b) => {
                if let Some(r) = req.rotation {
                    resp.rotation = Some(b.set_rotation(r));
                }
                if let Some(l) = req.brightness {
                    resp.brightness = Some(b.set_brightness(l));
                }
                if let Some(s) = req.sleep {
                    resp.sleep = Some(b.sleep(s));
                }
            }
            Backend::Framebuffer(b) => {
                if let Some(l) = req.brightness {
                    resp.brightness = Some(b.set_brightness(l));
                }
                if let Some(s) = req.sleep {
                    resp.sleep = Some(b.sleep(s));
                }
            }
            Backend::External(()) => return,
        }
        if let Some(r) = self.complete_display_requests(self.displays[d].id, req, resp, now) {
            // A hardware rotation was refused: turn the panel back.
            if let Backend::Flush(b) = &mut self.displays[d].backend {
                if let Err(e) = b.set_rotation(r) {
                    self.control_fault(d, DisplayControlFault::RotationFailed, e);
                }
            }
        }
    }

    /// What the refresh of display `d` does about its control state (only called when
    /// [`ControlState::needs_attention`]): `Some(outcome)` — do not draw now.
    #[cold]
    #[inline(never)]
    pub(crate) fn display_control_outcome(
        &mut self,
        d: usize,
        now: Instant,
    ) -> Option<crate::refresh::DisplayOutcome> {
        use crate::refresh::DisplayOutcome as Out;
        let disp = &mut self.displays[d];
        let c = &mut disp.control;
        if let Some(t) = c.ready_at {
            if now < t {
                // Settling: no command and no pixels before `t`.
                return Some(if c.asleep && !c.pending() {
                    Out::Idle
                } else {
                    Out::Due(t)
                });
            }
            c.ready_at = None;
        }
        if c.pending() && disp.refresher.job.is_none() && disp.refresher.flush_pending() {
            // Applied once the driver hands the buffer back.
            return Some(Out::Due(now + Duration::ms(1)));
        }
        if c.asleep {
            return Some(if c.pending() { Out::Due(now) } else { Out::Idle });
        }
        None
    }

    /// The idle-timeout part of a step's wake-up (see `EngineConfig::idle_timeout`).
    #[inline]
    pub(crate) fn idle_wake(&mut self, now: Instant, wake: crate::Wake) -> crate::Wake {
        let Some(timeout) = self.config.idle_timeout else {
            return wake;
        };
        let last = *self.last_activity.get_or_insert(now);
        let inactive = now.saturating_duration_since(last);
        if inactive >= timeout {
            if wake == crate::Wake::Idle {
                crate::Wake::IdleFor(inactive)
            } else {
                wake
            }
        } else {
            wake.min(crate::Wake::At(last + timeout))
        }
    }
}
