//! `rtos_notify`: the RTOS run-loop pattern, simulated on host threads (no window).
//!
//! An RTOS port of Twine is a [`Platform`] over the kernel's task notification: the UI task
//! sleeps in `wait(deadline)` (`FreeRTOS`: `ulTaskNotifyTake(pdTRUE, ticks)`, Zephyr
//! `k_sem_take`), and `notify()` — installed on the UI's waker by `twine::run::blocking` —
//! gives the notification (`vTaskNotifyGiveFromISR` / `xTaskNotifyGive`, `k_sem_give`). Other
//! tasks and interrupt handlers send data through a `Channel`; each send wakes the UI task,
//! which redraws and sleeps again. Nothing polls: between two readings the UI task blocks.
//!
//! Here a "kernel" made of a mutex and a condition variable stands in for the RTOS, the main
//! thread is the UI task and a second thread is a sensor task that sends a reading every
//! 200 ms. The UI renders into an in-memory display; the console shows the run loop's events.
//!
//! ```text
//! cargo xtask sim rtos_notify
//! RUST_LOG=twine=debug cargo xtask sim rtos_notify   # also log every update
//! ```

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration as StdDuration;

use twine::core::ColorFormat;
use twine::hal::{Clock, DisplayInfo, Platform};
use twine::prelude::*;
use twine::run::{self, LoopEvent};
use twine_testing::MemoryDisplay;

/// The simulated kernel: one binary "task notification" for the UI task, and the tick count.
mod kernel {
    use super::{Condvar, Mutex, OnceLock, StdDuration};

    /// The UI task's notification value (`true`: given, not yet taken).
    static NOTIFIED: Mutex<bool> = Mutex::new(false);
    static GIVEN: Condvar = Condvar::new();
    /// Time zero of the tick count.
    static BOOT: OnceLock<std::time::Instant> = OnceLock::new();

    /// `xTaskGetTickCount()` at 1 kHz.
    pub fn tick_count() -> u64 {
        u64::try_from(BOOT.get_or_init(std::time::Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// `ulTaskNotifyTake(pdTRUE, ticks)`: takes the notification, blocking up to `ticks`
    /// (`None`: forever). A notification given before the call is kept, so none is lost.
    pub fn notify_take(ticks: Option<u64>) {
        let mut given = NOTIFIED.lock().unwrap();
        let deadline = ticks.map(|t| tick_count() + t);
        while !*given {
            given = match deadline {
                None => GIVEN.wait(given).unwrap(),
                Some(d) => {
                    let now = tick_count();
                    if now >= d {
                        break;
                    }
                    GIVEN
                        .wait_timeout(given, StdDuration::from_millis(d - now))
                        .unwrap()
                        .0
                }
            };
        }
        *given = false;
    }

    /// `xTaskNotifyGive(ui_task)` (`vTaskNotifyGiveFromISR` in an interrupt handler).
    pub fn notify_give() {
        *NOTIFIED.lock().unwrap() = true;
        GIVEN.notify_one();
    }
}

/// The UI task's platform: the kernel's tick count is the clock, its task notification the
/// sleep and the wake-up.
#[derive(Clone, Copy)]
struct Rtos;

impl Clock for Rtos {
    fn now(&self) -> Instant {
        Instant::from_millis(kernel::tick_count())
    }
}

impl Platform for Rtos {
    fn wait(&mut self, deadline: Option<Instant>) {
        let ticks = deadline.map(|t| t.saturating_duration_since(self.now()).as_millis());
        if ticks == Some(0) {
            return;
        }
        kernel::notify_take(ticks);
    }

    fn yield_now(&mut self) {
        // `taskYIELD()`: the host scheduler stands in.
        std::thread::yield_now();
    }

    fn notify() {
        kernel::notify_give();
    }
}

/// Readings from the sensor task (any task or interrupt handler may send).
static READINGS: Channel<i32, 4> = Channel::new();

/// Readings the UI shows before the example ends.
const READING_COUNT: i32 = 5;

/// The last reading the UI received.
static LAST: AtomicI32 = AtomicI32::new(0);

fn app(cx: Scope) -> impl View {
    let temp = cx.signal(None::<i32>);
    cx.on_message(&READINGS, move |t| {
        LAST.store(t, Ordering::Relaxed);
        temp.set(Some(t));
    });
    column((
        label("Sensor task -> UI task"),
        label(move || match temp.get() {
            Some(t) => format!("{t} °C"),
            None => "waiting...".into(),
        })
        .test_id("reading"),
    ))
    .gap(4)
    .padding(8)
}

fn main() {
    let _ = env_logger::try_init();
    // The sensor task: a reading every 200 ms; each send wakes the UI task.
    std::thread::spawn(|| {
        for i in 1..=READING_COUNT {
            std::thread::sleep(StdDuration::from_millis(200));
            println!("[sensor] send {}", 20 + i);
            let _ = READINGS.try_send(20 + i);
        }
    });

    // The UI task.
    let mut platform = Rtos;
    let display = MemoryDisplay::new(DisplayInfo::new(160, 64, ColorFormat::Rgb565));
    let ui = Ui::builder(display)
        .runtime(Runtime::take().expect("runtime already taken"))
        .platform(&platform)
        .buffers(BufferMode::alloc(BufferSpec::PartialSingle { rows: 16 }))
        .theme(DefaultTheme::light())
        .build(app);
    let mut wakes = 0;
    run::blocking_with(ui, &mut platform, |ui, event| match event {
        LoopEvent::Idle(deadline) => {
            let frames = ui.engine().last_stats(ui.display()).frame;
            println!(
                "[ui] {} ms: frame {frames} drawn, sleeping until {deadline:?}",
                ui.now().as_millis()
            );
            if LAST.load(Ordering::Relaxed) == 20 + READING_COUNT {
                println!("[ui] all {READING_COUNT} readings shown after {wakes} wake-ups; exiting");
                std::process::exit(0);
            }
        }
        LoopEvent::Woken => {
            wakes += 1;
            println!("[ui] {} ms: woken", ui.now().as_millis());
        }
        _ => {}
    })
}
