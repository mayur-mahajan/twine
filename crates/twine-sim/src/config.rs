//! Simulator configuration: [`SimConfig`], [`SimInputs`], [`Headless`], [`ThemeSlot`].

use std::any::Any;
use std::fmt;
use std::path::PathBuf;

use twine_core::{Color, ColorFormat, Rotation};
use twine_hal::BufferSpec;

use crate::paths;

/// Placeholder for the theme the simulator applies to engine apps.
///
/// Holds any value until `twine-theme` exists.
// NOTE(P14.S01): becomes `Box<dyn Theme>`.
pub struct ThemeSlot(pub Box<dyn Any>);

impl fmt::Debug for ThemeSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThemeSlot(..)")
    }
}

/// Which simulated input devices are registered (all by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimInputs {
    /// Mouse left button → touch pointer.
    pub pointer: bool,
    /// Keyboard → keypad.
    pub keypad: bool,
    /// Mouse wheel and middle button → encoder.
    pub encoder: bool,
}

impl Default for SimInputs {
    fn default() -> Self {
        Self {
            pointer: true,
            keypad: true,
            encoder: true,
        }
    }
}

/// Headless mode: no window, a deterministic clock (16 ms per frame), optional script, PNG output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Headless {
    /// Frames to render when there is no script (default 60).
    pub frames: u32,
    /// A `.twinescript` to execute (see [`script`](crate::script)).
    pub script: Option<PathBuf>,
    /// Where `shot` PNGs and `final.png` are written (default
    /// `target/twine-sim/headless/<example-name>/`).
    pub out_dir: PathBuf,
}

impl Default for Headless {
    fn default() -> Self {
        Self {
            frames: 60,
            script: None,
            out_dir: paths::sim_dir().join("headless").join(paths::exe_name()),
        }
    }
}

/// Configuration of the simulator (`docs/design/10-simulator-testing.md` §1).
///
/// ```
/// use twine_core::ColorFormat;
/// use twine_sim::SimConfig;
///
/// let cfg = SimConfig::new(320, 240).title("demo").scale(9).format(ColorFormat::Rgb565Swapped);
/// assert_eq!(cfg.scale, 4); // clamped to 1..=4
/// assert_eq!(cfg.format, ColorFormat::Rgb565Swapped);
/// assert_eq!(cfg.fps_limit, Some(60));
/// ```
#[derive(Debug)]
pub struct SimConfig {
    /// Panel width in pixels (logical, after rotation).
    pub width: u16,
    /// Panel height in pixels (logical, after rotation).
    pub height: u16,
    /// Window title.
    pub title: String,
    /// Integer window scale, `1..=4`.
    pub scale: u8,
    /// Emulated panel pixel format (default `Rgb565`).
    pub format: ColorFormat,
    /// Draw buffer layout used by engine apps (from Phase 09).
    pub buffer_mode: BufferSpec,
    /// Emulated bus throughput in bits per second (`None` = instant flushes).
    pub bus_hz: Option<u32>,
    /// Display rotation (emulated in "hardware", like MIPI `MADCTL`).
    pub rotation: Rotation,
    /// The theme for engine apps.
    pub theme: Option<ThemeSlot>,
    /// Registered input devices.
    pub input: SimInputs,
    /// Headless mode when `Some`.
    pub headless: Option<Headless>,
    /// Colors of `0` and `1` bits of an `I1` panel (default black on `#B0C8A0`, "LCD green").
    pub mono_colors: (Color, Color),
    /// Frame rate limit of continuously animating apps (default 60, `None` = unlimited).
    pub fps_limit: Option<u16>,
}

/// Formats the simulator panel can emulate.
pub const SUPPORTED_FORMATS: [ColorFormat; 7] = [
    ColorFormat::Rgb565,
    ColorFormat::Rgb565Swapped,
    ColorFormat::Rgb888,
    ColorFormat::Xrgb8888,
    ColorFormat::Argb8888,
    ColorFormat::L8,
    ColorFormat::I1,
];

