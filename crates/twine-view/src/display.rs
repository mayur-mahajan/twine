//! Several displays on one [`Ui`]: [`DisplayBuilder`] (a display and what belongs to it —
//! draw buffers, inputs, theme, rotation reserve, command channel) and [`DisplayMut`] (run-time
//! control of one of the `Ui`'s displays).
//!
//! One engine drives every display of a `Ui`: [`UiBuilder::display`](crate::UiBuilder::display)
//! and [`Ui::mount_on`] add a display and build an application on it, in a child scope of the
//! `Ui`'s root scope. Each display has its own dirty areas, inputs, theme and theme mode,
//! rotation, brightness and sleep state, and its own [`DisplayCmd`] channel; all of them run in
//! the `Ui`'s one update cycle, woken by its one waker, so one run loop
//! ([`run::blocking`](crate::run::blocking), `twine_embassy::run`) drives them all.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use twine_core::{Fraction, Rotation};
use twine_engine::{
    BufferMode, DisplayCmd, DisplayHealth, DisplayId, Engine, EngineError, InputId, InputKind, IntoTheme,
    ThemeHook, ThemeMode,
};
use twine_hal::{DisplayDriver, DisplayInfo, FramebufferDisplay, InputDevice};
use twine_reactive::{Channel, Scope};

use crate::access::EngineAccess;
use crate::error::UiError;
use crate::typestate::{HasBuffers, NoBuffers};
use crate::ui::{DisplaySetup, Framebuffer, Partial, UiCore};
use crate::view::View;

/// Adds an input device to the engine, on a display.
pub(crate) type InputAdder = Box<dyn FnOnce(&mut Engine, DisplayId) -> Result<InputId, EngineError>>;
/// Registers the display-command channel on the scope of the display's application.
pub(crate) type CommandsSetter = Box<dyn FnOnce(Scope, DisplayId)>;

/// What belongs to a display besides its driver and buffers: inputs, theme, rotation reserve
/// and command channel (shared by [`DisplayBuilder`], the first display of a
/// [`UiBuilder`](crate::UiBuilder) and the `AsyncUiBuilder`'s display). Not generic, so the
/// installation code exists once in the binary.
#[derive(Default)]
pub(crate) struct DisplayParts {
    inputs: Vec<InputAdder>,
    pub(crate) theme: Option<Rc<dyn ThemeHook>>,
    commands: Option<CommandsSetter>,
    reserve_rotation: bool,
}

impl DisplayParts {
    pub(crate) fn input(&mut self, d: impl InputDevice + 'static) {
        self.inputs.push(Box::new(move |e, disp| e.add_input(d, disp)));
    }

    pub(crate) fn display_commands<const N: usize>(&mut self, commands: &'static Channel<DisplayCmd, N>) {
        self.commands = Some(Box::new(move |scope: Scope, display: DisplayId| {
            scope.on_message(commands, move |cmd: DisplayCmd| {
                let applied = EngineAccess::with(scope, |e| e.display_command(display, cmd));
                if !matches!(applied, Some(Ok(()))) {
                    twine_core::warn!(target: "twine::view", "display command {:?} not applied: {:?}", cmd, applied);
                }
            });
        }));
    }

    pub(crate) fn reserve_rotation(&mut self) {
        self.reserve_rotation = true;
    }

    /// Installs the parts on `display` (just added to `engine`): rotation reserve, theme,
    /// inputs (keypads and encoders join the default focus group). Returns the command-channel
    /// registration, to run on the application's scope once it is mounted.
    pub(crate) fn install(
        self,
        engine: &mut Engine,
        display: DisplayId,
    ) -> Result<Option<CommandsSetter>, UiError> {
        if self.reserve_rotation {
            engine.reserve_rotation(display)?;
        }
        if let Some(t) = self.theme {
            engine.set_theme(display, t);
        }
        let mut focus_inputs = Vec::new();
        for add in self.inputs {
            let id = add(engine, display)?;
            if matches!(
                engine.input_kind(id),
                Some(InputKind::Keypad | InputKind::Encoder)
            ) {
                focus_inputs.push(id);
            }
        }
        if !focus_inputs.is_empty() {
            let g = if let Some(g) = engine.default_group() {
                g
            } else {
                let g = engine.create_group()?;
                engine.set_default_group(Some(g));
                g
            };
            for id in focus_inputs {
                engine.set_input_group(id, Some(g));
            }
        }
        Ok(self.commands)
    }
}

