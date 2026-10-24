//! `cargo xtask sim partitions`: partitioned runtimes (feature `partitions`, ADR-001).
//!
//! Two panels in one simulator window ([`twine_sim::run_panels`]), each its own `Ui` on its own
//! engine and emulated display, in its own **partition** (its own reactive runtime, limits,
//! faults, step budget and heartbeat) — the way a product drives a safety-critical display
//! next to a non-critical one:
//!
//! - **left, partition 0 (critical)**: a speed from a sensor `Latest` in a safety-critical
//!   label, its heartbeat and its fault count;
//! - **right, partition 1 (non-critical)**: a media screen. Click it to give it the keyboard,
//!   then inject faults into it:
//!   - `1`: an effect loop (cut by partition 1's flush limit),
//!   - `2`: a flood of effects beyond its `Limits::max_pending_effects`,
//!   - `3`: slow updates (a binding doing 30 ms of work against a 1 ms step budget),
//!   - `4`: the fail-safe screen,
//!   - `5`: a stall (partition 1 is no longer updated; `5` again resumes it).
//!
//! Whatever happens on the right, the left panel keeps its speed current, its heartbeat
//! counting and its fault count at 0; partition 1's faults are logged. Data reaches the
//! critical UI only through `Latest` cells.

use std::cell::Cell;
use std::rc::Rc;

use twine::core::Partition;
use twine::hal::Key;
use twine::reactive::{Latest, Runtime};
use twine::view::prelude::*;
use twine_sim::{PanelCx, SimConfig, SimPanel, run_panels};

/// One panel.
const W: u16 = 160;
const H: u16 = 120;

/// The sensor value shown by the critical display.
static SPEED: Latest<u16> = Latest::new(0);
/// What the non-critical display shows about itself.
static B_STATUS: Latest<&'static str> = Latest::new("running");
/// The critical display's own status line (heartbeat, fault kinds seen).
static A_HEALTH: Latest<(u32, u32)> = Latest::new((0, 0));

thread_local! {
    /// Simulated work of the non-critical binding per evaluation (µs, busy-waited).
    static WORK_US: Cell<u64> = const { Cell::new(0) };
}

/// The signals the keys drive in the non-critical display: the effect loop's kick and the
/// slow binding's tick (handed out of the view once it is built).
type Controls = Rc<Cell<Option<(Signal<u32>, Signal<u32>)>>>;

/// The critical display: speed, heartbeat and fault count.
fn critical(cx: Scope) -> impl View {
    let speed = cx.watch(&SPEED);
    let health = cx.watch(&A_HEALTH);
    column((
        label("critical: partition 0"),
        label(text!("{} km/h", speed.get())).safety_critical(true),
        label(text!("heartbeat {}", health.get().0)),
        label(text!("faults {}", health.get().1)),
    ))
    .size(160, 120)
    .gap(4)
    .padding(6)
}

/// The non-critical display.
fn media(cx: Scope, controls: &Controls) -> impl View + use<> {
    let status = cx.watch(&B_STATUS);
    let loop_kick = cx.signal(0u32);
    let tick = cx.signal(0u32);
    controls.set(Some((loop_kick, tick)));
    // Fault 1: an effect that feeds itself once kicked.
    cx.effect(move || {
        if loop_kick.get() > 0 {
            loop_kick.set(loop_kick.get_untracked() + 1);
        }
    });
    column((
        label("media: partition 1"),
        label(text!("track {}", {
            let us = WORK_US.with(Cell::get);
            if us > 0 {
                let until = std::time::Instant::now() + std::time::Duration::from_micros(us);
                while std::time::Instant::now() < until {}
            }
            tick.get()
        })),
        label(text!("{}", status.get())),
        label("keys 1-5 inject faults"),
    ))
    .size(160, 120)
    .gap(4)
    .padding(6)
    .bg(Color::hex(0x001A_237E))
}

