//! The thermostat over its ports (R3.S05): the test owns the sensor cell and the command
//! outbox, so tests running in parallel never share them.

use twine_demos::thermostat::{Command, Ports, Reading, app};
use twine_testing::{TestUi, by_id, by_text};

/// Fresh ports for one test.
fn ports() -> Ports {
    Ports {
        sensor: TestUi::latest(Reading { celsius: 20.0 }),
        cmd: TestUi::outbox(),
    }
}

/// Every queued command.
fn drain(p: Ports) -> Vec<Command> {
    std::iter::from_fn(|| p.cmd.try_recv()).collect()
}

#[test]
fn thermostat_reads_its_sensor_port_and_sends_commands() {
    let p = ports();
    let mut t = TestUi::new(320, 240)
        .app_config(twine_demos::config())
        .mount(move |cx| app(cx, p));
    t.run_until_idle();
    assert_eq!(t.find(by_id("current")).text(), "20.0 °C");
    assert_eq!(t.find(by_id("state")).text(), "Heating");
    assert_eq!(
        drain(p),
        [Command::SetTarget(215), Command::Heat(true)],
        "start-up state"
    );

    // Readings faster than the UI: only the newest one is shown, nothing is dropped.
    for c in [21.0, 21.8, 22.5] {
        p.sensor.set(Reading { celsius: c });
    }
    t.run_until_idle();
    assert_eq!(t.find(by_id("current")).text(), "22.5 °C");
    assert_eq!(t.find(by_id("state")).text(), "Idle");
    assert_eq!(drain(p), [Command::Heat(false)]);

    t.find(by_text("+")).click();
    t.find(by_text("+")).click();
    t.run_until_idle();
    assert_eq!(t.find(by_id("target")).text(), "Target 22.5 °C");
    assert_eq!(drain(p), [Command::SetTarget(220), Command::SetTarget(225)]);
    t.assert_idle();
}

#[test]
fn thermostats_on_parallel_threads_with_their_own_ports_do_not_interfere() {
    // What the test harness does with parallel tests: each thread has its own runtime and UI,
    // and with per-test ports nothing is shared (with a global `SENSOR`, one test's reading
    // reached the other's UI).
    let run = |celsius: f32| {
        std::thread::spawn(move || {
            let p = ports();
            let mut t = TestUi::new(320, 240)
                .app_config(twine_demos::config())
                .mount(move |cx| app(cx, p));
            for _ in 0..20 {
                p.sensor.set(Reading { celsius });
                t.run_until_idle();
                assert_eq!(t.find(by_id("current")).text(), format!("{celsius:.1} °C"));
            }
            drain(p)
        })
    };
    let (a, b) = (run(18.0), run(24.0));
    assert_eq!(a.join().unwrap(), [Command::SetTarget(215), Command::Heat(true)]);
    assert_eq!(
        b.join().unwrap(),
        [Command::SetTarget(215), Command::Heat(true), Command::Heat(false)]
    );
}