/// A display and what belongs to it: its draw buffers, input devices, theme, rotation reserve
/// and [`DisplayCmd`] channel. [`UiBuilder`](crate::UiBuilder) configures its first display
/// with the same methods; a `DisplayBuilder` describes a further one for
/// [`UiBuilder::display`](crate::UiBuilder::display) or [`Ui::mount_on`](crate::Ui::mount_on).
///
/// `B` is the buffer state ([`typestate`](crate::typestate)): a partial display
/// ([`new`](Self::new)) starts with [`NoBuffers`] and can be used only once
/// [`buffers`](Self::buffers) has been called — a forgotten `.buffers(..)` is a compile error,
/// not a run-time one; a framebuffer display ([`framebuffer`](Self::framebuffer)) starts with
/// [`BufferMode::Full`].
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565));
/// let mut ui = Ui::builder(panel(96, 48))
///     .runtime(Runtime::take().unwrap())
///     .clock(MockClock::new())
///     .buffers(BufferMode::alloc(BufferSpec::default()))
///     .theme(DefaultTheme::light())
///     .display(
///         DisplayBuilder::new(panel(64, 32))
///             .buffers(BufferMode::alloc(BufferSpec::default()))
///             .theme(DefaultTheme::dark()),
///         |_| label("status"),
///     )
///     .build(|_| label("main"));
/// assert_eq!(ui.displays().count(), 2);
/// ui.update(); // draws both
/// ```
#[must_use]
pub struct DisplayBuilder<S, B = NoBuffers> {
    pub(crate) setup: S,
    buffers: B,
    pub(crate) parts: DisplayParts,
}

impl<S, B> core::fmt::Debug for DisplayBuilder<S, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayBuilder")
            .field("inputs", &self.parts.inputs.len())
            .field("theme", &self.parts.theme.as_ref().map(|t| t.name()))
            .field("reserve_rotation", &self.parts.reserve_rotation)
            .finish_non_exhaustive()
    }
}

impl<D: DisplayDriver + 'static> DisplayBuilder<Partial<D>> {
    /// A display flushed from partial draw buffers (as [`Ui::builder`](crate::Ui::builder)):
    /// it needs [`buffers`](Self::buffers) (checked at compile time).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// let aux = DisplayBuilder::new(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
    ///     .buffers(BufferMode::alloc(BufferSpec::default()));
    /// # let _ = aux;
    /// ```
    pub fn new(display: D) -> Self {
        Self::with_setup(Partial(display), NoBuffers)
    }
}

impl<D: FramebufferDisplay + 'static> DisplayBuilder<Framebuffer<D>, BufferMode> {
    /// A memory-mapped framebuffer display (as [`Ui::builder_fb`](crate::Ui::builder_fb));
    /// without [`buffers`](Self::buffers) it uses [`BufferMode::Full`] (the driver's
    /// framebuffers).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MockFramebufferDisplay;
    /// use twine_view::prelude::*;
    ///
    /// let fb = DisplayBuilder::framebuffer(MockFramebufferDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565), false, 0));
    /// # let _ = fb;
    /// ```
    pub fn framebuffer(display: D) -> Self {
        Self::with_setup(Framebuffer(display), BufferMode::full())
    }
}

impl<S, B> DisplayBuilder<S, B> {
    pub(crate) fn with_setup(setup: S, buffers: B) -> Self {
        Self {
            setup,
            buffers,
            parts: DisplayParts::default(),
        }
    }