/// The host side of the critical panel: the "sensor" and the health line.
fn critical_frame(cx: &mut PanelCx<'_>) {
    // A speed sweeping 0..=180 km/h, one step per 16 ms.
    let step = cx.now().as_millis() / 16 % 181;
    SPEED.set(u16::try_from(step).unwrap_or(0));
    let engine = cx.engine();
    let faults = u32::try_from(engine.fault_counts().kinds().len()).unwrap_or(u32::MAX);
    A_HEALTH.set((engine.heartbeat(), faults));
}

/// The host side of the non-critical panel: fault injection on keys, its faults logged.
fn media_frame(cx: &mut PanelCx<'_>, controls: &Controls, armed: &mut bool) {
    let Some((loop_kick, tick)) = controls.get() else {
        return;
    };
    if !*armed {
        // The static fail-safe screen, registered once at start-up.
        let (ui, engine) = cx.ui();
        let display = ui.display();
        if let Err(e) = ui.set_fail_safe(engine, display, |_| {
            container(label("SERVICE MODE")).size(160, 120).bg(Color::RED)
        }) {
            twine::core::error!(target: "twine::example", "fail-safe screen: {}", e);
        }
        ui.runtime().set_flush_iterations_limit(8);
        *armed = true;
    }
    let keys = cx.keys().to_vec();
    for key in keys {
        match key {
            Key::Char('1') => {
                loop_kick.set(1);
                B_STATUS.set("effect loop");
            }
            Key::Char('2') => {
                let scope = cx.ui().0.root_scope();
                for _ in 0..300 {
                    scope.effect(move || {
                        tick.get();
                    });
                }
                tick.update(|t| *t += 1);
                B_STATUS.set("effect flood");
            }
            Key::Char('3') => {
                WORK_US.with(|w| w.set(30_000));
                B_STATUS.set("slow updates");
            }
            Key::Char('4') => {
                let (ui, engine) = cx.ui();
                let display = ui.display();
                if let Err(e) = engine.enter_fail_safe(display, 4) {
                    twine::core::error!(target: "twine::example", "fail-safe: {}", e);
                }
            }
            Key::Char('5') => {
                let running = cx.is_running();
                cx.set_running(!running);
                B_STATUS.set(if running { "stalled" } else { "running" });
            }
            _ => {}
        }
    }
    if WORK_US.with(Cell::get) > 0 && cx.is_running() {
        tick.update(|t| *t += 1); // keep the slow binding busy
    }
    let (ui, engine) = cx.ui();
    for kind in ui.take_faults(engine).iter() {
        twine::core::warn!(target: "twine::example", "partition 1: {}", kind.name());
    }
}

fn main() {
    // One token per partition; each is claimed once (`Runtime::take` = partition 0).
    let rt_a = Runtime::take().expect("partition 0");
    let p1 = Partition::new(1).expect("feature `partitions`");
    let rt_b = Runtime::take_partition(p1).expect("partition 1");
    let config = |engine: EngineConfig, limits: Limits| {
        let mut c = AppConfig::new()
            .theme(&lvgl::THEME)
            .mode(ThemeMode::Dark)
            .limits(limits);
        c.engine = engine;
        c
    };
    let critical_panel = SimPanel::new(W, H, rt_a, critical)
        .title("critical")
        .app_config(config(EngineConfig::default(), Limits::DEFAULT))
        .on_frame(critical_frame);
    let controls: Controls = Rc::default();
    let c = controls.clone();
    let mut armed = false;
    // Partition 1's own bounds and budget: a small effect queue (key 2 floods it) and 1 ms per
    // update (key 3 overruns it).
    let budget = EngineConfig {
        step_budget: StepBudget::time(Duration::ms(1)),
        ..EngineConfig::default()
    };
    let media_panel = SimPanel::new(W, H, rt_b, move |cx| media(cx, &c))
        .title("media")
        .app_config(config(budget, Limits::DEFAULT.max_pending_effects(64)))
        .on_frame(move |cx| media_frame(cx, &controls, &mut armed));
    run_panels(
        SimConfig::new(2 * W, H).title("Partitions").scale(3),
        vec![critical_panel, media_panel],
    );
}
