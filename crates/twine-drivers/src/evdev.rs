//! Linux input events (`/dev/input/eventN`) as a pointer, feature `evdev` (needs `std`; Unix
//! hosts).
//!
//! | | |
//! |-|-|
//! | Device | `/dev/input/eventN` (read access, e.g. the `input` group); `/proc/bus/input/devices` lists them |
//! | Absolute devices | touchscreens, tablets: `ABS_X`/`ABS_Y` (or `ABS_MT_POSITION_X`/`_Y`), `BTN_TOUCH`/`BTN_LEFT`; device units mapped by a [`Calibration`] (default identity) |
//! | Relative devices | mice: `REL_X`/`REL_Y` move a pointer clamped to the screen, `BTN_LEFT` presses |
//! | Wake-up | [`PollHint::Interrupt`]: a reader thread decodes the events and calls the [`with_notify`](Evdev::with_notify) function after every report |
//!
//! A reader thread blocks on the device and publishes each complete report (`SYN_REPORT`) to
//! the driver; [`read`](InputDevice::read) only takes the latest state (a lock, no system call),
//! so the UI never blocks on input. Wire the notify function to the UI's waker (a `static`
//! [`UiWaker`](https://docs.rs/twine-reactive) given to `UiBuilder::waker`, then `fn wake() {
//! WAKER.wake() }`): the UI then sleeps until input arrives, without polling. When the device
//! goes away (read error or end of file) the driver reports [`DeviceHealth::Failed`].
//!
//! Touchscreen coordinates are device units (`EVIOCGABS` ranges are not read: no `ioctl`, no
//! `unsafe`): map them with a [`Calibration`] — for a device whose range is known, e.g.
//! `0..4096` on both axes of an 800 × 480 screen, `Calibration::from_points` of three corners —
//! or run the calibration demo. Points are clamped to the display the driver is fitted to.
//!
//! The event record layout (`struct input_event`: two `long`s of time, `u16` type, `u16` code,
//! `i32` value) is decoded with the host's `long` size; the module only uses portable Unix file
//! I/O, so it builds and is tested on every Unix host (with a file of records standing in for
//! the device), and is meant for Linux.
//!
//! ```
//! # let path = std::env::temp_dir().join(format!("twine-evdev-doc-{}", std::process::id()));
//! # let mut bytes = Vec::new();
//! # for (t, c, v) in [(3u16, 0u16, 120i32), (3, 1, 45), (1, 0x14a, 1), (0, 0, 0)] {
//! #     bytes.extend_from_slice(&[0u8; 2 * std::mem::size_of::<std::ffi::c_long>()]);
//! #     bytes.extend_from_slice(&t.to_ne_bytes());
//! #     bytes.extend_from_slice(&c.to_ne_bytes());
//! #     bytes.extend_from_slice(&v.to_ne_bytes());
//! # }
//! # std::fs::write(&path, bytes).unwrap();
//! use twine_core::{ColorFormat, Point};
//! use twine_drivers::evdev::Evdev;
//! use twine_hal::{DeviceHealth, DisplayInfo, InputData, InputDevice};
//!
//! // On a device: `Evdev::open("/dev/input/event0")`.
//! let mut touch = Evdev::open(&path).unwrap();
//! touch.fit_to_display(&DisplayInfo::new(320, 240, ColorFormat::Xrgb8888));
//! // The file ends after one report: the reader thread stops and the device fails.
//! while touch.health() != DeviceHealth::Failed {
//!     std::thread::yield_now();
//! }
//! # let _ = touch.read(); // the failed device's sample is released (the engine ignores it)
//! # std::fs::remove_file(&path).unwrap();
//! ```

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use twine_core::Point;
use twine_core::log::{info, warn};
use twine_hal::{
    Calibration, DeviceHealth, DisplayInfo, InputData, InputDevice, InputKind, PointerData, PollHint,
    TouchTransform,
};

/// `EV_SYN`, `EV_KEY`, `EV_REL`, `EV_ABS`.
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const BTN_LEFT: u16 = 0x110;
const BTN_TOUCH: u16 = 0x14a;

/// Bytes of one `struct input_event` on this host (two `long`s of time, then 8 bytes).
const RECORD: usize = 2 * core::mem::size_of::<std::ffi::c_long>() + 8;

/// The input state the reader thread publishes after each `SYN_REPORT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Report {
    /// Last absolute position (device units).
    abs: (i32, i32),
    /// Relative motion not yet taken by the driver.
    rel: (i32, i32),
    /// Whether any absolute position was reported (else the device is relative).
    absolute: bool,
    pressed: bool,
}

/// Decodes events into reports.
#[derive(Debug, Default)]
struct Decoder {
    pending: Report,
}

