//! `cargo xtask sim thermostat`: the message-driven thermostat. A background thread plays a
//! temperature sensor and sends a reading every 2 s through `SENSOR` (a lock-free channel
//! that wakes the UI); the temperature label glides to each value; `-`/`+` set the target.
//! Between two readings the UI is idle (0 % CPU).

use std::time::Duration as StdDuration;

use twine_demos::thermostat::{SENSOR, SensorMsg, app};
use twine_sim::SimConfig;

fn main() {
    std::thread::spawn(|| {
        let mut t = 20.0f32;
        let mut step = 0.7f32;
        loop {
            std::thread::sleep(StdDuration::from_secs(2));
            t += step;
            if !(17.0..=25.0).contains(&t) {
                step = -step;
            }
            // Never blocks; a full queue drops the reading (counted and logged by the UI).
            let _ = SENSOR.try_send(SensorMsg { celsius: t });
        }
    });
    twine_sim::run(SimConfig::new(320, 240).title("Thermostat").scale(2), app);
}
