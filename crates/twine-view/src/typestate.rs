//! The parts a [`UiBuilder`](crate::UiBuilder) (and `AsyncUiBuilder`) must be given before it
//! can build, checked by the compiler.
//!
//! A builder carries one type parameter per required part. It starts with the "missing"
//! marker ([`NoRuntime`], [`NoClock`], [`NoBuffers`], [`NoPlatform`]); the method that
//! provides the part ([`runtime`](crate::UiBuilder::runtime),
//! [`clock`](crate::UiBuilder::clock) / [`platform`](crate::UiBuilder::platform),
//! [`buffers`](crate::UiBuilder::buffers)) replaces the marker by the part itself, and
//! `build` / `try_build` exist only once every part is there. Forgetting one is a compile
//! error that names the missing call — never a run-time `InvalidConfig` error on a device:
//!
//! | Missing | Compile error says | Add |
//! |---------|--------------------|-----|
//! | runtime | "the `Ui` has no reactive runtime" | `.runtime(Runtime::take().expect(..))` |
//! | clock | "the `Ui` has no clock" | `.platform(&p)` or `.clock(c)` |
//! | draw buffers | "the display has no draw buffers" | `.buffers(BufferMode::partial_double_from(BUFS.take()..))` or `.buffers(BufferMode::alloc(..))` |
//! | async platform | "the `AsyncUi` has no platform" | `.platform(&EmbassyPlatform)` (or `.with_embassy_platform()`) |
//!
//! What only the device can tell stays a run-time error of `try_build`
//! ([`UiError`](crate::UiError)): a display the engine refuses (format not compiled in, too
//! many displays, node budget), buffers that do not fit the panel or are misaligned, inputs
//! the engine refuses, widgets that cannot be built.
//!
//! **Cost.** None at run time: the markers are zero-sized, a provided part is stored exactly
//! as before (no `Option` to check), and `build` forwards to one implementation per display
//! kind — the typestate adds no code per state to the binary.
//!
//! The marker traits ([`HasRuntime`], [`HasClock`], [`HasBuffers`], [`HasPlatform`]) are
//! sealed: only the provided parts implement them.
//!
//! ```compile_fail,E0277
//! // No runtime: "the `Ui` has no reactive runtime".
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock};
//! use twine_view::prelude::*;
//! # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
//! let ui = Ui::builder(panel)
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .build(|_| label("hi"));
//! ```
//!
//! ```compile_fail,E0277
//! // No clock: "the `Ui` has no clock".
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::MemoryDisplay;
//! use twine_view::prelude::*;
//! # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
//! let ui = Ui::builder(panel)
//!     .runtime(Runtime::take().unwrap())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .build(|_| label("hi"));
//! ```
//!
//! ```compile_fail,E0277
//! // No draw buffers: "the display has no draw buffers".
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock};
//! use twine_view::prelude::*;
//! # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
//! let ui = Ui::builder(panel)
//!     .runtime(Runtime::take().unwrap())
//!     .clock(MockClock::new())
//!     .try_build(|_| label("hi"));
//! ```
//!
//! ```compile_fail,E0277
//! // A further display without draw buffers: "the display has no draw buffers".
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock};
//! use twine_view::prelude::*;
//! # let panel = |w, h| MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565));
//! let mut ui = Ui::builder(panel(96, 48))
//!     .runtime(Runtime::take().unwrap())
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .build(|_| label("main"));
//! let aux = ui.mount_on(DisplayBuilder::new(panel(64, 32)), |_| label("aux"));
//! ```
//!
//! With every part, the same builder compiles (a framebuffer display needs no
//! `.buffers(..)`: it defaults to the driver's framebuffers):
//!
//! ```
//! # use twine_core::ColorFormat;
//! # use twine_hal::DisplayInfo;
//! # use twine_testing::{MemoryDisplay, MockClock, MockFramebufferDisplay};
//! use twine_view::prelude::*;
//! # let panel = MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565));
//! let rt = Runtime::take().unwrap();
//! let ui = Ui::builder(panel)
//!     .runtime(rt)
//!     .clock(MockClock::new())
//!     .buffers(BufferMode::alloc(BufferSpec::default()))
//!     .build(|_| label("hi"));
//! # let fb = MockFramebufferDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565), true, 0);
//! let fb_ui = Ui::builder_fb(fb).runtime(rt).clock(MockClock::new()).build(|_| label("hi"));
//! # let _ = (ui, fb_ui);
//! ```