impl Decoder {
    /// Applies one event; returns the report on `SYN_REPORT`.
    fn push(&mut self, kind: u16, code: u16, value: i32) -> Option<Report> {
        let p = &mut self.pending;
        match (kind, code) {
            (EV_ABS, ABS_X | ABS_MT_POSITION_X) => (p.abs.0, p.absolute) = (value, true),
            (EV_ABS, ABS_Y | ABS_MT_POSITION_Y) => (p.abs.1, p.absolute) = (value, true),
            (EV_REL, REL_X) => p.rel.0 = p.rel.0.saturating_add(value),
            (EV_REL, REL_Y) => p.rel.1 = p.rel.1.saturating_add(value),
            (EV_KEY, BTN_TOUCH | BTN_LEFT) => p.pressed = value != 0,
            (EV_SYN, SYN_REPORT) => {
                let report = *p;
                p.rel = (0, 0);
                return Some(report);
            }
            _ => {}
        }
        None
    }
}

/// The notify function, shared with the reader thread.
type Notify = Arc<Mutex<Option<fn()>>>;

/// What the reader thread shares with the driver.
#[derive(Debug, Default)]
struct Shared {
    report: Report,
    /// The thread ended (device gone or unreadable).
    failed: bool,
}

/// A Linux event device as a pointer [`InputDevice`] (see the [module docs](self)).
#[derive(Debug)]
pub struct Evdev {
    shared: Arc<Mutex<Shared>>,
    notify: Notify,
    cal: Calibration,
    width: u16,
    height: u16,
    /// Calibrated native points → logical screen (from the fit; pass-through until then).
    transform: TouchTransform,
    /// Pointer position of a relative device (screen pixels).
    pos: Point,
    last: PointerData,
    health: DeviceHealth,
}

impl Evdev {
    /// Opens the event device at `path` and starts its reader thread. Identity calibration,
    /// not clamped until fitted to a display, no notify function. Never panics: the thread is
    /// started with `std::thread::Builder::spawn`, so a thread that cannot be created is an
    /// error, not a panic.
    ///
    /// # Errors
    /// The device cannot be opened for reading, or the thread cannot be spawned.
    ///
    /// ```no_run
    /// // `no_run`: needs a Linux input event device and read access to it.
    /// use twine_drivers::evdev::Evdev;
    ///
    /// let touch = Evdev::open("/dev/input/event0")?;
    /// # let _ = touch;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let notify: Notify = Arc::default();
        let (s, n) = (shared.clone(), notify.clone());
        thread::Builder::new()
            .name(std::string::String::from("twine-evdev"))
            .spawn(move || reader(file, &s, &n))?;
        info!(target: "twine::driver", "evdev: reading {}", path.to_string_lossy().as_ref());
        Ok(Self {
            shared,
            notify,
            cal: Calibration::IDENTITY,
            width: u16::MAX,
            height: u16::MAX,
            transform: TouchTransform::PASS_THROUGH,
            pos: Point::new(0, 0),
            last: PointerData::default(),
            health: DeviceHealth::Ok,
        })
    }

    /// Calls `notify` (from the reader thread) after every input report, e.g. a function that
    /// wakes the UI (`fn wake() { WAKER.wake() }` with the `static UiWaker` given to
    /// `UiBuilder::waker`). It must not block. Never panics.
    ///
    /// ```no_run
    /// use twine_drivers::evdev::Evdev;
    ///
    /// fn wake() { /* e.g. WAKER.wake() */ }
    /// let touch = Evdev::open("/dev/input/event0")?.with_notify(wake);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub fn with_notify(self, notify: fn()) -> Self {
        *self.notify.lock().unwrap_or_else(PoisonError::into_inner) = Some(notify);
        self
    }

    /// Maps absolute device units to the display's **native** (unrotated) pixels (default
    /// [`Calibration::IDENTITY`]); the fit to the display then rotates them to the logical
    /// screen, so the calibration holds in every rotation. Not used for relative devices.
    /// Never panics.
    ///
    /// ```no_run
    /// use twine_drivers::evdev::Evdev;
    /// use twine_hal::Calibration;
    ///
    /// // A touchscreen reporting 0..4096 on both axes of an 800 × 480 screen.
    /// let cal = Calibration::from_points([(0, 0), (4096, 0), (0, 4096)], [(0, 0), (800, 0), (0, 480)]).unwrap();
    /// let touch = Evdev::open("/dev/input/event0")?.with_calibration(cal);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub fn with_calibration(mut self, cal: Calibration) -> Self {
        self.cal = cal;
        self
    }

    fn fit(&mut self, info: &DisplayInfo) {
        self.width = info.width;
        self.height = info.height;
        self.transform = TouchTransform::for_display(info);
        // A mouse starts in the middle of the screen.
        self.pos = Point::new(i32::from(info.width) / 2, i32::from(info.height) / 2);
    }

    fn clamp(&self, x: i32, y: i32) -> Point {
        Point::new(
            x.clamp(0, i32::from(self.width.max(1)) - 1),
            y.clamp(0, i32::from(self.height.max(1)) - 1),
        )
    }
}

