//! `cargo xtask sim thermostat`: the thermostat over its ports. A background thread plays a
//! temperature sensor and writes a reading every 2 s into `SENSOR` (a `Latest` cell: the
//! newest value wins, it never blocks and wakes the UI); the temperature label glides to each
//! value; `-`/`+` set the target. A second thread plays the controller task: it sleeps until
//! the UI sends a command through `CMD` (an `Outbox` whose notify function unparks it) and
//! logs it. Between two readings the UI is idle (0 % CPU).

use std::sync::OnceLock;
use std::thread::Thread;
use std::time::Duration as StdDuration;

use twine::prelude::{Latest, Outbox, Overflow};
use twine_demos::thermostat::{Command, Ports, Reading, app};
use twine_sim::SimConfig;

/// The newest sensor reading (on a device: written by the ADC interrupt or a sensor task).
static SENSOR: Latest<Reading> = Latest::new(Reading { celsius: 20.0 });
/// Commands to the controller; when it falls behind, the oldest command is dropped (only the
/// newest set-point and heater state matter).
static CMD: Outbox<Command, 4> = Outbox::new().on_full(Overflow::DropOldest);
/// The controller thread, unparked by the outbox's notify function.
static CONTROLLER: OnceLock<Thread> = OnceLock::new();

fn main() {
    let controller = std::thread::spawn(|| {
        loop {
            while let Some(cmd) = CMD.try_recv() {
                println!("controller: {cmd:?}");
            }
            std::thread::park(); // until the next command (spurious returns are fine)
        }
    });
    let _ = CONTROLLER.set(controller.thread().clone());
    // On an RTOS: `xTaskNotifyGive(CONTROLLER_TASK)`; with an executor: `CMD.recv().await`.
    CMD.waker().set_notify(|| {
        if let Some(t) = CONTROLLER.get() {
            t.unpark();
        }
    });
    std::thread::spawn(|| {
        let mut t = 20.0f32;
        let mut step = 0.7f32;
        loop {
            std::thread::sleep(StdDuration::from_secs(2));
            t += step;
            if !(17.0..=25.0).contains(&t) {
                step = -step;
            }
            SENSOR.set(Reading { celsius: t });
        }
    });
    let ports = Ports {
        sensor: &SENSOR,
        cmd: &CMD,
    };
    let cfg = SimConfig::new(320, 240)
        .title("Thermostat")
        .scale(2)
        .app_config(twine_demos::config());
    twine_sim::run(cfg, move |cx| app(cx, ports));
}