use alloc::boxed::Box;

use twine_engine::BufferMode;
use twine_hal::Clock;
use twine_reactive::Runtime;

/// The sealed part of the marker traits: hands the provided part to `build`.
pub(crate) mod sealed {
    /// A provided part of type `T` (implemented only by `T` itself).
    pub trait Part<T> {
        /// The part.
        fn into_part(self) -> T;
    }
}

/// The builder has no reactive runtime yet: call `.runtime(rt)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoRuntime;

/// The builder has no clock yet: call `.platform(&p)` or `.clock(c)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoClock;

/// The display has no draw buffers yet: call `.buffers(m)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoBuffers;

/// The async builder has no platform yet: call `.platform(&p)`.
#[cfg(feature = "async")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoPlatform;

/// A builder state that holds the reactive runtime ([`Runtime`]; set with
/// [`UiBuilder::runtime`](crate::UiBuilder::runtime)). Sealed.
#[diagnostic::on_unimplemented(
    message = "the `Ui` has no reactive runtime",
    label = "`.runtime(..)` was not called on this builder",
    note = "take the runtime once, early in `main`, and give it to the builder: `.runtime(Runtime::take().expect(\"runtime already taken\"))`"
)]
pub trait HasRuntime: sealed::Part<Runtime> {}

impl sealed::Part<Runtime> for Runtime {
    #[inline]
    fn into_part(self) -> Runtime {
        self
    }
}
impl HasRuntime for Runtime {}

/// A builder state that holds the clock (set with [`UiBuilder::platform`](crate::UiBuilder::platform)
/// or [`UiBuilder::clock`](crate::UiBuilder::clock)). Sealed.
#[diagnostic::on_unimplemented(
    message = "the `Ui` has no clock",
    label = "neither `.platform(..)` nor `.clock(..)` was called on this builder",
    note = "give the builder the platform the loop waits with (`.platform(&platform)`), or only a time source (`.clock(clock)`)"
)]
pub trait HasClock: sealed::Part<Box<dyn Clock>> {}

impl sealed::Part<Box<dyn Clock>> for Box<dyn Clock> {
    #[inline]
    fn into_part(self) -> Box<dyn Clock> {
        self
    }
}
impl HasClock for Box<dyn Clock> {}

/// A builder state that holds the draw buffers of a display (set with
/// [`UiBuilder::buffers`](crate::UiBuilder::buffers) or
/// [`DisplayBuilder::buffers`](crate::DisplayBuilder::buffers); a framebuffer display starts
/// with [`BufferMode::Full`]). Sealed.
#[diagnostic::on_unimplemented(
    message = "the display has no draw buffers",
    label = "`.buffers(..)` was not called for this display",
    note = "firmware: declare them with `draw_buffers!` and pass `.buffers(BufferMode::partial_double_from(BUFS.take().expect(..)))`; heap: `.buffers(BufferMode::alloc(BufferSpec::default()))`"
)]
pub trait HasBuffers: sealed::Part<BufferMode> {}

impl sealed::Part<BufferMode> for BufferMode {
    #[inline]
    fn into_part(self) -> BufferMode {
        self
    }
}
impl HasBuffers for BufferMode {}

/// An async builder state that holds the platform (set with `AsyncUiBuilder::platform`).
/// Sealed.
#[cfg(feature = "async")]
#[diagnostic::on_unimplemented(
    message = "the `AsyncUi` has no platform",
    label = "`.platform(..)` was not called on this builder",
    note = "give the builder its async platform: `.platform(&EmbassyPlatform)` (or `.with_embassy_platform()` from `twine_embassy::UiBuilderExt`)"
)]
pub trait HasPlatform: sealed::Part<Box<dyn twine_hal::AsyncPlatform>> {}

#[cfg(feature = "async")]
impl sealed::Part<Box<dyn twine_hal::AsyncPlatform>> for Box<dyn twine_hal::AsyncPlatform> {
    #[inline]
    fn into_part(self) -> Box<dyn twine_hal::AsyncPlatform> {
        self
    }
}
#[cfg(feature = "async")]
impl HasPlatform for Box<dyn twine_hal::AsyncPlatform> {}