/// The reader thread: decodes records until the device fails.
fn reader(mut file: File, shared: &Mutex<Shared>, notify: &Mutex<Option<fn()>>) {
    let mut decoder = Decoder::default();
    let mut record = [0u8; RECORD];
    let err = loop {
        if let Err(e) = file.read_exact(&mut record) {
            break e;
        }
        let tail = &record[RECORD - 8..];
        let kind = u16::from_ne_bytes([tail[0], tail[1]]);
        let code = u16::from_ne_bytes([tail[2], tail[3]]);
        let value = i32::from_ne_bytes([tail[4], tail[5], tail[6], tail[7]]);
        if let Some(report) = decoder.push(kind, code, value) {
            {
                let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                let rel = s.report.rel;
                s.report = Report {
                    // Motion the driver has not taken yet accumulates.
                    rel: (
                        rel.0.saturating_add(report.rel.0),
                        rel.1.saturating_add(report.rel.1),
                    ),
                    ..report
                };
            }
            call(notify);
        }
    };
    warn!(target: "twine::driver", "evdev: device lost (os error {:?})", err.raw_os_error());
    shared.lock().unwrap_or_else(PoisonError::into_inner).failed = true;
    call(notify);
}

fn call(notify: &Mutex<Option<fn()>>) {
    let f = *notify.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(f) = f {
        f();
    }
}

impl InputDevice for Evdev {
    fn kind(&self) -> InputKind {
        InputKind::Pointer
    }

    fn read(&mut self) -> InputData {
        let (report, failed) = {
            let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let r = s.report;
            s.report.rel = (0, 0);
            (r, s.failed)
        };
        if failed {
            self.health = DeviceHealth::Failed;
            self.last.pressed = false;
            return InputData::Pointer(self.last);
        }
        let point = if report.absolute {
            // Calibrated to native panel pixels, then rotated to the logical screen.
            let (x, y) = self.cal.apply(report.abs.0, report.abs.1);
            self.transform.apply(x, y)
        } else {
            self.pos = self.clamp(
                self.pos.x.saturating_add(report.rel.0),
                self.pos.y.saturating_add(report.rel.1),
            );
            self.pos
        };
        self.last = PointerData {
            point,
            pressed: report.pressed,
        };
        InputData::Pointer(self.last)
    }

    /// Interrupt-driven: the reader thread calls the notify function.
    fn poll_hint(&self) -> PollHint {
        PollHint::Interrupt
    }

    fn health(&self) -> DeviceHealth {
        if self.health.is_failed() || self.shared.lock().unwrap_or_else(PoisonError::into_inner).failed {
            DeviceHealth::Failed
        } else {
            DeviceHealth::Ok
        }
    }