    /// The draw buffers (see [`UiBuilder::buffers`](crate::UiBuilder::buffers): required for a
    /// partial display — at compile time —, never allocated implicitly). A later call
    /// replaces them.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// draw_buffers!(static AUX: 1 x 8 rows x 64 px @ Rgb565);
    /// let aux = DisplayBuilder::new(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
    ///     .buffers(BufferMode::partial_single_from(AUX.take().expect("taken once")));
    /// # let _ = aux;
    /// ```
    pub fn buffers(self, m: BufferMode) -> DisplayBuilder<S, BufferMode> {
        DisplayBuilder {
            setup: self.setup,
            buffers: m,
            parts: self.parts,
        }
    }

    /// Adds an input device of this display (any number; see
    /// [`UiBuilder::input`](crate::UiBuilder::input)). Pointer devices are fitted to this
    /// display and hit-test only its screens. Keypads and encoders join the `Ui`'s focus group
    /// (the engine's default group, created when the first one is added): the engine has one
    /// default group, so keypads of several displays share it.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::{DisplayInfo, InputData, InputDevice, InputKind, PointerData};
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// struct Idle;
    /// impl InputDevice for Idle {
    ///     fn kind(&self) -> InputKind { InputKind::Pointer }
    ///     fn read(&mut self) -> InputData { InputData::Pointer(PointerData::default()) }
    /// }
    /// let aux = DisplayBuilder::new(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565))).input(Idle);
    /// # let _ = aux;
    /// ```
    pub fn input(mut self, d: impl InputDevice + 'static) -> Self {
        self.parts.input(d);
        self
    }

    /// The theme of this display (each display has its own theme and theme mode): a theme
    /// value or a shared one ([`IntoTheme`], e.g. one `Rc` for several displays). A later call
    /// replaces it.
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// let panel = || MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
    /// let aux = DisplayBuilder::new(panel()).theme(DefaultTheme::dark());
    /// let shared = Rc::new(DefaultTheme::light()); // one theme, two displays
    /// let left = DisplayBuilder::new(panel()).theme(shared.clone());
    /// let right = DisplayBuilder::new(panel()).theme(shared);
    /// # let _ = (aux, left, right);
    /// ```
    pub fn theme(mut self, t: impl IntoTheme) -> Self {
        self.parts.theme = Some(t.into_theme());
        self
    }

    /// Reserves the memory software rotation of this display needs (see
    /// [`UiBuilder::reserve_rotation`](crate::UiBuilder::reserve_rotation)).
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// let aux = DisplayBuilder::new(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
    ///     .reserve_rotation();
    /// # let _ = aux;
    /// ```
    pub fn reserve_rotation(mut self) -> Self {
        self.parts.reserve_rotation();
        self
    }

    /// Applies the [`DisplayCmd`]s sent to `commands` to **this** display (see
    /// [`UiBuilder::display_commands`](crate::UiBuilder::display_commands)); every display
    /// can have its own channel. A later call replaces the channel.
    ///
    /// ```
    /// use twine_core::ColorFormat;
    /// use twine_hal::DisplayInfo;
    /// use twine_testing::MemoryDisplay;
    /// use twine_view::prelude::*;
    ///
    /// static AUX_CMDS: Channel<DisplayCmd, 4> = Channel::new();
    /// let aux = DisplayBuilder::new(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
    ///     .display_commands(&AUX_CMDS);
    /// # let _ = aux;
    /// ```
    pub fn display_commands<const N: usize>(mut self, commands: &'static Channel<DisplayCmd, N>) -> Self {
        self.parts.display_commands(commands);
        self
    }
}

impl<S: DisplaySetup, B: HasBuffers> DisplayBuilder<S, B> {
    /// Adds the display to `engine` with its buffers, rotation reserve, theme and inputs.
    /// Returns its id and the command-channel registration, to run on the application's
    /// scope once it is mounted.
    pub(crate) fn add(self, engine: &mut Engine) -> Result<(DisplayId, Option<CommandsSetter>), UiError> {
        let display = self.setup.add(engine, self.buffers.into_part())?;
        let commands = self.parts.install(engine, display)?;
        Ok((display, commands))
    }
}

