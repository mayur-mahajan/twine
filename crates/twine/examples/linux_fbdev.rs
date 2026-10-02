//! `linux_fbdev`: Twine on hosted Linux without a window system — the framebuffer device
//! (`/dev/fbN`) as the display, an input-event device (`/dev/input/eventN`, touchscreen or
//! mouse) as the pointer, and the blocking run loop on a host thread
//! (`twine::run::blocking` over `StdPlatform`).
//!
//! ```text
//! cargo run -p twine --example linux_fbdev -- [fb index] [input device]
//! cargo run -p twine --example linux_fbdev -- 0 /dev/input/event2
//! ```
//!
//! Run it on a Linux console (not inside a desktop session, which owns the framebuffer), as a
//! user in the `video` and `input` groups. `/proc/bus/input/devices` lists the event devices.
//! Nothing polls: the UI thread sleeps until its next deadline or until the input reader
//! thread wakes it (`Evdev::with_notify` → the `static` [`UiWaker`] given to the builder →
//! the platform's notify). The touch driver needs no transform: the `Ui` fits it to the
//! display. A touchscreen whose range is not the screen's size needs a `Calibration`
//! (`Evdev::with_calibration`, e.g. from the calibration demo).
//!
//! Headless (`TWINE_SIM_HEADLESS`, as `cargo xtask sim-smoke` runs every example) there is no
//! device to draw on: the example only reports that and exits.

#[cfg(unix)]
mod hosted {
    use std::process::ExitCode;

    use twine::drivers::evdev::Evdev;
    use twine::drivers::fbdev::FbDev;
    use twine::hal::DisplayDriver;
    use twine::platform::StdPlatform;
    use twine::prelude::*;

    /// The UI's waker: the input thread wakes the UI through it.
    static WAKER: UiWaker = UiWaker::new();

    /// Called by the input reader thread after every input report.
    fn wake() {
        WAKER.wake();
    }

    pub fn main() -> ExitCode {
        if std::env::var_os("TWINE_SIM_HEADLESS").is_some() {
            println!("linux_fbdev: needs a Linux framebuffer and an input device; nothing to do headless");
            return ExitCode::SUCCESS;
        }
        let mut args = std::env::args().skip(1);
        let fb: u8 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
        let input = args.next().unwrap_or_else(|| "/dev/input/event0".into());
        let display = match FbDev::open(fb) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("linux_fbdev: cannot open /dev/fb{fb}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let info = display.info();
        println!(
            "linux_fbdev: /dev/fb{fb} {}x{} {:?}",
            info.width, info.height, info.format
        );
        let mut platform = StdPlatform::new();
        let builder = Ui::builder(display)
            .runtime(Runtime::take().expect("runtime already taken"))
            .platform(&platform)
            .waker(&WAKER)
            .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 40 }))
            .theme(DefaultTheme::light());
        let builder = match Evdev::open(&input) {
            Ok(pointer) => builder.input(pointer.with_notify(wake)),
            Err(e) => {
                eprintln!("linux_fbdev: no input from {input}: {e}");
                builder
            }
        };
        let ui = builder.build(twine_demos::counter::app);
        twine::run::blocking(ui, &mut platform)
    }
}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    hosted::main()
}

#[cfg(not(unix))]
fn main() {
    println!("linux_fbdev: runs on Linux (framebuffer and input-event devices)");
}