    /// Clamps points to the display (and centres a mouse pointer on it).
    fn fit_to_display(&mut self, info: &DisplayInfo) {
        self.fit(info);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::vec::Vec;
    use twine_core::ColorFormat;

    fn record(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut r = std::vec![0u8; RECORD - 8];
        r.extend_from_slice(&kind.to_ne_bytes());
        r.extend_from_slice(&code.to_ne_bytes());
        r.extend_from_slice(&value.to_ne_bytes());
        r
    }

    /// A named pipe standing in for the device: a writer thread opens it, sends `events` on
    /// [`send`](Self::send) and closes it (the device goes away) on [`close`](Self::close).
    struct Fifo {
        path: PathBuf,
        step: mpsc::Sender<()>,
        writer: thread::JoinHandle<()>,
    }

    impl Fifo {
        fn new(name: &str, events: &[(u16, u16, i32)]) -> Self {
            let path = std::env::temp_dir().join(std::format!("twine-evdev-{name}-{}", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let made = std::process::Command::new("mkfifo").arg(&path).status().unwrap();
            assert!(made.success(), "mkfifo");
            let bytes: Vec<u8> = events.iter().flat_map(|&(t, c, v)| record(t, c, v)).collect();
            let (step, steps) = mpsc::channel::<()>();
            let p = path.clone();
            let writer = thread::spawn(move || {
                let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
                let _ = steps.recv();
                f.write_all(&bytes).unwrap();
                let _ = steps.recv();
            });
            Self { path, step, writer }
        }

        /// Writes the events.
        fn send(&self) {
            self.step.send(()).unwrap();
        }

        /// Closes the pipe: the driver's reader sees the end of the device.
        fn close(self) {
            let _ = self.step.send(());
            self.writer.join().unwrap();
            std::fs::remove_file(&self.path).unwrap();
        }
    }

    fn wait_for(dev: &Evdev, f: impl Fn(&Shared) -> bool) {
        while !f(&dev.shared.lock().unwrap()) {
            thread::yield_now();
        }
    }

    fn fitted(dev: Evdev) -> Evdev {
        let mut dev = dev;
        dev.fit_to_display(&DisplayInfo::new(320, 240, ColorFormat::Xrgb8888));
        dev
    }

    #[test]
    fn decoder_reports_on_syn_and_resets_motion() {
        let mut d = Decoder::default();
        assert_eq!(d.push(EV_ABS, ABS_MT_POSITION_X, 10), None);
        assert_eq!(d.push(EV_ABS, ABS_Y, 20), None);
        assert_eq!(d.push(EV_KEY, BTN_TOUCH, 1), None);
        assert_eq!(d.push(EV_REL, REL_X, 3), None);
        let r = d.push(EV_SYN, SYN_REPORT, 0).unwrap();
        assert_eq!(
            (r.abs, r.absolute, r.pressed, r.rel),
            ((10, 20), true, true, (3, 0))
        );
        assert_eq!(d.push(EV_SYN, SYN_REPORT, 0).unwrap().rel, (0, 0));
        assert_eq!(d.push(EV_KEY, 0x1c, 1), None, "other keys are ignored");
    }

    #[test]
    fn touchscreen_reports_calibrated_clamped_points_then_fails_when_gone() {
        // Device units 0..4096 → 320 × 240.
        let cal =
            Calibration::from_points([(0, 0), (4096, 0), (0, 4096)], [(0, 0), (320, 0), (0, 240)]).unwrap();
        let fifo = Fifo::new(
            "abs",
            &[
                (EV_ABS, ABS_X, 2048),
                (EV_ABS, ABS_Y, 9000), // beyond the screen: clamped
                (EV_KEY, BTN_TOUCH, 1),
                (EV_SYN, SYN_REPORT, 0),
            ],
        );
        let mut dev = fitted(Evdev::open(&fifo.path).unwrap().with_calibration(cal));
        fifo.send();
        wait_for(&dev, |s| s.report.pressed);
        assert_eq!(
            dev.read(),
            InputData::Pointer(PointerData {
                point: Point::new(160, 239),
                pressed: true
            })
        );
        assert_eq!(dev.health(), DeviceHealth::Ok);
        fifo.close();
        wait_for(&dev, |s| s.failed);
        assert_eq!(dev.health(), DeviceHealth::Failed);
        assert!(matches!(dev.read(), InputData::Pointer(p) if !p.pressed && p.point == Point::new(160, 239)));
    }

    #[test]
    fn mouse_moves_a_clamped_pointer_from_the_centre() {
        let fifo = Fifo::new(
            "rel",
            &[
                (EV_REL, REL_X, 30),
                (EV_REL, REL_Y, -500),
                (EV_KEY, BTN_LEFT, 1),
                (EV_SYN, SYN_REPORT, 0),
            ],
        );
        let mut dev = fitted(Evdev::open(&fifo.path).unwrap());
        fifo.send();
        wait_for(&dev, |s| s.report.pressed);
        assert_eq!(
            dev.read(),
            InputData::Pointer(PointerData {
                point: Point::new(190, 0),
                pressed: true
            })
        );
        // The motion was taken: the pointer stays.
        assert!(matches!(dev.read(), InputData::Pointer(p) if p.point == Point::new(190, 0)));
        assert_eq!(dev.poll_hint(), PollHint::Interrupt);
        fifo.close();
    }

    #[test]
    fn notify_is_called_per_report_and_when_the_device_goes() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CALLS: AtomicU32 = AtomicU32::new(0);
        fn count() {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        let fifo = Fifo::new(
            "notify",
            &[
                (EV_KEY, BTN_LEFT, 1),
                (EV_SYN, SYN_REPORT, 0),
                (EV_KEY, BTN_LEFT, 0),
                (EV_SYN, SYN_REPORT, 0),
            ],
        );
        let dev = Evdev::open(&fifo.path).unwrap().with_notify(count);
        fifo.send();
        while CALLS.load(Ordering::Relaxed) < 2 {
            thread::yield_now(); // both reports
        }
        assert!(!dev.shared.lock().unwrap().failed);
        fifo.close();
        wait_for(&dev, |s| s.failed);
        // `failed` is set before the last notify: wait for it.
        while CALLS.load(Ordering::Relaxed) < 3 {
            thread::yield_now();
        }
        assert_eq!(CALLS.load(Ordering::Relaxed), 3, "two reports and the failure");
    }
}