/// Adds `display` to the engine and builds `app` on it (see [`Ui::mount_on`](crate::Ui::mount_on)).
pub(crate) fn mount_display<S: DisplaySetup, B: HasBuffers, V: View>(
    core: &mut UiCore,
    engine: &mut Engine,
    display: DisplayBuilder<S, B>,
    app: impl FnOnce(Scope) -> V,
) -> Result<DisplayId, UiError> {
    let (id, commands) = display.add(engine)?;
    let scope = core.mount_on(engine, id, app)?;
    if let Some(register) = commands {
        register(scope, id);
    }
    Ok(id)
}

/// Run-time control of one display of a [`Ui`](crate::Ui) ([`Ui::display_mut`](crate::Ui::display_mut)):
/// rotation, brightness, sleep, theme and theme mode, health — independently of the `Ui`'s
/// other displays. The setters record a request the engine applies at the next update between
/// two frames, and wake the `Ui`. Never panics; allocates nothing (but
/// [`set_theme`](Self::set_theme)'s `Rc`).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_hal::DisplayInfo;
/// use twine_testing::{MemoryDisplay, MockClock};
/// use twine_view::prelude::*;
///
/// let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565)).with_power_control();
/// let mut ui = Ui::builder(panel(96, 48))
///     .runtime(Runtime::take().unwrap())
///     .clock(MockClock::new())
///     .buffers(BufferMode::alloc(BufferSpec::default()))
///     .build(|_| label("main"));
/// let aux = ui
///     .mount_on(DisplayBuilder::new(panel(64, 32)).buffers(BufferMode::alloc(BufferSpec::default())), |_| label("aux"))
///     .unwrap();
/// ui.display_mut(aux).unwrap().set_sleep(true); // only the auxiliary display sleeps
/// ui.update();
/// assert!(ui.display_mut(aux).unwrap().asleep());
/// assert!(!ui.display_asleep());
/// ```
pub struct DisplayMut<'a> {
    pub(crate) engine: &'a mut Engine,
    pub(crate) core: &'a UiCore,
    pub(crate) display: DisplayId,
}

impl core::fmt::Debug for DisplayMut<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayMut")
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

