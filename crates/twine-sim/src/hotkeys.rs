//! Simulator hotkeys (F1…F12).
//!
//! Hotkeys are consumed by the simulator and never forwarded to the keypad. `F1`, `F9` and `F10`
//! are handled by the simulator itself; the others are dispatched to callbacks registered with
//! [`HotkeyRegistry::on`] by engine runners, and only log a hint when no callback is registered.

use std::fmt;
use std::path::{Path, PathBuf};

use winit::keyboard::NamedKey;

/// A simulator hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hotkey {
    /// F1: print the hotkey table.
    Help,
    /// F2: refresh-area debug overlay.
    RefreshDebug,
    /// F3: layout bounds overlay.
    LayoutBounds,
    /// F4: performance monitor.
    PerfMonitor,
    /// F5: slow motion ×0.25.
    SlowMotion,
    /// F6: pause time.
    Pause,
    /// F7: single-step 16 ms.
    Step,
    /// F8: dump the widget tree.
    DumpTree,
    /// F9: screenshot PNG.
    Screenshot,
    /// F10: start / stop recording frames.
    Record,
    /// F12: toggle light / dark theme.
    ThemeToggle,
}

impl Hotkey {
    /// All hotkeys in F-key order.
    pub const ALL: [Hotkey; 11] = [
        Hotkey::Help,
        Hotkey::RefreshDebug,
        Hotkey::LayoutBounds,
        Hotkey::PerfMonitor,
        Hotkey::SlowMotion,
        Hotkey::Pause,
        Hotkey::Step,
        Hotkey::DumpTree,
        Hotkey::Screenshot,
        Hotkey::Record,
        Hotkey::ThemeToggle,
    ];

    /// The hotkey of function key `F<n>` (F11 has none).
    #[must_use]
    pub const fn from_fkey(n: u8) -> Option<Hotkey> {
        Some(match n {
            1 => Hotkey::Help,
            2 => Hotkey::RefreshDebug,
            3 => Hotkey::LayoutBounds,
            4 => Hotkey::PerfMonitor,
            5 => Hotkey::SlowMotion,
            6 => Hotkey::Pause,
            7 => Hotkey::Step,
            8 => Hotkey::DumpTree,
            9 => Hotkey::Screenshot,
            10 => Hotkey::Record,
            12 => Hotkey::ThemeToggle,
            _ => return None,
        })
    }

    /// The function key number (`1..=12`).
    #[must_use]
    pub const fn fkey(self) -> u8 {
        match self {
            Hotkey::Help => 1,
            Hotkey::RefreshDebug => 2,
            Hotkey::LayoutBounds => 3,
            Hotkey::PerfMonitor => 4,
            Hotkey::SlowMotion => 5,
            Hotkey::Pause => 6,
            Hotkey::Step => 7,
            Hotkey::DumpTree => 8,
            Hotkey::Screenshot => 9,
            Hotkey::Record => 10,
            Hotkey::ThemeToggle => 12,
        }
    }

    /// Parses `"F1"` … `"F12"`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Hotkey> {
        name.strip_prefix('F')?.parse().ok().and_then(Hotkey::from_fkey)
    }

    /// The hotkey of a winit named key.
    #[must_use]
    pub const fn from_winit(key: NamedKey) -> Option<Hotkey> {
        let n = match key {
            NamedKey::F1 => 1,
            NamedKey::F2 => 2,
            NamedKey::F3 => 3,
            NamedKey::F4 => 4,
            NamedKey::F5 => 5,
            NamedKey::F6 => 6,
            NamedKey::F7 => 7,
            NamedKey::F8 => 8,
            NamedKey::F9 => 9,
            NamedKey::F10 => 10,
            NamedKey::F11 => 11,
            NamedKey::F12 => 12,
            _ => return None,
        };
        Hotkey::from_fkey(n)
    }

    /// A short description for the help table.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Hotkey::Help => "print this help",
            Hotkey::RefreshDebug => "refresh-area debug overlay",
            Hotkey::LayoutBounds => "layout bounds overlay",
            Hotkey::PerfMonitor => "performance monitor",
            Hotkey::SlowMotion => "slow motion x0.25",
            Hotkey::Pause => "pause time",
            Hotkey::Step => "single-step 16 ms (while paused)",
            Hotkey::DumpTree => "dump widget tree to stdout",
            Hotkey::Screenshot => "screenshot to target/twine-sim/shot-<n>.png",
            Hotkey::Record => "start/stop recording to target/twine-sim/rec-<n>/",
            Hotkey::ThemeToggle => "toggle light/dark theme",
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "F{} ({:?})", self.fkey(), self)
    }
}