/// Parses a format name as used by `TWINE_SIM_FORMAT` (`rgb565`, `rgb565swapped`, `rgb888`,
/// `xrgb8888`, `argb8888`, `l8`, `i1`; case-insensitive, `_` ignored).
#[must_use]
pub fn parse_format(name: &str) -> Option<ColorFormat> {
    let n: String = name
        .chars()
        .filter(|c| *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    SUPPORTED_FORMATS
        .into_iter()
        .find(|f| f.name().replace('_', "").to_ascii_lowercase() == n)
}

impl SimConfig {
    /// A `width × height` panel with defaults: title "twine", scale 1, `Rgb565`, 40-row double
    /// buffers, no bus emulation, no rotation, all inputs, windowed, 60 fps.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            title: "twine".into(),
            scale: 1,
            format: ColorFormat::Rgb565,
            buffer_mode: BufferSpec::default(),
            bus_hz: None,
            rotation: Rotation::Deg0,
            theme: None,
            input: SimInputs::default(),
            headless: None,
            mono_colors: (Color::BLACK, Color::hex(0xB0_C8_A0)),
            fps_limit: Some(60),
        }
    }

    /// Sets the window title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the window scale; values outside `1..=4` are clamped with a warning.
    #[must_use]
    pub fn scale(mut self, scale: u8) -> Self {
        let clamped = scale.clamp(1, 4);
        if clamped != scale {
            log::warn!(target: "twine::sim", "scale {scale} out of range 1..=4, using {clamped}");
        }
        self.scale = clamped;
        self
    }

    /// Sets the emulated panel format; unsupported formats are ignored with a warning
    /// (see [`SUPPORTED_FORMATS`]).
    #[must_use]
    pub fn format(mut self, format: ColorFormat) -> Self {
        if SUPPORTED_FORMATS.contains(&format) {
            self.format = format;
        } else {
            log::warn!(target: "twine::sim", "panel format {format} is not supported, keeping {}", self.format);
        }
        self
    }

    /// Sets the draw buffer layout for engine apps.
    #[must_use]
    pub fn buffers(mut self, mode: BufferSpec) -> Self {
        self.buffer_mode = mode;
        self
    }

    /// Emulates a bus of `hz` bits per second (`None` or 0 = instant).
    #[must_use]
    pub fn bus_hz(mut self, hz: Option<u32>) -> Self {
        self.bus_hz = hz.filter(|h| *h > 0);
        self
    }

    /// Sets the display rotation.
    #[must_use]
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    /// Runs headless (or windowed with `None`).
    #[must_use]
    pub fn headless(mut self, headless: Option<Headless>) -> Self {
        self.headless = headless;
        self
    }

    /// Sets the colors of `0` and `1` bits of an `I1` panel.
    #[must_use]
    pub fn mono_colors(mut self, zero: Color, one: Color) -> Self {
        self.mono_colors = (zero, one);
        self
    }

    /// Sets the frame rate limit (`None` = unlimited; 0 is treated as `None`).
    #[must_use]
    pub fn fps_limit(mut self, fps: Option<u16>) -> Self {
        self.fps_limit = fps.filter(|f| *f > 0);
        self
    }

    /// Applies the environment: `TWINE_SIM_SCALE`, `TWINE_SIM_HEADLESS=1`,
    /// `TWINE_SIM_SCRIPT`, `TWINE_SIM_FRAMES`, `TWINE_SIM_BUS_HZ`, `TWINE_SIM_FORMAT`.
    ///
    /// Invalid values are ignored with a warning.
    #[must_use]
    pub fn from_env(self) -> Self {
        self.from_lookup(|k| std::env::var(k).ok())
    }

    /// Like [`from_env`](Self::from_env) with a custom variable lookup (for tests).
    #[must_use]
    pub fn from_lookup(mut self, lookup: impl Fn(&str) -> Option<String>) -> Self {
        fn parse<T: std::str::FromStr>(key: &str, v: &str) -> Option<T> {
            let r = v.trim().parse().ok();
            if r.is_none() {
                log::warn!(target: "twine::sim", "ignoring invalid {key}={v:?}");
            }
            r
        }
        if let Some(s) = lookup("TWINE_SIM_SCALE").and_then(|v| parse::<u8>("TWINE_SIM_SCALE", &v)) {
            self = self.scale(s);
        }
        if let Some(hz) = lookup("TWINE_SIM_BUS_HZ").and_then(|v| parse::<u32>("TWINE_SIM_BUS_HZ", &v)) {
            self = self.bus_hz(Some(hz));
        }
        if let Some(v) = lookup("TWINE_SIM_FORMAT") {
            match parse_format(&v) {
                Some(f) => self.format = f,
                None => log::warn!(target: "twine::sim", "ignoring unknown TWINE_SIM_FORMAT={v:?}"),
            }
        }
        if lookup("TWINE_SIM_HEADLESS").is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes")) {
            let mut h = self.headless.take().unwrap_or_default();
            if let Some(p) = lookup("TWINE_SIM_SCRIPT").filter(|p| !p.trim().is_empty()) {
                h.script = Some(PathBuf::from(p));
            }
            if let Some(n) = lookup("TWINE_SIM_FRAMES").and_then(|v| parse::<u32>("TWINE_SIM_FRAMES", &v)) {
                h.frames = n;
            }
            self.headless = Some(h);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn scale_is_clamped() {
        assert_eq!(SimConfig::new(10, 10).scale(0).scale, 1);
        assert_eq!(SimConfig::new(10, 10).scale(3).scale, 3);
        assert_eq!(SimConfig::new(10, 10).scale(200).scale, 4);
    }

    #[test]
    fn from_env_parses_values() {
        let env: HashMap<&str, &str> = [
            ("TWINE_SIM_SCALE", "3"),
            ("TWINE_SIM_HEADLESS", "1"),
            ("TWINE_SIM_BUS_HZ", "10000000"),
            ("TWINE_SIM_SCRIPT", "a/b.twinescript"),
            ("TWINE_SIM_FRAMES", "12"),
            ("TWINE_SIM_FORMAT", "rgb565swapped"),
        ]
        .into_iter()
        .collect();
        let cfg = SimConfig::new(320, 240).from_lookup(|k| env.get(k).map(ToString::to_string));
        assert_eq!(cfg.scale, 3);
        assert_eq!(cfg.bus_hz, Some(10_000_000));
        assert_eq!(cfg.format, ColorFormat::Rgb565Swapped);
        let h = cfg.headless.expect("headless");
        assert_eq!(h.frames, 12);
        assert_eq!(h.script, Some(PathBuf::from("a/b.twinescript")));

        // Invalid or absent values leave the defaults.
        let bad: HashMap<&str, &str> = [("TWINE_SIM_SCALE", "x"), ("TWINE_SIM_HEADLESS", "0")]
            .into_iter()
            .collect();
        let cfg = SimConfig::new(1, 1)
            .scale(2)
            .from_lookup(|k| bad.get(k).map(ToString::to_string));
        assert_eq!(cfg.scale, 2);
        assert!(cfg.headless.is_none());
    }

    #[test]
    fn format_names() {
        assert_eq!(parse_format("I1"), Some(ColorFormat::I1));
        assert_eq!(parse_format("rgb565_swapped"), Some(ColorFormat::Rgb565Swapped));
        assert_eq!(parse_format("a8"), None);
        let cfg = SimConfig::new(1, 1).format(ColorFormat::I4);
        assert_eq!(cfg.format, ColorFormat::Rgb565);
    }
}