impl DisplayMut<'_> {
    /// The display's id in the engine. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// assert_eq!(ui.display_mut(d).unwrap().id(), d);
    /// ```
    #[must_use]
    pub fn id(&self) -> DisplayId {
        self.display
    }

    /// The scope of the application on this display (see [`UiCore::display_scope`]): the
    /// root scope for the first display. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// assert_eq!(ui.display_mut(d).unwrap().scope(), ui.root_scope());
    /// ```
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.core
            .display_scope(self.display)
            .unwrap_or(self.core.root_scope())
    }

    /// The description of the display: its current logical size and rotation. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// let info = ui.display_mut(d).unwrap().info();
    /// assert_eq!((info.width, info.height), (64, 32));
    /// ```
    #[must_use]
    pub fn info(&self) -> DisplayInfo {
        crate::ui::display_info(self.engine, self.display)
    }

    /// Rotates the display at the next update (see [`Ui::set_rotation`](crate::Ui::set_rotation)).
    /// Never panics.
    ///
    /// # Errors
    /// [`UiError::Engine`] with [`EngineError::InvalidConfig`] for a framebuffer display.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.display_mut(d).unwrap().set_rotation(Rotation::Deg270).unwrap();
    /// ui.update();
    /// assert_eq!(ui.display_mut(d).unwrap().info().rotation, Rotation::Deg270);
    /// ```
    pub fn set_rotation(&mut self, rotation: Rotation) -> Result<(), UiError> {
        self.engine.set_rotation(self.display, rotation)?;
        self.core.waker().wake();
        Ok(())
    }

    /// Sets the panel brightness at the next update (see
    /// [`Ui::set_brightness`](crate::Ui::set_brightness)). Idempotent; never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.display_mut(d).unwrap().set_brightness(Fraction::pct(60));
    /// ui.update();
    /// assert_eq!(ui.engine().display_brightness(d), Some(Fraction::pct(60)));
    /// ```
    pub fn set_brightness(&mut self, level: Fraction) {
        let _ = self.engine.set_display_brightness(self.display, level);
        self.core.waker().wake();
    }

    /// Puts the display to sleep (`true`) or wakes it (`false`) at the next update (see
    /// [`Ui::set_display_sleep`](crate::Ui::set_display_sleep)); the other displays keep
    /// drawing. Idempotent; never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.display_mut(d).unwrap().set_sleep(true);
    /// ui.update();
    /// assert!(ui.display_mut(d).unwrap().asleep());
    /// ```
    pub fn set_sleep(&mut self, sleep: bool) {
        let _ = self.engine.set_display_sleep(self.display, sleep);
        self.core.waker().wake();
    }

    /// Whether the display is asleep (a sleep request was applied by an update). Never
    /// panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// assert!(!ui.display_mut(d).unwrap().asleep());
    /// ```
    #[must_use]
    pub fn asleep(&self) -> bool {
        self.engine.display_asleep(self.display)
    }

    /// Switches the display's theme (every node of the display is re-styled and the display
    /// redrawn once; the other displays keep theirs): a theme value or a shared one
    /// ([`IntoTheme`]). Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.display_mut(d).unwrap().set_theme(DefaultTheme::dark());
    /// ui.update();
    /// assert_eq!(ui.display_mut(d).unwrap().theme_mode(), ThemeMode::Dark);
    /// ```
    pub fn set_theme(&mut self, theme: impl IntoTheme) {
        self.engine.set_theme(self.display, theme);
        self.core.waker().wake();
    }

    /// Switches the display's theme mode ([`Engine::set_theme_mode`]; e.g. dark on one
    /// display, night on another). The application on the display sees it through
    /// [`use_theme`](crate::use_theme). Idempotent; never panics. (Example: see
    /// [`Ui::display_mut`](crate::Ui::display_mut).)
    pub fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.engine.set_theme_mode(self.display, mode);
        self.core.waker().wake();
    }

    /// The display's theme mode. Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// assert_eq!(ui.display_mut(d).unwrap().theme_mode(), ThemeMode::Light);
    /// ```
    #[must_use]
    pub fn theme_mode(&self) -> ThemeMode {
        self.engine.theme_mode(self.display)
    }

    /// The display's flush health (see [`Engine::display_health`]). Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// ui.update();
    /// let health = ui.display_mut(d).unwrap().health().unwrap();
    /// assert_eq!(health.state, DisplayState::Healthy);
    /// ```
    #[must_use]
    pub fn health(&self) -> Option<DisplayHealth> {
        self.engine.display_health(self.display)
    }

    /// Recovers the display after flush failures (see
    /// [`Ui::recover_display`](crate::Ui::recover_display)). Never panics.
    ///
    /// ```
    /// # use twine_core::ColorFormat;
    /// # use twine_hal::DisplayInfo;
    /// # use twine_testing::{MemoryDisplay, MockClock};
    /// # use twine_view::prelude::*;
    /// # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)).with_power_control().with_rotation_control();
    /// # let mut ui = Ui::builder(panel).runtime(Runtime::take().unwrap()).clock(MockClock::new()).buffers(BufferMode::alloc(BufferSpec::default())).build(|_| label("hi"));
    /// let d = ui.display();
    /// // e.g. after the panel was power-cycled following `DisplayState::Halted`:
    /// ui.display_mut(d).unwrap().recover();
    /// ui.update();
    /// ```
    pub fn recover(&mut self) {
        let _ = self.engine.recover_display(self.display);
        self.core.waker().wake();
    }
}