/// The hotkey table printed by `F1`.
#[must_use]
pub fn help_text() -> String {
    use std::fmt::Write as _;
    let mut s = String::from("twine simulator hotkeys:\n");
    for h in Hotkey::ALL {
        let _ = writeln!(s, "  F{:<3} {}", h.fkey(), h.description());
    }
    s
}

/// Callbacks for hotkeys, registered by the engine runners.
// NOTE(P09.S05): `run_engine` registers F8 (tree dump).
// NOTE(P09.S10): F2/F3/F4 (refresh debug, layout bounds, perf monitor).
// NOTE(P12.S05): F5/F6/F7 (slow motion, pause, step).
// NOTE(P14.S10): F12 (theme toggle).
#[derive(Default)]
pub struct HotkeyRegistry {
    handlers: Vec<(Hotkey, Box<dyn FnMut()>)>,
}

impl fmt::Debug for HotkeyRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.handlers.iter().map(|(h, _)| h))
            .finish()
    }
}

impl HotkeyRegistry {
    /// Registers `f` for `hotkey` (several handlers per hotkey run in registration order).
    pub fn on(&mut self, hotkey: Hotkey, f: Box<dyn FnMut()>) {
        self.handlers.push((hotkey, f));
    }

    /// Runs the handlers of `hotkey`; returns whether there was any.
    pub fn dispatch(&mut self, hotkey: Hotkey) -> bool {
        let mut any = false;
        for (h, f) in &mut self.handlers {
            if *h == hotkey {
                f();
                any = true;
            }
        }
        any
    }
}

/// The first `<dir>/shot-<n>.png` (n = 0, 1, …) for which `exists` is false: screenshots never
/// overwrite each other.
#[must_use]
pub fn screenshot_path(dir: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    first_free(dir, "shot-", ".png", exists)
}

/// The first `<dir>/rec-<n>` for which `exists` is false.
#[must_use]
pub fn recording_dir(dir: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    first_free(dir, "rec-", "", exists)
}

fn first_free(dir: &Path, prefix: &str, suffix: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    (0..=u32::MAX)
        .map(|n| dir.join(format!("{prefix}{n}{suffix}")))
        .find(|p| !exists(p))
        .unwrap_or_else(|| dir.join(format!("{prefix}overflow{suffix}")))
}

/// The file name of frame `k` of a recording.
#[must_use]
pub fn frame_file(k: u32) -> String {
    format!("frame-{k:05}.png")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn f_keys_map_to_hotkeys() {
        for h in Hotkey::ALL {
            assert_eq!(Hotkey::from_fkey(h.fkey()), Some(h));
            assert_eq!(Hotkey::from_name(&format!("F{}", h.fkey())), Some(h));
        }
        assert_eq!(Hotkey::from_fkey(11), None);
        assert_eq!(Hotkey::from_fkey(0), None);
        assert_eq!(Hotkey::from_name("F13"), None);
        assert_eq!(Hotkey::from_name("9"), None);
        assert_eq!(Hotkey::from_winit(NamedKey::F9), Some(Hotkey::Screenshot));
        assert_eq!(Hotkey::from_winit(NamedKey::F11), None);
        assert_eq!(Hotkey::from_winit(NamedKey::Enter), None);
        assert!(help_text().contains("F9"));
    }

    #[test]
    fn screenshot_path_increments() {
        let dir = Path::new("/x");
        let taken = [dir.join("shot-0.png"), dir.join("shot-1.png")];
        assert_eq!(
            screenshot_path(dir, |p| taken.iter().any(|t| t == p)),
            dir.join("shot-2.png")
        );
        assert_eq!(screenshot_path(dir, |_| false), dir.join("shot-0.png"));
        assert_eq!(recording_dir(dir, |p| p == dir.join("rec-0")), dir.join("rec-1"));
        assert_eq!(frame_file(7), "frame-00007.png");
    }

    #[test]
    fn registry_dispatches() {
        let hits = Rc::new(Cell::new(0));
        let mut r = HotkeyRegistry::default();
        let h = hits.clone();
        r.on(Hotkey::DumpTree, Box::new(move || h.set(h.get() + 1)));
        assert!(r.dispatch(Hotkey::DumpTree));
        assert!(!r.dispatch(Hotkey::Pause));
        assert_eq!(hits.get(), 1);
    }
}
