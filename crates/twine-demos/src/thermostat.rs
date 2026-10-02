//! The thermostat: the measured temperature arrives from a sensor task or interrupt through a
//! [`Latest`] cell (the newest reading wins, nothing queues up); the label glides to each new
//! value; `−`/`+` set the target; the state line follows a memo (it changes only when heating
//! starts or stops). Set-point and heater commands leave through an [`Outbox`] to the
//! controller task.
//!
//! **Ports.** The application names no global: it receives its cross-context objects in a
//! [`Ports`] struct. The firmware or simulator declares them (as `static`s, e.g. next to the
//! interrupt handler that writes the sensor) and passes references; a test passes fresh ones
//! (`TestUi::latest`, `TestUi::outbox`), so tests running in parallel never share them.
//!
//! Between two sensor readings the UI does no work at all.

use twine::prelude::*;

/// A sensor reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The measured temperature.
    pub celsius: f32,
}

/// What the UI tells the controller task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The target temperature changed (tenths of a degree); also sent once at start-up.
    SetTarget(i32),
    /// Heating should start (`true`) or stop (`false`); sent at start-up and on every change.
    Heat(bool),
}

/// The thermostat's connections to the rest of the system, passed to [`app`] by whoever owns
/// them (`main`, the simulator, a test).
///
/// ```
/// use twine::prelude::*;
/// use twine_demos::thermostat::{Command, Ports, Reading};
///
/// // In the firmware: written by the ADC interrupt / read by the controller task.
/// static SENSOR: Latest<Reading> = Latest::new(Reading { celsius: 20.0 });
/// static CMD: Outbox<Command, 4> = Outbox::new().on_full(Overflow::DropOldest);
///
/// let ports = Ports { sensor: &SENSOR, cmd: &CMD };
/// // Ui::builder(display)...build(move |cx| app(cx, ports));
/// # let _ = ports;
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Ports {
    /// The newest reading (set from a sensor task or interrupt: `SENSOR.set(..)`, never
    /// blocks, wakes the UI).
    pub sensor: &'static Latest<Reading>,
    /// Commands to the controller task (`try_recv`, or `recv().await` with the `async`
    /// feature).
    pub cmd: &'static Outbox<Command, 4>,
}

/// The thermostat application over its [`Ports`].
///
/// ```
/// use twine_demos::thermostat::{Command, Ports, Reading, app};
/// use twine_testing::{TestUi, by_id, by_text};
///
/// let ports = Ports {
///     sensor: TestUi::latest(Reading { celsius: 20.0 }),
///     cmd: TestUi::outbox(),
/// };
/// let mut t = TestUi::new(320, 240).mount(move |cx| app(cx, ports));
/// ports.sensor.set(Reading { celsius: 23.5 });
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("current")).text(), "23.5 °C");
/// assert_eq!(t.find(by_id("state")).text(), "Idle");
/// assert_eq!(ports.cmd.try_recv(), Some(Command::SetTarget(215)));
/// assert_eq!(ports.cmd.try_recv(), Some(Command::Heat(true))); // 20.0 < 21.5 at start-up
/// assert_eq!(ports.cmd.try_recv(), Some(Command::Heat(false))); // 23.5 > 21.5
/// ```
#[allow(clippy::cast_precision_loss)] // tenths of a degree: far below 2^23
#[allow(clippy::cast_possible_truncation)] // tenths of a degree of a plausible temperature
pub fn app(cx: Scope, ports: Ports) -> impl View {
    let reading = cx.watch(ports.sensor);
    let current = cx.memo(move || (reading.get().celsius * 10.0) as i32); // tenths of a degree
    let target = cx.signal(215i32);
    let cmd = ports.cmd;
    // The controller learns every set-point and heating change; a full outbox is the
    // application's policy (here the `static`'s `Overflow`), never a blocked UI.
    cx.effect(move || {
        let _ = cmd.try_send(Command::SetTarget(target.get()));
    });
    let shown = cx.tween(move || current.get(), AnimSpec::new(Duration::ms(400)).ease_out());
    let heating = cx.memo(move || current.get() < target.get());
    cx.effect(move || {
        let _ = cmd.try_send(Command::Heat(heating.get()));
    });

    column((
        label(text!("{:.1} °C", shown.get() as f32 / 10.0))
            .font(&fonts::MONTSERRAT_20)
            .test_id("current"),
        row((
            button(label("-")).on_click(move || target.update(|t| *t -= 5)),
            label(text!("Target {:.1} °C", target.get() as f32 / 10.0)).test_id("target"),
            button(label("+")).on_click(move || target.update(|t| *t += 5)),
        ))
        .gap(8)
        .align_items(CrossAlign::Center),
        label(text!("{}", if heating.get() { "Heating" } else { "Idle" }))
            // Design elements: the colors follow the theme and its mode.
            .text_color(move || {
                if heating.get() {
                    design::DANGER
                } else {
                    design::ON_SURFACE_MUTED
                }
            })
            .test_id("state"),
    ))
    .gap(12)
    .padding(16)
    .align_items(CrossAlign::Center)
    .fill()
}
